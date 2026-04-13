//! Virtual Execution System (VES) operations and trait definitions.
//!
//! This module defines the trait hierarchy used by instruction handlers and intrinsics
//! to interact with the VM state. This abstraction layer decouples the execution logic
//! from the concrete `VesContext` implementation.
//!
//! # Trait Hierarchy
//!
//! - [`VesOps`]: The unified trait that combines all other operational traits. This is the
//!   primary trait used by instruction handlers.
//! - [`StackOps`]: Operations for manipulating the evaluation stack (push, pop, peek, locals).
//! - [`CallOps`]: Frame management and method dispatch operations.
//! - [`ExceptionOps`]: Exception throwing and flow control (leave, endfinally).
//! - [`ResolutionOps`]: Type and method resolution services.
//! - [`RawMemoryOps`]: Low-level, unsafe memory access for unaligned reads/writes.
//! - [`ReflectionOps`]: Reflection-specific operations and runtime type information.
//!
//! # Usage
//!
//! Handlers should typically take a generic parameter `T: VesOps<'gc> + ?Sized`.
//! This allows them to work with both `VesContext` and potentially other implementations
//! for testing or specialized execution.
use crate::{
    MethodInfo, ResolutionContext, StackSlotIndex, StepResult,
    resolver::VmResolverService,
    stack::StackFrame,
    state::{ReflectionRegistry, SharedGlobalState, StaticStorageManager},
    sync::Arc,
};
use dotnet_types::{
    TypeDescription,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    runtime::RuntimeType,
};
use dotnet_value::{
    StackValue,
    object::{Object as ObjectInstance, ObjectRef},
};
use dotnetdll::prelude::{FieldSource, MethodSource, MethodType};

pub use dotnet_runtime_memory::ops::MemoryOps;
pub use dotnet_vm_ops::ops::{
    AllStackOps, ArgumentOps, CallOps as BaseCallOps, EvalStackOps,
    ExceptionContext as BaseExceptionContext, ExceptionOps, LoaderOps as BaseLoaderOps, LocalOps,
    MemoryOps as BaseMemoryOps, PInvokeContext as BasePInvokeContext, RawMemoryOps,
    ReflectionOps as BaseReflectionOps, ResolutionOps as BaseResolutionOps,
    StackOps as BaseStackOps, StaticsOps as BaseStaticsOps, ThreadOps, TypedStackOps, VariableOps,
    VesBaseOps, VesInternals,
};

pub trait StackOps<'gc>: BaseStackOps<'gc> + AllStackOps<'gc> {
    fn current_frame(&self) -> &StackFrame<'gc>;
    fn current_frame_mut(&mut self) -> &mut StackFrame<'gc>;

    fn get_slot(&self, index: StackSlotIndex) -> StackValue<'gc>;
    fn get_slot_ref(&self, index: StackSlotIndex) -> &StackValue<'gc>;
    fn set_slot(&mut self, index: StackSlotIndex, value: StackValue<'gc>);
    fn get_slot_address(&self, index: StackSlotIndex) -> std::ptr::NonNull<u8>;
}

pub trait ResolutionOps<'gc>: BaseResolutionOps<'gc> {
    fn current_context(&self) -> ResolutionContext<'_>;
    fn with_generics<'b>(&self, lookup: &'b GenericLookup) -> ResolutionContext<'b>;
}

pub trait IntrinsicDispatchOps<'gc> {
    fn is_intrinsic_field_cached(&self, field: FieldDescription) -> bool;
    fn is_intrinsic_cached(&self, method: MethodDescription) -> bool;
    fn execute_intrinsic_field(
        &mut self,
        field: FieldDescription,
        type_generics: Arc<[ConcreteType]>,
        is_address: bool,
    ) -> StepResult;
    fn execute_intrinsic_call(
        &mut self,
        method: MethodDescription,
        lookup: &GenericLookup,
    ) -> StepResult;
}

