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
    state::SharedGlobalState,
    sync::Arc,
    tracer::Tracer,
};
use dotnet_types::{
    TypeDescription,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    runtime::RuntimeType,
};
use dotnet_utils::{BorrowScopeOps, ByteOffset};
use dotnet_value::{
    CLRString, StackValue,
    object::{Object as ObjectInstance, ObjectHandle, ObjectRef},
    pointer::{ManagedPtr, PointerOrigin},
};
use dotnetdll::prelude::{FieldSource, MethodType};

pub use crate::memory::ops::MemoryOps;

pub trait EvalStackOps<'gc> {
    fn push(&mut self, value: StackValue<'gc>);
    fn pop(&mut self) -> StackValue<'gc>;
    fn pop_safe(&mut self) -> Result<StackValue<'gc>, crate::error::VmError>;
    fn pop_multiple(&mut self, count: usize) -> Vec<StackValue<'gc>>;
    fn peek_multiple(&self, count: usize) -> Vec<StackValue<'gc>>;
    fn dup(&mut self);
    fn peek(&self) -> Option<StackValue<'gc>>;
    fn peek_stack(&self) -> StackValue<'gc>;
    fn peek_stack_at(&self, offset: usize) -> StackValue<'gc>;
    fn top_of_stack(&self) -> crate::StackSlotIndex;
}

pub trait TypedStackOps<'gc>: EvalStackOps<'gc> {
    fn push_i32(&mut self, value: i32) {
        self.push(StackValue::Int32(value));
    }
    fn push_i64(&mut self, value: i64) {
        self.push(StackValue::Int64(value));
    }
    fn push_f64(&mut self, value: f64) {
        self.push(StackValue::NativeFloat(value));
    }
    fn push_obj(&mut self, value: ObjectRef<'gc>) {
        self.push(StackValue::ObjectRef(value));
    }
    fn push_ptr(
        &mut self,
        ptr: *mut u8,
        t: TypeDescription,
        is_pinned: bool,
        owner: Option<ObjectRef<'gc>>,
        offset: Option<ByteOffset>,
    ) {
        self.push(StackValue::managed_ptr_with_owner(
            ptr, t, owner, is_pinned, offset,
        ));
    }
    fn push_isize(&mut self, value: isize) {
        self.push(StackValue::NativeInt(value));
    }
    fn push_value_type(&mut self, value: ObjectInstance<'gc>) {
        self.push(StackValue::ValueType(value));
    }
    fn push_managed_ptr(&mut self, value: ManagedPtr<'gc>) {
        self.push(StackValue::ManagedPtr(value));
    }
    fn push_string(&mut self, value: CLRString);

    #[must_use]
    fn pop_i32(&mut self) -> i32 {
        self.pop().as_i32()
    }
    #[must_use]
    fn pop_i64(&mut self) -> i64 {
        self.pop().as_i64()
    }
    #[must_use]
    fn pop_f64(&mut self) -> f64 {
        self.pop().as_f64()
    }
    #[must_use]
    fn pop_isize(&mut self) -> isize {
        self.pop().as_isize()
    }
    #[must_use]
    fn pop_obj(&mut self) -> ObjectRef<'gc> {
        self.pop().as_object_ref()
    }
    #[must_use]
    fn pop_ptr(&mut self) -> *mut u8 {
        self.pop().as_ptr()
    }
    #[must_use]
    fn pop_value_type(&mut self) -> ObjectInstance<'gc> {
        self.pop().as_value_type()
    }
    #[must_use]
    fn pop_managed_ptr(&mut self) -> ManagedPtr<'gc> {
        self.pop().as_managed_ptr()
    }
}

pub trait LocalOps<'gc> {
    fn get_local(&self, index: crate::LocalIndex) -> StackValue<'gc>;
    fn set_local(&mut self, index: crate::LocalIndex, value: StackValue<'gc>);
    fn get_local_address(&self, index: crate::LocalIndex) -> std::ptr::NonNull<u8>;
    fn get_local_info_for_managed_ptr(
        &self,
        index: crate::LocalIndex,
    ) -> (std::ptr::NonNull<u8>, bool);
}

pub trait ArgumentOps<'gc> {
    fn get_argument(&self, index: crate::ArgumentIndex) -> StackValue<'gc>;
    fn set_argument(&mut self, index: crate::ArgumentIndex, value: StackValue<'gc>);
    fn get_argument_address(&self, index: crate::ArgumentIndex) -> std::ptr::NonNull<u8>;
}

