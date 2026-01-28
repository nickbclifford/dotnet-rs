use crate::{
    vm_expect_stack, vm_pop, vm_push, vm_trace, vm_trace_branch, vm_trace_field,
    vm_trace_instruction,
};
use dotnet_assemblies::decompose_type_source;
use dotnet_types::{
    generics::GenericLookup,
    members::{FieldDescription, MethodDescription},
    TypeDescription,
};
use dotnet_utils::{gc::GCHandle, is_ptr_aligned_to_field};
use dotnet_value::{
    layout::HasLayout,
    object::{CTSValue, HeapStorage, ManagedPtrMetadata, ObjectRef},
    pointer::{ManagedPtr, ManagedPtrOwner, UnmanagedPtr},
    string::CLRString,
    StackValue,
};
use dotnetdll::prelude::*;
use std::{cmp::Ordering as CmpOrdering, ptr, slice};

use super::{
    exceptions::{ExceptionState, HandlerAddress, UnwindTarget},
    intrinsics::*,
    layout::{type_layout, LayoutFactory},
    resolution::{TypeResolutionExt, ValueResolution},
    sync::{AtomicU16, AtomicU32, AtomicU64, AtomicU8, Ordering as AtomicOrdering},
    threading::ThreadManagerOps,
    CallStack, MethodInfo, ResolutionContext, StepResult,
};

/// Check if a method is an intrinsic (with caching).
/// This is called on every method invocation, so caching is critical for performance.
pub fn is_intrinsic_cached(stack: &CallStack, method: MethodDescription) -> bool {
    // Check cache first
    if let Some(cached) = stack.shared.caches.intrinsic_cache.get(&method) {
        stack.shared.metrics.record_intrinsic_cache_hit();
        return *cached;
    }

    // Cache miss - perform check
    stack.shared.metrics.record_intrinsic_cache_miss();
    let result = is_intrinsic(
        method,
        stack.shared.loader,
        &stack.shared.caches.intrinsic_registry,
    );

    // Cache the result
    stack.shared.caches.intrinsic_cache.insert(method, result);
    result
}

