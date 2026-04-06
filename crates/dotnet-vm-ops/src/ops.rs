//! Core VES operation traits.
//!
//! This module defines the primitive trait hierarchy used by instruction handlers
//! and intrinsics. These traits depend only on the lower-level crates (`dotnet-value`,
//! `dotnet-types`, `dotnet-utils`) and not on any `dotnet-vm` internals.
//!
//! Higher-level traits (`StackOps`, `ResolutionOps`, `ReflectionOps`, `LoaderOps`,
//! `StaticsOps`, `CallOps`, `VesInternals`, `VesOps`) remain in `dotnet-vm` because
//! they reference internal VM types (`StackFrame`, `ResolutionContext`, `SharedGlobalState`, etc.).
use crate::StepResult;
use dotnet_assemblies::AssemblyLoader;
use dotnet_tracer::Tracer;
use dotnet_types::{
    TypeDescription,
    error::{CompareExchangeError, MemoryAccessError, TypeResolutionError, VmError},
    members::MethodDescription,
};
use dotnet_utils::{
    ArenaId, ArgumentIndex, BorrowScopeOps, ByteOffset, LocalIndex, StackSlotIndex,
};
use dotnet_value::{
    CLRString, StackValue,
    layout::LayoutManager,
    object::{CTSValue, ObjectRef, ValueType, Vector},
    pointer::{ManagedPtr, PointerOrigin},
};
use std::sync::Arc;

pub trait EvalStackOps<'gc> {
    fn push(&mut self, value: StackValue<'gc>);
    fn pop(&mut self) -> StackValue<'gc>;
    fn pop_safe(&mut self) -> Result<StackValue<'gc>, VmError>;
    fn pop_multiple(&mut self, count: usize) -> Vec<StackValue<'gc>>;
    fn peek_multiple(&self, count: usize) -> Vec<StackValue<'gc>>;
    fn dup(&mut self);
    fn peek(&self) -> Option<StackValue<'gc>>;
    fn peek_stack(&self) -> StackValue<'gc>;
    fn peek_stack_at(&self, offset: usize) -> StackValue<'gc>;
    fn top_of_stack(&self) -> StackSlotIndex;
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
    fn push_value_type(&mut self, value: dotnet_value::object::Object<'gc>) {
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
    fn pop_value_type(&mut self) -> dotnet_value::object::Object<'gc> {
        self.pop().as_value_type()
    }
    #[must_use]
    fn pop_managed_ptr(&mut self) -> ManagedPtr<'gc> {
        self.pop().as_managed_ptr()
    }
}

pub trait LocalOps<'gc> {
    fn get_local(&self, index: LocalIndex) -> StackValue<'gc>;
    fn set_local(&mut self, index: LocalIndex, value: StackValue<'gc>);
    fn get_local_address(&self, index: LocalIndex) -> std::ptr::NonNull<u8>;
    fn get_local_info_for_managed_ptr(&self, index: LocalIndex) -> (std::ptr::NonNull<u8>, bool);
}

pub trait ArgumentOps<'gc> {
    fn get_argument(&self, index: ArgumentIndex) -> StackValue<'gc>;
    fn set_argument(&mut self, index: ArgumentIndex, value: StackValue<'gc>);
    fn get_argument_address(&self, index: ArgumentIndex) -> std::ptr::NonNull<u8>;
}

/// A combination trait for operations on local variables and arguments.
pub trait VariableOps<'gc>: LocalOps<'gc> + ArgumentOps<'gc> {}
impl<'gc, T: LocalOps<'gc> + ArgumentOps<'gc> + ?Sized> VariableOps<'gc> for T {}

/// A combination trait for all stack-related operations (evaluation stack, typed ops, and variables).
pub trait AllStackOps<'gc>: EvalStackOps<'gc> + TypedStackOps<'gc> + VariableOps<'gc> {}
impl<'gc, T: EvalStackOps<'gc> + TypedStackOps<'gc> + VariableOps<'gc> + ?Sized> AllStackOps<'gc>
    for T
{
}

pub trait ExceptionOps<'gc> {
    fn throw_by_name(&mut self, name: &str) -> StepResult;
    fn throw_by_name_with_message(&mut self, name: &str, message: &str) -> StepResult;
    fn throw_by_name_with_inner(
        &mut self,
        name: &str,
        message: &str,
        inner: ObjectRef<'gc>,
    ) -> StepResult;
    fn throw(&mut self, exception: ObjectRef<'gc>) -> StepResult;
    fn rethrow(&mut self) -> StepResult;
    fn leave(&mut self, target_ip: usize) -> StepResult;
    fn endfinally(&mut self) -> StepResult;
    fn endfilter(&mut self, result: i32) -> StepResult;
    fn ret(&mut self) -> StepResult;
}

