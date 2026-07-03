use crate::{
    BasePointer, ByteOffset, MethodInfo, ResolutionContext, StepResult,
    resolution::TypeResolutionExt,
    stack::{
        context::VesContext,
        ops::{
            EvalStackOps, LoaderOps, StaticsOps, TypedStackOps, VmCallOps, VmLoaderOps,
            VmResolutionOps,
        },
    },
};
use dotnet_types::{
    TypeDescription, error::TypeResolutionError, generics::GenericLookup,
    members::MethodDescription,
};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, Object as ObjectInstance, ObjectRef},
    pointer::ManagedPtr,
};
use dotnetdll::prelude::{
    BaseType, Instruction, MethodMemberIndex, MethodSource, MethodType, ParameterType,
};
use std::{
    fmt::Write as _,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering as AtomicOrdering},
    },
};

// invariant: VM internal state is inconsistent; continuing would be unsafe.
vm_cold_panic!(
    fn panic_not_enough_values_for_call(method: &MethodInfo<'static>, num_args: usize) =>
        "not enough values on stack for call: args={} in {:?}",
        num_args, method.source
);
// invariant: VM internal state is inconsistent; continuing would be unsafe.
vm_cold_panic!(
    fn panic_expected_boxed_value_type_in_unbox_canonicalization() =>
        "Expected boxed value type in unbox canonicalization"
);
// invariant: VM internal state is inconsistent; continuing would be unsafe.
vm_cold_panic!(
    fn panic_stack_height_underflow_for_call(
        frame_height: crate::StackSlotIndex,
        num_args: usize,
        source: &MethodDescription
    ) =>
        "Not enough values on stack for call: height={}, args={} in {:?}",
        frame_height, num_args, source
);
// invariant: VM internal state is inconsistent; continuing would be unsafe.
vm_cold_panic!(fn panic_tail_call_requires_current_frame() => "tail call requires a current frame");
// invariant: VM internal state is inconsistent; continuing would be unsafe.
vm_cold_panic!(fn panic_jmp_requires_current_frame() => "jmp requires a current frame");

#[cold]
#[inline(never)]
fn intrinsic_not_found_step_result(method: &MethodDescription) -> StepResult {
    StepResult::Error(
        crate::error::ExecutionError::NotImplemented(
            format!("intrinsic not found: {:?}", method).into(),
        )
        .into(),
    )
}

#[cold]
#[inline(never)]
fn no_body_in_executing_method_step_result(method: &MethodDescription) -> StepResult {
    StepResult::Error(
        crate::error::ExecutionError::NotImplemented(
            format!(
                "no body in executing method: {}.{}",
                method.parent.type_name(),
                method.method().name
            )
            .into(),
        )
        .into(),
    )
}

fn try_no_body_runtime_stub<'gc, T: EvalStackOps<'gc>>(
    ctx: &mut T,
    method: &MethodDescription,
) -> Option<StepResult> {
    if method.parent.type_name() == "System.Dynamic.Utils.DelegateHelpers"
        && method.method().name.as_ref()
            == "<CreateObjectArrayDelegateRefEmit>g__ForceAllowDynamicCode|19_1"
    {
        // This method is a compiler-generated UnsafeAccessor extern with no IL body.
        // dotnet-rs doesn't support DynamicMethod emission, so treat ForceAllowDynamicCode
        // scope creation as a no-op and return null IDisposable.
        let _ = ctx.pop();
        ctx.push(StackValue::ObjectRef(ObjectRef(None)));
        return Some(StepResult::Continue);
    }

    None
}

fn is_boolean_parameter(parameter: &ParameterType<MethodType>) -> bool {
    matches!(
        parameter,
        ParameterType::Value(MethodType::Base(base)) if matches!(&**base, BaseType::Boolean)
    )
}

fn try_redirect_expression_compile_to_interpreter(
    method: &MethodDescription,
) -> Option<MethodDescription> {
    let parent_name = method.parent.type_name();
    if parent_name != "System.Linq.Expressions.LambdaExpression"
        && parent_name != "System.Linq.Expressions.Expression`1"
    {
        return None;
    }

    if method.method().name.as_ref() != "Compile" {
        return None;
    }

    if !method.signature().parameters.is_empty() {
        return None;
    }

    method
        .parent
        .definition()
        .methods
        .iter()
        .enumerate()
        .find_map(|(index, m)| {
            if m.name.as_ref() != "Compile" {
                return None;
            }

            let candidate = MethodDescription::new(
                method.parent.clone(),
                method.parent_generics.clone(),
                method.method_resolution.clone(),
                MethodMemberIndex::Method(index),
            );

            let signature = candidate.signature();
            if signature.parameters.len() == 1 && is_boolean_parameter(&signature.parameters[0].1) {
                Some(candidate)
            } else {
                None
            }
        })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ResolvedMethodDispatchMode {
    VmContext,
    ExecutionEngineApi,
}

fn triage_call_sample_interval() -> Option<u64> {
    static INTERVAL: OnceLock<Option<u64>> = OnceLock::new();
    *INTERVAL.get_or_init(|| {
        let Ok(raw) = std::env::var("DOTNET_RS_TRIAGE_CALLS") else {
            return None;
        };
        let raw = raw.trim();
        if raw.is_empty() || raw == "0" || raw.eq_ignore_ascii_case("off") {
            return None;
        }
        if raw == "1" || raw.eq_ignore_ascii_case("true") {
            return Some(1_000);
        }
        raw.parse::<u64>().ok().filter(|interval| *interval > 0)
    })
}

fn trace_call_frame_sample(ctx: &VesContext<'_, '_>, method: &MethodInfo<'static>) {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    static FIND_NAV_CALLS: AtomicU64 = AtomicU64::new(0);

    let Some(interval) = triage_call_sample_interval() else {
        return;
    };

    let call = CALLS.fetch_add(1, AtomicOrdering::Relaxed) + 1;
    let target_type = method.source.parent.type_name();
    let target_method = method.source.method().name.as_ref();
    let find_nav_call = if target_type
        == "Microsoft.EntityFrameworkCore.Metadata.Internal.EntityType"
        && target_method == "FindNavigation"
        && std::env::var_os("DOTNET_RS_TRIAGE_FINDNAV").is_some()
    {
        FIND_NAV_CALLS.fetch_add(1, AtomicOrdering::Relaxed) + 1
    } else {
        0
    };
    let force_find_nav_sample = (1..=8).contains(&find_nav_call);

    if !force_find_nav_sample && call > 20 && !call.is_multiple_of(interval) {
        return;
    }

    let mut frames = String::new();
    let frame_count = ctx.frame_stack.frames.len();
    let frame_window = if force_find_nav_sample { 24 } else { 8 };
    let start = frame_count.saturating_sub(frame_window);
    for (idx, frame) in ctx.frame_stack.frames.iter().enumerate().skip(start) {
        let source = &frame.state.info_handle.source;
        let _ = writeln!(
            frames,
            "\n    #{idx} {}.{} ip={} stack_height={}",
            source.parent.type_name(),
            source.method().name,
            frame.state.ip,
            frame.stack_height.as_usize()
        );
    }

    eprintln!(
        "[CALL-TRIAGE] call={} find_nav_call={} depth_before={} eval_slots={} heap_live={} target={}.{}{}",
        call,
        find_nav_call,
        frame_count,
        ctx.evaluation_stack.stack.len(),
        ctx.local.heap.live_object_count(),
        target_type,
        target_method,
        frames
    );
}

const DEFAULT_MANAGED_FRAME_LIMIT: usize = 65_536;
const MANAGED_FRAME_ABORT_WINDOW: usize = 8;

fn managed_frame_limit() -> Option<usize> {
    static LIMIT: OnceLock<Option<usize>> = OnceLock::new();
    *LIMIT.get_or_init(|| {
        let Ok(raw) = std::env::var("DOTNET_RS_MANAGED_FRAME_LIMIT") else {
            return Some(DEFAULT_MANAGED_FRAME_LIMIT);
        };
        let raw = raw.trim();
        if raw.is_empty() || raw == "0" || raw.eq_ignore_ascii_case("off") {
            return None;
        }
        raw.parse::<usize>()
            .ok()
            .and_then(|limit| (limit > 0).then_some(limit))
            .or(Some(DEFAULT_MANAGED_FRAME_LIMIT))
    })
}

fn managed_frame_abort(
    ctx: &VesContext<'_, '_>,
    method: &MethodInfo<'static>,
) -> Option<crate::error::VmError> {
    let limit = managed_frame_limit()?;

    let depth_before = ctx.frame_stack.frames.len();
    let depth_after = depth_before.saturating_add(1);
    if depth_after <= limit {
        return None;
    }

    let mut top_frames = String::new();
    let start = depth_before.saturating_sub(MANAGED_FRAME_ABORT_WINDOW);
    for (idx, frame) in ctx.frame_stack.frames.iter().enumerate().skip(start) {
        let source = &frame.state.info_handle.source;
        let _ = writeln!(
            top_frames,
            "\n    #{idx} {}.{} ip={} stack_height={}",
            source.parent.type_name(),
            source.method().name,
            frame.state.ip,
            frame.stack_height.as_usize()
        );
    }

    let message = format!(
        "managed frame depth budget exceeded: limit={limit} depth_before={depth_before} depth_after={depth_after} eval_slots={} heap_live={} target={}.{} top_frames:{}",
        ctx.evaluation_stack.stack.len(),
        ctx.local.heap.live_object_count(),
        method.source.parent.type_name(),
        method.source.method().name,
        top_frames,
    );

    Some(crate::error::VmError::Execution(
        crate::error::ExecutionError::Aborted(message.into()),
    ))
}

impl<'a, 'gc> VmCallOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn return_frame(&mut self) -> StepResult {
        VesContext::return_frame(self)
    }

    #[inline]
    fn pop_call_args_into_buffer(&mut self, count: usize) {
        VesContext::pop_call_args_into_buffer(self, count);
    }

    #[inline]
    fn call_args_buffer_mut(&mut self) -> &mut Vec<StackValue<'gc>> {
        self.call_args_buffer
    }

    fn constructor_frame(
        &mut self,
        instance: ObjectInstance<'gc>,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
    ) -> Result<(), TypeResolutionError> {
        let gc = self.gc;
        let desc = instance.description.clone();

        let value = if desc.is_value_type(&self.current_context())? {
            StackValue::ValueType(instance)
        } else {
            let in_heap = ObjectRef::new(gc, HeapStorage::Obj(Box::new(instance)));
            self.register_new_object(&in_heap);
            StackValue::ObjectRef(in_heap)
        };

        let num_params = method.signature.parameters.len();
        self.pop_call_args_into_buffer(num_params);
        let mut args = std::mem::take(self.call_args_buffer);

        if desc.is_value_type(&self.current_context())? {
            self.push(value);
            let index = self.evaluation_stack.top_of_stack() - 1;
            let ptr = self.evaluation_stack.get_slot_address(index).as_ptr() as *mut _;
            self.push(StackValue::managed_stack_ptr(
                index,
                ByteOffset(0),
                ptr,
                desc,
                false,
            ));
        } else {
            self.push(value.clone());
            self.push(value);
        }

        for arg in args.drain(..) {
            self.push(arg);
        }
        *self.call_args_buffer = args;
        self.call_frame(method, generic_inst)
    }

    #[inline]
    fn call_frame(
        &mut self,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
    ) -> Result<(), TypeResolutionError> {
        let _gc = self.gc;
        trace_call_frame_sample(self, &method);
        self.trace_method_entry_for_call_frame(&method);

        let num_args = method.signature.instance as usize + method.signature.parameters.len();
        let Some(argument_base) = self.evaluation_stack.top_of_stack().checked_sub(num_args) else {
            panic_not_enough_values_for_call(&method, num_args);
        };

        let locals_base = self.evaluation_stack.top_of_stack();
        let local_slot_count = method.locals.len();
        self.evaluation_stack
            .reserve_slots(locals_base.as_usize() + local_slot_count + method.max_stack);
        let pinned_locals = self.init_locals(
            method.source.clone(),
            method.locals,
            &generic_inst,
            locals_base,
        )?;

        let stack_base = locals_base + pinned_locals.len();

        // Canonicalize 'this' for value type instance methods
        if method.signature.instance {
            let this_val = self.evaluation_stack.get_slot(argument_base);
            if let StackValue::ObjectRef(obj) = this_val {
                let td = method.source.parent.clone();
                if td.is_value_type(&self.current_context())? {
                    // Unbox this to a managed pointer. This is required when a virtual call
                    // on a boxed value type reaches a value type override.
                    let ptr = obj.as_heap_storage(|storage| match storage {
                        HeapStorage::Boxed(o) | HeapStorage::Obj(o) => unsafe {
                            o.instance_storage.raw_data_ptr()
                        },
                        _ => panic_expected_boxed_value_type_in_unbox_canonicalization(),
                    });
                    let managed_ptr = ManagedPtr::new(
                        std::ptr::NonNull::new(ptr),
                        td,
                        Some(obj),
                        false,
                        Some(ByteOffset(0)),
                    );
                    self.evaluation_stack
                        .set_slot_at(argument_base, StackValue::ManagedPtr(managed_ptr.into()));
                }
            }
        }

        if let Some(frame) = self.frame_stack.current_frame_opt_mut() {
            if frame.stack_height < crate::StackSlotIndex(num_args) {
                panic_stack_height_underflow_for_call(
                    frame.stack_height,
                    num_args,
                    &frame.state.info_handle.source,
                );
            }
            frame.stack_height -= num_args;
        }

        self.frame_stack.push_frame(
            BasePointer {
                arguments: argument_base,
                locals: locals_base,
                stack: stack_base,
            },
            method,
            generic_inst,
            pinned_locals,
        );
        Ok(())
    }

    #[inline]
    fn entrypoint_frame(
        &mut self,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
        args: Vec<StackValue<'gc>>,
    ) -> Result<(), TypeResolutionError> {
        let _gc = self.gc;
        let argument_base = self.evaluation_stack.top_of_stack();
        let arg_count = args.len();
        let local_slot_count = method.locals.len();
        self.evaluation_stack.reserve_slots(
            argument_base.as_usize() + arg_count + local_slot_count + method.max_stack,
        );
        for a in args {
            self.push(a);
        }
        let locals_base = self.evaluation_stack.top_of_stack();
        let pinned_locals = self.init_locals(
            method.source.clone(),
            method.locals,
            &generic_inst,
            locals_base,
        )?;
        let stack_base = locals_base + pinned_locals.len();

        self.frame_stack.push_frame(
            BasePointer {
                arguments: argument_base,
                locals: locals_base,
                stack: stack_base,
            },
            method,
            generic_inst,
            pinned_locals,
        );
        Ok(())
    }

    fn dispatch_method(&mut self, method: MethodDescription, lookup: GenericLookup) -> StepResult {
        let _gc = self.gc;

        // ECMA-335 §II.10.5.3.3: Types without beforefieldinit must be initialized on
        // static method calls, instance calls for value types, and constructor calls.
        if !method.parent.before_field_init() {
            let is_static = !method.signature().instance;
            let is_value_type = dotnet_vm_ops::vm_try!(
                method.parent.clone().is_value_type(&self.current_context())
            );
            let is_constructor = method.method().name == ".ctor";

            if is_static || is_value_type || is_constructor {
                let res = self.initialize_static_storage(method.parent.clone(), lookup.clone());
                if res != StepResult::Continue {
                    return res;
                }
            }
        }

        if let Some(compile_bool_overload) = try_redirect_expression_compile_to_interpreter(&method)
        {
            // dotnet-rs does not implement DynamicMethod emission yet.
            // Route parameterless Expression/Lambda Compile() through Compile(bool)
            // with preferInterpretation=true so System.Linq.Expressions uses the
            // interpreter path instead of Reflection.Emit.
            self.push_i32(1);
            return self.dispatch_method(compile_bool_overload, lookup);
        }

        self.dispatch_resolved_method(method, lookup)
    }

    #[inline]
    fn unified_dispatch(
        &mut self,
        source: &MethodSource,
        this_type: Option<TypeDescription>,
        ctx: Option<&ResolutionContext<'_>>,
    ) -> StepResult {
        self.unified_dispatch_common(
            source,
            this_type,
            ctx,
            "unified_dispatch",
            Self::dispatch_method,
        )
    }

    #[inline]
    fn unified_dispatch_tail(
        &mut self,
        source: &MethodSource,
        this_type: Option<TypeDescription>,
        ctx: Option<&ResolutionContext<'_>>,
    ) -> StepResult {
        self.unified_dispatch_common(
            source,
            this_type,
            ctx,
            "unified_dispatch_tail",
            Self::dispatch_method_tail,
        )
    }

    fn rebind_lookup_for_ldftn(
        &self,
        lookup: &mut GenericLookup,
        receiver: &ObjectRef<'gc>,
        resolved: &MethodDescription,
    ) {
        // Enrich type generics from the receiver object's instantiated generics, mirroring
        // the merge step done for callvirt in merge_receiver_lookup_for_virtual_dispatch.
        let receiver_lookup = if receiver.0.is_some() {
            receiver.as_heap_storage(|storage| match storage {
                HeapStorage::Obj(instance) => instance.generics.clone(),
                HeapStorage::Boxed(instance) => instance.generics.clone(),
                HeapStorage::Vec(_) | HeapStorage::Str(_) => GenericLookup::default(),
            })
        } else {
            GenericLookup::default()
        };
        if receiver_lookup.type_generics.len() > lookup.type_generics.len() {
            lookup.type_generics = receiver_lookup.type_generics;
        }
        // Rebind to the resolved declaring-type arity, mirroring rebind_lookup_to_resolved_method.
        self.rebind_lookup_to_resolved_method(lookup, resolved);
    }

    #[inline]
    fn unified_dispatch_jmp(
        &mut self,
        source: &MethodSource,
        ctx: Option<&ResolutionContext<'_>>,
    ) -> StepResult {
        let context = ctx.cloned().unwrap_or_else(|| self.current_context());

        tracing::debug!("unified_dispatch_jmp: source={:?}", source);

        let (resolved, lookup) = match self.resolver().find_generic_method(source, &context) {
            Ok(v) => v,
            Err(e) => return StepResult::Error(e.into()),
        };

        // jmp is always a direct call (non-virtual)
        self.dispatch_method_jmp(resolved, lookup)
    }
}