/// Check if a field is an intrinsic (with caching).
pub fn is_intrinsic_field_cached(stack: &CallStack, field: FieldDescription) -> bool {
    // Check cache first
    if let Some(cached) = stack.shared.caches.intrinsic_field_cache.get(&field) {
        stack.shared.metrics.record_intrinsic_field_cache_hit();
        return *cached;
    }

    // Cache miss - perform check
    stack.shared.metrics.record_intrinsic_field_cache_miss();
    let result = is_intrinsic_field(
        field,
        stack.shared.loader,
        &stack.shared.caches.intrinsic_registry,
    );

    // Cache the result
    stack
        .shared
        .caches
        .intrinsic_field_cache
        .insert(field, result);
    result
}

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    /// Check for GC safe point and suspend if stop-the-world is requested.
    ///
    /// This method should be called:
    /// - Before long-running operations (large allocations, reflection, etc.)
    /// - Periodically during loops with many iterations
    /// - At method entry/exit points (already happens via executor loop)
    ///
    /// Note: The current implementation checks on every instruction via the executor loop.
    /// For better performance, future optimization could check only on backward branches
    /// (detected by IP decreasing), method entries, and explicit calls in long operations.
    #[inline]
    pub fn check_gc_safe_point(&self) {
        let thread_manager = &self.shared.thread_manager;
        if thread_manager.is_gc_stop_requested() {
            let managed_id = self.thread_id.get();
            if managed_id != 0 {
                #[cfg(feature = "multithreaded-gc")]
                thread_manager.safe_point(managed_id, &self.shared.gc_coordinator);
                #[cfg(not(feature = "multithreaded-gc"))]
                thread_manager.safe_point(managed_id, &Default::default());
            }
        }
    }

    fn find_generic_method(&self, source: &MethodSource) -> (MethodDescription, GenericLookup) {
        let ctx = self.current_context();

        let mut new_lookup = ctx.generics.clone();

        let method = match source {
            MethodSource::User(u) => *u,
            MethodSource::Generic(g) => {
                new_lookup.method_generics =
                    g.parameters.iter().map(|t| ctx.make_concrete(t)).collect();
                g.base
            }
        };

        if let UserMethod::Reference(r) = method {
            if let MethodReferenceParent::Type(t) = &ctx.resolution[r].parent {
                let parent = ctx.make_concrete(t);
                if let BaseType::Type {
                    source: TypeSource::Generic { parameters, .. },
                    ..
                } = parent.get()
                {
                    new_lookup.type_generics = parameters.clone();
                }
            }
        }

        (ctx.locate_method(method, ctx.generics), new_lookup)
    }

    /// Dispatches a method call through the appropriate mechanism.
    ///
    /// Call priority order:
    /// 1. Intrinsics (including DirectIntercept) - VM implementation always wins
    /// 2. P/Invoke - External native methods
    /// 3. Managed - CIL bytecode execution
    ///
    /// Note: DirectIntercept intrinsics are checked first, ensuring VM
    /// implementation is used even when BCL implementation exists.
    fn dispatch_method(
        &mut self,
        gc: GCHandle<'gc>,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult {
        // Check intrinsics first - this includes DirectIntercept methods
        // that MUST bypass any BCL implementation
        if is_intrinsic_cached(self, method) {
            intrinsic_call(gc, self, method, &lookup)
        } else if method.method.pinvoke.is_some() {
            self.external_call(method, gc);
            StepResult::InstructionStepped
        } else {
            if method.method.internal_call {
                panic!("intrinsic not found: {:?}", method);
            }
            self.call_frame(
                gc,
                MethodInfo::new(method, &lookup, self.shared.clone()),
                lookup,
            );
            StepResult::InstructionStepped
        }
    }

    /// Unified dispatch pipeline for method calls.
    ///
    /// This function provides a single, predictable entry point for all method
    /// dispatch operations.
    ///
    /// Pipeline stages:
    /// 1. Generic method resolution
    /// 2. Virtual dispatch (if needed) - integrates with VirtualOverride intrinsics
    /// 3. Final dispatch - intrinsics (DirectIntercept included) → P/Invoke → managed
    ///
    /// Parameters:
    /// - `source`: The method source from the instruction
    /// - `this_type`: Optional runtime type for virtual dispatch
    /// - `ctx`: Optional resolution context (for constrained calls)
    ///
    /// Returns: StepResult indicating the outcome of the dispatch
    fn unified_dispatch(
        &mut self,
        gc: GCHandle<'gc>,
        source: &MethodSource,
        this_type: Option<TypeDescription>,
        ctx: Option<&ResolutionContext<'gc, 'm>>,
    ) -> StepResult {
        // Stage 1: Resolve generic method from instruction operand
        let (base_method, lookup) = self.find_generic_method(source);

        // Stage 2: Virtual dispatch if needed
        let method = if let Some(runtime_type) = this_type {
            // Virtual call: resolve based on runtime type
            // resolve_virtual_method checks for VirtualOverride intrinsics
            self.resolve_virtual_method(base_method, runtime_type, ctx)
        } else {
            // Static call: use method as-is
            base_method
        };

        // Stage 3: Final dispatch through the standard pipeline
        // DirectIntercept intrinsics are prioritized here
        self.dispatch_method(gc, method, lookup)
    }

    pub fn step(&mut self, gc: GCHandle<'gc>) -> StepResult {
        if matches!(
            self.execution.exception_mode,
            ExceptionState::Throwing(_)
                | ExceptionState::Searching { .. }
                | ExceptionState::Unwinding { .. }
        ) {
            return self.handle_exception(gc);
        }

        use Instruction::*;

        macro_rules! statics {
            (|$s:ident| $body:expr) => {{
                let $s = self.statics();
                $body
            }};
        }

        macro_rules! push {
            ($($args:tt)*) => {
                vm_push!(self, gc, $($args)*)
            };
        }
        macro_rules! pop {
            () => {
                vm_pop!(self, gc)
            };
        }

        macro_rules! state {
            (|$state:ident| $body:expr) => {{
                let frame = self.current_frame_mut();
                let $state = &mut frame.state;
                $body
            }};
        }

        let initial_frame_count = self.execution.frames.len();
        let mut moved_ip = false;

        macro_rules! branch {
            ($ip:expr) => {{
                let target = *$ip;
                vm_trace_branch!(self, "BR", target as usize, true);
                state!(|s| s.ip = target);
                moved_ip = true;
            }};
        }
        macro_rules! conditional_branch {
            ($condition:expr, $ip:expr) => {{
                let cond = $condition;
                let target = *$ip;
                vm_trace_branch!(self, "BR_COND", target as usize, cond);
                if cond {
                    state!(|s| s.ip = target);
                    moved_ip = true;
                }
            }};
        }
        macro_rules! equal {
            () => {{
                let value2 = pop!();
                let value1 = pop!();
                value1 == value2
            }};
        }

        macro_rules! compare {
            ($sgn:expr, $op:tt ( $order:pat )) => {{
                let value2 = pop!();
                let value1 = pop!();
                matches!(value1.compare(&value2, *$sgn), Some($order))
            }};
        }

        macro_rules! check_special_fields {
            ($field:ident, $is_address:expr) => {
                if $field.field.literal {
                    todo!(
                        "field {}::{} has literal",
                        $field.parent.type_name(),
                        $field.field.name
                    );
                }
                if let Some(c) = &$field.field.default {
                    todo!(
                        "field {}::{} has constant {:?}",
                        $field.parent.type_name(),
                        $field.field.name,
                        c
                    );
                }
                if is_intrinsic_field_cached(self, $field) {
                    let result = intrinsic_field(
                        gc,
                        self,
                        $field,
                        self.current_context().generics.type_generics.clone(),
                        $is_address,
                    );
                    if result == StepResult::InstructionStepped {
                        self.increment_ip();
                    }
                    return result;
                }
            };
        }

        macro_rules! is_nullish {
            ($val:expr) => {
                match $val {
                    StackValue::Int32(i) => i == 0,
                    StackValue::Int64(i) => i == 0,
                    StackValue::NativeInt(i) => i == 0,
                    StackValue::ObjectRef(ObjectRef(o)) => o.is_none(),
                    StackValue::UnmanagedPtr(_) | StackValue::ManagedPtr(_) => false,
                    v => panic!("invalid type on stack ({:?}) for truthiness check", v),
                }
            };
        }

        let i = state!(|s| &s.info_handle.instructions[s.ip]);
        let ip = state!(|s| s.ip);
        let i_res = state!(|s| s.info_handle.source.resolution());

        vm_trace_instruction!(self, ip, &i.show(i_res.definition()));

        match i {
            Add => {
                let v2 = pop!();
                let v1 = pop!();
                push!(v1 + v2);
            }
            AddOverflow(sgn) => {
                let v2 = pop!();
                let v1 = pop!();
                match v1.checked_add(v2, *sgn) {
                    Ok(v) => push!(v),
                    Err(e) => return self.throw_by_name(gc, e),
                }
            }
            And => {
                let v2 = pop!();
                let v1 = pop!();
                push!(v1 & v2);
            }
            ArgumentList => todo!("arglist"),
            BranchEqual(i) => {
                conditional_branch!(equal!(), i)
            }
            BranchGreaterOrEqual(sgn, i) => {
                conditional_branch!(
                    compare!(sgn, >= (CmpOrdering::Greater | CmpOrdering::Equal)),
                    i
                )
            }
            BranchGreater(sgn, i) => {
                conditional_branch!(compare!(sgn, > (CmpOrdering::Greater)), i)
            }
            BranchLessOrEqual(sgn, i) => {
                conditional_branch!(
                    compare!(sgn, <= (CmpOrdering::Less | CmpOrdering::Equal)),
                    i
                )
            }
            BranchLess(sgn, i) => conditional_branch!(compare!(sgn, < (CmpOrdering::Less)), i),
            BranchNotEqual(i) => {
                conditional_branch!(!equal!(), i)
            }
            Branch(i) => branch!(i),
            Breakpoint => todo!("break"),
            BranchFalsy(i) => {
                conditional_branch!(is_nullish!(pop!()), i)
            }
            BranchTruthy(i) => {
                conditional_branch!(!is_nullish!(pop!()), i)
            }
            Call { param0: source, .. } => {
                // Use unified dispatch pipeline for static calls
                let result = self.unified_dispatch(gc, source, None, None);

                if let StepResult::InstructionStepped = result {
                    if initial_frame_count != self.execution.frames.len() {
                        moved_ip = true;
                    }
                } else {
                    return result;
                }
            }
            CallConstrained(constraint, source) => {
                // according to the standard, this doesn't really make sense
                // because the constrained prefix should only be on callvirt
                // however, this appears to be used for static interface dispatch?

                let constraint_type = self.current_context().make_concrete(constraint);
                let (method, lookup) = self.find_generic_method(source);

                let td = self.loader().find_concrete_type(constraint_type.clone());

                for o in td.definition().overrides.iter() {
                    let target = self
                        .current_context()
                        .locate_method(o.implementation, &lookup);
                    let declaration = self.current_context().locate_method(o.declaration, &lookup);
                    if method == declaration {
                        vm_trace!(self, "-- dispatching to {:?} --", target);
                        // Note: Uses dispatch_method directly since method is already resolved
                        // via explicit override lookup (static interface dispatch pattern)
                        let result = self.dispatch_method(gc, target, lookup);
                        if let StepResult::InstructionStepped = result {
                            if initial_frame_count == self.execution.frames.len() {
                                self.increment_ip();
                            }
                        }
                        return result;
                    }
                }

                panic!(
                    "could not find method to dispatch to for constrained call({:?}, {:?})",
                    constraint_type, method
                );
            }
            CallIndirect { .. } => todo!("calli"),
            CompareEqual => {
                let val = equal!() as i32;
                push!(Int32(val))
            }
            CompareGreater(sgn) => {
                let val = compare!(sgn, > (CmpOrdering::Greater)) as i32;
                push!(Int32(val))
            }
            CheckFinite => {
                vm_expect_stack!(let NativeFloat(f) = pop!());
                if f.is_infinite() || f.is_nan() {
                    return self.throw_by_name(gc, "System.ArithmeticException");
                }
                push!(NativeFloat(f));
            }
            CompareLess(sgn) => {
                let val = compare!(sgn, < (CmpOrdering::Less)) as i32;
                push!(Int32(val))
            }
            Convert(t) => {
                let value = pop!();

                macro_rules! simple_cast {
                    ($t:ty) => {
                        match value {
                            StackValue::Int32(i) => i as $t,
                            StackValue::Int64(i) => i as $t,
                            StackValue::NativeInt(i) => i as $t,
                            StackValue::NativeFloat(f) => f as $t,
                            v => panic!(
                                "invalid type on stack ({:?}) for conversion to {}",
                                v,
                                stringify!($t)
                            ),
                        }
                    };
                }

                macro_rules! convert_short_ints {
                    ($t:ty) => {{
                        let i = simple_cast!($t);
                        push!(Int32(i as i32));
                    }};
                }

                macro_rules! convert_long_ints {
                    ($variant:ident ( $t:ty )) => {{
                        let i = simple_cast!($t);
                        push!($variant(i));
                    }};
                    ($variant:ident ( $t:ty as $vt:ty )) => {{
                        let i = match value {
                            // all Rust casts from signed types will sign extend
                            // so first we have to make them unsigned so they'll properly zero extend
                            StackValue::Int32(i) => (i as u32) as $t,
                            StackValue::Int64(i) => (i as u64) as $t,
                            StackValue::NativeInt(i) => (i as usize) as $t,
                            StackValue::UnmanagedPtr(UnmanagedPtr(p)) => (p.as_ptr() as usize) as $t,
                            StackValue::ManagedPtr(ManagedPtr { value: p, .. }) => (p.map_or(0, |ptr| ptr.as_ptr() as usize)) as $t,
                            StackValue::NativeFloat(f) => {
                                todo!("truncate {} towards zero for conversion to {}", f, stringify!($t))
                            }
                            v => panic!("invalid type on stack ({:?}) for conversion to {}", v, stringify!($t)),
                        };
                        push!($variant(i as $vt));
                    }}
                }

                match t {
                    ConversionType::Int8 => convert_short_ints!(i8),
                    ConversionType::UInt8 => convert_short_ints!(u8),
                    ConversionType::Int16 => convert_short_ints!(i16),
                    ConversionType::UInt16 => convert_short_ints!(u16),
                    ConversionType::Int32 => convert_short_ints!(i32),
                    ConversionType::UInt32 => convert_short_ints!(u32),
                    ConversionType::Int64 => convert_long_ints!(Int64(i64)),
                    ConversionType::UInt64 => convert_long_ints!(Int64(u64 as i64)),
                    ConversionType::IntPtr => convert_long_ints!(NativeInt(isize)),
                    ConversionType::UIntPtr => convert_long_ints!(NativeInt(usize as isize)),
                }
            }
            ConvertOverflow(t, sgn) => {
                let value = pop!();
                todo!(
                    "{:?} conversion to {:?} with overflow detection ({:?})",
                    t,
                    sgn,
                    value
                )
            }
            ConvertFloat32 => {
                let v = match pop!() {
                    StackValue::Int32(i) => i as f32,
                    StackValue::Int64(i) => i as f32,
                    StackValue::NativeInt(i) => i as f32,
                    StackValue::NativeFloat(i) => i as f32,
                    rest => panic!(
                        "invalid type on stack ({:?}) for conversion to float32",
                        rest
                    ),
                };
                push!(NativeFloat(v as f64));
            }
            ConvertFloat64 => {
                let v = match pop!() {
                    StackValue::Int32(i) => i as f64,
                    StackValue::Int64(i) => i as f64,
                    StackValue::NativeInt(i) => i as f64,
                    StackValue::NativeFloat(i) => i,
                    rest => panic!(
                        "invalid type on stack ({:?}) for conversion to float64",
                        rest
                    ),
                };
                push!(NativeFloat(v));
            }
            ConvertUnsignedToFloat => {
                let value = pop!();
                todo!("conv.r.un({:?})", value)
            }
            CopyMemoryBlock { .. } => {
                let size = match pop!() {
                    StackValue::Int32(i) => i as usize,
                    StackValue::NativeInt(i) => i as usize,
                    rest => panic!(
                        "invalid type for size in cpblk (expected int32 or native int, received {:?})",
                        rest
                    ),
                };
                let src_val = pop!();
                let src = match &src_val {
                    StackValue::NativeInt(i) => *i as *const u8,
                    StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr() as *const u8,
                    StackValue::ManagedPtr(m) => m.value.map_or(ptr::null(), |p| p.as_ptr()),
                    rest => panic!(
                        "invalid type for src in cpblk (expected pointer, received {:?})",
                        rest
                    ),
                };
                let dest_val = pop!();
                let dest = match &dest_val {
                    StackValue::NativeInt(i) => *i as *mut u8,
                    StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr(),
                    StackValue::ManagedPtr(m) => m.value.map_or(ptr::null_mut(), |p| p.as_ptr()),
                    rest => panic!(
                        "invalid type for dest in cpblk (expected pointer, received {:?})",
                        rest
                    ),
                };

                // Bounds Check Dest
                if let StackValue::ManagedPtr(dest_mp) = &dest_val {
                    if let Some(owner) = dest_mp.owner {
                        let (base_addr, storage_size) = match owner {
                            ManagedPtrOwner::Heap(h) => {
                                let inner = h.borrow();
                                match &inner.storage {
                                    HeapStorage::Obj(o) => (o.instance_storage.get().as_ptr(), o.instance_storage.get().len()),
                                    HeapStorage::Vec(v) => (v.get().as_ptr(), v.get().len()),
                                    _ => (ptr::null(), 0),
                                }
                            }
                            ManagedPtrOwner::Stack(s) => unsafe {
                                (s.as_ref().instance_storage.get().as_ptr(), s.as_ref().instance_storage.get().len())
                            },
                        };

                        if !base_addr.is_null() {
                            let dest_offset = (dest as usize).wrapping_sub(base_addr as usize);
                            if dest_offset.checked_add(size).map_or(true, |end| end > storage_size) {
                                panic!("Heap corruption detected! Cpblk writing {} bytes at offset {} into storage of size {}", size, dest_offset, storage_size);
                            }
                        }
                    }
                }

                // Check GC safe point before large memory block copy operations
                // Threshold: copying more than 4KB of data
                const LARGE_COPY_THRESHOLD: usize = 4096;
                if size > LARGE_COPY_THRESHOLD {
                    self.check_gc_safe_point();
                }

                unsafe {
                    ptr::copy_nonoverlapping(src, dest, size);
                }
            }
            Divide(sgn) => {
                let v2 = pop!();
                let v1 = pop!();
                match v1.div(v2, *sgn) {
                    Ok(v) => push!(v),
                    Err(e) => return self.throw_by_name(gc, e),
                }
            }
            Duplicate => {
                let val = pop!();
                push!(val.clone());
                push!(val);
            }
            EndFilter => {
                let result = pop!();
                let result_val = match result {
                    StackValue::Int32(i) => i,
                    _ => panic!("EndFilter expected Int32, found {:?}", result),
                };

                let (exception, handler) = match self.execution.exception_mode {
                    ExceptionState::Filtering { exception, handler } => (exception, handler),
                    _ => panic!("EndFilter called but not in Filtering mode"),
                };

                // Restore suspended state
                // Note: we use handler.frame_index because the filter ran in that frame.
                // It might have called other methods, but those should have returned by now.
                self.execution
                    .stack
                    .truncate(self.execution.frames[handler.frame_index].base.stack);
                self.execution
                    .stack
                    .append(&mut self.execution.suspended_stack);
                self.execution
                    .frames
                    .append(&mut self.execution.suspended_frames);

                let frame = &mut self.execution.frames[handler.frame_index];
                frame.exception_stack.pop();
                frame.state.ip = self.execution.original_ip;
                frame.stack_height = self.execution.original_stack_height;

                if result_val != 0 {
                    // Filter matched!
                    self.execution.exception_mode = ExceptionState::Unwinding {
                        exception: Some(exception),
                        target: UnwindTarget::Handler(handler),
                        cursor: HandlerAddress {
                            frame_index: self.execution.frames.len() - 1,
                            section_index: 0,
                            handler_index: 0,
                        },
                    };
                } else {
                    // Filter did not match, continue searching
                    let mut next_cursor = handler;
                    next_cursor.handler_index += 1;
                    self.execution.exception_mode = ExceptionState::Searching {
                        exception,
                        cursor: next_cursor,
                    };
                }
                return self.handle_exception(gc);
            }
            EndFinally => match self.execution.exception_mode {
                ExceptionState::ExecutingHandler {
                    exception,
                    target,
                    cursor,
                } => {
                    self.execution.exception_mode = ExceptionState::Unwinding {
                        exception,
                        target,
                        cursor,
                    };
                    return self.handle_exception(gc);
                }
                _ => panic!(
                    "endfinally called outside of handler, state: {:?}",
                    self.execution.exception_mode
                ),
            },
            InitializeMemoryBlock { .. } => {
                let size = match pop!() {
                    StackValue::Int32(i) => i as usize,
                    StackValue::NativeInt(i) => i as usize,
                    rest => panic!(
                        "invalid type for size in initblk (expected int32 or native int, received {:?})",
                        rest
                    ),
                };
                let val = match pop!() {
                    StackValue::Int32(i) => i as u8,
                    rest => panic!(
                        "invalid type for value in initblk (expected int32, received {:?})",
                        rest
                    ),
                };
                let addr_val = pop!();
                let addr = match &addr_val {
                    StackValue::NativeInt(i) => *i as *mut u8,
                    StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr(),
                    StackValue::ManagedPtr(m) => m.value.map_or(ptr::null_mut(), |p| p.as_ptr()),
                    rest => panic!(
                        "invalid type for address in initblk (expected pointer, received {:?})",
                        rest
                    ),
                };

                // Bounds Check
                if let StackValue::ManagedPtr(mp) = &addr_val {
                    if let Some(owner) = mp.owner {
                        let (base_addr, storage_size) = match owner {
                            ManagedPtrOwner::Heap(h) => {
                                let inner = h.borrow();
                                match &inner.storage {
                                    HeapStorage::Obj(o) => (o.instance_storage.get().as_ptr(), o.instance_storage.get().len()),
                                    HeapStorage::Vec(v) => (v.get().as_ptr(), v.get().len()),
                                    _ => (ptr::null(), 0),
                                }
                            }
                            ManagedPtrOwner::Stack(s) => unsafe {
                                (s.as_ref().instance_storage.get().as_ptr(), s.as_ref().instance_storage.get().len())
                            },
                        };

                        if !base_addr.is_null() {
                            let dest_offset = (addr as usize).wrapping_sub(base_addr as usize);
                            if dest_offset.checked_add(size).map_or(true, |end| end > storage_size) {
                                panic!("Heap corruption detected! Initblk writing {} bytes at offset {} into storage of size {}", size, dest_offset, storage_size);
                            }
                        }
                    }
                }

                unsafe {
                    ptr::write_bytes(addr, val, size);
                }
            }
            Jump(_) => todo!("jmp"),
            LoadArgument(i) => {
                let arg = self.get_argument(*i as usize);
                push!(arg);
            }
            LoadArgumentAddress(i) => {
                let arg = self.get_argument(*i as usize);
                let ctx = self.current_context();
                let live_type = ctx.stack_value_type(&arg);
                push!(managed_ptr(
                    self.get_argument_address(*i as usize).as_ptr() as *mut _,
                    live_type,
                    None,
                    true
                ));
            }
            LoadConstantInt32(i) => push!(Int32(*i)),
            LoadConstantInt64(i) => push!(Int64(*i)),
            LoadConstantFloat32(f) => push!(NativeFloat(*f as f64)),
            LoadConstantFloat64(f) => push!(NativeFloat(*f)),
            LoadMethodPointer(source) => {
                let (method, lookup) = self.find_generic_method(source);
                let idx = self.get_runtime_method_index(method, lookup);
                push!(NativeInt(idx as isize));
            }
            LoadIndirect { param0: t, .. } => {
                let ptr = pop!().as_ptr();
                push!(unsafe { StackValue::load(ptr, *t) });
            }
            LoadLocal(i) => {
                let local = self.get_local(*i as usize);
                push!(local);
            }
            LoadLocalAddress(i) => {
                let local = self.get_local(*i as usize);
                let ctx = self.current_context();
                let live_type = ctx.stack_value_type(&local);
                let pinned = match state!(|s| &s.info_handle.locals[*i as usize]) {
                    LocalVariable::Variable { pinned, .. } => *pinned,
                    _ => false,
                };

                let (ptr, owner, _) = self.get_local_info_for_managed_ptr(*i as usize);

                push!(managed_ptr(
                    ptr.as_ptr() as *mut _,
                    live_type,
                    owner,
                    pinned
                ));
            }
            LoadNull => push!(null()),
            Leave(jump_target) => {
                // If we're already executing a handler (e.g., a finally block during unwinding),
                // we need to preserve that state. The Leave instruction in this case just means
                // "exit this finally block and continue unwinding".
                self.execution.exception_mode = match self.execution.exception_mode {
                    // We're inside a finally/fault handler. Transition back to Unwinding
                    // to continue processing remaining handlers.
                    ExceptionState::ExecutingHandler {
                        exception,
                        target,
                        cursor,
                    } => ExceptionState::Unwinding {
                        exception,
                        target,
                        cursor,
                    },
                    // Normal Leave: start a new unwind operation
                    _ => ExceptionState::Unwinding {
                        exception: None,
                        target: UnwindTarget::Instruction(*jump_target),
                        cursor: HandlerAddress {
                            frame_index: self.execution.frames.len() - 1,
                            section_index: 0,
                            handler_index: 0,
                        },
                    },
                };
                return self.handle_exception(gc);
            }
            LocalMemoryAllocate => {
                let size = match pop!() {
                    StackValue::Int32(i) => {
                        if i < 0 {
                            return self.throw_by_name(gc, "System.OverflowException");
                        }
                        i as usize
                    }
                    StackValue::NativeInt(i) => {
                        if i < 0 {
                            return self.throw_by_name(gc, "System.OverflowException");
                        }
                        i as usize
                    }
                    v => panic!(
                        "invalid type on stack ({:?}) for local memory allocation size",
                        v
                    ),
                };
                // Defensive check: limit local allocation to 128MB
                if size > 0x800_0000 {
                    return self.throw_by_name(gc, "System.OutOfMemoryException");
                }
                let ptr = state!(|s| {
                    let loc = s.memory_pool.len();
                    s.memory_pool.extend(vec![0; size]);
                    s.memory_pool[loc..].as_mut_ptr()
                });
                push!(unmanaged_ptr(ptr));
            }
            Multiply => {
                let v2 = pop!();
                let v1 = pop!();
                push!(v1 * v2);
            }
            MultiplyOverflow(sgn) => {
                let v2 = pop!();
                let v1 = pop!();
                match v1.checked_mul(v2, *sgn) {
                    Ok(v) => push!(v),
                    Err(e) => return self.throw_by_name(gc, e),
                }
            }
            Negate => {
                let v = pop!();
                push!(-v);
            }
            NoOperation => {} // :3
            Not => {
                let v = pop!();
                push!(!v);
            }
            Or => {
                let v2 = pop!();
                let v1 = pop!();
                push!(v1 | v2);
            }
            Pop => {
                pop!();
            }
            Remainder(sgn) => {
                let v2 = pop!();
                let v1 = pop!();
                match v1.rem(v2, *sgn) {
                    Ok(v) => push!(v),
                    Err(e) => return self.throw_by_name(gc, e),
                }
            }
            Return => {
                // If we're executing a handler (e.g., inside a finally block) in the CURRENT frame,
                // we need to continue unwinding through any remaining handlers before actually returning.
                let frame_index = self.execution.frames.len() - 1;
                if let ExceptionState::ExecutingHandler {
                    exception, cursor, ..
                } = self.execution.exception_mode
                {
                    // Only treat this as a return-inside-handler if the handler being executed
                    // is in the current frame. If it's in a parent frame, this is just a normal
                    // return from a nested call.
                    if cursor.frame_index == frame_index {
                        // Change the target to indicate we want to return after unwinding,
                        // not jump to the original target
                        self.execution.exception_mode = ExceptionState::Unwinding {
                            exception,
                            target: UnwindTarget::Instruction(usize::MAX),
                            cursor,
                        };
                        return self.handle_exception(gc);
                    }
                }

                // If there are any finally blocks in the current frame, we need to execute them
                // before returning. Check if there are any exception handling sections.
                let frame_index = self.execution.frames.len() - 1;
                let has_finally_blocks = state!(|s| !s.info_handle.exceptions.is_empty());

                if has_finally_blocks {
                    // Start unwinding to execute any finally blocks before returning
                    self.execution.exception_mode = ExceptionState::Unwinding {
                        exception: None,
                        // We use a special target that indicates we want to return after unwinding
                        target: UnwindTarget::Instruction(usize::MAX), // Special sentinel value
                        cursor: HandlerAddress {
                            frame_index,
                            section_index: 0,
                            handler_index: 0,
                        },
                    };
                    return self.handle_exception(gc);
                }

                // No handlers to execute, just return normally
                return StepResult::MethodReturned;
            }
            ShiftLeft => {
                let v2 = pop!();
                let v1 = pop!();
                push!(v1 << v2);
            }
            ShiftRight(sgn) => {
                let v2 = pop!();
                let v1 = pop!();
                push!(v1.shr(v2, *sgn));
            }
            StoreArgument(i) => {
                let val = pop!();
                self.set_argument(gc, *i as usize, val);
            }
            StoreIndirect {
                param0: store_type, ..
            } => {
                let val = pop!();
                let addr_val = pop!();
                let ptr = addr_val.as_ptr();

                // Bounds check
                if let StackValue::ManagedPtr(mp) = &addr_val {
                    if let Some(owner) = mp.owner {
                        let (base_addr, storage_size) = match owner {
                            ManagedPtrOwner::Heap(h) => {
                                let inner = h.borrow();
                                match &inner.storage {
                                    HeapStorage::Obj(o) => (o.instance_storage.get().as_ptr(), o.instance_storage.get().len()),
                                    HeapStorage::Vec(v) => (v.get().as_ptr(), v.get().len()),
                                    _ => (ptr::null(), 0),
                                }
                            }
                            ManagedPtrOwner::Stack(s) => unsafe {
                                (s.as_ref().instance_storage.get().as_ptr(), s.as_ref().instance_storage.get().len())
                            },
                        };

                        if !base_addr.is_null() {
                            let dest_offset = (ptr as usize).wrapping_sub(base_addr as usize);
                            let size = match store_type {
                                StoreType::Int8 => 1,
                                StoreType::Int16 => 2,
                                StoreType::Int32 => 4,
                                StoreType::Int64 => 8,
                                StoreType::IntPtr => std::mem::size_of::<usize>(),
                                StoreType::Float32 => 4,
                                StoreType::Float64 => 8,
                                StoreType::Object => std::mem::size_of::<usize>(),
                            };

                            if size > 0 && dest_offset.checked_add(size).map_or(true, |end| end > storage_size) {
                                panic!("Heap corruption detected! StoreIndirect writing {} bytes at offset {} into storage of size {}", size, dest_offset, storage_size);
                            }
                        }
                    }
                }

                unsafe { val.store(ptr, *store_type) };
            }
            StoreLocal(i) => {
                let val = pop!();
                self.set_local(gc, *i as usize, val);
            }
            Subtract => {
                let v2 = pop!();
                let v1 = pop!();
                push!(v1 - v2);
            }
            SubtractOverflow(sgn) => {
                let v2 = pop!();
                let v1 = pop!();
                match v1.checked_sub(v2, *sgn) {
                    Ok(v) => push!(v),
                    Err(e) => return self.throw_by_name(gc, e),
                }
            }
            Switch(targets) => {
                vm_expect_stack!(let Int32(value as usize) = pop!());
                conditional_branch!(value < targets.len(), &targets[value]);
            }
            Xor => {
                let v2 = pop!();
                let v1 = pop!();
                push!(v1 ^ v2);
            }
            BoxValue(t) => {
                let t = self.current_context().make_concrete(t);

                let value = pop!();

                if let StackValue::ObjectRef(_) = value {
                    // boxing is a noop for all reference types
                    push!(value);
                } else {
                    let obj = ObjectRef::new(
                        gc,
                        HeapStorage::Boxed(self.current_context().new_value_type(&t, value)),
                    );
                    self.register_new_object(&obj);
                    push!(ObjectRef(obj));
                }
            }
            CallVirtual { param0: source, .. } => {
                // Use unified dispatch pipeline for virtual calls
                // Note: We still need to pop args to extract this_type before dispatch

                // Determine number of arguments to extract this_type
                let (base_method, _) = self.find_generic_method(source);
                let num_args = 1 + base_method.method.signature.parameters.len();
                let mut args = Vec::new();
                for _ in 0..num_args {
                    args.push(pop!());
                }
                args.reverse();

                // Extract runtime type from this argument (value types are passed as managed pointers - I.8.9.7)
                let this_value = args[0].clone();
                let this_type = match this_value {
                    StackValue::ObjectRef(ObjectRef(None)) => {
                        return self.throw_by_name(gc, "System.NullReferenceException")
                    }
                    StackValue::ObjectRef(ObjectRef(Some(o))) => {
                        self.current_context().get_heap_description(o)
                    },
                    StackValue::ManagedPtr(m) => m.inner_type,
                    rest => panic!("invalid this argument for virtual call (expected ObjectRef or ManagedPtr, received {:?})", rest)
                };

                // TODO: check explicit overrides

                // Push arguments back and dispatch
                for a in args {
                    push!(a);
                }
                let result = self.unified_dispatch(gc, source, Some(this_type), None);

                if let StepResult::InstructionStepped = result {
                    if initial_frame_count != self.execution.frames.len() {
                        moved_ip = true;
                    }
                } else {
                    return result;
                }
            }
            CallVirtualConstrained(constraint, source) => {
                let (base_method, lookup) = self.find_generic_method(source);

                // Pop all arguments (this + parameters)
                let num_args = 1 + base_method.method.signature.parameters.len();
                let mut args: Vec<_> = (0..num_args).map(|_| pop!()).collect();
                args.reverse();

                let ctx = self.current_context();

                let constraint_type_source = ctx.make_concrete(constraint);
                let constraint_type = self
                    .loader()
                    .find_concrete_type(constraint_type_source.clone());

                // Determine dispatch strategy based on constraint type
                let method = if constraint_type.is_value_type(&ctx) {
                    // Value type: check for direct override first
                    if let Some(overriding_method) =
                        self.loader().find_method_in_type_with_substitution(
                            constraint_type,
                            &base_method.method.name,
                            &base_method.method.signature,
                            base_method.resolution(),
                            &lookup,
                        )
                    {
                        // Value type has its own implementation
                        overriding_method
                    } else {
                        // No override: box the value and use base implementation
                        let value_size = type_layout(constraint_type_source.clone(), &ctx).size();
                        let value_data =
                            unsafe { slice::from_raw_parts(args[0].as_ptr(), value_size) };
                        let value = ctx.read_cts_value(&constraint_type_source, value_data, gc);

                        let boxed = ObjectRef::new(
                            gc,
                            HeapStorage::Boxed(
                                ctx.new_value_type(&constraint_type_source, value.into_stack()),
                            ),
                        );
                        self.register_new_object(&boxed);

                        args[0] = StackValue::ObjectRef(boxed);
                        let this_type = ctx.get_heap_description(boxed.0.unwrap());
                        self.resolve_virtual_method(base_method, this_type, Some(&ctx))
                    }
                } else {
                    // Reference type: dereference the managed pointer
                    vm_expect_stack!(let ManagedPtr(m) = args[0].clone());
                    let ptr = match m.value {
                        Some(p) => p.as_ptr(),
                        None => return self.throw_by_name(gc, "System.NullReferenceException"),
                    };
                    debug_assert!(
                        (ptr as usize).is_multiple_of(align_of::<ObjectRef>()),
                        "ManagedPtr value is not aligned for ObjectRef"
                    );
                    // Create a slice from the pointer. We know ObjectRef is pointer-sized.
                    let value_bytes = unsafe { slice::from_raw_parts(ptr, ObjectRef::SIZE) };
                    let obj_ref = unsafe { ObjectRef::read_branded(value_bytes, gc) };

                    if obj_ref.0.is_none() {
                        return self.throw_by_name(gc, "System.NullReferenceException");
                    }

                    args[0] = StackValue::ObjectRef(obj_ref);

                    // For reference types with constrained callvirt, try to find the method
                    // implementation directly in the constraint type first
                    if let Some(impl_method) = self.loader().find_method_in_type_with_substitution(
                        constraint_type,
                        &base_method.method.name,
                        &base_method.method.signature,
                        base_method.resolution(),
                        &lookup,
                    ) {
                        impl_method
                    } else {
                        // Fall back to normal virtual dispatch
                        let this_type = ctx.get_heap_description(obj_ref.0.unwrap());
                        self.resolve_virtual_method(base_method, this_type, Some(&ctx))
                    }
                };

                for arg in args {
                    push!(arg);
                }

                // Note: CallVirtualConstrained uses dispatch_method directly
                // instead of unified_dispatch because it performs custom method resolution
                // (boxing value types, constraint-specific lookup) that doesn't fit the
                // standard virtual dispatch pattern. The method is already fully resolved here.
                let result = self.dispatch_method(gc, method, lookup);

                if let StepResult::InstructionStepped = result {
                    if initial_frame_count != self.execution.frames.len() {
                        moved_ip = true;
                    }
                } else {
                    return result;
                }
            }
            CallVirtualTail(_) => todo!("tail.callvirt"),
            CastClass { param0: target, .. } => {
                vm_expect_stack!(let ObjectRef(target_obj) = pop!());
                if let ObjectRef(Some(o)) = target_obj {
                    let ctx = self.current_context();
                    let obj_type = ctx.get_heap_description(o);
                    let target_ct = ctx.make_concrete(target);

                    if ctx.is_a(obj_type.into(), target_ct) {
                        push!(ObjectRef(target_obj));
                    } else {
                        return self.throw_by_name(gc, "System.InvalidCastException");
                    }
                } else {
                    // castclass returns null for null (III.4.3)
                    push!(null());
                }
            }
            CopyObject(_) => todo!("cpobj"),
            InitializeForObject(t) => {
                let ctx = self.current_context();
                let layout = type_layout(ctx.make_concrete(t), &ctx);
                let target = pop!().as_ptr();

                debug_assert!(!target.is_null(), "initobj target address is null");
                let s = unsafe { slice::from_raw_parts_mut(target, layout.size()) };
                s.fill(0);
            }
            IsInstance(target) => {
                vm_expect_stack!(let ObjectRef(target_obj) = pop!());
                if let ObjectRef(Some(o)) = target_obj {
                    let ctx = self.current_context();
                    let obj_type = ctx.get_heap_description(o);
                    let target_ct = ctx.make_concrete(target);

                    if ctx.is_a(obj_type.into(), target_ct) {
                        push!(ObjectRef(target_obj));
                    } else {
                        push!(null());
                    }
                } else {
                    // isinst returns null for null inputs (III.4.6)
                    push!(null());
                }
            }
            LoadElement {
                param0: load_type, ..
            } => {
                let index_val = pop!();
                let index = match index_val {
                    StackValue::Int32(i) => i as usize,
                    StackValue::NativeInt(i) => i as usize,
                    rest => panic!("invalid index for ldelem: {:?}", rest),
                };
                vm_expect_stack!(let ObjectRef(obj) = pop!());

                if obj.0.is_none() {
                    return self.throw_by_name(gc, "System.NullReferenceException");
                }

                let ctx = self.current_context();
                let load_type = ctx.make_concrete(load_type);
                let value = obj.as_vector(|array| {
                    if index >= array.layout.length {
                        return Err(());
                    }
                    let elem_size = array.layout.element_layout.size();
                    let target = &array.get()[(elem_size * index)..(elem_size * (index + 1))];
                    Ok(ctx.read_cts_value(&load_type, target, gc).into_stack())
                });
                match value {
                    Ok(v) => push!(v),
                    Err(_) => return self.throw_by_name(gc, "System.IndexOutOfRangeException"),
                }
            }
            LoadElementPrimitive {
                param0: load_type, ..
            } => {
                use LoadType::*;

                let index = match pop!() {
                    StackValue::Int32(i) => i as usize,
                    StackValue::NativeInt(i) => i as usize,
                    rest => panic!(
                        "invalid index for ldelem (expected int32 or native int, received {:?})",
                        rest
                    ),
                };
                let array = pop!();

                vm_expect_stack!(let ObjectRef(obj) = array);

                if obj.0.is_none() {
                    return self.throw_by_name(gc, "System.NullReferenceException");
                }

                let elem_size: usize = match load_type {
                    Int8 | UInt8 => 1,
                    Int16 | UInt16 => 2,
                    Int32 | UInt32 => 4,
                    Int64 => 8,
                    Float32 => 4,
                    Float64 => 8,
                    IntPtr | Object => ObjectRef::SIZE,
                };

                let value = obj.as_vector(|array| {
                    if index >= array.layout.length {
                        return Err(());
                    }
                    let target = &array.get()[(elem_size * index)..(elem_size * (index + 1))];
                    macro_rules! from_bytes {
                        ($t:ty) => {
                            <$t>::from_ne_bytes(
                                target.try_into().expect("source data was too small"),
                            )
                        };
                    }

                    Ok(match load_type {
                        Int8 => StackValue::Int32(from_bytes!(i8) as i32),
                        UInt8 => StackValue::Int32(from_bytes!(u8) as i32),
                        Int16 => StackValue::Int32(from_bytes!(i16) as i32),
                        UInt16 => StackValue::Int32(from_bytes!(u16) as i32),
                        Int32 => StackValue::Int32(from_bytes!(i32)),
                        UInt32 => StackValue::Int32(from_bytes!(u32) as i32),
                        Int64 => StackValue::Int64(from_bytes!(i64)),
                        Float32 => StackValue::NativeFloat(from_bytes!(f32) as f64),
                        Float64 => StackValue::NativeFloat(from_bytes!(f64)),
                        IntPtr => StackValue::NativeInt(from_bytes!(usize) as isize),
                        Object => StackValue::ObjectRef(unsafe { ObjectRef::read_branded(target, gc) }),
                    })
                });
                match value {
                    Ok(v) => push!(v),
                    Err(_) => return self.throw_by_name(gc, "System.IndexOutOfRangeException"),
                }
            }
            LoadElementAddress { param0: t, .. } | LoadElementAddressReadonly(t) => {
                let index = match pop!() {
                    StackValue::Int32(i) => i as usize,
                    StackValue::NativeInt(i) => i as usize,
                    rest => panic!("invalid index for ldelema: {:?}", rest),
                };
                vm_expect_stack!(let ObjectRef(obj) = pop!());
                let Some(h) = obj.0 else {
                    return self.throw_by_name(gc, "System.NullReferenceException");
                };
                let ctx = self.current_context();
                let concrete_t = ctx.make_concrete(t);
                let element_layout = type_layout(concrete_t.clone(), &ctx);

                let data = h.borrow();
                let ptr = match &data.storage {
                    HeapStorage::Vec(v) => {
                        if index >= v.layout.length {
                            panic!("IndexOutOfRangeException");
                        }
                        // SAFETY: v.get() returns a slice of the array's storage. index is checked against
                        // the array length. The pointer arithmetic is safe as it stays within the allocated
                        // storage for the array.
                        unsafe { v.get().as_ptr().add(index * element_layout.size()) as *mut u8 }
                    }
                    _ => panic!("ldelema on non-vector"),
                };
                let target_type = self.loader().find_concrete_type(concrete_t);
                push!(managed_ptr(
                    ptr,
                    target_type,
                    Some(ManagedPtrOwner::Heap(h)),
                    false
                ));
            }
            LoadField {
                param0: source,
                volatile,
                ..
            } => {
                let (field, lookup) = self.locate_field(*source);

                check_special_fields!(field, false);

                let parent = pop!();

                let ctx = self
                    .current_context()
                    .for_type_with_generics(field.parent, &lookup);
                let name = &field.field.name;
                let t = ctx.get_field_type(field);

                let ordering = if *volatile {
                    AtomicOrdering::SeqCst
                } else {
                    AtomicOrdering::Acquire
                };

                let read_data = |d: &[u8]| -> CTSValue<'gc> { ctx.read_cts_value(&t, d, gc) };
                let read_from_pointer = |ptr: *mut u8| {
                    debug_assert!(!ptr.is_null(), "Attempted to read field from null pointer");
                    let layout = LayoutFactory::instance_field_layout_cached(
                        field.parent,
                        &ctx,
                        Some(&self.shared.metrics),
                    );
                    let field_layout = layout.get_field(field.parent, name.as_ref()).unwrap();

                    let size = field_layout.layout.size();
                    let field_ptr = unsafe { ptr.add(field_layout.position) };

                    // For pointer-sized types, use atomic load even if not volatile to ensure atomicity
                    // as required by ECMA-335.
                    if size <= size_of::<usize>() {
                        // Check alignment before attempting atomic operations
                        if !is_ptr_aligned_to_field(field_ptr, size) {
                            // Fall back to non-atomic read_unchecked if not aligned
                            let slice = unsafe { slice::from_raw_parts(field_ptr, size) };
                            return read_data(slice);
                        }

                        macro_rules! load_atomic {
                            ($t:ty) => {{
                                let val = unsafe { (*(field_ptr as *const $t)).load(ordering) };
                                let bytes = val.to_ne_bytes();
                                read_data(&bytes)
                            }};
                        }

                        match size {
                            1 => load_atomic!(AtomicU8),
                            2 => load_atomic!(AtomicU16),
                            4 => load_atomic!(AtomicU32),
                            8 => load_atomic!(AtomicU64),
                            _ => {
                                let slice = unsafe { slice::from_raw_parts(field_ptr, size) };
                                read_data(slice)
                            }
                        }
                    } else {
                        // SAFETY: ptr is a raw pointer to either heap storage, a stack slot, or unmanaged memory.
                        // The offset is calculated based on the type's field layout, and the size matches the field's type.
                        let slice = unsafe {
                            slice::from_raw_parts(
                                ptr.add(field_layout.position),
                                field_layout.layout.size(),
                            )
                        };
                        read_data(slice)
                    }
                };

                let value = match parent {
                    StackValue::ObjectRef(ObjectRef(None)) => {
                        return self.throw_by_name(gc, "System.NullReferenceException")
                    }
                    StackValue::ObjectRef(ObjectRef(Some(h))) => {
                        let intercepted = if field.parent.type_name()
                            == "System.Runtime.CompilerServices.RawArrayData"
                        {
                            let data = h.borrow();
                            if let HeapStorage::Vec(ref vector) = data.storage {
                                if name == "Length" {
                                    Some(CTSValue::Value(dotnet_value::object::ValueType::UInt32(
                                        vector.layout.length as u32,
                                    )))
                                } else if name == "Data" {
                                    let b = if vector.layout.length > 0 {
                                        vector.get()[0]
                                    } else {
                                        0
                                    };
                                    Some(CTSValue::Value(dotnet_value::object::ValueType::UInt8(b)))
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        if let Some(val) = intercepted {
                            val
                        } else {
                            let object_type = self.current_context().get_heap_description(h);
                            if !self
                                .current_context()
                                .is_a(object_type.into(), field.parent.into())
                            {
                                panic!(
                                    "tried to load field {}::{} from object of type {}",
                                    field.parent.type_name(),
                                    name,
                                    object_type.type_name()
                                )
                            }

                            let data = h.borrow();
                            match &data.storage {
                                HeapStorage::Vec(_) => todo!("field on array"),
                                HeapStorage::Obj(o) => {
                                    if !o.instance_storage.has_field(field.parent, name) {
                                        let current_method =
                                            self.current_frame().state.info_handle.source;
                                        panic!(
                                        "field {}::{} not found in object of type {} while executing method {}::{}",
                                        field.parent.type_name(),
                                        name,
                                        object_type.type_name(),
                                        current_method.parent.type_name(),
                                        current_method.method.name
                                    );
                                    }
                                    let field_data = o.instance_storage.get_field_atomic(
                                        field.parent,
                                        name,
                                        ordering,
                                    );
                                    let mut val = read_data(&field_data);
                                    // Metadata recovery
                                    if let CTSValue::Value(dotnet_value::object::ValueType::Pointer(
                                        ref mut mp,
                                    )) = val
                                    {
                                        let field_layout = o
                                            .instance_storage
                                            .layout()
                                            .get_field(field.parent, name.as_ref())
                                            .unwrap();
                                        if let Some(metadata) = o
                                            .managed_ptr_metadata
                                            .borrow()
                                            .get(field_layout.position)
                                        {
                                            mp.owner = metadata.recover_owner();
                                        }
                                    }
                                    val
                                }
                                HeapStorage::Str(_) => todo!("field on string"),
                                HeapStorage::Boxed(_) => todo!("field on boxed value type"),
                            }
                        }
                    }
                    StackValue::ValueType(ref o) => {
                        if !self
                            .current_context()
                            .is_a(o.description.into(), field.parent.into())
                        {
                            panic!(
                                "tried to load field {}::{} from object of type {}",
                                field.parent.type_name(),
                                name,
                                o.description.type_name()
                            )
                        }
                        let mut val =
                            read_data(&o.instance_storage.get_field_local(field.parent, name));
                        // Metadata recovery
                        if let CTSValue::Value(dotnet_value::object::ValueType::Pointer(
                            ref mut mp,
                        )) = val
                        {
                            let field_layout = o
                                .instance_storage
                                .layout()
                                .get_field(field.parent, name.as_ref())
                                .unwrap();
                            if let Some(metadata) =
                                o.managed_ptr_metadata.borrow().get(field_layout.position)
                            {
                                mp.owner = metadata.recover_owner();
                            }
                        }
                        val
                    }
                    StackValue::NativeInt(i) => read_from_pointer(i as *mut u8),
                    StackValue::UnmanagedPtr(UnmanagedPtr(ptr)) => read_from_pointer(ptr.as_ptr()),
                    StackValue::ManagedPtr(m) => {
                        let mut val = read_from_pointer(
                            m.value.expect("System.NullReferenceException").as_ptr(),
                        );
                        // Metadata recovery
                        if let CTSValue::Value(dotnet_value::object::ValueType::Pointer(
                            ref mut mp,
                        )) = val
                        {
                            if let Some(owner) = m.owner {
                                let target_addr =
                                    m.value.expect("System.NullReferenceException").as_ptr()
                                        as usize;
                                let layout = LayoutFactory::instance_field_layout_cached(
                                    field.parent,
                                    &ctx,
                                    Some(&self.shared.metrics),
                                );
                                let field_layout =
                                    layout.get_field(field.parent, name.as_ref()).unwrap();
                                let final_addr = target_addr + field_layout.position;

                                match owner {
                                    ManagedPtrOwner::Heap(h) => {
                                        let borrow = h.borrow();
                                        if let Some(o) = borrow.storage.as_obj() {
                                            let base = o.instance_storage.get().as_ptr() as usize;
                                            let offset = final_addr - base;
                                            if let Some(metadata) =
                                                o.managed_ptr_metadata.borrow().get(offset)
                                            {
                                                mp.owner = metadata.recover_owner();
                                            }
                                        }
                                    }
                                    ManagedPtrOwner::Stack(s) => {
                                        let o = unsafe { s.as_ref() };
                                        let base = o.instance_storage.get().as_ptr() as usize;
                                        let offset = final_addr - base;
                                        if let Some(metadata) =
                                            o.managed_ptr_metadata.borrow().get(offset)
                                        {
                                            mp.owner = metadata.recover_owner();
                                        }
                                    }
                                }
                            }
                        }
                        val
                    }
                    rest => panic!("stack value {:?} has no fields", rest),
                };

                vm_trace_field!(self, "LOAD", name, &value);
                push!(value.into_stack())
            }
            LoadFieldAddress(source) => {
                let (field, lookup) = self.locate_field(*source);
                let name = &field.field.name;

                check_special_fields!(field, true);

                let parent = pop!();

                let ctx = self
                    .current_context()
                    .for_type_with_generics(field.parent, &lookup);

                let (source_ptr, owner, pinned): (*mut u8, _, _) = match parent {
                    StackValue::ObjectRef(ObjectRef(None)) => {
                        return self.throw_by_name(gc, "System.NullReferenceException")
                    }
                    StackValue::ObjectRef(ObjectRef(Some(h))) => {
                        // Intercept System.Runtime.CompilerServices.RawArrayData access on arrays
                        // This acts as an intrinsic for Unsafe.As<Array, RawArrayData> usage
                        if field.parent.type_name() == "System.Runtime.CompilerServices.RawArrayData" {
                            let data = h.borrow();
                            if let HeapStorage::Vec(ref vector) = data.storage {
                                let ptr = if name == "Data" {
                                    vector.get().as_ptr() as *mut u8
                                } else if name == "Length" {
                                    // Pointer to length (usize), compatible with uint (u32) in Little Endian
                                    (&vector.layout.length as *const usize) as *mut u8
                                } else {
                                    ptr::null_mut()
                                };

                                if !ptr.is_null() {
                                    let target_type = self.current_context().get_field_desc(field);
                                    drop(data);
                                    push!(StackValue::managed_ptr(
                                        ptr,
                                        target_type,
                                        Some(ManagedPtrOwner::Heap(h)),
                                        false
                                    ));
                                    self.increment_ip();
                                    return StepResult::InstructionStepped;
                                }
                            }
                        }

                        let object_type = ctx.get_heap_description(h);
                        if !ctx.is_a(object_type.into(), field.parent.into()) {
                            panic!(
                                "tried to load field {}::{} from object of type {}",
                                field.parent.type_name(),
                                name,
                                object_type.type_name()
                            )
                        }

                        let data = h.borrow();
                        let source_ptr: *mut u8 = match &data.storage {
                            HeapStorage::Vec(_) => todo!("field on array"),
                            HeapStorage::Obj(o) => o.instance_storage.get().as_ptr() as *mut u8,
                            HeapStorage::Str(_) => {
                                panic!("field on string: {}::{}", field.parent.type_name(), name);
                            }
                            HeapStorage::Boxed(_) => todo!("field on boxed value type"),
                        };
                        (source_ptr, Some(ManagedPtrOwner::Heap(h)), false)
                    }
                    StackValue::NativeInt(i) => (i as *mut u8, None, false),
                    StackValue::UnmanagedPtr(UnmanagedPtr(ptr)) => (ptr.as_ptr(), None, false),
                    StackValue::ManagedPtr(m) => (
                        m.value.map_or(ptr::null_mut(), |p| p.as_ptr()),
                        m.owner,
                        m.pinned,
                    ),
                    StackValue::ValueType(ref o) => (
                        o.instance_storage.get().as_ptr() as *mut u8,
                        Some(ManagedPtrOwner::Stack(ptr::NonNull::from(&**o))),
                        false,
                    ),
                    rest => panic!("cannot load field address from stack value {:?}", rest),
                };
                debug_assert!(
                    !source_ptr.is_null(),
                    "Attempted to load field address from null pointer"
                );

                let layout = LayoutFactory::instance_field_layout_cached(
                    field.parent,
                    &ctx,
                    Some(&self.shared.metrics),
                );
                // SAFETY: source_ptr is a valid pointer to the start of an object or value type's storage.
                // The offset is obtained from the field layout, which is guaranteed to be within bounds
                // for the given type.
                let ptr = unsafe {
                    source_ptr.add(
                        layout
                            .get_field(field.parent, name.as_ref())
                            .unwrap()
                            .position,
                    )
                };

                let target_type = self.current_context().get_field_desc(field);

                if let StackValue::UnmanagedPtr(_) | StackValue::NativeInt(_) = parent {
                    push!(unmanaged_ptr(ptr))
                } else {
                    push!(managed_ptr(ptr, target_type, owner, pinned))
                }
            }
            LoadFieldSkipNullCheck(_) => todo!("no.nullcheck ldfld"),
            LoadLength => {
                vm_expect_stack!(let ObjectRef(obj) = pop!());
                let h = if let Some(h) = obj.0 {
                    h
                } else {
                    return self.throw_by_name(gc, "System.NullReferenceException");
                };
                let inner = h.borrow();
                let len = match &inner.storage {
                    HeapStorage::Vec(v) => v.layout.length as isize,
                    HeapStorage::Str(s) => s.len() as isize,
                    HeapStorage::Obj(o) => {
                        panic!("ldlen called on Obj: {:?}", o.description.type_name());
                    }
                    HeapStorage::Boxed(b) => {
                        panic!("ldlen called on Boxed value (expected Vec or Str): {:?}", b);
                    }
                };
                push!(NativeInt(len));
            }
            LoadObject {
                param0: load_type, ..
            } => {
                let addr = pop!();
                let source_ptr = addr.as_ptr();

                let ctx = self.current_context();
                let load_type = ctx.make_concrete(load_type);
                let layout = type_layout(load_type.clone(), &ctx);
                // SAFETY: source_ptr is a valid pointer to memory containing a value of the given type,
                // and layout.size() correctly represents the size of that type.
                let source = unsafe { slice::from_raw_parts(source_ptr, layout.size()) };
                let mut value = self
                    .current_context()
                    .read_cts_value(&load_type, source, gc)
                    .into_stack();

                // Recover ManagedPtr metadata from source side-table if the address is a ManagedPtr
                if let StackValue::ManagedPtr(src_mp) = &addr {
                    if let Some(owner) = src_mp.owner {
                        let base_addr = match owner {
                            ManagedPtrOwner::Heap(h) => {
                                let inner = h.borrow();
                                match &inner.storage {
                                    HeapStorage::Obj(o) => o.instance_storage.get().as_ptr(),
                                    HeapStorage::Vec(v) => v.get().as_ptr(),
                                    _ => ptr::null(),
                                }
                            }
                            ManagedPtrOwner::Stack(s) => unsafe {
                                s.as_ref().instance_storage.get().as_ptr()
                            },
                        };

                        if !base_addr.is_null() {
                            let src_offset = (source_ptr as usize).wrapping_sub(base_addr as usize);
                            let size = layout.size();

                            // Get metadata from owner
                            let metadata_map = match owner {
                                ManagedPtrOwner::Heap(h) => {
                                    let inner = h.borrow();
                                    match &inner.storage {
                                        HeapStorage::Obj(o) => {
                                            Some(o.managed_ptr_metadata.borrow().metadata.clone())
                                        }
                                        HeapStorage::Vec(v) => {
                                            Some(v.managed_ptr_metadata.borrow().metadata.clone())
                                        }
                                        _ => None,
                                    }
                                }
                                ManagedPtrOwner::Stack(s) => unsafe {
                                    Some(s.as_ref().managed_ptr_metadata.borrow().metadata.clone())
                                },
                            };

                            if let Some(map) = metadata_map {
                                match &mut value {
                                    StackValue::ValueType(obj) => {
                                        for (offset, meta) in map {
                                            if offset >= src_offset
                                                && offset < src_offset + size
                                            {
                                                let rel_offset = offset - src_offset;
                                                obj.register_metadata(rel_offset, meta, gc);
                                            }
                                        }
                                    }
                                    StackValue::ManagedPtr(mp) => {
                                        if let Some(meta) = map.get(&src_offset) {
                                            mp.owner = meta.recover_owner();
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                push!(value);
            }
            LoadStaticField {
                param0: source,
                volatile,
                ..
            } => {
                let (field, lookup) = self.locate_field(*source);
                let name = &field.field.name;

                if self.initialize_static_storage(gc, field.parent, lookup.clone()) {
                    return StepResult::InstructionStepped;
                }

                check_special_fields!(field, false);
                let ctx = self
                    .current_context()
                    .for_type_with_generics(field.parent, &lookup);

                let ordering = if *volatile {
                    AtomicOrdering::SeqCst
                } else {
                    AtomicOrdering::Acquire
                };

                // Thread-safe path: use GlobalState
                let value = {
                    let storage = self.statics().get(field.parent, &lookup);
                    let field_data = storage
                        .storage
                        .get_field_atomic(field.parent, name, ordering);
                    let t = ctx.make_concrete(&field.field.return_type);
                    ctx.read_cts_value(&t, &field_data, gc).into_stack()
                };

                vm_trace_field!(self, "LOAD_STATIC", name, &value);
                push!(value)
            }
            LoadStaticFieldAddress(source) => {
                let (field, lookup) = self.locate_field(*source);
                let name = &field.field.name;

                if self.initialize_static_storage(gc, field.parent, lookup.clone()) {
                    return StepResult::InstructionStepped;
                }

                let ctx = self
                    .current_context()
                    .for_type_with_generics(field.parent, &lookup);
                let field_type = ctx.get_field_desc(field);
                let value = statics!(|s| {
                    let storage = s.get(field.parent, &lookup);
                    let field_data = storage.storage.get_field_local(field.parent, name);
                    // Static fields are rooted by the assembly's static storage.
                    // ManagedPtr metadata for static fields would need its own side-table if supported.
                    StackValue::managed_ptr(field_data.as_ptr() as *mut _, field_type, None, true)
                });

                push!(value)
            }
            LoadString(cs) => {
                let obj = ObjectRef::new(gc, HeapStorage::Str(CLRString::new(cs.clone())));
                self.register_new_object(&obj);
                push!(ObjectRef(obj))
            }
            LoadTokenField(source) => {
                let (field, lookup) = self.current_context().locate_field(*source);
                let field_obj = self.get_runtime_field_obj(gc, field, lookup);

                let rfh = self.loader().corlib_type("System.RuntimeFieldHandle");
                let instance = self.current_context().new_object(rfh);
                field_obj.write(&mut instance.instance_storage.get_field_mut_local(rfh, "_value"));

                push!(ValueType(Box::new(instance)));
            }
            LoadTokenMethod(source) => {
                let (method, lookup) = self.find_generic_method(source);
                let method_obj = self.get_runtime_method_obj(gc, method, lookup);

                let rmh = self.loader().corlib_type("System.RuntimeMethodHandle");
                let instance = self.current_context().new_object(rmh);
                method_obj.write(&mut instance.instance_storage.get_field_mut_local(rmh, "_value"));

                push!(ValueType(Box::new(instance)));
            }
            LoadTokenType(target) => {
                let target_type = self.make_runtime_type(&self.current_context(), target);

                let instance = self.get_handle_for_type(gc, target_type);

                push!(ValueType(Box::new(instance)))
            }
            LoadVirtualMethodPointer { param0: source, .. } => {
                let (base_method, lookup) = self.find_generic_method(source);
                vm_expect_stack!(let ObjectRef(obj) = pop!());
                let ObjectRef(Some(o)) = obj else {
                    return self.throw_by_name(gc, "System.NullReferenceException");
                };
                let object_type = self.current_context().get_heap_description(o);
                let method = self.resolve_virtual_method(base_method, object_type, None);

                let idx = self.get_runtime_method_index(method, lookup);
                push!(NativeInt(idx as isize));
            }
            MakeTypedReference(_) => todo!("mkrefany"),
            NewArray(elem_type) => {
                let length = match pop!() {
                    StackValue::Int32(i) => {
                        if i < 0 {
                            return self.throw_by_name(gc, "System.OverflowException");
                        }
                        i as usize
                    }
                    StackValue::NativeInt(i) => {
                        if i < 0 {
                            return self.throw_by_name(gc, "System.OverflowException");
                        }
                        i as usize
                    }
                    rest => panic!(
                        "invalid length for newarr (expected int32 or native int, received {:?})",
                        rest
                    ),
                };

                // Check for GC safe point before large allocations
                // Threshold: arrays with > 1024 elements
                const LARGE_ARRAY_THRESHOLD: usize = 1024;
                if length > LARGE_ARRAY_THRESHOLD {
                    self.check_gc_safe_point();
                }

                let ctx = self.current_context();
                let elem_type = ctx.normalize_type(ctx.make_concrete(elem_type));

                let v = ctx.new_vector(elem_type, length);
                let o = ObjectRef::new(gc, HeapStorage::Vec(v));
                self.register_new_object(&o);
                push!(ObjectRef(o));
            }
            NewObject(ctor) => {
                let (mut method, lookup) = self.find_generic_method(&MethodSource::User(*ctor));
                let parent = method.parent;
                if let (None, Some(ts)) = (&method.method.body, &parent.definition().extends) {
                    let (ut, _) = decompose_type_source(ts);
                    let type_name = ut.type_name(parent.resolution.definition());
                    // delegate types are only allowed to have these base types
                    if matches!(
                        type_name.as_ref(),
                        "System.Delegate" | "System.MulticastDelegate"
                    ) {
                        let base = self.loader().corlib_type(&type_name);
                        method = MethodDescription {
                            parent: base,
                            method_resolution: base.resolution,
                            method: base
                                .definition()
                                .methods
                                .iter()
                                .find(|m| m.name == ".ctor")
                                .unwrap(),
                        };
                    }
                }

                let method_name = &*method.method.name;
                let parent_name = method.parent.definition().type_name();
                if parent_name == "System.IntPtr"
                    && method_name == ".ctor"
                    && method.method.signature.parameters.len() == 1
                {
                    vm_expect_stack!(let Int32(i) = pop!());
                    push!(NativeInt(i as isize));
                } else {
                    if is_intrinsic_cached(self, method) {
                        return intrinsic_call(gc, self, method, &lookup);
                    }

                    if self.initialize_static_storage(gc, parent, lookup.clone()) {
                        return StepResult::InstructionStepped;
                    }

                    let new_ctx = self
                        .current_context()
                        .for_type_with_generics(parent, &lookup);
                    let instance = new_ctx.new_object(parent);

                    self.constructor_frame(
                        gc,
                        instance,
                        MethodInfo::new(method, &lookup, self.shared.clone()),
                        lookup,
                    );
                    moved_ip = true;
                }
            }
            ReadTypedReferenceType => todo!("refanytype"),
            ReadTypedReferenceValue(_) => todo!("refanyval"),
            Rethrow => {
                let exception = self
                    .current_frame()
                    .exception_stack
                    .last()
                    .cloned()
                    .expect("rethrow without active exception");
                self.execution.exception_mode = ExceptionState::Throwing(exception);
                return self.handle_exception(gc);
            }
            Sizeof(t) => {
                let ctx = self.current_context();
                let target = ctx.make_concrete(t);
                let layout = type_layout(target, &ctx);
                push!(Int32(layout.size() as i32));
            }
            StoreElement { param0: source, .. } => {
                let value = pop!();
                let index_val = pop!();
                let index = match index_val {
                    StackValue::Int32(i) => i as usize,
                    StackValue::NativeInt(i) => i as usize,
                    rest => panic!("invalid index for stelem: {:?}", rest),
                };
                vm_expect_stack!(let ObjectRef(obj) = pop!());
                let ObjectRef(Some(heap)) = obj else {
                    return self.throw_by_name(gc, "System.NullReferenceException");
                };
                {
                    let inner = heap.borrow();
                    let HeapStorage::Vec(array) = &inner.storage else {
                        panic!("expected array for stelem, received {:?}", inner.storage)
                    };

                    if index >= array.layout.length {
                        drop(inner);
                        return self.throw_by_name(gc, "System.IndexOutOfRangeException");
                    }
                }
                let mut inner = heap.borrow_mut(gc);
                let HeapStorage::Vec(array) = &mut inner.storage else {
                    panic!("expected array for stelem, received {:?}", inner.storage)
                };

                let ctx = self.current_context();
                let store_type = ctx.make_concrete(source);
                let elem_size = array.layout.element_layout.size();

                let target = &mut array.get_mut()[(elem_size * index)..(elem_size * (index + 1))];
                let data = ctx.new_cts_value(&store_type, value);
                data.write(target);
            }
            StoreElementPrimitive {
                param0: store_type, ..
            } => {
                use StoreType::*;

                let value = pop!();
                let index = match pop!() {
                    StackValue::Int32(i) => i as usize,
                    StackValue::NativeInt(i) => i as usize,
                    rest => panic!(
                        "invalid index for stelem (expected int32 or native int, received {:?})",
                        rest
                    ),
                };
                let array = pop!();

                let mut data: Vec<u8> = match value {
                    StackValue::Int32(i) => i.to_ne_bytes().to_vec(),
                    StackValue::Int64(i) => i.to_ne_bytes().to_vec(),
                    StackValue::NativeInt(i) => i.to_ne_bytes().to_vec(),
                    StackValue::NativeFloat(f) => f.to_ne_bytes().to_vec(),
                    StackValue::ObjectRef(o) => {
                        let mut vec = vec![0; ObjectRef::SIZE];
                        o.write(&mut vec);
                        vec
                    }
                    StackValue::UnmanagedPtr(UnmanagedPtr(p)) => {
                        (p.as_ptr() as usize).to_ne_bytes().to_vec()
                    }
                    StackValue::ManagedPtr(m) => (m.value.map_or(0, |p| p.as_ptr() as usize))
                        .to_ne_bytes()
                        .to_vec(),
                    StackValue::ValueType(_) => {
                        panic!("received valuetype for StoreElementPrimitive")
                    }
                    #[cfg(feature = "multithreaded-gc")]
                    StackValue::CrossArenaObjectRef(ptr, _) => {
                        let mut vec = vec![0; ObjectRef::SIZE];
                        vec.copy_from_slice(&(ptr.as_ptr() as usize).to_ne_bytes());
                        vec
                    }
                };

                vm_expect_stack!(let ObjectRef(obj) = array);
                let ObjectRef(Some(heap)) = obj else {
                    return self.throw_by_name(gc, "System.NullReferenceException");
                };
                {
                    let inner = heap.borrow();
                    let HeapStorage::Vec(array) = &inner.storage else {
                        panic!("expected array for stelem, received {:?}", inner.storage)
                    };
                    if index >= array.layout.length {
                        drop(inner);
                        return self.throw_by_name(gc, "System.IndexOutOfRangeException");
                    }
                }
                let mut inner = heap.borrow_mut(gc);
                let HeapStorage::Vec(array) = &mut inner.storage else {
                    panic!("expected array for stelem, received {:?}", inner.storage)
                };

                let elem_size: usize = match store_type {
                    Int8 => 1,
                    Int16 => 2,
                    Int32 => 4,
                    Int64 => 8,
                    Float32 => 4,
                    Float64 => 8,
                    IntPtr | Object => ObjectRef::SIZE,
                };
                data.truncate(elem_size);

                let target = &mut array.get_mut()[(elem_size * index)..(elem_size * (index + 1))];
                target.copy_from_slice(&data);
            }
            StoreField {
                param0: source,
                volatile,
                ..
            } => {
                let value = pop!();
                let parent = pop!();

                let (field, lookup) = self.locate_field(*source);
                let ctx = self
                    .current_context()
                    .for_type_with_generics(field.parent, &lookup);

                let t = ctx.get_field_type(field);
                let name = &field.field.name;

                vm_trace_field!(self, "STORE", name, &value);

                let ordering = if *volatile {
                    AtomicOrdering::SeqCst
                } else {
                    AtomicOrdering::Release
                };

                let cts_value = ctx.new_cts_value(&t, value.clone());
                let write_data = |dest: &mut [u8]| cts_value.write(dest);
                let slice_from_pointer = |dest: *mut u8| {
                    let layout = LayoutFactory::instance_field_layout_cached(
                        field.parent,
                        &ctx,
                        Some(&self.shared.metrics),
                    );
                    let field_layout = layout.get_field(field.parent, name.as_ref()).unwrap();
                    unsafe {
                        slice::from_raw_parts_mut(
                            dest.add(field_layout.position),
                            field_layout.layout.size(),
                        )
                    }
                };

                let write_to_pointer_atomic = |ptr: *mut u8| {
                    let layout = LayoutFactory::instance_field_layout_cached(
                        field.parent,
                        &ctx,
                        Some(&self.shared.metrics),
                    );
                    let field_layout = layout.get_field(field.parent, name.as_ref()).unwrap();
                    let size = field_layout.layout.size();
                    let field_ptr = unsafe { ptr.add(field_layout.position) };

                    let mut val_bytes = vec![0u8; size];
                    write_data(&mut val_bytes);

                    if size <= size_of::<usize>() {
                        // Check alignment before attempting atomic operations
                        if !is_ptr_aligned_to_field(field_ptr, size) {
                            // Fall back to non-atomic write if not aligned
                            write_data(slice_from_pointer(ptr));
                            return;
                        }

                        macro_rules! store_atomic {
                            ($t:ty as $atomic_t:ty) => {{
                                let val = <$t>::from_ne_bytes(val_bytes.try_into().unwrap());
                                unsafe { (*(field_ptr as *const $atomic_t)).store(val, ordering) }
                            }};
                        }

                        match size {
                            1 => store_atomic!(u8 as AtomicU8),
                            2 => store_atomic!(u16 as AtomicU16),
                            4 => store_atomic!(u32 as AtomicU32),
                            8 => store_atomic!(u64 as AtomicU64),
                            _ => write_data(slice_from_pointer(ptr)),
                        }
                    } else {
                        write_data(slice_from_pointer(ptr));
                    }
                };

                match parent {
                    StackValue::ObjectRef(ObjectRef(None)) => {
                        return self.throw_by_name(gc, "System.NullReferenceException")
                    }
                    StackValue::ObjectRef(ObjectRef(Some(h))) => {
                        let object_type = ctx.get_heap_description(h);
                        if !ctx.is_a(object_type.into(), field.parent.into()) {
                            panic!(
                                "tried to store field {}::{} to object of type {}",
                                field.parent.type_name(),
                                name,
                                object_type.type_name()
                            )
                        }

                        let mut data = h.borrow_mut(gc);
                        match &mut data.storage {
                            HeapStorage::Vec(_) => todo!("field on array"),
                            HeapStorage::Obj(o) => {
                                let field_layout = o
                                    .instance_storage
                                    .layout()
                                    .get_field(field.parent, name.as_ref())
                                    .cloned()
                                    .unwrap_or_else(|| {
                                        panic!(
                                            "field {}::{} not found in layout for type {}",
                                            field.parent.type_name(),
                                            name,
                                            object_type.type_name()
                                        )
                                    });

                                // Register managed pointer metadata in side-table if needed
                                if let StackValue::ManagedPtr(m) = &value {
                                    o.register_managed_ptr(field_layout.position, m, gc);
                                }

                                let mut val_bytes = vec![0u8; field_layout.layout.size()];
                                write_data(&mut val_bytes);
                                o.instance_storage.set_field_atomic(
                                    field.parent,
                                    name,
                                    &val_bytes,
                                    ordering,
                                );
                            }
                            HeapStorage::Str(_) => todo!("field on string"),
                            HeapStorage::Boxed(_) => todo!("field on boxed value type"),
                        }
                    }
                    StackValue::NativeInt(i) => write_to_pointer_atomic(i as *mut u8),
                    StackValue::UnmanagedPtr(UnmanagedPtr(ptr)) => {
                        write_to_pointer_atomic(ptr.as_ptr())
                    }
                    StackValue::ManagedPtr(target_ptr) => {
                        if let StackValue::ManagedPtr(value_ptr) = &value {
                            if let Some(owner) = target_ptr.owner {
                                let layout = LayoutFactory::instance_field_layout_cached(
                                    field.parent,
                                    &ctx,
                                    Some(&self.shared.metrics),
                                );
                                let field_layout =
                                    layout.get_field(field.parent, name.as_ref()).unwrap();

                                let target_addr = target_ptr
                                    .value
                                    .expect("System.NullReferenceException")
                                    .as_ptr()
                                    as usize;
                                let final_addr = target_addr + field_layout.position;

                                match owner {
                                    ManagedPtrOwner::Heap(h) => {
                                        let borrow = h.borrow();
                                        if let Some(o) = borrow.storage.as_obj() {
                                            let base = o.instance_storage.get().as_ptr() as usize;
                                            let offset = final_addr - base;
                                            o.register_managed_ptr(offset, value_ptr, gc);
                                        }
                                    }
                                    ManagedPtrOwner::Stack(s) => {
                                        let o = unsafe { s.as_ref() };
                                        let base = o.instance_storage.get().as_ptr() as usize;
                                        let offset = final_addr - base;
                                        o.register_managed_ptr(offset, value_ptr, gc);
                                    }
                                }
                            }
                        }

                        write_to_pointer_atomic(
                            target_ptr
                                .value
                                .expect("System.NullReferenceException")
                                .as_ptr(),
                        )
                    }
                    rest => panic!(
                        "invalid type on stack (expected object or pointer, received {:?})",
                        rest
                    ),
                }
            }
            StoreFieldSkipNullCheck(_) => todo!("no.nullcheck stfld"),
            StoreObject { param0: t, .. } => {
                let value = pop!();
                let addr = pop!();

                let ctx = self.current_context();
                let concrete_t = ctx.make_concrete(t);

                let dest_ptr = match addr {
                    StackValue::NativeInt(i) => i as *mut u8,
                    StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr(),
                    StackValue::ManagedPtr(ManagedPtr { value: p, .. }) => {
                        p.expect("System.NullReferenceException").as_ptr()
                    }
                    _ => panic!("stobj: expected pointer on stack, got {:?}", addr),
                };

                let layout = self.type_layout_cached(concrete_t.clone());
                let mut bytes = vec![0u8; layout.size()];
                ctx.new_cts_value(&concrete_t, value.clone()).write(&mut bytes);

                unsafe {
                    ptr::copy_nonoverlapping(bytes.as_ptr(), dest_ptr, bytes.len());
                }

                // FIX: Propagate ManagedPtr metadata to destination side-table
                if let StackValue::ManagedPtr(dest_mp) = addr {
                    if let Some(owner) = dest_mp.owner {
                        let (base_addr, storage_size) = match owner {
                            ManagedPtrOwner::Heap(h) => {
                                let inner = h.borrow();
                                match &inner.storage {
                                    HeapStorage::Obj(o) => (o.instance_storage.get().as_ptr(), o.instance_storage.get().len()),
                                    HeapStorage::Vec(v) => (v.get().as_ptr(), v.get().len()),
                                    _ => (ptr::null(), 0),
                                }
                            }
                            ManagedPtrOwner::Stack(s) => unsafe {
                                (s.as_ref().instance_storage.get().as_ptr(), s.as_ref().instance_storage.get().len())
                            },
                        };

                        if !base_addr.is_null() {
                            let dest_offset = (dest_ptr as usize).wrapping_sub(base_addr as usize);
                            if dest_offset.checked_add(bytes.len()).is_none_or(|end| end > storage_size) {
                                panic!("Heap corruption detected! StoreObject writing {} bytes at offset {} into storage of size {}", bytes.len(), dest_offset, storage_size);
                            }

                            let src_metadata: Vec<(usize, ManagedPtrMetadata)> = match &value {
                                StackValue::ValueType(obj) => obj
                                    .managed_ptr_metadata
                                    .borrow()
                                    .metadata
                                    .iter()
                                    .map(|(k, v)| (*k, v.clone()))
                                    .collect(),
                                StackValue::ManagedPtr(mp) => {
                                    vec![(0, ManagedPtrMetadata::from_managed_ptr(mp))]
                                }
                                _ => vec![],
                            };

                            match owner {
                                ManagedPtrOwner::Heap(h) => {
                                    let mut inner = h.borrow_mut(gc);
                                    if let Some(o) = inner.storage.as_obj_mut() {
                                        for (offset, metadata) in src_metadata {
                                            o.register_metadata(dest_offset + offset, metadata, gc);
                                        }
                                    } else if let HeapStorage::Vec(v) = &mut inner.storage {
                                        for (offset, metadata) in src_metadata {
                                            v.register_metadata(dest_offset + offset, metadata, gc);
                                        }
                                    }
                                }
                                ManagedPtrOwner::Stack(mut s) => {
                                    let o = unsafe { s.as_mut() };
                                    for (offset, metadata) in src_metadata {
                                        o.register_metadata(dest_offset + offset, metadata, gc);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            StoreStaticField {
                param0: source,
                volatile,
                ..
            } => {
                let (field, lookup) = self.locate_field(*source);
                let name = &field.field.name;

                if self.initialize_static_storage(gc, field.parent, lookup.clone()) {
                    return StepResult::InstructionStepped;
                }

                let value = pop!();
                let ctx = self
                    .current_context()
                    .for_type_with_generics(field.parent, &lookup);

                let ordering = if *volatile {
                    AtomicOrdering::SeqCst
                } else {
                    AtomicOrdering::Release
                };

                // Thread-safe path: use GlobalState
                {
                    let storage = self.statics().get(field.parent, &lookup);
                    let t = ctx.make_concrete(&field.field.return_type);
                    vm_trace_field!(self, "STORE_STATIC", name, &value);

                    let layout = storage.layout();
                    let field_layout = layout.get_field(field.parent, name.as_ref()).unwrap();
                    let mut val_bytes = vec![0u8; field_layout.layout.size()];
                    ctx.new_cts_value(&t, value).write(&mut val_bytes);
                    storage
                        .storage
                        .set_field_atomic(field.parent, name, &val_bytes, ordering);
                }
            }
            Throw => {
                let exc = pop!();
                let StackValue::ObjectRef(exc) = exc else {
                    panic!(
                        "Throw expects an object reference on the stack, received {:?}",
                        exc
                    )
                };
                if exc.0.is_none() {
                    return self.throw_by_name(gc, "System.NullReferenceException");
                }
                self.execution.exception_mode = ExceptionState::Throwing(exc);
                return self.handle_exception(gc);
            }
            UnboxIntoAddress { .. } => todo!("unbox"),
            UnboxIntoValue(target) => {
                let val = pop!();
                let ctx = self.current_context();
                let target_ct = ctx.make_concrete(target);

                let is_vt = match target_ct.get() {
                    BaseType::Type { .. } => {
                        let td = self.loader().find_concrete_type(target_ct.clone());
                        td.is_value_type(&ctx)
                    }
                    BaseType::Vector(_, _)
                    | BaseType::Array(_, _)
                    | BaseType::Object
                    | BaseType::String => false,
                    _ => true, // Primitives, IntPtr, etc are value types
                };

                if is_vt {
                    // If it's a value type, unbox it.
                    let StackValue::ObjectRef(obj) = val else {
                        panic!("unbox.any: expected object on stack, got {:?}", val);
                    };
                    if obj.0.is_none() {
                        // unbox.any on null value type throws NullReferenceException (III.4.33)
                        return self.throw_by_name(gc, "System.NullReferenceException");
                    }

                    push!(obj.as_heap_storage(|storage| {
                        match storage {
                            HeapStorage::Boxed(v) => CTSValue::Value(v.clone()).into_stack(),
                            HeapStorage::Obj(o) => {
                                // Boxed struct is just an Object of that struct type.
                                StackValue::ValueType(Box::new(o.clone()))
                            }
                            _ => panic!("unbox.any: expected boxed value, got {:?}", storage),
                        }
                    }));
                } else {
                    // Reference type: identical to castclass.
                    let StackValue::ObjectRef(target_obj) = val else {
                        panic!("unbox.any: expected object on stack, got {:?}", val);
                    };
                    if let ObjectRef(Some(o)) = target_obj {
                        let obj_type = ctx.get_heap_description(o);
                        if ctx.is_a(obj_type.into(), target_ct) {
                            push!(ObjectRef(target_obj));
                        } else {
                            return self.throw_by_name(gc, "System.InvalidCastException");
                        }
                    } else {
                        push!(null());
                    }
                }
            }
        }
        if !moved_ip && initial_frame_count == self.execution.frames.len() {
            self.increment_ip();
        }
        StepResult::InstructionStepped
    }
}