pub trait RawMemoryOps<'gc>: BorrowScopeOps {
    fn as_borrow_scope(&self) -> &dyn BorrowScopeOps;

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
        layout: &LayoutManager,
    ) -> Result<(), MemoryAccessError>;

    /// # Safety
    ///
    /// The caller must ensure that `offset` represents a valid memory location relative to `origin`.
    /// The `layout` must match the expected type stored at the location.
    unsafe fn read_unaligned(
        &self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        layout: &LayoutManager,
        type_desc: Option<TypeDescription>,
    ) -> Result<StackValue<'gc>, MemoryAccessError>;

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
    ) -> Result<(), MemoryAccessError>;

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
    ) -> Result<(), MemoryAccessError>;

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
    ) -> Result<u64, CompareExchangeError>;

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
    ) -> Result<u64, MemoryAccessError>;

    /// Atomically adds a value to a memory location.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `offset` represents a valid memory location relative to `origin`.
    unsafe fn exchange_add_atomic(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        value: u64,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<u64, MemoryAccessError>;

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
    ) -> Result<u64, MemoryAccessError>;

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
    ) -> Result<(), MemoryAccessError>;

    #[must_use]
    fn check_gc_safe_point(&self) -> bool;

    /// # Safety
    ///
    /// The returned pointer is valid for the duration of the current method frame.
    /// It must not be stored in a way that outlives the frame.
    fn localloc(&mut self, size: usize) -> *mut u8;
}

pub trait ThreadOps {
    fn thread_id(&self) -> ArenaId;
}

pub trait VesBaseOps {
    fn tracer_enabled(&self) -> bool;
    fn tracer(&self) -> &Tracer;
    fn indent(&self) -> usize;
}

pub trait LoaderOps {
    fn loader(&self) -> &Arc<AssemblyLoader>;
}

pub trait CallOps<'gc> {
    fn return_frame(&mut self) -> StepResult;
}

pub trait MemoryOps<'gc> {
    fn no_active_borrows_token(&self) -> dotnet_utils::NoActiveBorrows<'_>;

    fn gc_with_token(
        &self,
        _token: &dotnet_utils::NoActiveBorrows<'_>,
    ) -> dotnet_utils::gc::GCHandle<'gc>;
    fn new_vector(
        &self,
        element: dotnet_types::generics::ConcreteType,
        size: usize,
    ) -> Result<Vector<'gc>, TypeResolutionError>;
    fn new_object(
        &self,
        td: TypeDescription,
    ) -> Result<dotnet_value::object::Object<'gc>, TypeResolutionError>;
    fn new_value_type(
        &self,
        t: &dotnet_types::generics::ConcreteType,
        data: StackValue<'gc>,
    ) -> Result<ValueType<'gc>, TypeResolutionError>;
    fn new_cts_value(
        &self,
        t: &dotnet_types::generics::ConcreteType,
        data: StackValue<'gc>,
    ) -> Result<CTSValue<'gc>, TypeResolutionError>;
    fn read_cts_value(
        &self,
        t: &dotnet_types::generics::ConcreteType,
        data: &[u8],
    ) -> Result<CTSValue<'gc>, TypeResolutionError>;
    fn box_value(
        &self,
        t: &dotnet_types::generics::ConcreteType,
        data: StackValue<'gc>,
    ) -> Result<ObjectRef<'gc>, TypeResolutionError>;
    #[must_use]
    fn clone_object(&self, obj: ObjectRef<'gc>) -> ObjectRef<'gc>;
    fn register_new_object(&self, instance: &ObjectRef<'gc>);
}

pub trait ExceptionContext<'gc>:
    TypedStackOps<'gc>
    + ResolutionOps<'gc>
    + ReflectionOps<'gc>
    + LoaderOps
    + MemoryOps<'gc>
    + VesInternals<'gc>
    + VesBaseOps
{
}

pub trait PInvokeContext<'gc>:
    StackOps<'gc>
    + RawMemoryOps<'gc>
    + ExceptionOps<'gc>
    + ResolutionOps<'gc>
    + VesInternals<'gc>
    + LoaderOps
    + MemoryOps<'gc>
    + VesBaseOps
{
    fn pin_object(&mut self, object: ObjectRef<'gc>);
    fn unpin_object(&mut self, object: ObjectRef<'gc>);
}

pub trait ResolutionOps<'gc> {
    fn stack_value_type(
        &self,
        val: &StackValue<'gc>,
    ) -> Result<TypeDescription, TypeResolutionError>;
    fn make_concrete(
        &self,
        t: &dotnetdll::prelude::MethodType,
    ) -> Result<dotnet_types::generics::ConcreteType, TypeResolutionError>;
    fn is_a(
        &self,
        value: dotnet_types::generics::ConcreteType,
        ancestor: dotnet_types::generics::ConcreteType,
    ) -> Result<bool, TypeResolutionError>;
    fn instance_field_layout(
        &self,
        td: TypeDescription,
    ) -> Result<dotnet_value::layout::FieldLayoutManager, TypeResolutionError>;
}

pub trait ReflectionOps<'gc>: MemoryOps<'gc> {
    fn get_heap_description(
        &self,
        object: ObjectRef<'gc>,
    ) -> Result<TypeDescription, TypeResolutionError>;
}

pub trait VesInternals<'gc> {
    fn back_up_ip(&mut self);
    fn branch(&mut self, target: usize);
    fn conditional_branch(&mut self, condition: bool, target: usize) -> bool;
    fn increment_ip(&mut self);
    fn state(&self) -> &crate::MethodState;
    fn state_mut(&mut self) -> &mut crate::MethodState;

    fn exception_mode(&self) -> &crate::ExceptionState<'gc>;
    fn exception_mode_mut(&mut self) -> &mut crate::ExceptionState<'gc>;

    fn original_ip(&self) -> usize;
    fn original_ip_mut(&mut self) -> &mut usize;

    fn original_stack_height(&self) -> StackSlotIndex;
    fn original_stack_height_mut(&mut self) -> &mut StackSlotIndex;

    fn current_intrinsic(&self) -> Option<MethodDescription>;
    fn set_current_intrinsic(&mut self, method: Option<MethodDescription>);

    fn unwind_frame(&mut self);

    fn evaluation_stack(&self) -> &crate::EvaluationStack<'gc>;
    fn evaluation_stack_mut(&mut self) -> &mut crate::EvaluationStack<'gc>;
    fn frame_stack(&self) -> &crate::FrameStack<'gc>;
    fn frame_stack_mut(&mut self) -> &mut crate::FrameStack<'gc>;
}

pub trait StaticsOps<'gc> {
    fn initialize_static_storage(
        &mut self,
        description: TypeDescription,
        generics: dotnet_types::generics::GenericLookup,
    ) -> StepResult;
}

