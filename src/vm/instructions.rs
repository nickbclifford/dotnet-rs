use crate::{
    match_method,
    utils::decompose_type_source,
    value::{
        layout::{type_layout, FieldLayoutManager, HasLayout},
        string::CLRString,
        CTSValue, GenericLookup, HeapStorage, ManagedPtr, MethodDescription, Object, ObjectRef,
        ResolutionContext, StackValue, TypeDescription, UnmanagedPtr, ValueType, Vector,
    },
    vm::{
        exceptions::{ExceptionState, HandlerAddress, UnwindTarget},
        intrinsics::*,
        CallStack, GCHandle, MethodInfo, StepResult,
    },
};

use dotnetdll::prelude::*;
use std::cmp::Ordering;

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
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

        (ctx.locate_method(method, &new_lookup), new_lookup)
    }

    pub fn initialize_static_storage(
        &mut self,
        gc: GCHandle<'gc>,
        description: TypeDescription,
        generics: GenericLookup,
    ) -> bool {
        let ctx = ResolutionContext {
            resolution: description.resolution,
            generics: &generics,
            assemblies: self.runtime.assemblies,
            type_owner: Some(description),
            method_owner: None,
        };
        let value = {
            let mut statics = self.runtime.statics.borrow_mut();
            statics.init(description, &ctx)
        };
        if let Some(m) = value {
            vm_msg!(
                self,
                "-- calling static constructor (will return to ip {}) --",
                self.current_frame().state.ip
            );
            self.call_frame(gc, MethodInfo::new(m, &generics, self.runtime.assemblies), generics);
            true
        } else {
            false
        }
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
        use NumberSign::*;

        macro_rules! statics {
            (|$s:ident| $body:expr) => {{
                #[allow(unused_mut)]
                let mut $s = self.runtime.statics.borrow_mut();
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
                vm_pop!(self)
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
                state!(|s| s.ip = *$ip);
                moved_ip = true;
            }};
        }
        macro_rules! conditional_branch {
            ($condition:expr, $ip:expr) => {{
                if $condition {
                    branch!($ip);
                }
            }};
        }
        macro_rules! equal {
            () => {{
                use StackValue::*;
                let value2 = pop!();
                let value1 = pop!();
                match (value1, value2) {
                    (ManagedPtr(m), NativeInt(r)) => m.value as isize == r,
                    (NativeInt(l), ManagedPtr(m)) => l == m.value as isize,
                    (ObjectRef(l), ObjectRef(r)) => l == r,
                    (l, r) => l.partial_cmp(&r) == Some(Ordering::Equal),
                }
            }};
        }
        macro_rules! compare {
            ($sgn:expr, $op:tt ( $order:pat )) => {{
                use StackValue::*;
                let value2 = pop!();
                let value1 = pop!();
                match (value1, value2, $sgn) {
                    (Int32(l), Int32(r), Unsigned) => {
                        (l as u32) $op (r as u32)
                    }
                    (Int32(l), NativeInt(r), Unsigned) => {
                        (l as usize) $op (r as usize)
                    }
                    (Int64(l), Int64(r), Unsigned) => {
                        (l as u64) $op (r as u64)
                    }
                    (NativeInt(l), Int32(r), Unsigned) => {
                        (l as usize) $op (r as usize)
                    }
                    (NativeInt(l), NativeInt(r), Unsigned) => {
                        (l as usize) $op (r as usize)
                    }
                    (l, r, _) => {
                        matches!(l.partial_cmp(&r), Some($order))
                    }
                }
            }}
        }

        macro_rules! binary_arith_op {
            ($method:ident (f64 $op:tt) $(, { $($pat:pat => $arm:expr, )* } )?) => {{
                use StackValue::*;
                let value2 = pop!();
                let value1 = pop!();
                match (value1, value2) {
                    (Int32(i1), Int32(i2)) => {
                        push!(Int32(i1.$method(i2)))
                    }
                    (Int32(i1), NativeInt(i2)) => {
                        push!(NativeInt((i1 as isize).$method(i2)))
                    }
                    (Int64(i1), Int64(i2)) => {
                        push!(Int64(i1.$method(i2)))
                    }
                    (NativeInt(i1), Int32(i2)) => {
                        push!(NativeInt(i1.$method(i2 as isize)))
                    }
                    (NativeInt(i1), NativeInt(i2)) => {
                        push!(NativeInt(i1.$method(i2)))
                    }
                    (NativeFloat(f1), NativeFloat(f2)) => {
                        push!(NativeFloat(f1 $op f2))
                    }
                    $($($pat => $arm,)*)?
                    (v1, v2) => panic!(
                        "invalid types on stack ({:?}, {:?}) for {} operation",
                        v1,
                        v2,
                        stringify!($method)
                    ),
                }
            }};
            ($op:tt $(, { $($pat:pat => $arm:expr, )* })?) => {{
                use StackValue::*;
                let value2 = pop!();
                let value1 = pop!();
                match (value1, value2) {
                    (Int32(i1), Int32(i2)) => {
                        push!(Int32(i1 $op i2))
                    }
                    (Int32(i1), NativeInt(i2)) => {
                        push!(NativeInt((i1 as isize) $op i2))
                    }
                    (Int64(i1), Int64(i2)) => {
                        push!(Int64(i1 $op i2))
                    }
                    (NativeInt(i1), Int32(i2)) => {
                        push!(NativeInt(i1 $op (i2 as isize)))
                    }
                    (NativeInt(i1), NativeInt(i2)) => {
                        push!(NativeInt(i1 $op i2))
                    }
                    (NativeFloat(f1), NativeFloat(f2)) => {
                        push!(NativeFloat(f1 $op f2))
                    }
                    $($($pat => $arm,)*)?
                    (v1, v2) => panic!(
                        "invalid types on stack ({:?}, {:?}) for {} operation",
                        v1,
                        v2,
                        stringify!($op)
                    ),
                }
            }}
        }
        macro_rules! binary_checked_op {
            ($sign:expr, $method:ident (f64 $op:tt) $(, { $($pat:pat => $arm:expr, )* })?) => {{
                use StackValue::*;
                let value2 = pop!();
                let value1 = pop!();
                match (value1, value2, $sign) {
                    (Int32(i1), Int32(i2), Signed) => {
                        let Some(val) = i1.$method(i2) else { return self.throw_by_name(gc, "System.OverflowException") };
                        push!(Int32(val))
                    }
                    (Int32(i1), NativeInt(i2), Signed) => {
                        let Some(val) = (i1 as isize).$method(i2) else { return self.throw_by_name(gc, "System.OverflowException") };
                        push!(NativeInt(val))
                    }
                    (Int64(i1), Int64(i2), Signed) => {
                        let Some(val) = i1.$method(i2) else { return self.throw_by_name(gc, "System.OverflowException") };
                        push!(Int64(val));
                    }
                    (NativeInt(i1), Int32(i2), Signed) => {
                        let Some(val) = i1.$method(i2 as isize) else { return self.throw_by_name(gc, "System.OverflowException") };
                        push!(NativeInt(val));
                    }
                    (NativeInt(i1), NativeInt(i2), Signed) => {
                        let Some(val) = i1.$method(i2) else { return self.throw_by_name(gc, "System.OverflowException") };
                        push!(NativeInt(val));
                    }
                    (NativeFloat(f1), NativeFloat(f2), Signed) => {
                        push!(NativeFloat(f1 $op f2));
                    }
                    (Int32(i1), Int32(i2), Unsigned) => {
                        let Some(val) = (i1 as u32).$method(i2 as u32) else { return self.throw_by_name(gc, "System.OverflowException") };
                        push!(Int32(val as i32))
                    }
                    (Int32(i1), NativeInt(i2), Unsigned) => {
                        let Some(val) = (i1 as usize).$method(i2 as usize) else { return self.throw_by_name(gc, "System.OverflowException") };
                        push!(NativeInt(val as isize))
                    }
                    (Int64(i1), Int64(i2), Unsigned) => {
                        let Some(val) = (i1 as u64).$method(i2 as u64) else { return self.throw_by_name(gc, "System.OverflowException") };
                        push!(Int64(val as i64));
                    }
                    (NativeInt(i1), Int32(i2), Unsigned) => {
                        let Some(val) = i1.$method(i2 as isize) else { return self.throw_by_name(gc, "System.OverflowException") };
                        push!(NativeInt(val as isize));
                    }
                    (NativeInt(i1), NativeInt(i2), Unsigned) => {
                        let Some(val) = i1.$method(i2) else { return self.throw_by_name(gc, "System.OverflowException") };
                        push!(NativeInt(val as isize));
                    }
                    $($($pat => $arm,)*)?
                    (v1, v2, _) => panic!(
                        "invalid types on stack ({:?}, {:?}) for {} {:?} operation",
                        v1,
                        v2,
                        stringify!($method),
                        $sign
                    ),
                }
            }}
        }
        macro_rules! binary_int_op {
            ($op:tt) => {{
                use StackValue::*;
                let value2 = pop!();
                let value1 = pop!();
                match (value1, value2) {
                    (Int32(i1), Int32(i2)) => {
                        push!(Int32(i1 $op i2))
                    }
                    (Int32(i1), NativeInt(i2)) => {
                        push!(NativeInt((i1 as isize) $op i2))
                    }
                    (Int64(i1), Int64(i2)) => {
                        push!(Int64(i1 $op i2))
                    }
                    (NativeInt(i1), Int32(i2)) => {
                        push!(NativeInt(i1 $op (i2 as isize)))
                    }
                    (NativeInt(i1), NativeInt(i2)) => {
                        push!(NativeInt(i1 $op i2))
                    }
                    (v1, v2) => panic!(
                        "invalid types on stack ({:?}, {:?}) for {} operation",
                        v1,
                        v2,
                        stringify!($op)
                    ),
                }
            }};
            ($op:tt as unsigned) => {{
                use StackValue::*;
                let value2 = pop!();
                let value1 = pop!();
                match (value1, value2) {
                    (Int32(i1), Int32(i2)) => {
                        push!(Int32(((i1 as u32) $op (i2 as u32)) as i32))
                    }
                    (Int32(i1), NativeInt(i2)) => {
                        push!(NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
                    }
                    (Int64(i1), Int64(i2)) => {
                        push!(Int64(((i1 as u64) $op (i2 as u64)) as i64))
                    }
                    (NativeInt(i1), Int32(i2)) => {
                        push!(NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
                    }
                    (NativeInt(i1), NativeInt(i2)) => {
                        push!(NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
                    }
                    (v1, v2) => panic!(
                        "invalid types on stack ({:?}, {:?}) for {} unsigned operation",
                        v1,
                        v2,
                        stringify!($op)
                    ),
                }
            }}
        }
        macro_rules! shift_op {
            ($op:tt) => {{
                use StackValue::*;
                let value2 = pop!();
                let value1 = pop!();
                match (value1, value2) {
                    (Int32(i1), Int32(i2)) => {
                        push!(Int32(i1 $op i2))
                    }
                    (Int32(i1), NativeInt(i2)) => {
                        push!(Int32(i1 $op i2))
                    }
                    (Int64(i1), Int32(i2)) => {
                        push!(Int64(i1 $op i2))
                    }
                    (Int64(i1), NativeInt(i2)) => {
                        push!(Int64(i1 $op i2))
                    }
                    (NativeInt(i1), Int32(i2)) => {
                        push!(NativeInt(i1 $op i2))
                    }
                    (NativeInt(i1), NativeInt(i2)) => {
                        push!(NativeInt(i1 $op i2))
                    }
                    (v1, v2) => panic!(
                        "invalid types on stack ({:?}, {:?}) for {} operation",
                        v1,
                        v2,
                        stringify!($op)
                    ),
                }
            }};
            ($op:tt as unsigned) => {{
                use StackValue::*;
                let value2 = pop!();
                let value1 = pop!();
                match (value1, value2) {
                    (Int32(i1), Int32(i2)) => {
                        push!(Int32(((i1 as u32) $op (i2 as u32)) as i32))
                    }
                    (Int32(i1), NativeInt(i2)) => {
                        push!(Int32(((i1 as u32) $op (i2 as u32)) as i32))
                    }
                    (Int64(i1), Int32(i2)) => {
                        push!(Int64(((i1 as u64) $op (i2 as u64)) as i64))
                    }
                    (Int64(i1), NativeInt(i2)) => {
                        push!(Int64(((i1 as u64) $op (i2 as u64)) as i64))
                    }
                    (NativeInt(i1), Int32(i2)) => {
                        push!(NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
                    }
                    (NativeInt(i1), NativeInt(i2)) => {
                        push!(NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
                    }
                    (v1, v2) => panic!(
                        "invalid types on stack ({:?}, {:?}) for {} operation",
                        v1,
                        v2,
                        stringify!($op)
                    ),
                }
            }}
        }

        macro_rules! check_special_fields {
            ($field:ident) => {
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
                if is_intrinsic_field($field, self.runtime.assemblies) {
                    intrinsic_field(
                        gc,
                        self,
                        $field,
                        self.current_context().generics.type_generics.clone(),
                    );
                    self.increment_ip();
                    return StepResult::InstructionStepped;
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
                    StackValue::UnmanagedPtr(UnmanagedPtr(p))
                    | StackValue::ManagedPtr(ManagedPtr { value: p, .. }) => p.is_null(),
                    v => panic!("invalid type on stack ({:?}) for truthiness check", v),
                }
            };
        }

        let i = state!(|s| &s.info_handle.instructions[s.ip]);
        let ip = state!(|s| s.ip);
        let i_res = state!(|s| s.info_handle.source.resolution());

        vm_msg!(
            self,
            "[#{} | ip @ {}] {}",
            self.execution.frames.len() - 1,
            ip,
            i.show(i_res.0)
        );

        self.debug_dump();

        match i {
            Add => binary_arith_op!(wrapping_add (f64 +), {
                (Int32(i), ManagedPtr(m)) => {
                    // TODO: proper mechanisms for safety and pointer arithmetic
                    unsafe {
                        push!(ManagedPtr(m.map_value(|p| p.offset(i as isize))))
                    }
                },
                (NativeInt(i), ManagedPtr(m)) => unsafe {
                    push!(ManagedPtr(m.map_value(|p| p.offset(i))))
                },
                (ManagedPtr(m), Int32(i)) => unsafe {
                    push!(ManagedPtr(m.map_value(|p| p.offset(i as isize))))
                },
                (ManagedPtr(m), NativeInt(i)) => unsafe {
                    push!(ManagedPtr(m.map_value(|p| p.offset(i))))
                },
            }),
            AddOverflow(sgn) => binary_checked_op!(sgn, checked_add(f64+), {
                // TODO: pointer stuff
            }),
            And => binary_int_op!(&),
            ArgumentList => todo!("arglist"),
            BranchEqual(i) => {
                conditional_branch!(equal!(), i)
            }
            BranchGreaterOrEqual(sgn, i) => {
                conditional_branch!(compare!(sgn, >= (Ordering::Greater | Ordering::Equal)), i)
            }
            BranchGreater(sgn, i) => {
                conditional_branch!(compare!(sgn, > (Ordering::Greater)), i)
            }
            BranchLessOrEqual(sgn, i) => {
                conditional_branch!(compare!(sgn, <= (Ordering::Less | Ordering::Equal)), i)
            }
            BranchLess(sgn, i) => conditional_branch!(compare!(sgn, < (Ordering::Less)), i),
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
                let (method, lookup) = self.find_generic_method(source);

                if is_intrinsic(method, self.runtime.assemblies) {
                    let result = intrinsic_call(gc, self, method, lookup);
                    if let StepResult::InstructionStepped = result {
                        // If intrinsic created a new frame, prevent auto-increment (executor will handle it on return)
                        // If intrinsic completed inline, allow auto-increment at end of step()
                        if initial_frame_count != self.execution.frames.len() {
                            moved_ip = true;
                        }
                    } else {
                        return result;
                    }
                } else if method.method.pinvoke.is_some() {
                    self.external_call(method, gc);
                } else {
                    self.call_frame(
                        gc,
                        MethodInfo::new(method, &lookup, self.runtime.assemblies),
                        lookup,
                    );
                    moved_ip = true;
                }
            }
            CallConstrained(constraint, source) => {
                // according to the standard, this doesn't really make sense
                // because the constrained prefix should only be on callvirt
                // however, this appears to be used for static interface dispatch?

                let constraint_type = self.current_context().make_concrete(constraint);
                let (method, lookup) = self.find_generic_method(source);

                let td = self.runtime.assemblies.find_concrete_type(constraint_type.clone());

                for o in td.definition.overrides.iter() {
                    let target = self
                        .current_context()
                        .locate_method(o.implementation, &lookup);
                    let declaration = self.current_context().locate_method(o.declaration, &lookup);
                    if method == declaration {
                        vm_msg!(self, "-- dispatching to {:?} --", target);
                        if is_intrinsic(target, self.runtime.assemblies) {
                            let result = intrinsic_call(gc, self, target, lookup);
                            if let StepResult::InstructionStepped = result {
                                if initial_frame_count == self.execution.frames.len() {
                                    self.increment_ip();
                                }
                            }
                            return result;
                        } else if target.method.pinvoke.is_some() {
                            self.external_call(target, gc);
                            self.increment_ip();
                            return StepResult::InstructionStepped;
                        } else {
                            self.call_frame(
                                gc,
                                MethodInfo::new(target, &lookup, self.runtime.assemblies),
                                lookup,
                            );
                            return StepResult::InstructionStepped;
                        }
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
                let val = compare!(sgn, > (Ordering::Greater)) as i32;
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
                let val = compare!(sgn, < (Ordering::Less)) as i32;
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
                            StackValue::UnmanagedPtr(UnmanagedPtr(p)) |
                            StackValue::ManagedPtr(ManagedPtr { value: p, .. }) => (p as usize) as $t,
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
                let src = match pop!() {
                    StackValue::NativeInt(i) => i as *const u8,
                    StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p as *const u8,
                    StackValue::ManagedPtr(m) => m.value as *const u8,
                    rest => panic!(
                        "invalid type for src in cpblk (expected pointer, received {:?})",
                        rest
                    ),
                };
                let dest = match pop!() {
                    StackValue::NativeInt(i) => i as *mut u8,
                    StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p,
                    StackValue::ManagedPtr(m) => m.value,
                    rest => panic!(
                        "invalid type for dest in cpblk (expected pointer, received {:?})",
                        rest
                    ),
                };
                unsafe {
                    std::ptr::copy_nonoverlapping(src, dest, size);
                }
            }
            Divide(sgn) => {
                let v2 = pop!();
                let v1 = pop!();
                match &v2 {
                    StackValue::Int32(0) | StackValue::Int64(0) | StackValue::NativeInt(0) => {
                        return self.throw_by_name(gc, "System.DivideByZeroException");
                    }
                    _ => {}
                }
                push!(v1);
                push!(v2);
                match sgn {
                    Signed => binary_arith_op!(/),
                    Unsigned => binary_int_op!(/ as unsigned),
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
                self.execution.stack
                    .truncate(self.execution.frames[handler.frame_index].base.stack);
                self.execution.stack.append(&mut self.execution.suspended_stack);
                self.execution.frames.append(&mut self.execution.suspended_frames);

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
                _ => panic!("endfinally called outside of handler"),
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
                let addr = match pop!() {
                    StackValue::NativeInt(i) => i as *mut u8,
                    StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p,
                    StackValue::ManagedPtr(m) => m.value,
                    rest => panic!(
                        "invalid type for address in initblk (expected pointer, received {:?})",
                        rest
                    ),
                };
                unsafe {
                    std::ptr::write_bytes(addr, val, size);
                }
            }
            Jump(_) => todo!("jmp"),
            LoadArgument(i) => {
                let arg = self.get_argument(*i as usize);
                push!(arg);
            }
            LoadArgumentAddress(i) => {
                let arg = self.get_argument(*i as usize);
                let live_type = arg.contains_type(&self.current_context());
                push!(managed_ptr(
                    self.get_argument_address(*i as usize) as *mut _,
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

                macro_rules! load_as_i32 {
                    ($t:ty) => {{
                        let val: $t = unsafe { *(ptr as *const _) };
                        push!(Int32(val as i32));
                    }};
                }

                match t {
                    LoadType::Int8 => load_as_i32!(i8),
                    LoadType::UInt8 => load_as_i32!(u8),
                    LoadType::Int16 => load_as_i32!(i16),
                    LoadType::UInt16 => load_as_i32!(u16),
                    LoadType::Int32 => load_as_i32!(i32),
                    LoadType::UInt32 => load_as_i32!(u32),
                    LoadType::Int64 => {
                        let val: i64 = unsafe { *(ptr as *const _) };
                        push!(Int64(val));
                    }
                    LoadType::Float32 => {
                        let val: f32 = unsafe { *(ptr as *const _) };
                        push!(NativeFloat(val as f64));
                    }
                    LoadType::Float64 => {
                        let val: f64 = unsafe { *(ptr as *const _) };
                        push!(NativeFloat(val));
                    }
                    LoadType::IntPtr => {
                        let val: isize = unsafe { *(ptr as *const _) };
                        push!(NativeInt(val));
                    }
                    LoadType::Object => {
                        let val = unsafe { *(ptr as *const ObjectRef) };
                        push!(ObjectRef(val));
                    }
                }
            }
            LoadLocal(i) => {
                let local = self.get_local(*i as usize);
                push!(local);
            }
            LoadLocalAddress(i) => {
                let local = self.get_local(*i as usize);
                let live_type = local.contains_type(&self.current_context());
                let pinned = match state!(|s| &s.info_handle.locals[*i as usize]) {
                    LocalVariable::Variable { pinned, .. } => *pinned,
                    _ => false,
                };
                push!(managed_ptr(
                    self.get_local_address(*i as usize) as *mut _,
                    live_type,
                    None,
                    pinned
                ));
            }
            LoadNull => push!(null()),
            Leave(jump_target) => {
                self.execution.exception_mode = ExceptionState::Unwinding {
                    exception: None,
                    target: UnwindTarget::Instruction(*jump_target),
                    cursor: HandlerAddress {
                        frame_index: self.execution.frames.len() - 1,
                        section_index: 0,
                        handler_index: 0,
                    },
                };
                return self.handle_exception(gc);
            }
            LocalMemoryAllocate => {
                let size = match pop!() {
                    StackValue::Int32(i) => i as usize,
                    StackValue::NativeInt(i) => i as usize,
                    v => panic!(
                        "invalid type on stack ({:?}) for local memory allocation size",
                        v
                    ),
                };
                let ptr = state!(|s| {
                    let loc = s.memory_pool.len();
                    s.memory_pool.extend(vec![0; size]);
                    s.memory_pool[loc..].as_mut_ptr()
                });
                push!(unmanaged_ptr(ptr));
            }
            Multiply => binary_arith_op!(wrapping_mul (f64 *)),
            MultiplyOverflow(sgn) => binary_checked_op!(sgn, checked_mul (f64 *)),
            Negate => match pop!() {
                StackValue::Int32(i) => push!(Int32(-i)),
                StackValue::Int64(i) => push!(Int64(-i)),
                StackValue::NativeInt(i) => push!(NativeInt(-i)),
                v => panic!(
                    "invalid type on stack ({:?}) for logical NOT, expected integer",
                    v
                ),
            },
            NoOperation => {} // :3
            Not => match pop!() {
                StackValue::Int32(i) => push!(Int32(!i)),
                StackValue::Int64(i) => push!(Int64(!i)),
                StackValue::NativeInt(i) => push!(NativeInt(!i)),
                v => panic!(
                    "invalid type on stack ({:?}) for bitwise NOT, expected integer",
                    v
                ),
            },
            Or => binary_int_op!(|),
            Pop => {
                pop!();
            }
            Remainder(sgn) => {
                let v2 = pop!();
                let v1 = pop!();
                match &v2 {
                    StackValue::Int32(0) | StackValue::Int64(0) | StackValue::NativeInt(0) => {
                        return self.throw_by_name(gc, "System.DivideByZeroException");
                    }
                    _ => {}
                }
                push!(v1);
                push!(v2);
                match sgn {
                    Signed => binary_arith_op!(%),
                    Unsigned => binary_int_op!(% as unsigned),
                }
            }
            Return => {
                // expects a single value on stack for non-void methods
                // call stack manager will put this in the right spot for the caller
                return StepResult::MethodReturned;
            }
            ShiftLeft => shift_op!(<<),
            ShiftRight(sgn) => match sgn {
                Signed => shift_op!(>>),
                Unsigned => shift_op!(>> as unsigned),
            },
            StoreArgument(i) => {
                let val = pop!();
                self.set_argument(gc, *i as usize, val);
            }
            StoreIndirect {
                param0: store_type, ..
            } => {
                let val = pop!();
                let ptr = pop!().as_ptr();

                macro_rules! store_from_i32 {
                    ($t:ty) => {{
                        vm_expect_stack!(let Int32(v) = val);
                        let p = ptr as *mut $t;
                        unsafe {
                            *p = v as $t;
                        }
                    }};
                }

                match store_type {
                    StoreType::Int8 => store_from_i32!(i8),
                    StoreType::Int16 => store_from_i32!(i16),
                    StoreType::Int32 => store_from_i32!(i32),
                    StoreType::Int64 => {
                        vm_expect_stack!(let Int64(v) = val);
                        let p = ptr as *mut i64;
                        unsafe {
                            *p = v;
                        }
                    }
                    StoreType::Float32 => {
                        vm_expect_stack!(let NativeFloat(v) = val);
                        let p = ptr as *mut f32;
                        unsafe {
                            *p = v as f32;
                        }
                    }
                    StoreType::Float64 => {
                        vm_expect_stack!(let NativeFloat(v) = val);
                        let p = ptr as *mut f64;
                        unsafe {
                            *p = v;
                        }
                    }
                    StoreType::IntPtr => {
                        vm_expect_stack!(let NativeInt(v) = val);
                        let p = ptr as *mut isize;
                        unsafe {
                            *p = v;
                        }
                    }
                    StoreType::Object => {
                        vm_expect_stack!(let ObjectRef(o) = val);
                        let p = ptr as *mut ObjectRef;
                        unsafe {
                            *p = o;
                        }
                    }
                }
            }
            StoreLocal(i) => {
                let val = pop!();
                self.set_local(gc, *i as usize, val);
            }
            Subtract => binary_arith_op!(wrapping_sub (f64 -), {
                (ManagedPtr(m), Int32(i)) => unsafe {
                    push!(ManagedPtr(m.map_value(|p| p.offset(-i as isize))))
                },
                (ManagedPtr(m), NativeInt(i)) => unsafe {
                    push!(ManagedPtr(m.map_value(|p| p.offset(-i))))
                },
                (ManagedPtr(m1), ManagedPtr(m2)) => {
                    push!(NativeInt((m1.value as isize) - (m2.value as isize)))
                },
            }),
            SubtractOverflow(sgn) => binary_checked_op!(sgn, checked_sub (f64 -), {
                // TODO: pointer stuff
            }),
            Switch(targets) => {
                vm_expect_stack!(let Int32(value as usize) = pop!());
                conditional_branch!(value < targets.len(), &targets[value]);
            }
            Xor => binary_int_op!(^),
            BoxValue(t) => {
                let t = self.current_context().make_concrete(t);

                let value = pop!();

                if let StackValue::ObjectRef(_) = value {
                    // boxing is a noop for all reference types
                    push!(value);
                } else {
                    let obj = ObjectRef::new(
                        gc,
                        HeapStorage::Boxed(ValueType::new(&t, &self.current_context(), value)),
                    );
                    self.register_new_object(&obj);
                    push!(ObjectRef(obj));
                }
            }
            CallVirtual { param0: source, .. } => {
                let (base_method, lookup) = self.find_generic_method(source);

                let num_args = 1 + base_method.method.signature.parameters.len();
                let mut args = Vec::new();
                for _ in 0..num_args {
                    args.push(pop!());
                }
                args.reverse();

                // value types are passed as managed pointers (I.8.9.7)
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

                let method = self.resolve_virtual_method(base_method, this_type, None);

                for a in args {
                    push!(a);
                }
                if is_intrinsic(method, self.runtime.assemblies) {
                    let result = intrinsic_call(gc, self, method, lookup);
                    if let StepResult::InstructionStepped = result {
                        // If intrinsic created a new frame, prevent auto-increment (executor will handle it on return)
                        // If intrinsic completed inline, allow auto-increment at end of step()
                        if initial_frame_count != self.execution.frames.len() {
                            moved_ip = true;
                        }
                    } else {
                        return result;
                    }
                } else if method.method.pinvoke.is_some() {
                    self.external_call(method, gc);
                } else {
                    self.call_frame(
                        gc,
                        MethodInfo::new(method, &lookup, self.runtime.assemblies),
                        lookup,
                    );
                    moved_ip = true;
                }
            }
            CallVirtualConstrained(_, _) => todo!("constrained.callvirt"),
            CallVirtualTail(_) => todo!("tail.callvirt"),
            CastClass { param0: target, .. } => {
                vm_expect_stack!(let ObjectRef(target_obj) = pop!());
                if let ObjectRef(Some(o)) = target_obj {
                    let ctx = self.current_context();
                    let obj_type = ctx.get_heap_description(o);
                    let target_type = self
                        .runtime
                        .assemblies
                        .find_concrete_type(ctx.make_concrete(target));

                    if ctx.is_a(obj_type, target_type) {
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

                let s = unsafe { std::slice::from_raw_parts_mut(target, layout.size()) };
                s.fill(0);
            }
            IsInstance(target) => {
                vm_expect_stack!(let ObjectRef(target_obj) = pop!());
                if let ObjectRef(Some(o)) = target_obj {
                    let ctx = self.current_context();
                    let obj_type = ctx.get_heap_description(o);
                    let target_type = self
                        .runtime
                        .assemblies
                        .find_concrete_type(ctx.make_concrete(target));

                    if ctx.is_a(obj_type, target_type) {
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
                    Ok(CTSValue::read(&load_type, &ctx, target).into_stack())
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
                        Object => StackValue::ObjectRef(ObjectRef::read(target)),
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
                let ptr = match &*data {
                    HeapStorage::Vec(v) => {
                        if index >= v.layout.length {
                            panic!("IndexOutOfRangeException");
                        }
                        unsafe { v.get().as_ptr().add(index * element_layout.size()) as *mut u8 }
                    }
                    _ => panic!("ldelema on non-vector"),
                };
                let target_type = self.runtime.assemblies.find_concrete_type(concrete_t);
                push!(managed_ptr(ptr, target_type, Some(h), false));
            }
            LoadField {
                param0: source,
                volatile: _, // TODO
                ..
            } => {
                let parent = pop!();
                let (field, lookup) = self.locate_field(*source);

                check_special_fields!(field);

                let ctx = self
                    .current_context()
                    .for_type_with_generics(field.parent, &lookup);
                let name = &field.field.name;
                let t = ctx.get_field_type(field);

                let read_data = |d| CTSValue::read(&t, &ctx, d);
                let read_from_pointer = |ptr: *mut u8| {
                    let layout = FieldLayoutManager::instance_fields(field.parent, &ctx);
                    let field_layout = layout.fields.get(name.as_ref()).unwrap();
                    let slice = unsafe {
                        std::slice::from_raw_parts(
                            ptr.add(field_layout.position),
                            field_layout.layout.size(),
                        )
                    };
                    read_data(slice)
                };

                let value = match parent {
                    StackValue::ObjectRef(ObjectRef(None)) => {
                        return self.throw_by_name(gc, "System.NullReferenceException")
                    }
                    StackValue::ObjectRef(ObjectRef(Some(h))) => {
                        let object_type = self.current_context().get_heap_description(h);
                        if !self.current_context().is_a(object_type, field.parent) {
                            panic!(
                                "tried to load field {}::{} from object of type {}",
                                field.parent.type_name(),
                                name,
                                object_type.type_name()
                            )
                        }

                        let data = h.borrow();
                        match &*data {
                            HeapStorage::Vec(_) => todo!("field on array"),
                            HeapStorage::Obj(o) => read_data(o.instance_storage.get_field(name)),
                            HeapStorage::Str(_) => todo!("field on string"),
                            HeapStorage::Boxed(_) => todo!("field on boxed value type"),
                        }
                    }
                    StackValue::ValueType(ref o) => {
                        if !self.current_context().is_a(o.description, field.parent) {
                            panic!(
                                "tried to load field {}::{} from object of type {}",
                                field.parent.type_name(),
                                name,
                                o.description.type_name()
                            )
                        }
                        read_data(o.instance_storage.get_field(name))
                    }
                    StackValue::NativeInt(i) => read_from_pointer(i as *mut u8),
                    StackValue::UnmanagedPtr(UnmanagedPtr(ptr))
                    | StackValue::ManagedPtr(ManagedPtr { value: ptr, .. }) => {
                        read_from_pointer(ptr)
                    }
                    rest => panic!("stack value {:?} has no fields", rest),
                };

                push!(value.into_stack())
            }
            LoadFieldAddress(source) => {
                let (field, lookup) = self.locate_field(*source);
                let name = &field.field.name;
                let parent = pop!();
                let ctx = self
                    .current_context()
                    .for_type_with_generics(field.parent, &lookup);

                let (source_ptr, owner, pinned) = match parent {
                    StackValue::ObjectRef(ObjectRef(None)) => {
                        return self.throw_by_name(gc, "System.NullReferenceException")
                    }
                    StackValue::ObjectRef(ObjectRef(Some(h))) => {
                        let object_type = ctx.get_heap_description(h);
                        if !ctx.is_a(object_type, field.parent) {
                            panic!(
                                "tried to load field {}::{} from object of type {}",
                                field.parent.type_name(),
                                name,
                                object_type.type_name()
                            )
                        }

                        let data = h.borrow();
                        let ptr = match &*data {
                            HeapStorage::Vec(_) => todo!("field on array"),
                            HeapStorage::Obj(o) => o.instance_storage.get().as_ptr() as *mut u8,
                            HeapStorage::Str(_) => todo!("field on string"),
                            HeapStorage::Boxed(_) => todo!("field on boxed value type"),
                        };
                        (ptr, Some(h), false)
                    }
                    StackValue::NativeInt(i) => (i as *mut u8, None, false),
                    StackValue::UnmanagedPtr(UnmanagedPtr(ptr)) => (ptr, None, false),
                    StackValue::ManagedPtr(ManagedPtr {
                        value: ptr,
                        owner,
                        pinned,
                        ..
                    }) => (ptr, owner, pinned),
                    rest => panic!("cannot load field address from stack value {:?}", rest),
                };

                let layout =
                    FieldLayoutManager::instance_fields(field.parent, &self.current_context());
                let ptr =
                    unsafe { source_ptr.add(layout.fields.get(name.as_ref()).unwrap().position) };

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
                if obj.0.is_none() {
                    return self.throw_by_name(gc, "System.NullReferenceException");
                }
                let len = obj.as_vector(|a| a.layout.length as isize);
                push!(NativeInt(len));
            }
            LoadObject {
                param0: load_type, ..
            } => {
                let source_ptr = pop!().as_ptr();

                let ctx = self.current_context();
                let load_type = ctx.make_concrete(load_type);
                let layout = type_layout(load_type.clone(), &ctx);
                let source = unsafe { std::slice::from_raw_parts(source_ptr, layout.size()) };
                let value =
                    CTSValue::read(&load_type, &self.current_context(), source).into_stack();
                push!(value);
            }
            LoadStaticField { param0: source, .. } => {
                let (field, lookup) = self.locate_field(*source);
                let name = &field.field.name;

                if self.initialize_static_storage(gc, field.parent, lookup.clone()) {
                    return StepResult::InstructionStepped;
                }

                check_special_fields!(field);
                let ctx = self
                    .current_context()
                    .for_type_with_generics(field.parent, &lookup);
                let value = statics!(|s| {
                    let field_data = s.get(field.parent).get_field(name);
                    let t = ctx.make_concrete(&field.field.return_type);
                    CTSValue::read(&t, &ctx, field_data).into_stack()
                });
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
                    let field_data = s.get(field.parent).get_field(name);
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

                let rfh = self.runtime.assemblies.corlib_type("System.RuntimeFieldHandle");
                let mut instance = Object::new(rfh, &self.current_context());
                field_obj.write(instance.instance_storage.get_field_mut("_value"));

                push!(ValueType(Box::new(instance)));
            }
            LoadTokenMethod(source) => {
                let (method, lookup) = self.find_generic_method(source);
                let method_obj = self.get_runtime_method_obj(gc, method, lookup);

                let rmh = self.runtime.assemblies.corlib_type("System.RuntimeMethodHandle");
                let mut instance = Object::new(rmh, &self.current_context());
                method_obj.write(instance.instance_storage.get_field_mut("_value"));

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
                    StackValue::Int32(i) => i as usize,
                    StackValue::NativeInt(i) => i as usize,
                    rest => panic!(
                        "invalid length for newarr (expected int32 or native int, received {:?})",
                        rest
                    ),
                };
                let ctx = self.current_context();
                let elem_type = ctx.normalize_type(ctx.make_concrete(elem_type));

                let v = Vector::new(elem_type, length, &ctx);
                let o = ObjectRef::new(gc, HeapStorage::Vec(v));
                self.register_new_object(&o);
                push!(ObjectRef(o));
            }
            NewObject(ctor) => {
                let (mut method, lookup) = self.find_generic_method(&MethodSource::User(*ctor));
                let parent = method.parent;
                if let (None, Some(ts)) = (&method.method.body, &parent.definition.extends) {
                    let (ut, _) = decompose_type_source(ts);
                    let type_name = ut.type_name(parent.resolution.0);
                    // delegate types are only allowed to have these base types
                    if matches!(
                        type_name.as_ref(),
                        "System.Delegate" | "System.MulticastDelegate"
                    ) {
                        let base = self.runtime.assemblies.corlib_type(&type_name);
                        method = MethodDescription {
                            parent: base,
                            method: base
                                .definition
                                .methods
                                .iter()
                                .find(|m| m.name == ".ctor")
                                .unwrap(),
                        };
                    }
                }

                if match_method!(method, "System.IntPtr"::".ctor"(int)) {
                    vm_expect_stack!(let Int32(i) = pop!());
                    push!(NativeInt(i as isize));
                } else {
                    if self.initialize_static_storage(gc, parent, lookup.clone()) {
                        return StepResult::InstructionStepped;
                    }

                    let new_ctx = self
                        .current_context()
                        .for_type_with_generics(parent, &lookup);
                    let instance = Object::new(parent, &new_ctx);

                    self.constructor_frame(
                        gc,
                        instance,
                        MethodInfo::new(method, &lookup, self.runtime.assemblies),
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
                let mut heap = heap.borrow_mut(gc);
                let HeapStorage::Vec(array) = &mut *heap else {
                    panic!("expected array for stelem, received {:?}", heap)
                };

                if index >= array.layout.length {
                    return self.throw_by_name(gc, "System.IndexOutOfRangeException");
                }

                let ctx = self.current_context();
                let store_type = ctx.make_concrete(source);
                let elem_size = array.layout.element_layout.size();

                let target = &mut array.get_mut()[(elem_size * index)..(elem_size * (index + 1))];
                let data = CTSValue::new(&store_type, &ctx, value);
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
                        (p as usize).to_ne_bytes().to_vec()
                    }
                    StackValue::ManagedPtr(m) => (m.value as usize).to_ne_bytes().to_vec(),
                    StackValue::ValueType(_) => {
                        panic!("received valuetype for StoreElementPrimitive")
                    }
                };

                vm_expect_stack!(let ObjectRef(obj) = array);
                let ObjectRef(Some(heap)) = obj else {
                    return self.throw_by_name(gc, "System.NullReferenceException");
                };
                let mut heap = heap.borrow_mut(gc);
                let HeapStorage::Vec(array) = &mut *heap else {
                    panic!("expected array for stelem, received {:?}", heap)
                };

                if index >= array.layout.length {
                    return self.throw_by_name(gc, "System.IndexOutOfRangeException");
                }

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
                volatile: _, // TODO
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

                let write_data = |dest: &mut [u8]| CTSValue::new(&t, &ctx, value).write(dest);
                let slice_from_pointer = |dest: *mut u8| {
                    let layout = FieldLayoutManager::instance_fields(field.parent, &ctx);
                    let field_layout = layout.fields.get(name.as_ref()).unwrap();
                    unsafe {
                        std::slice::from_raw_parts_mut(
                            dest.add(field_layout.position),
                            field_layout.layout.size(),
                        )
                    }
                };

                match parent {
                    StackValue::ObjectRef(ObjectRef(None)) => {
                        return self.throw_by_name(gc, "System.NullReferenceException")
                    }
                    StackValue::ObjectRef(ObjectRef(Some(h))) => {
                        let object_type = ctx.get_heap_description(h);
                        if !ctx.is_a(object_type, field.parent) {
                            panic!(
                                "tried to store field {}::{} to object of type {}",
                                field.parent.type_name(),
                                name,
                                object_type.type_name()
                            )
                        }

                        let mut data = h.unlock(gc).borrow_mut();
                        match &mut *data {
                            HeapStorage::Vec(_) => todo!("field on array"),
                            HeapStorage::Obj(o) => {
                                write_data(o.instance_storage.get_field_mut(name))
                            }
                            HeapStorage::Str(_) => todo!("field on string"),
                            HeapStorage::Boxed(_) => todo!("field on boxed value type"),
                        }
                    }
                    StackValue::NativeInt(i) => write_data(slice_from_pointer(i as *mut u8)),
                    StackValue::UnmanagedPtr(UnmanagedPtr(ptr))
                    | StackValue::ManagedPtr(ManagedPtr { value: ptr, .. }) => {
                        write_data(slice_from_pointer(ptr))
                    }
                    rest => panic!(
                        "invalid type on stack (expected object or pointer, received {:?})",
                        rest
                    ),
                }
            }
            StoreFieldSkipNullCheck(_) => todo!("no.nullcheck stfld"),
            StoreObject { .. } => todo!("stobj"),
            StoreStaticField { param0: source, .. } => {
                let (field, lookup) = self.locate_field(*source);
                let name = &field.field.name;

                if self.initialize_static_storage(gc, field.parent, lookup.clone()) {
                    return StepResult::InstructionStepped;
                }

                let value = pop!();
                let ctx = self
                    .current_context()
                    .for_type_with_generics(field.parent, &lookup);
                statics!(|s| {
                    let field_data = s.get_mut(field.parent).get_field_mut(name);
                    let t = ctx.make_concrete(&field.field.return_type);
                    CTSValue::new(&t, &ctx, value).write(field_data);
                });
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
            UnboxIntoValue(_) => todo!("unbox.any"),
        }
        if !moved_ip && initial_frame_count == self.execution.frames.len() {
            self.increment_ip();
        }
        StepResult::InstructionStepped
    }
}