pub trait ReflectionLookupOps<'gc> {
    fn get_runtime_method_index(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> usize;
    fn get_runtime_type(&mut self, target: RuntimeType) -> ObjectRef<'gc>;
    fn get_runtime_method_obj(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> ObjectRef<'gc>;
    fn get_runtime_field_obj(
        &mut self,
        field: FieldDescription,
        lookup: GenericLookup,
    ) -> ObjectRef<'gc>;
}

pub trait ReflectionOps<'gc>:
    BaseReflectionOps<'gc> + IntrinsicDispatchOps<'gc> + ReflectionLookupOps<'gc> + MemoryOps<'gc>
{
    fn pre_initialize_reflection(&mut self);
    fn make_runtime_type(&self, ctx: &ResolutionContext<'_>, source: &MethodType) -> RuntimeType;
    fn locate_field(
        &self,
        handle: FieldSource,
    ) -> Result<(FieldDescription, GenericLookup), TypeResolutionError>;
    fn resolve_runtime_type(&self, obj: ObjectRef<'gc>) -> RuntimeType;
    fn resolve_runtime_method(&self, obj: ObjectRef<'gc>) -> (MethodDescription, GenericLookup);
    fn resolve_runtime_field(&self, obj: ObjectRef<'gc>) -> (FieldDescription, GenericLookup);
    fn lookup_method_by_index(&self, index: usize) -> (MethodDescription, GenericLookup);
    fn reflection(&self) -> ReflectionRegistry<'_, 'gc>;
}

pub trait LoaderOps: BaseLoaderOps {
    fn loader_arc(&self) -> Arc<dotnet_assemblies::AssemblyLoader>;
    fn resolver(&self) -> VmResolverService;
    fn shared(&self) -> &Arc<SharedGlobalState>;
}

pub trait StaticsOps<'gc>: BaseStaticsOps<'gc> {
    fn statics(&self) -> &StaticStorageManager;
}

pub trait CallOps<'gc>: BaseCallOps<'gc> {
    fn constructor_frame(
        &mut self,
        instance: ObjectInstance<'gc>,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
    ) -> Result<(), TypeResolutionError>;

    fn call_frame(
        &mut self,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
    ) -> Result<(), TypeResolutionError>;

    fn entrypoint_frame(
        &mut self,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
        args: Vec<StackValue<'gc>>,
    ) -> Result<(), TypeResolutionError>;

    fn dispatch_method(&mut self, method: MethodDescription, lookup: GenericLookup) -> StepResult;

    fn unified_dispatch(
        &mut self,
        source: &MethodSource,
        this_type: Option<TypeDescription>,
        ctx: Option<&ResolutionContext<'_>>,
    ) -> StepResult;

    fn unified_dispatch_tail(
        &mut self,
        source: &MethodSource,
        this_type: Option<TypeDescription>,
        ctx: Option<&ResolutionContext<'_>>,
    ) -> StepResult;

    fn unified_dispatch_jmp(
        &mut self,
        source: &MethodSource,
        ctx: Option<&ResolutionContext<'_>>,
    ) -> StepResult;
}

dotnet_vm_ops::trait_alias! {
    #[no_blanket_impl]
    pub trait ExceptionContext<'gc> =
        BaseExceptionContext<'gc>
        + TypedStackOps<'gc>
        + ResolutionOps<'gc>
        + ReflectionOps<'gc>
        + LoaderOps
        + MemoryOps<'gc>
        + VesInternals<'gc>
        + VesBaseOps;
}

pub trait PInvokeContext<'gc>:
    BasePInvokeContext<'gc>
    + StackOps<'gc>
    + RawMemoryOps<'gc>
    + ExceptionOps<'gc>
    + ResolutionOps<'gc>
    + VesInternals<'gc>
    + LoaderOps
    + MemoryOps<'gc>
    + VesBaseOps
{
}

pub trait VesOps<'gc>:
    ExceptionContext<'gc>
    + PInvokeContext<'gc>
    + StaticsOps<'gc>
    + ThreadOps
    + CallOps<'gc>
    + dotnet_intrinsics_string::IntrinsicStringHost<'gc>
    + dotnet_intrinsics_delegates::DelegateInvokeHost<'gc>
    + dotnet_intrinsics_reflection::ReflectionIntrinsicHost<'gc>
    + dotnet_intrinsics_span::SpanIntrinsicHost<'gc>
    + dotnet_intrinsics_threading::ThreadingIntrinsicHost<'gc>
    + dotnet_intrinsics_unsafe::UnsafeIntrinsicHost<'gc>
{
    fn run(&mut self) -> StepResult;
    fn handle_return(&mut self) -> StepResult;
    fn handle_exception(&mut self) -> StepResult;
    fn process_pending_finalizers(&mut self) -> StepResult;
}
