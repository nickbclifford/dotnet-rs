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
//! - [`VmStackOps`]: VM-local stack operations (frame and slot access helpers).
//! - [`VmCallOps`]: VM-local frame management and method dispatch operations.
//! - [`ExceptionOps`]: Exception throwing and flow control (leave, endfinally).
//! - [`VmResolutionOps`]: VM-local type and method resolution services.
//! - [`RawMemoryOps`]: Low-level, unsafe memory access for unaligned reads/writes.
//! - [`VmReflectionOps`]: VM-local reflection and runtime type lookup operations.
//!
//! # Usage
//!
//! Handlers should typically take a generic parameter `T: VesOps<'gc> + ?Sized`.
//! This allows them to work with both `VesContext` and potentially other implementations
//! for testing or specialized execution.
//!
//! For the P3.S1 trait/call-site inventory and hot-path mapping, see
//! `docs/p3_s1_trait_inventory.md`.
use crate::{
    ByteOffset, MethodInfo, ResolutionContext, StackSlotIndex, StepResult,
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
    pointer::PointerOrigin,
};
use dotnetdll::prelude::{FieldSource, MethodSource, MethodType};

pub use dotnet_runtime_memory::ops::MemoryOps;
pub use dotnet_vm_ops::ops::{
    ArgumentOps, CallOps, EvalStackOps, ExceptionContext, ExceptionOps, LoaderOps, LocalOps,
    PInvokeContext, RawMemoryOps, ReflectionOps, ResolutionOps, SimdCapabilityOps,
    SimdIntrinsicHost as VmSimdIntrinsicHost, StackOps, StaticsOps, ThreadOps, TypedStackOps,
    VariableOps, VesBaseOps, VesInternals,
};

pub trait VmStackOps<'gc>: StackOps<'gc> {
    fn current_frame(&self) -> &StackFrame<'gc>;
    fn current_frame_mut(&mut self) -> &mut StackFrame<'gc>;

    fn get_slot(&self, index: StackSlotIndex) -> StackValue<'gc>;
    fn get_slot_ref(&self, index: StackSlotIndex) -> &StackValue<'gc>;
    fn set_slot(&mut self, index: StackSlotIndex, value: StackValue<'gc>);
    fn get_slot_address(&self, index: StackSlotIndex) -> std::ptr::NonNull<u8>;
    fn get_local_info_for_managed_ptr(
        &self,
        index: crate::LocalIndex,
    ) -> (std::ptr::NonNull<u8>, bool);
    fn get_argument_address(&self, index: crate::ArgumentIndex) -> std::ptr::NonNull<u8>;
}

pub trait VmRawMemoryOps<'gc>: RawMemoryOps<'gc> {
    fn resolve_address(
        &self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
    ) -> std::ptr::NonNull<u8>;

    fn localloc(&mut self, size: usize) -> *mut u8;
}

pub trait VmResolutionOps<'gc>: ResolutionOps<'gc> {
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

pub trait VmReflectionOps<'gc>:
    ReflectionOps<'gc> + IntrinsicDispatchOps<'gc> + ReflectionLookupOps<'gc> + MemoryOps<'gc>
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

pub trait VmLoaderOps: LoaderOps {
    fn resolver(&self) -> VmResolverService;
    fn shared(&self) -> &Arc<SharedGlobalState>;
}

pub trait VmStaticsOps<'gc>: StaticsOps<'gc> {
    fn statics(&self) -> &StaticStorageManager;
}

pub trait VmCallOps<'gc>: CallOps<'gc> {
    fn return_frame(&mut self) -> StepResult;

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

pub trait VmExceptionContext<'gc>:
    ExceptionContext<'gc>
    + TypedStackOps<'gc>
    + VmResolutionOps<'gc>
    + VmReflectionOps<'gc>
    + VmLoaderOps
    + MemoryOps<'gc>
    + VesInternals<'gc>
    + VesBaseOps
{
}

pub trait VmPInvokeContext<'gc>:
    PInvokeContext<'gc>
    + VmStackOps<'gc>
    + VmRawMemoryOps<'gc>
    + ExceptionOps<'gc>
    + VmResolutionOps<'gc>
    + VesInternals<'gc>
    + VmLoaderOps
    + MemoryOps<'gc>
    + VesBaseOps
{
}

pub trait VesOps<'gc>:
    PInvokeContext<'gc>
    + VmStackOps<'gc>
    + VmRawMemoryOps<'gc>
    + VmResolutionOps<'gc>
    + VmReflectionOps<'gc>
    + VmLoaderOps
    + VmStaticsOps<'gc>
    + ThreadOps
    + VmCallOps<'gc>
    + dotnet_intrinsics_string::IntrinsicStringHost<'gc>
    + dotnet_intrinsics_delegates::DelegateInvokeHost<'gc>
    + dotnet_intrinsics_reflection::ReflectionIntrinsicHost<'gc>
    + dotnet_intrinsics_simd::SimdIntrinsicHost<'gc>
    + dotnet_intrinsics_span::SpanIntrinsicHost<'gc>
    + dotnet_intrinsics_threading::ThreadingIntrinsicHost<'gc>
    + dotnet_intrinsics_unsafe::UnsafeIntrinsicHost<'gc>
{
    fn run(&mut self) -> StepResult;
    fn handle_return(&mut self) -> StepResult;
    fn handle_exception(&mut self) -> StepResult;
    fn process_pending_finalizers(&mut self) -> StepResult;
}