pub trait StackOps<'gc>: TypedStackOps<'gc> + LocalOps<'gc> + ArgumentOps<'gc> {}

/// Host contract for string intrinsic handlers.
///
/// Required capabilities used by current handlers:
/// - stack reads/writes (`push`, `pop`, `peek_stack_at`)
/// - exception throws (`throw_by_name_with_message`)
/// - raw/string memory access (`read_unaligned`, `write_unaligned`, `check_gc_safe_point`)
/// - allocation/boxing helpers (`new_object`, `new_vector`, `box_value`)
/// - method dispatch and type resolution (`return_frame`, `make_concrete`, `get_heap_description`)
pub trait StringIntrinsicHost<'gc>:
    EvalStackOps<'gc>
    + TypedStackOps<'gc>
    + ExceptionOps<'gc>
    + RawMemoryOps<'gc>
    + LoaderOps
    + MemoryOps<'gc>
    + CallOps<'gc>
    + ResolutionOps<'gc>
    + ReflectionOps<'gc>
    + StackOps<'gc>
    + ThreadOps
{
}
impl<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + ExceptionOps<'gc>
        + RawMemoryOps<'gc>
        + LoaderOps
        + MemoryOps<'gc>
        + CallOps<'gc>
        + ResolutionOps<'gc>
        + ReflectionOps<'gc>
        + StackOps<'gc>
        + ThreadOps
        + ?Sized,
> StringIntrinsicHost<'gc> for T
{
}

/// Host contract for delegate intrinsic handlers.
///
/// Required capabilities used by current handlers:
/// - delegate argument stack handling (`pop_multiple`, `push`)
/// - exception throws for unsupported paths
/// - runtime method lookup/dispatch (`dispatch_method`, `lookup_method_by_index`)
/// - runtime state access for multicast frames (`frame_stack_mut`)
pub trait DelegateIntrinsicHost<'gc>:
    EvalStackOps<'gc>
    + ExceptionOps<'gc>
    + LoaderOps
    + MemoryOps<'gc>
    + CallOps<'gc>
    + VesInternals<'gc>
    + ReflectionOps<'gc>
    + ResolutionOps<'gc>
{
}
impl<
    'gc,
    T: EvalStackOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + MemoryOps<'gc>
        + CallOps<'gc>
        + VesInternals<'gc>
        + ReflectionOps<'gc>
        + ResolutionOps<'gc>
        + ?Sized,
