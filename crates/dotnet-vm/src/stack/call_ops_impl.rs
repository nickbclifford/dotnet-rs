use crate::{
    BasePointer, ByteOffset, MethodInfo, ResolutionContext, StepResult,
    resolution::TypeResolutionExt,
    stack::{
        context::VesContext,
        ops::{
            ArgumentOps, CallArgumentBufferOps, EvalStackOps, ExceptionOps, LoaderOps,
            TypedStackOps, UnifiedDispatchTarget, VmCallOps, VmLoaderOps, VmResolutionOps,
        },
        receiver_object_generics,
    },
};
use dotnet_runtime_memory::ops::BaseMemoryOps;
use dotnet_types::{
    TypeDescription,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, Object as ObjectInstance, ObjectRef},
    pointer::ManagedPtr,
};
use dotnet_vm_data::frame_homes::FrameHomes;
use dotnet_vm_ops::NULL_REF_MSG;
use dotnetdll::prelude::{
    BaseType, Instruction, Kind, MethodMemberIndex, MethodSource, MethodType, ParameterType,
    TypeSource, UserType,
};
use std::{fmt::Write as _, sync::OnceLock};

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
vm_cold_panic!(fn panic_tail_call_requires_current_frame() => "tail call requires a current frame");
// invariant: VM internal state is inconsistent; continuing would be unsafe.
vm_cold_panic!(fn panic_jmp_requires_current_frame() => "jmp requires a current frame");

const INVALID_CAST_MSG: &str = "Specified cast is not valid.";

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

fn concrete_from_type_description_and_lookup(
    ty: &TypeDescription,
    lookup: &GenericLookup,
) -> ConcreteType {
    let arity = ty.definition().generic_parameters.len();
    if arity == 0 {
        return ty.clone().into();
    }

    let parameters: Vec<_> = lookup.type_generics.iter().take(arity).cloned().collect();
    if parameters.len() != arity {
        return ty.clone().into();
    }

    ConcreteType::new(
        ty.resolution.clone(),
        BaseType::Type {
            source: TypeSource::Generic {
                base: UserType::Definition(ty.index),
                parameters,
            },
            value_kind: None,
        },
    )
}

fn concrete_from_method_parent(method: &MethodDescription) -> ConcreteType {
    let arity = method.parent.definition().generic_parameters.len();
    if arity == 0 {
        return method.parent.clone().into();
    }

    let parameters: Vec<_> = method
        .parent_generics
        .type_generics
        .iter()
        .take(arity)
        .cloned()
        .collect();
    if parameters.len() != arity {
        return method.parent.clone().into();
    }

    ConcreteType::new(
        method.parent.resolution.clone(),
        BaseType::Type {
            source: TypeSource::Generic {
                base: UserType::Definition(method.parent.index),
                parameters,
            },
            value_kind: None,
        },
    )
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
pub(crate) enum ResolvedMethodDispatchMode {
    VmContext,
    InlineCall,
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

impl<'a, 'gc> CallArgumentBufferOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn pop_call_args_into_buffer(&mut self, count: usize) {
        VesContext::pop_call_args_into_buffer(self, count);
    }

    #[inline]
    fn call_args_buffer_mut(&mut self) -> &mut Vec<StackValue<'gc>> {
        &mut self.call_state.arguments
    }
}

/// Allocates the home block a frame of `method` needs before the frame is pushed, under a
/// fresh serial.
fn allocate_frame_homes(
    frames: &mut dotnet_vm_data::stack::FrameStack<'_>,
    method: &MethodInfo<'static>,
) -> Result<FrameHomes, TypeResolutionError> {
    let Some(layout) = &method.homes else {
        return Ok(FrameHomes::default());
    };
    FrameHomes::allocate(layout.clone(), frames.issue_home_serial()).map_err(|error| {
        TypeResolutionError::MassiveAllocation(
            format!(
                "local home block of {} bytes for {:?}: {error:?}",
                layout.size(),
                method.source
            )
            .into(),
        )
    })
}