impl<'a, 'gc> VesContext<'a, 'gc> {
    pub(crate) fn managed_frame_abort_error(
        &self,
        method: &MethodInfo<'static>,
    ) -> Option<crate::error::VmError> {
        managed_frame_abort(self, method)
    }

    pub(crate) fn dispatch_resolved_method(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult {
        self.dispatch_resolved_method_with_mode(
            method,
            lookup,
            ResolvedMethodDispatchMode::VmContext,
        )
    }

    pub(crate) fn dispatch_resolved_method_for_engine_api(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult {
        self.dispatch_resolved_method_with_mode(
            method,
            lookup,
            ResolvedMethodDispatchMode::ExecutionEngineApi,
        )
    }

    fn dispatch_resolved_method_with_mode(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
        mode: ResolvedMethodDispatchMode,
    ) -> StepResult {
        let intrinsic_metadata = crate::intrinsics::classify_intrinsic(
            method.clone(),
            self.loader(),
            Some(&self.shared.caches.intrinsic_registry),
        );
        // Instrumented benchmarks for current workloads report intrinsic_call_total=0.
        if vm_unlikely!(intrinsic_metadata.is_some()) {
            return match mode {
                ResolvedMethodDispatchMode::VmContext => {
                    let metadata = intrinsic_metadata.expect("intrinsic metadata must exist");
                    crate::intrinsics::dispatch_method_intrinsic(
                        metadata.handler,
                        self,
                        method,
                        &lookup,
                    )
                }
                ResolvedMethodDispatchMode::ExecutionEngineApi => {
                    crate::intrinsics::intrinsic_call(self, method, &lookup)
                }
            };
        }

        if method.method().pinvoke.is_some() {
            let shared = self.shared.clone();
            return dotnet_pinvoke::external_call(self, method, &shared.pinvoke);
        }

        if method.body().is_none() {
            if mode == ResolvedMethodDispatchMode::ExecutionEngineApi
                && method.method().internal_call
            {
                return intrinsic_not_found_step_result(&method);
            }

            if let Some(result) = try_no_body_runtime_stub(self, &method) {
                return result;
            }

            if let Some(result) =
                dotnet_intrinsics_delegates::try_delegate_dispatch(self, method.clone(), &lookup)
            {
                return result;
            }

            return no_body_in_executing_method_step_result(&method);
        }

        let info = match self
            .shared
            .caches
            .get_method_info(method, &lookup, self.shared.clone())
        {
            Ok(v) => v,
            Err(e) => return StepResult::Error(e.into()),
        };
        if let Some(err) = self.managed_frame_abort_error(&info) {
            return StepResult::Error(err);
        }
        dotnet_vm_ops::vm_try!(self.call_frame(info, lookup));
        StepResult::FramePushed
    }

    #[inline]
    fn unified_dispatch_common(
        &mut self,
        source: &MethodSource,
        this_type: Option<TypeDescription>,
        ctx: Option<&ResolutionContext<'_>>,
        dispatch_label: &str,
        dispatch_fn: fn(&mut Self, MethodDescription, GenericLookup) -> StepResult,
    ) -> StepResult {
        let context = ctx.cloned().unwrap_or_else(|| self.current_context());

        tracing::debug!(
            "{}: source={:?}, this_type={:?}",
            dispatch_label,
            source,
            this_type
        );

        let (resolved, mut lookup) = match self.resolver().find_generic_method(source, &context) {
            Ok(v) => v,
            Err(e) => return StepResult::Error(e.into()),
        };

        if this_type.is_some() {
            self.merge_receiver_lookup_for_virtual_dispatch(&resolved, &mut lookup);
        }

        let had_receiver = this_type.is_some();
        let final_method = if let Some(this_type) = this_type {
            match self
                .resolver()
                .resolve_virtual_method(resolved, this_type, &lookup, &context)
            {
                Ok(v) => v,
                Err(e) => return StepResult::Error(e.into()),
            }
        } else {
            resolved
        };

        if had_receiver {
            self.rebind_lookup_to_resolved_method(&mut lookup, &final_method);
        }

        dispatch_fn(self, final_method, lookup)
    }

    fn rebind_lookup_to_resolved_method(
        &self,
        lookup: &mut GenericLookup,
        final_method: &MethodDescription,
    ) {
        let parent_arity = final_method.parent.definition().generic_parameters.len();
        if parent_arity == 0 {
            lookup.type_generics = Vec::new().into();
            return;
        }

        if final_method.parent_generics.type_generics.len() == parent_arity {
            lookup.type_generics = final_method.parent_generics.type_generics.clone();
            return;
        }

        // Fallback for edge cases where virtual resolution returns the right method but
        // an incomplete declaring-type lookup. Preserve receiver-instantiated generics,
        // trimmed to the declaring type's arity.
        lookup.type_generics = lookup
            .type_generics
            .iter()
            .take(parent_arity)
            .cloned()
            .collect::<Vec<_>>()
            .into();
    }

    fn merge_receiver_lookup_for_virtual_dispatch(
        &self,
        resolved: &MethodDescription,
        lookup: &mut GenericLookup,
    ) {
        // Virtual/interface calls can originate from non-generic call sites (e.g. IEnumerator)
        // while the receiver object itself is a closed generic type. Preserve that receiver
        // generic context for downstream field/type resolution when caller lookup is empty.
        let num_args = 1 + resolved.signature().parameters.len();
        let this_value = self.peek_stack_at(num_args - 1);

        let receiver_lookup = match this_value {
            StackValue::ObjectRef(ObjectRef(Some(obj))) => {
                ObjectRef(Some(obj)).as_heap_storage(|storage| match storage {
                    HeapStorage::Obj(instance) => instance.generics.clone(),
                    HeapStorage::Boxed(instance) => instance.generics.clone(),
                    HeapStorage::Vec(_) | HeapStorage::Str(_) => GenericLookup::default(),
                })
            }
            StackValue::ManagedPtr(ptr) => match ptr.origin().owner() {
                Some(owner) => owner.as_heap_storage(|storage| match storage {
                    HeapStorage::Obj(instance) => instance.generics.clone(),
                    HeapStorage::Boxed(instance) => instance.generics.clone(),
                    HeapStorage::Vec(_) | HeapStorage::Str(_) => GenericLookup::default(),
                }),
                None => GenericLookup::default(),
            },
            _ => GenericLookup::default(),
        };

        if !receiver_lookup.type_generics.is_empty() {
            // For instance/virtual dispatch the receiver's instantiated type arguments are
            // authoritative for `!0`, `!1`, ... on the target method's declaring type.
            // Preserve them even when the caller already has same-arity type generics from an
            // unrelated generic context (e.g. Expression<TDelegate>), otherwise `typeof(!0)` in
            // the callee can resolve against the caller and leak incorrect runtime types.
            lookup.type_generics = receiver_lookup.type_generics.clone();
        }
    }

    fn should_honor_tail_call(&self, arg_count: usize) -> bool {
        let frame = self.frame_stack.current_frame();

        // ECMA-335: tail. is ignored when exiting a synchronized method.
        if frame.state.info_handle.source.method().synchronized {
            return false;
        }

        // Avoid tail-call frame replacement for special frames that have deferred-return semantics
        // in `return_frame()` (cctor initialization, finalizer processing, multicast, etc.).
        if frame.state.info_handle.is_cctor
            || frame.is_finalizer
            || frame.multicast_state.is_some()
            || frame.awaiting_invoke_return.is_some()
        {
            return false;
        }

        // ECMA-335: tail. call must be immediately followed by `ret`.
        let ip = frame.state.ip;
        let instrs = frame.state.info_handle.instructions;
        if ip + 1 >= instrs.len() {
            return false;
        }
        if !matches!(instrs[ip + 1], Instruction::Return) {
            return false;
        }

        // ECMA-335: stack must be empty except for the call arguments.
        if frame.stack_height != crate::StackSlotIndex(arg_count) {
            return false;
        }

        // ECMA-335: cannot be used to transfer control out of try/filter/catch/finally blocks.
        for sec in frame.state.info_handle.exceptions.iter() {
            if sec.instructions.contains(&ip) {
                return false;
            }
            for handler in &sec.handlers {
                if handler.instructions.contains(&ip) {
                    return false;
                }
            }
        }

        true
    }

    fn dispatch_method_tail(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult {
        // If we can't safely tail-call, fall back to the regular call path.
        if let Some(metadata) = crate::intrinsics::classify_intrinsic(
            method.clone(),
            self.loader(),
            Some(&self.shared.caches.intrinsic_registry),
        ) {
            return crate::intrinsics::dispatch_method_intrinsic(
                metadata.handler,
                self,
                method,
                &lookup,
            );
        }
        if method.method().pinvoke.is_some() {
            let shared = self.shared.clone();
            return dotnet_pinvoke::external_call(self, method, &shared.pinvoke);
        }

        if method.body().is_none() {
            // Delegate dispatch may emulate a call without a managed body; do not attempt to
            // tail-call optimize it.
            return self.dispatch_method(method, lookup);
        }

        let info = match self
            .shared
            .caches
            .get_method_info(method, &lookup, self.shared.clone())
        {
            Ok(v) => v,
            Err(e) => return StepResult::Error(e.into()),
        };

        let arg_count = info.signature.instance as usize + info.signature.parameters.len();
        if !self.should_honor_tail_call(arg_count) {
            if let Some(err) = self.managed_frame_abort_error(&info) {
                return StepResult::Error(err);
            }
            dotnet_vm_ops::vm_try!(self.call_frame(info, lookup));
            return StepResult::FramePushed;
        }

        // Preserve the call arguments.
        let (args_base, clear_from, old_top) = {
            let frame = self.frame_stack.current_frame();
            (
                frame.base.stack,
                frame.base.arguments,
                self.evaluation_stack.top_of_stack(),
            )
        };
        self.copy_slots_into_call_args_buffer(args_base, arg_count);
        let mut args = std::mem::take(self.call_args_buffer);

        // Pop/discard the current frame and clear its stack slots.
        if !self.pop_and_recycle_frame_with_trace() {
            panic_tail_call_requires_current_frame();
        }

        for i in clear_from.as_usize()..old_top.as_usize() {
            self.evaluation_stack
                .set_slot(crate::StackSlotIndex(i), StackValue::null());
        }
        self.evaluation_stack.truncate(clear_from);

        // Re-push arguments onto the (now) caller stack, or directly onto the eval stack if this
        // was the last frame.
        if self.frame_stack.current_frame_opt_mut().is_some() {
            for a in args.drain(..) {
                self.push(a);
            }
        } else {
            for a in args.drain(..) {
                self.evaluation_stack.push(a);
            }
        }
        *self.call_args_buffer = args;

        if let Some(err) = self.managed_frame_abort_error(&info) {
            return StepResult::Error(err);
        }
        dotnet_vm_ops::vm_try!(self.call_frame(info, lookup));
        StepResult::FramePushed
    }

    fn dispatch_method_jmp(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult {
        let target_sig = method.signature();
        let (arg_count, args_base, clear_from) = {
            let frame = self.frame_stack.current_frame();

            // ECMA-335: evaluation stack shall be empty.
            if frame.stack_height != crate::StackSlotIndex(0) {
                return StepResult::Error(crate::error::VmError::Execution(
                    crate::error::ExecutionError::Aborted(
                        "jmp requires empty evaluation stack".into(),
                    ),
                ));
            }

            // ECMA-335: cannot be used to transfer control out of
            // try/filter/catch/fault/finally blocks.
            let ip = frame.state.ip;
            for sec in frame.state.info_handle.exceptions.iter() {
                if sec.instructions.contains(&ip) {
                    return StepResult::Error(crate::error::VmError::Execution(
                        crate::error::ExecutionError::Aborted(
                            "jmp out of try/catch/finally block".into(),
                        ),
                    ));
                }
                for handler in &sec.handlers {
                    if handler.instructions.contains(&ip) {
                        return StepResult::Error(crate::error::VmError::Execution(
                            crate::error::ExecutionError::Aborted(
                                "jmp out of exception handler".into(),
                            ),
                        ));
                    }
                }
            }

            // Signature matching check
            let current_sig = &frame.state.info_handle.signature;

            let loader = self.loader_arc();
            let comparer = dotnet_types::comparer::TypeComparer::new(loader.as_ref());
            let res_ctx = self.current_context();
            let res_s = &res_ctx.resolution;

            if !comparer.signatures_equal(
                res_s,
                current_sig,
                Some(res_ctx.generics), // Current generics
                res_s,
                target_sig,
                Some(&lookup), // Target generics
            ) {
                return StepResult::Error(crate::error::VmError::Execution(
                    crate::error::ExecutionError::Aborted("jmp signature mismatch".into()),
                ));
            }

            (
                target_sig.instance as usize + target_sig.parameters.len(),
                frame.base.arguments,
                frame.base.arguments,
            )
        };

        self.copy_slots_into_call_args_buffer(args_base, arg_count);
        let mut args = std::mem::take(self.call_args_buffer);

        // Discard the current frame and its locals/eval stack
        let old_top = self.evaluation_stack.top_of_stack();

        if !self.pop_and_recycle_frame_with_trace() {
            panic_jmp_requires_current_frame();
        }

        for i in clear_from.as_usize()..old_top.as_usize() {
            self.evaluation_stack
                .set_slot(crate::StackSlotIndex(i), StackValue::null());
        }
        self.evaluation_stack.truncate(clear_from);

        // Push arguments back onto the stack for the new call
        for a in args.drain(..) {
            self.push(a);
        }
        *self.call_args_buffer = args;

        // Push new frame
        let info = match self
            .shared
            .caches
            .get_method_info(method, &lookup, self.shared.clone())
        {
            Ok(v) => v,
            Err(e) => return StepResult::Error(e.into()),
        };

        if let Some(err) = self.managed_frame_abort_error(&info) {
            return StepResult::Error(err);
        }
        dotnet_vm_ops::vm_try!(self.call_frame(info, lookup));
        StepResult::FramePushed
    }
}