pub trait StackOps<'gc, 'm>:
    EvalStackOps<'gc> + TypedStackOps<'gc> + LocalOps<'gc> + ArgumentOps<'gc>
{
    fn current_frame(&self) -> &crate::stack::StackFrame<'gc, 'm>;
    fn current_frame_mut(&mut self) -> &mut crate::stack::StackFrame<'gc, 'm>;

    fn get_slot(&self, index: crate::StackSlotIndex) -> StackValue<'gc>;
    fn get_slot_ref(&self, index: crate::StackSlotIndex) -> &StackValue<'gc>;
    fn set_slot(&mut self, index: crate::StackSlotIndex, value: StackValue<'gc>);
    fn get_slot_address(&self, index: crate::StackSlotIndex) -> std::ptr::NonNull<u8>;
}

pub trait ExceptionOps<'gc> {
    fn throw_by_name(&mut self, name: &str) -> crate::StepResult;
    fn throw(&mut self, exception: ObjectRef<'gc>) -> crate::StepResult;
    fn rethrow(&mut self) -> crate::StepResult;
    fn leave(&mut self, target_ip: usize) -> crate::StepResult;
    fn endfinally(&mut self) -> crate::StepResult;
    fn endfilter(&mut self, result: i32) -> crate::StepResult;
    fn ret(&mut self) -> crate::StepResult;
}

pub trait ResolutionOps<'gc, 'm> {
    fn stack_value_type(
        &self,
        val: &StackValue<'gc>,
    ) -> Result<TypeDescription, TypeResolutionError>;
    fn make_concrete(&self, t: &MethodType) -> Result<ConcreteType, TypeResolutionError>;
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

pub trait RawMemoryOps<'gc>: BorrowScopeOps {
    /// Resolves a `PointerOrigin` and `ByteOffset` to a concrete memory address.
    /// This is the central point for address calculation in the VM.
    fn resolve_address(
        &self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
    ) -> std::ptr::NonNull<u8>;

    /// # Safety
    ///
    /// The caller must ensure that `offset` represents a valid memory location relative to `origin`.
    /// The `layout` must match the expected type of `value`.
    unsafe fn write_unaligned(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        value: StackValue<'gc>,
        layout: &dotnet_value::layout::LayoutManager,
    ) -> Result<(), String>;

    /// # Safety
    ///
    /// The caller must ensure that `offset` represents a valid memory location relative to `origin`.
    /// The `layout` must match the expected type stored at the location.
    unsafe fn read_unaligned(
        &self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        layout: &dotnet_value::layout::LayoutManager,
        type_desc: Option<TypeDescription>,
    ) -> Result<StackValue<'gc>, String>;

    /// Safely writes raw bytes to a memory location.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `offset` represents a valid memory location relative to `origin`.
    unsafe fn write_bytes(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        data: &[u8],
    ) -> Result<(), String>;

    /// Safely reads raw bytes from a memory location.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `offset` represents a valid memory location relative to `origin`.
    unsafe fn read_bytes(
        &self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        dest: &mut [u8],
    ) -> Result<(), String>;

    /// Atomically compares and exchanges a value in memory.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `offset` represents a valid memory location relative to `origin`.
    #[allow(clippy::too_many_arguments)]
    unsafe fn compare_exchange_atomic(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        expected: u64,
        new: u64,
        size: usize,
        success: dotnet_utils::sync::Ordering,
        failure: dotnet_utils::sync::Ordering,
    ) -> Result<u64, u64>;

    /// Atomically exchanges a value in memory.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `offset` represents a valid memory location relative to `origin`.
    unsafe fn exchange_atomic(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        value: u64,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<u64, String>;

    /// Atomically loads a value from memory.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `offset` represents a valid memory location relative to `origin`.
    unsafe fn load_atomic(
        &self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<u64, String>;

    /// Atomically stores a value to memory.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `offset` represents a valid memory location relative to `origin`.
    unsafe fn store_atomic(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        value: u64,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<(), String>;

    fn check_gc_safe_point(&self);
}

pub trait ReflectionOps<'gc, 'm>: MemoryOps<'gc> {
    fn pre_initialize_reflection(&mut self);
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
    fn make_runtime_type(
        &self,
        ctx: &crate::ResolutionContext<'_, 'm>,
        source: &MethodType,
    ) -> RuntimeType;
    fn get_heap_description(
        &self,
        object: ObjectHandle<'gc>,
    ) -> Result<TypeDescription, TypeResolutionError>;
    fn locate_field(
        &self,
        handle: FieldSource,
    ) -> Result<(FieldDescription, GenericLookup), TypeResolutionError>;
    fn is_intrinsic_field_cached(&self, field: FieldDescription) -> bool;
    fn is_intrinsic_cached(&self, method: MethodDescription) -> bool;
    fn resolve_runtime_type(&self, obj: ObjectRef<'gc>) -> RuntimeType;
    fn resolve_runtime_method(&self, obj: ObjectRef<'gc>) -> (MethodDescription, GenericLookup);
    fn resolve_runtime_field(&self, obj: ObjectRef<'gc>) -> (FieldDescription, GenericLookup);
    fn lookup_method_by_index(&self, index: usize) -> (MethodDescription, GenericLookup);
    fn reflection(&self) -> crate::state::ReflectionRegistry<'_, 'gc>;
}

pub trait LoaderOps<'m> {
    fn loader(&self) -> &'m dotnet_assemblies::AssemblyLoader;
    fn resolver(&self) -> crate::resolver::ResolverService<'m>;
    fn shared(&self) -> &Arc<SharedGlobalState<'m>>;
}

pub trait StaticsOps<'gc> {
    fn statics(&self) -> &crate::state::StaticStorageManager;
    fn initialize_static_storage(
        &mut self,
        description: TypeDescription,
        generics: GenericLookup,
    ) -> crate::StepResult;
}

pub trait ThreadOps {
    fn thread_id(&self) -> dotnet_utils::ArenaId;
}

pub trait CallOps<'gc, 'm> {
    fn constructor_frame(
        &mut self,
        instance: ObjectInstance<'gc>,
        method: crate::MethodInfo<'m>,
        generic_inst: GenericLookup,
    ) -> Result<(), TypeResolutionError>;

    fn call_frame(
        &mut self,
        method: crate::MethodInfo<'m>,
        generic_inst: GenericLookup,
    ) -> Result<(), TypeResolutionError>;

    fn entrypoint_frame(
        &mut self,
        method: crate::MethodInfo<'m>,
        generic_inst: GenericLookup,
        args: Vec<StackValue<'gc>>,
    ) -> Result<(), TypeResolutionError>;

    fn dispatch_method(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> crate::StepResult;

    fn unified_dispatch(
        &mut self,
        source: &dotnetdll::prelude::MethodSource,
        this_type: Option<TypeDescription>,
        ctx: Option<&crate::ResolutionContext<'_, 'm>>,
    ) -> crate::StepResult;
}

pub trait VesOps<'gc, 'm>:
    StackOps<'gc, 'm>
    + EvalStackOps<'gc>
    + TypedStackOps<'gc>
    + LocalOps<'gc>
    + ArgumentOps<'gc>
    + ExceptionOps<'gc>
    + ResolutionOps<'gc, 'm>
    + ReflectionOps<'gc, 'm>
    + LoaderOps<'m>
    + StaticsOps<'gc>
    + ThreadOps
    + CallOps<'gc, 'm>
    + MemoryOps<'gc>
    + PoolOps
    + RawMemoryOps<'gc>
{
    fn run(&mut self) -> crate::StepResult;
    fn handle_return(&mut self) -> crate::StepResult;
    fn handle_exception(&mut self) -> crate::StepResult;
    fn tracer_enabled(&self) -> bool;
    fn tracer(&self) -> &Tracer;
    fn indent(&self) -> usize;
    fn process_pending_finalizers(&mut self) -> crate::StepResult;
    fn back_up_ip(&mut self);
    fn branch(&mut self, target: usize);
    fn conditional_branch(&mut self, condition: bool, target: usize) -> bool;
    fn increment_ip(&mut self);
    fn state(&self) -> &crate::MethodState<'m>;
    fn state_mut(&mut self) -> &mut crate::MethodState<'m>;

    fn exception_mode(&self) -> &crate::exceptions::ExceptionState<'gc>;
    fn exception_mode_mut(&mut self) -> &mut crate::exceptions::ExceptionState<'gc>;

    fn current_intrinsic(&self) -> Option<MethodDescription>;
    fn set_current_intrinsic(&mut self, method: Option<MethodDescription>);

    fn evaluation_stack(&self) -> &crate::stack::evaluation_stack::EvaluationStack<'gc>;
    fn evaluation_stack_mut(&mut self)
    -> &mut crate::stack::evaluation_stack::EvaluationStack<'gc>;
    fn frame_stack(&self) -> &crate::stack::frames::FrameStack<'gc, 'm>;
    fn frame_stack_mut(&mut self) -> &mut crate::stack::frames::FrameStack<'gc, 'm>;
    fn original_ip(&self) -> usize;
    fn original_ip_mut(&mut self) -> &mut usize;
    fn original_stack_height(&self) -> crate::StackSlotIndex;
    fn original_stack_height_mut(&mut self) -> &mut crate::StackSlotIndex;

    fn unwind_frame(&mut self);

    fn pin_object(&mut self, object: ObjectRef<'gc>);
    fn unpin_object(&mut self, object: ObjectRef<'gc>);
}