> DelegateIntrinsicHost<'gc> for T
{
}

/// Host contract for span intrinsic handlers.
///
/// Required capabilities used by current handlers:
/// - stack and local access (`peek_stack`, `get_local`, `set_local`)
/// - pointer/data reads and writes (`read_unaligned`, `write_unaligned`)
/// - allocation and reflection helpers (`new_object`, `get_heap_description`)
/// - element type/layout resolution (`make_concrete`, `instance_field_layout`)
pub trait SpanIntrinsicHost<'gc>:
    EvalStackOps<'gc>
    + TypedStackOps<'gc>
    + ExceptionOps<'gc>
    + LoaderOps
    + MemoryOps<'gc>
    + RawMemoryOps<'gc>
    + ReflectionOps<'gc>
    + ResolutionOps<'gc>
    + CallOps<'gc>
    + StackOps<'gc>
{
}
impl<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + MemoryOps<'gc>
        + RawMemoryOps<'gc>
        + ReflectionOps<'gc>
        + ResolutionOps<'gc>
        + CallOps<'gc>
        + StackOps<'gc>
        + ?Sized,
> SpanIntrinsicHost<'gc> for T
{
}

/// Host contract for unsafe intrinsic handlers.
///
/// Required capabilities used by current handlers:
/// - raw pointer stack conversions (`pop`, `push_ptr`, `push_managed_ptr`)
/// - memory safety checks and raw reads/writes (`read_bytes`, `write_bytes`)
/// - runtime type sizing/layout (`make_concrete`, `instance_field_layout`)
/// - object/reference helpers (`new_object`, `get_heap_description`)
pub trait UnsafeIntrinsicHost<'gc>:
    EvalStackOps<'gc>
    + TypedStackOps<'gc>
    + ExceptionOps<'gc>
    + LoaderOps
    + MemoryOps<'gc>
    + RawMemoryOps<'gc>
    + ReflectionOps<'gc>
    + ResolutionOps<'gc>
    + StackOps<'gc>
{
}
impl<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + MemoryOps<'gc>
        + RawMemoryOps<'gc>
        + ReflectionOps<'gc>
        + ResolutionOps<'gc>
        + StackOps<'gc>
        + ?Sized,
> UnsafeIntrinsicHost<'gc> for T
{
}

/// Host contract for threading intrinsic handlers.
///
/// Required capabilities used by current handlers:
/// - monitor/interlocked stack operations (`pop`, `push`, `peek`)
/// - exception throws for invalid synchronization inputs
/// - atomic/raw memory operations (`compare_exchange_atomic`, `load_atomic`, `store_atomic`)
/// - thread identity queries (`thread_id`)
pub trait ThreadingIntrinsicHost<'gc>:
    EvalStackOps<'gc>
    + TypedStackOps<'gc>
    + ExceptionOps<'gc>
    + LoaderOps
    + MemoryOps<'gc>
    + RawMemoryOps<'gc>
    + StackOps<'gc>
    + ThreadOps
{
}
impl<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + MemoryOps<'gc>
        + RawMemoryOps<'gc>
        + StackOps<'gc>
        + ThreadOps
        + ?Sized,
> ThreadingIntrinsicHost<'gc> for T
{
}

/// Host contract for reflection intrinsic handlers.
///
/// Required capabilities used by current handlers:
/// - runtime type/member stack plumbing (`push_obj`, `pop_obj`, `pop_multiple`)
/// - reflection lookup and resolution (`get_heap_description`, `make_concrete`, `is_a`)
/// - method construction/invocation (`return_frame`, frame state via `VesInternals`)
/// - static storage initialization for constructed generic types
pub trait ReflectionIntrinsicHost<'gc>:
    EvalStackOps<'gc>
    + TypedStackOps<'gc>
    + ExceptionOps<'gc>
    + LoaderOps
    + MemoryOps<'gc>
    + RawMemoryOps<'gc>
    + ReflectionOps<'gc>
    + ResolutionOps<'gc>
    + CallOps<'gc>
    + VesInternals<'gc>
    + StaticsOps<'gc>
{
}
impl<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + MemoryOps<'gc>
        + RawMemoryOps<'gc>
        + ReflectionOps<'gc>
        + ResolutionOps<'gc>
        + CallOps<'gc>
        + VesInternals<'gc>
        + StaticsOps<'gc>
        + ?Sized,
> ReflectionIntrinsicHost<'gc> for T
{
}