impl<'a, 'gc> VmCallOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn return_frame(&mut self) -> StepResult {
        VesContext::return_frame(self)
    }

    fn enter_synchronized_method(&mut self) -> StepResult {
        VesContext::enter_synchronized_method(self)
    }

    fn prepare_constructor_receiver(
        &mut self,
        instance: ObjectInstance<'gc>,
    ) -> Result<
        (
            dotnet_vm_data::frame_homes::ConstructorResult,
            StackValue<'gc>,
        ),
        TypeResolutionError,
    > {
        use dotnet_vm_data::frame_homes::{ConstructorResult, FrameHomeLayout};
        let fields = instance.instance_storage.layout().clone();
        let layout = std::sync::Arc::new(FrameHomeLayout::new(
            (*fields).clone(),
            Box::new([]),
            Box::new([]),
        ));
        let serial = self.frame_stack.issue_home_serial();
        let frame = self.frame_stack.current_frame_mut();
        let address = frame
            .homes
            .allocate_temporary(layout, serial, &mut frame.state.allocations)
            .map_err(|error| {
                TypeResolutionError::MassiveAllocation(
                    format!("constructor receiver allocation failed: {error:?}").into(),
                )
            })?;
        instance.instance_storage.with_data(|bytes| {
            // SAFETY: F10.RawMemoryAccessValid — the new home has precisely this instance's
            // layout and size and does not overlap the source instance storage.
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), address.as_ptr(), bytes.len()) };
        });
        let result_slot = self.evaluation_stack.top_of_stack();
        let pointer = ManagedPtr::new_frame_home(
            address,
            instance.description.clone(),
            serial,
            ByteOffset::new(0),
            false,
        );
        let result = ConstructorResult {
            result_slot,
            serial,
            description: instance.description,
            generics: instance.generics,
            fields,
        };
        self.push(StackValue::null());
        Ok((result, StackValue::ManagedPtr(pointer.into())))
    }

    fn complete_constructor_receiver(
        &mut self,
        result: dotnet_vm_data::frame_homes::ConstructorResult,
    ) -> Result<(), TypeResolutionError> {
        use dotnet_value::storage::FieldStorage;
        let address = dotnet_value::pointer::frame_home_window(
            self,
            result.serial,
            0,
            result.fields.total_size,
        )
        .map_err(|error| {
            TypeResolutionError::InvalidLayout(
                format!("constructor receiver is no longer live: {error:?}").into(),
            )
        })?;
        let mut bytes = vec![0; result.fields.total_size];
        // SAFETY: F10.FrameHomeChecked — frame_home_window checked this complete range;
        // the fresh destination does not overlap the receiver and copying cannot safepoint.
        unsafe { std::ptr::copy_nonoverlapping(address.as_ptr(), bytes.as_mut_ptr(), bytes.len()) };
        let instance = ObjectInstance::new(
            result.description,
            result.generics,
            FieldStorage::new(result.fields, bytes),
        );
        self.evaluation_stack
            .set_slot(result.result_slot, StackValue::ValueType(instance));
        Ok(())
    }

    fn defer_native_constructor_receiver(
        &mut self,
        method: &MethodDescription,
        result: dotnet_vm_data::frame_homes::ConstructorResult,
    ) -> bool {
        if let Some(work) = self.call_state.native.last_mut()
            && work.constructor
            && work.method.0 == *method
            && work.caller_depth == self.frame_stack.len()
            && work.caller_ip == self.frame_stack.state().ip
        {
            assert!(work.constructor_result.is_none());
            work.constructor_result = Some(result);
            true
        } else {
            false
        }
    }

    fn defer_constructor_receiver(
        &mut self,
        result: dotnet_vm_data::frame_homes::ConstructorResult,
    ) {
        let frame = self.frame_stack.current_frame_mut();
        assert!(
            frame.constructor_result.is_none(),
            "constructor completion already installed"
        );
        frame.constructor_result = Some(Box::new(result));
    }

    fn constructor_frame(
        &mut self,
        instance: ObjectInstance<'gc>,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
    ) -> Result<(), TypeResolutionError> {
        let gc = self.gc;
        let desc = instance.description.clone();

        let is_value_type = desc.is_value_type(&self.current_context())?;
        let num_params = method.signature.parameters.len();
        self.pop_call_args_into_buffer(num_params);
        let mut args = std::mem::take(&mut self.call_state.arguments);
        let completion = if is_value_type {
            let (result, pointer) = self.prepare_constructor_receiver(instance)?;
            self.push(pointer);
            Some(result)
        } else {
            let in_heap = self.alloc_obj_ref(gc, instance);
            self.push(StackValue::ObjectRef(in_heap));
            self.push(StackValue::ObjectRef(in_heap));
            None
        };
        for arg in args.drain(..) {
            self.push(arg);
        }
        self.call_state.arguments = args;
        self.call_frame(method, generic_inst)?;
        if let Some(result) = completion {
            self.defer_constructor_receiver(result);
        }
        Ok(())
    }

    #[inline]
    fn call_frame(
        &mut self,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
    ) -> Result<(), TypeResolutionError> {
        let _gc = self.gc;
        // The CLR resolves type tokens while preparing a method, before its body and handlers
        // run. Validate them before modifying the caller's stack or installing the new frame.
        let resolution_context = ResolutionContext {
            generics: &generic_inst,
            state: self.shared.resolution_shared(),
            resolution: method.source.resolution(),
            type_owner: None,
            method_owner: None,
        };
        for instruction in method.instructions {
            if let Instruction::LoadTokenType(ty) = instruction
                && let Ok(concrete) = resolution_context.make_concrete(ty)
                && matches!(concrete.get(), BaseType::Type { .. })
                && matches!(
                    self.loader().find_concrete_type(concrete),
                    Err(TypeResolutionError::SynchronizedValueType)
                )
            {
                return Err(TypeResolutionError::SynchronizedValueType);
            }
        }
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
            &method.source,
            method.locals,
            method.homes.as_deref(),
            &generic_inst,
            locals_base,
        )?;
        let homes = allocate_frame_homes(self.frame_stack, &method)?;

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
                        // SAFETY: F1.GcHandleRooted — The active object borrow keeps the matched instance storage
                        // stable while its base address is captured into an owner-backed ManagedPtr.
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
                        Some(ByteOffset::new(0)),
                    );
                    self.evaluation_stack
                        .set_slot_at(argument_base, StackValue::ManagedPtr(managed_ptr.into()));
                }
            }
        }

        if let Some(frame) = self.frame_stack.current_frame_opt_mut() {
            frame.stack_height = frame.stack_height.checked_sub(num_args).expect(
                "call argument consumption exceeded current frame stack height: VM invariant violated",
            );
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
        self.install_frame_homes(homes)
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
            &method.source,
            method.locals,
            method.homes.as_deref(),
            &generic_inst,
            locals_base,
        )?;
        let homes = allocate_frame_homes(self.frame_stack, &method)?;
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
        self.install_frame_homes(homes)
    }

    fn dispatch_method(&mut self, method: MethodDescription, lookup: GenericLookup) -> StepResult {
        self.dispatch_method_with_mode(method, lookup, ResolvedMethodDispatchMode::VmContext)
    }

    fn dispatch_method_inline(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult {
        VesContext::dispatch_method_inline(self, method, lookup)
    }

    #[inline]
    fn unified_dispatch(
        &mut self,
        target: UnifiedDispatchTarget<'_>,
        this_type: Option<TypeDescription>,
        ctx: Option<&ResolutionContext<'_>>,
    ) -> StepResult {
        self.unified_dispatch_common(
            target,
            this_type,
            ctx,
            "unified_dispatch",
            Self::dispatch_method_inline,
        )
    }

    #[inline]
    fn unified_dispatch_tail(
        &mut self,
        target: UnifiedDispatchTarget<'_>,
        this_type: Option<TypeDescription>,
        ctx: Option<&ResolutionContext<'_>>,
    ) -> StepResult {
        self.unified_dispatch_common(
            target,
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
            receiver_object_generics(*receiver)
        } else {
            GenericLookup::default()
        };
        if receiver_lookup.type_generics.len() > lookup.type_generics.len() {
            lookup.set_type_generics(receiver_lookup.type_generics);
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
    pub(crate) fn dispatch_method_with_mode(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
        mode: ResolvedMethodDispatchMode,
    ) -> StepResult {
        let _gc = self.gc;

        let initialized = self.prepare_method_initialization(&method, &lookup, mode);
        if initialized != StepResult::Continue {
            return initialized;
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

        self.dispatch_resolved_method_with_mode(method, lookup, mode)
    }

    pub(crate) fn prepare_method_initialization(
        &mut self,
        method: &MethodDescription,
        lookup: &GenericLookup,
        mode: ResolvedMethodDispatchMode,
    ) -> StepResult {
        // ECMA-335 §II.10.5.3.3: initialize before static, value-type instance,
        // and constructor calls when the type is not beforefieldinit.
        if !method.parent.before_field_init() {
            let is_static = !method.signature().instance;
            let is_value_type = dotnet_vm_ops::vm_try!(
                method.parent.clone().is_value_type(&self.current_context())
            );
            if is_static || is_value_type || method.method().name == ".ctor" {
                // The initializer runs as an `ensure_initialized` effect; the dispatch resumes
                // from its operands once it completes.
                return self.initialize_type_then(
                    method.parent.clone(),
                    lookup,
                    super::native_effects::InitializerResume::Dispatch(
                        method.clone(),
                        lookup.clone(),
                        mode,
                    ),
                );
            }
        }
        StepResult::Continue
    }

    pub(crate) fn dispatch_method_inline(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult {
        self.dispatch_method_with_mode(method, lookup, ResolvedMethodDispatchMode::InlineCall)
    }

    pub(crate) fn managed_frame_abort_error(
        &self,
        method: &MethodInfo<'static>,
    ) -> Option<crate::error::VmError> {
        managed_frame_abort(self, method)
    }

    fn method_info_for_dispatch(
        &self,
        method: MethodDescription,
        lookup: &GenericLookup,
    ) -> Result<MethodInfo<'static>, crate::error::VmError> {
        self.shared
            .caches
            .get_method_info(method, lookup, self.shared.clone())
            .map_err(Into::into)
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
            let metadata = intrinsic_metadata.expect("intrinsic metadata must exist");
            return crate::intrinsics::dispatch_method_intrinsic(
                metadata.handler,
                self,
                method,
                &lookup,
                if mode == ResolvedMethodDispatchMode::InlineCall {
                    dotnet_vm_ops::effects::NativeDispatchMode::InlineCall
                } else {
                    dotnet_vm_ops::effects::NativeDispatchMode::Other
                },
            );
        }

        if method.method().pinvoke.is_some() {
            let shared = self.shared.clone();
            return dotnet_pinvoke::external_call(self, method, &shared.pinvoke);
        }

        if method.body().is_none() {
            if let Some(result) = try_no_body_runtime_stub(self, &method) {
                return result;
            }

            if mode == ResolvedMethodDispatchMode::InlineCall
                && let Some(result) = self.try_start_multicast(method.clone(), &lookup)
            {
                return result;
            }
            let delegate_result = if mode == ResolvedMethodDispatchMode::InlineCall {
                dotnet_intrinsics_delegates::helpers::try_delegate_dispatch_inline(
                    self,
                    method.clone(),
                    &lookup,
                )
            } else {
                dotnet_intrinsics_delegates::try_delegate_dispatch(self, method.clone(), &lookup)
            };
            if let Some(result) = delegate_result {
                return result;
            }

            // Some interface implementation bodies use `call` to reach an abstract
            // virtual/interface slot. Resolve those like a last-chance callvirt
            // instead of executing the declaration itself.
            if let Some(result) =
                self.try_dispatch_no_body_abstract_virtual(method.clone(), lookup.clone(), mode)
            {
                return result;
            }

            return no_body_in_executing_method_step_result(&method);
        }

        let info = dotnet_vm_ops::vm_try!(self.method_info_for_dispatch(method, &lookup));
        if let Some(err) = self.managed_frame_abort_error(&info) {
            return StepResult::Error(err);
        }
        dotnet_vm_ops::vm_try!(self.call_frame(info, lookup));
        StepResult::FramePushed
    }

    fn try_dispatch_no_body_abstract_virtual(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
        mode: ResolvedMethodDispatchMode,
    ) -> Option<StepResult> {
        if !method.signature().instance
            || !method.method().virtual_member
            || !method.method().abstract_member
        {
            return None;
        }

        let num_args = 1 + method.signature().parameters.len();
        let this_value = self.peek_stack_at(num_args - 1);
        let this_type = match this_value {
            StackValue::ObjectRef(ObjectRef(None)) => {
                self.drop_top(num_args);
                return Some(
                    self.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG),
                );
            }
            StackValue::ObjectRef(ObjectRef(Some(o))) => {
                match self.current_context().get_heap_description(o) {
                    Ok(v) => v,
                    Err(e) => return Some(StepResult::Error(e.into())),
                }
            }
            StackValue::ManagedPtr(m) => m.inner_type(),
            rest => {
                self.drop_top(num_args);
                return Some(StepResult::type_error(
                    "ObjectRef or ManagedPtr",
                    format!("{:?}", rest),
                ));
            }
        };

        let mut dispatch_lookup = lookup;
        self.merge_receiver_lookup_for_virtual_dispatch(&method, &mut dispatch_lookup);

        let resolved = match self.resolver().resolve_virtual_method(
            method.clone(),
            this_type.clone(),
            &dispatch_lookup,
            &self.current_context(),
        ) {
            Ok(v) => v,
            Err(e) => {
                if let Some(result) = self.try_interface_method_not_found_invalid_cast(
                    &method,
                    &this_type,
                    &dispatch_lookup,
                    &e,
                ) {
                    return Some(result);
                }
                return Some(StepResult::Error(e.into()));
            }
        };

        if resolved == method {
            return None;
        }

        self.rebind_lookup_to_resolved_method(&mut dispatch_lookup, &resolved);
        Some(self.dispatch_method_with_mode(
            resolved,
            dispatch_lookup,
            if mode == ResolvedMethodDispatchMode::InlineCall {
                ResolvedMethodDispatchMode::InlineCall
            } else {
                ResolvedMethodDispatchMode::VmContext
            },
        ))
    }

    fn try_interface_method_not_found_invalid_cast(
        &mut self,
        base_method: &MethodDescription,
        this_type: &TypeDescription,
        lookup: &GenericLookup,
        err: &TypeResolutionError,
    ) -> Option<StepResult> {
        if !matches!(err, TypeResolutionError::MethodNotFound(_)) {
            return None;
        }

        if !matches!(base_method.parent.definition().flags.kind, Kind::Interface) {
            return None;
        }

        let receiver_type = concrete_from_type_description_and_lookup(this_type, lookup);
        let interface_type = concrete_from_method_parent(base_method);
        if self
            .loader()
            .comparer()
            .is_assignable_to(&receiver_type, &interface_type)
        {
            return None;
        }

        Some(self.throw_by_name_with_message("System.InvalidCastException", INVALID_CAST_MSG))
    }

    #[inline]
    fn unified_dispatch_common(
        &mut self,
        target: UnifiedDispatchTarget<'_>,
        this_type: Option<TypeDescription>,
        ctx: Option<&ResolutionContext<'_>>,
        dispatch_label: &str,
        dispatch_fn: fn(&mut Self, MethodDescription, GenericLookup) -> StepResult,
    ) -> StepResult {
        let context = ctx.cloned().unwrap_or_else(|| self.current_context());

        tracing::debug!(
            "{}: pre_resolved={}, this_type={:?}",
            dispatch_label,
            matches!(&target, UnifiedDispatchTarget::Resolved(..)),
            this_type
        );

        let (resolved, mut lookup) = match target {
            UnifiedDispatchTarget::Source(source) => {
                match self.resolver().find_generic_method(source, &context) {
                    Ok(v) => v,
                    Err(e) => return StepResult::Error(e.into()),
                }
            }
            UnifiedDispatchTarget::Resolved(method, lookup) => (method, lookup),
        };

        if this_type.is_some() {
            self.merge_receiver_lookup_for_virtual_dispatch(&resolved, &mut lookup);
        }

        let had_receiver = this_type.is_some();
        let final_method = if let Some(this_type) = this_type {
            match self.resolver().resolve_virtual_method(
                resolved.clone(),
                this_type.clone(),
                &lookup,
                &context,
            ) {
                Ok(v) => v,
                Err(e) => {
                    if let Some(result) = self.try_interface_method_not_found_invalid_cast(
                        &resolved, &this_type, &lookup, &e,
                    ) {
                        return result;
                    }
                    return StepResult::Error(e.into());
                }
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
            lookup.set_type_generics(Vec::new().into());
            return;
        }

        if final_method.parent_generics.type_generics.len() == parent_arity {
            lookup.set_type_generics(final_method.parent_generics.type_generics.clone());
            return;
        }

        // Fallback for edge cases where virtual resolution returns the right method but
        // an incomplete declaring-type lookup. Preserve receiver-instantiated generics,
        // trimmed to the declaring type's arity.
        let type_generics = lookup
            .type_generics
            .iter()
            .take(parent_arity)
            .cloned()
            .collect::<Vec<_>>()
            .into();
        lookup.set_type_generics(type_generics);
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
                receiver_object_generics(ObjectRef(Some(obj)))
            }
            StackValue::ManagedPtr(ptr) => match ptr.origin().owner() {
                Some(owner) => receiver_object_generics(owner),
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
            lookup.set_type_generics(receiver_lookup.type_generics.clone());
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
            || frame.native_return.is_some()
            || frame.constructor_result.is_some()
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
        if frame.stack_height != crate::StackSlotIndex::new(arg_count) {
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
        if crate::intrinsics::classify_intrinsic(
            method.clone(),
            self.loader(),
            Some(&self.shared.caches.intrinsic_registry),
        )
        .is_some()
        {
            return self.dispatch_method_inline(method, lookup);
        }
        if method.method().pinvoke.is_some() {
            let shared = self.shared.clone();
            return dotnet_pinvoke::external_call(self, method, &shared.pinvoke);
        }

        if method.body().is_none() {
            // Delegate dispatch may emulate a call without a managed body; do not attempt to
            // tail-call optimize it.
            return self.dispatch_method_inline(method, lookup);
        }

        let info = dotnet_vm_ops::vm_try!(self.method_info_for_dispatch(method, &lookup));

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
        let mut args = std::mem::take(&mut self.call_state.arguments);

        // Pop/discard the current frame and clear its stack slots.
        if !self.pop_and_recycle_frame_with_trace() {
            panic_tail_call_requires_current_frame();
        }

        for i in clear_from.as_usize()..old_top.as_usize() {
            self.evaluation_stack
                .set_slot(crate::StackSlotIndex::new(i), StackValue::null());
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
        self.call_state.arguments = args;

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
        let (arg_count, clear_from) = {
            let frame = self.frame_stack.current_frame();

            // ECMA-335: evaluation stack shall be empty.
            if frame.stack_height != crate::StackSlotIndex::new(0) {
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
            )
        };

        // Argument homes are authoritative after starg or an indirect write.
        self.call_state.arguments.clear();
        for index in 0..arg_count {
            let value = self.get_argument(crate::ArgumentIndex::new(index));
            self.call_state.arguments.push(value);
        }
        let mut args = std::mem::take(&mut self.call_state.arguments);

        let constructor_result = self
            .frame_stack
            .current_frame_mut()
            .constructor_result
            .take();
        let native_return = self.frame_stack.current_frame_mut().native_return.take();

        // Discard the current frame and its locals/eval stack
        let old_top = self.evaluation_stack.top_of_stack();

        if !self.pop_and_recycle_frame_with_trace() {
            panic_jmp_requires_current_frame();
        }

        for i in clear_from.as_usize()..old_top.as_usize() {
            self.evaluation_stack
                .set_slot(crate::StackSlotIndex::new(i), StackValue::null());
        }
        self.evaluation_stack.truncate(clear_from);

        // Push arguments back onto the stack for the new call
        for a in args.drain(..) {
            self.push(a);
        }
        self.call_state.arguments = args;

        // Push new frame
        let info = dotnet_vm_ops::vm_try!(self.method_info_for_dispatch(method, &lookup));

        if let Some(err) = self.managed_frame_abort_error(&info) {
            return StepResult::Error(err);
        }
        dotnet_vm_ops::vm_try!(self.call_frame(info, lookup));
        self.frame_stack.current_frame_mut().constructor_result = constructor_result;
        self.frame_stack.current_frame_mut().native_return = native_return;
        StepResult::FramePushed
    }
}
