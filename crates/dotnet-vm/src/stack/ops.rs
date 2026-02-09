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
//! - [`PoolOps`]: Access to VM pools and local memory allocation.
//! - [`RawMemoryOps`]: Low-level, unsafe memory access for unaligned reads/writes.
//! - [`ReflectionOps`]: Reflection-specific operations and runtime type information.
//!
//! # Usage
//!
//! Handlers should typically take a generic parameter `T: VesOps<'gc, 'm> + ?Sized`.
//! This allows them to work with both `VesContext` and potentially other implementations
//! for testing or specialized execution.
use crate::{
    memory::ops::MemoryOps,
    state::SharedGlobalState,
    sync::{Arc, MutexGuard},
    tracer::Tracer,
};
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    runtime::RuntimeType,
};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    CLRString, StackValue,
    object::{Object as ObjectInstance, ObjectHandle, ObjectRef},
    pointer::ManagedPtr,
};
use dotnetdll::prelude::{FieldSource, MethodType};

pub trait StackOps<'gc, 'm> {
    fn push(&mut self, gc: GCHandle<'gc>, value: StackValue<'gc>);
    fn push_i32(&mut self, gc: GCHandle<'gc>, value: i32);
    fn push_i64(&mut self, gc: GCHandle<'gc>, value: i64);
    fn push_f64(&mut self, gc: GCHandle<'gc>, value: f64);
    fn push_obj(&mut self, gc: GCHandle<'gc>, value: ObjectRef<'gc>);
    fn push_ptr(&mut self, gc: GCHandle<'gc>, ptr: *mut u8, t: TypeDescription, is_pinned: bool);
    fn push_isize(&mut self, gc: GCHandle<'gc>, value: isize);
    fn push_value_type(&mut self, gc: GCHandle<'gc>, value: ObjectInstance<'gc>);
    fn push_managed_ptr(&mut self, gc: GCHandle<'gc>, value: ManagedPtr<'gc>);
    fn push_string(&mut self, gc: GCHandle<'gc>, value: CLRString);

    #[must_use]
    fn pop(&mut self, gc: GCHandle<'gc>) -> StackValue<'gc>;
    #[must_use]
    fn pop_i32(&mut self, gc: GCHandle<'gc>) -> i32;
    #[must_use]
    fn pop_i64(&mut self, gc: GCHandle<'gc>) -> i64;
    #[must_use]
    fn pop_f64(&mut self, gc: GCHandle<'gc>) -> f64;
    #[must_use]
    fn pop_isize(&mut self, gc: GCHandle<'gc>) -> isize;
    #[must_use]
    fn pop_obj(&mut self, gc: GCHandle<'gc>) -> ObjectRef<'gc>;
    #[must_use]
    fn pop_ptr(&mut self, gc: GCHandle<'gc>) -> *mut u8;
    #[must_use]
    fn pop_value_type(&mut self, gc: GCHandle<'gc>) -> ObjectInstance<'gc>;
    #[must_use]
    fn pop_managed_ptr(&mut self, gc: GCHandle<'gc>) -> ManagedPtr<'gc>;
    #[must_use]
    fn pop_multiple(&mut self, gc: GCHandle<'gc>, count: usize) -> Vec<StackValue<'gc>>;
    #[must_use]
    fn peek_multiple(&self, count: usize) -> Vec<StackValue<'gc>>;

    fn dup(&mut self, gc: GCHandle<'gc>);
    fn peek(&self) -> Option<StackValue<'gc>>;
    fn peek_stack(&self) -> StackValue<'gc>;
    fn peek_stack_at(&self, offset: usize) -> StackValue<'gc>;

    fn get_local(&self, index: usize) -> StackValue<'gc>;
    fn set_local(&mut self, gc: GCHandle<'gc>, index: usize, value: StackValue<'gc>);
    fn get_argument(&self, index: usize) -> StackValue<'gc>;
    fn set_argument(&mut self, gc: GCHandle<'gc>, index: usize, value: StackValue<'gc>);

    fn get_local_address(&self, index: usize) -> std::ptr::NonNull<u8>;
    fn get_argument_address(&self, index: usize) -> std::ptr::NonNull<u8>;

    fn current_frame(&self) -> &crate::stack::StackFrame<'gc, 'm>;
    fn current_frame_mut(&mut self) -> &mut crate::stack::StackFrame<'gc, 'm>;
    fn get_local_info_for_managed_ptr(&self, index: usize) -> (std::ptr::NonNull<u8>, bool);

    fn get_slot(&self, index: usize) -> StackValue<'gc>;
    fn get_slot_ref(&self, index: usize) -> &StackValue<'gc>;
    fn set_slot(&mut self, gc: GCHandle<'gc>, index: usize, value: StackValue<'gc>);
    fn top_of_stack(&self) -> usize;
    fn get_slot_address(&self, index: usize) -> std::ptr::NonNull<u8>;
}

pub trait ExceptionOps<'gc> {
    fn throw_by_name(&mut self, gc: GCHandle<'gc>, name: &str) -> crate::StepResult;
    fn throw(&mut self, gc: GCHandle<'gc>, exception: ObjectRef<'gc>) -> crate::StepResult;
    fn rethrow(&mut self, gc: GCHandle<'gc>) -> crate::StepResult;
    fn leave(&mut self, gc: GCHandle<'gc>, target_ip: usize) -> crate::StepResult;
    fn endfinally(&mut self, gc: GCHandle<'gc>) -> crate::StepResult;
    fn endfilter(&mut self, gc: GCHandle<'gc>, result: i32) -> crate::StepResult;
    fn ret(&mut self, gc: GCHandle<'gc>) -> crate::StepResult;
}

pub trait ResolutionOps<'gc, 'm> {
    fn stack_value_type(&self, val: &StackValue<'gc>) -> TypeDescription;
    fn make_concrete(&self, t: &MethodType) -> ConcreteType;
    fn current_context(&self) -> crate::ResolutionContext<'_, 'm>;
    fn with_generics<'b>(&self, lookup: &'b GenericLookup) -> crate::ResolutionContext<'b, 'm>;
}

pub trait PoolOps {
    /// # Safety
    ///
    /// The returned pointer is valid for the duration of the current method frame.
    /// It must not be stored in a way that outlives the frame.
    fn localloc(&mut self, size: usize) -> *mut u8;
}

pub trait RawMemoryOps<'gc> {
    /// # Safety
    ///
    /// The caller must ensure that `ptr` is valid for writes and properly represents the memory location.
    /// If `owner` is provided, it must be the object that contains `ptr` to ensure GC safety.
    /// The `layout` must match the expected type of `value`.
    unsafe fn write_unaligned(
        &mut self,
        ptr: *mut u8,
        owner: Option<ObjectRef<'gc>>,
        value: StackValue<'gc>,
        layout: &dotnet_value::layout::LayoutManager,
    ) -> Result<(), String>;

    /// # Safety
    ///
    /// The caller must ensure that `ptr` is valid for reads and points to a memory location.
    /// If `owner` is provided, it must be the object that contains `ptr` to ensure GC safety.
    /// The `layout` must match the expected type stored at `ptr`.
    unsafe fn read_unaligned(
        &self,
        ptr: *const u8,
        owner: Option<ObjectRef<'gc>>,
        layout: &dotnet_value::layout::LayoutManager,
        type_desc: Option<TypeDescription>,
    ) -> Result<StackValue<'gc>, String>;

    fn check_gc_safe_point(&self);
}

pub trait ReflectionOps<'gc, 'm>: crate::memory::ops::MemoryOps<'gc> {
    fn pre_initialize_reflection(&mut self, gc: GCHandle<'gc>);
    fn get_runtime_method_index(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> usize;
    fn get_runtime_type(&mut self, gc: GCHandle<'gc>, target: RuntimeType) -> ObjectRef<'gc>;
    fn get_runtime_method_obj(
        &mut self,
        gc: GCHandle<'gc>,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> ObjectRef<'gc>;
    fn get_runtime_field_obj(
        &mut self,
        gc: GCHandle<'gc>,
        field: FieldDescription,
        lookup: GenericLookup,
    ) -> ObjectRef<'gc>;
    fn make_runtime_type(
        &self,
        ctx: &crate::ResolutionContext<'_, 'm>,
        source: &MethodType,
    ) -> RuntimeType;
    fn get_heap_description(&self, object: ObjectHandle<'gc>) -> TypeDescription;
    fn locate_field(&self, handle: FieldSource) -> (FieldDescription, GenericLookup);
    fn loader(&self) -> &'m dotnet_assemblies::AssemblyLoader;
    fn resolver(&self) -> crate::resolver::ResolverService<'m>;
    fn shared(&self) -> &Arc<SharedGlobalState<'m>>;
    fn statics(&self) -> &crate::state::StaticStorageManager;
    fn is_intrinsic_field_cached(&self, field: FieldDescription) -> bool;
    fn is_intrinsic_cached(&self, method: MethodDescription) -> bool;
    fn resolve_runtime_type(&self, obj: ObjectRef<'gc>) -> RuntimeType;
    fn resolve_runtime_method(&self, obj: ObjectRef<'gc>) -> (MethodDescription, GenericLookup);
    fn resolve_runtime_field(&self, obj: ObjectRef<'gc>) -> (FieldDescription, GenericLookup);
    fn lookup_method_by_index(&self, index: usize) -> (MethodDescription, GenericLookup);
    fn reflection(&self) -> crate::state::ReflectionRegistry<'_, 'gc>;
    fn thread_id(&self) -> usize;
    fn initialize_static_storage(
        &mut self,
        gc: GCHandle<'gc>,
        description: TypeDescription,
        generics: GenericLookup,
    ) -> crate::StepResult;
}

pub trait CallOps<'gc, 'm> {
    fn constructor_frame(
        &mut self,
        gc: GCHandle<'gc>,
        instance: ObjectInstance<'gc>,
        method: crate::MethodInfo<'m>,
        generic_inst: GenericLookup,
    );

    fn call_frame(
        &mut self,
        gc: GCHandle<'gc>,
        method: crate::MethodInfo<'m>,
        generic_inst: GenericLookup,
    );

    fn entrypoint_frame(
        &mut self,
        gc: GCHandle<'gc>,
        method: crate::MethodInfo<'m>,
        generic_inst: GenericLookup,
        args: Vec<StackValue<'gc>>,
    );

    fn dispatch_method(
        &mut self,
        gc: GCHandle<'gc>,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> crate::StepResult;

    fn unified_dispatch(
        &mut self,
        gc: GCHandle<'gc>,
        source: &dotnetdll::prelude::MethodSource,
        this_type: Option<TypeDescription>,
        ctx: Option<&crate::ResolutionContext<'_, 'm>>,
    ) -> crate::StepResult;
}

pub trait VesOps<'gc, 'm>:
    StackOps<'gc, 'm>
    + ExceptionOps<'gc>
    + ResolutionOps<'gc, 'm>
    + ReflectionOps<'gc, 'm>
    + CallOps<'gc, 'm>
    + MemoryOps<'gc>
    + PoolOps
    + RawMemoryOps<'gc>
{
    fn run(&mut self, gc: GCHandle<'gc>) -> crate::StepResult;
    fn handle_exception(&mut self, gc: GCHandle<'gc>) -> crate::StepResult;
    fn tracer_enabled(&self) -> bool;
    fn tracer(&self) -> MutexGuard<'_, Tracer>;
    fn indent(&self) -> usize;
    fn process_pending_finalizers(&mut self, gc: GCHandle<'gc>) -> crate::StepResult;
    fn back_up_ip(&mut self);
    fn branch(&mut self, target: usize);
    fn conditional_branch(&mut self, condition: bool, target: usize) -> bool;
    fn increment_ip(&mut self);
    fn state(&self) -> &crate::MethodState<'m>;
    fn state_mut(&mut self) -> &mut crate::MethodState<'m>;

    fn exception_mode(&self) -> &crate::exceptions::ExceptionState<'gc>;
    fn exception_mode_mut(&mut self) -> &mut crate::exceptions::ExceptionState<'gc>;
    fn evaluation_stack(&self) -> &crate::stack::evaluation_stack::EvaluationStack<'gc>;
    fn evaluation_stack_mut(&mut self)
    -> &mut crate::stack::evaluation_stack::EvaluationStack<'gc>;
    fn frame_stack(&self) -> &crate::stack::frames::FrameStack<'gc, 'm>;
    fn frame_stack_mut(&mut self) -> &mut crate::stack::frames::FrameStack<'gc, 'm>;
    fn original_ip(&self) -> usize;
    fn original_ip_mut(&mut self) -> &mut usize;
    fn original_stack_height(&self) -> usize;
    fn original_stack_height_mut(&mut self) -> &mut usize;

    fn unwind_frame(&mut self, gc: GCHandle<'gc>);
}
