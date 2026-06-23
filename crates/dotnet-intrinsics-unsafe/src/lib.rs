//! Unsafe, pointer, and marshalling intrinsic handlers with explicit host seams.
//!
//! This crate provides `#[dotnet_intrinsic]` handlers for low-level runtime
//! APIs including `System.Runtime.CompilerServices.Unsafe`,
//! `System.Buffer`/`System.SpanHelpers`, and
//! `System.Runtime.InteropServices.Marshal`.
//!
//! ## Host Trait
//!
//! VM contexts integrating this crate implement [`UnsafeIntrinsicHost<'gc>`].
//! That trait extends [`VmUnsafeIntrinsicHost<'gc>`] from `dotnet-vm-ops`
//! with crate-specific hooks used by unsafe/marshalling handlers:
//!
//! - `unsafe_type_layout` and `unsafe_resolve_runtime_type` for runtime
//!   type/layout lookup.
//! - `unsafe_check_read_safety` and `unsafe_lookup_owner_layout_and_base` for
//!   pointer safety checks and owner/base recovery.
//! - `unsafe_get_last_pinvoke_error` and `unsafe_set_last_pinvoke_error` for
//!   `Marshal.GetLastPInvokeError`/`Marshal.SetLastPInvokeError` state.
//!
//! See `docs/BUILD_TIME_CODE_GENERATION.md` for how `#[dotnet_intrinsic]`
//! handlers are discovered and wired into generated intrinsic dispatch tables.
use dotnet_types::{
    error::{ExecutionError, MemoryAccessError, TypeResolutionError},
    generics::ConcreteType,
};
use dotnet_value::{layout::LayoutManager, object::ObjectRef, pointer::PointerOrigin};
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::{NULL_REF_MSG, ops::UnsafeIntrinsicHost as VmUnsafeIntrinsicHost};
use std::sync::Arc;

pub mod buffer;
pub mod marshal;
pub(crate) mod mem_helpers;
pub mod unsafe_ptr;

pub trait UnsafeIntrinsicHost<'gc>: VmUnsafeIntrinsicHost<'gc> {
    fn unsafe_type_layout(
        &self,
        t: ConcreteType,
    ) -> Result<Arc<LayoutManager>, TypeResolutionError>;

    fn unsafe_check_read_safety(
        &self,
        result_layout: &LayoutManager,
        src_layout: Option<&LayoutManager>,
        src_ptr_offset: usize,
    ) -> Result<(), MemoryAccessError>;

    fn unsafe_lookup_owner_layout_and_base(
        &self,
        ptr: *mut u8,
        origin: &PointerOrigin<'gc>,
    ) -> (Option<LayoutManager>, Option<usize>);

    fn unsafe_get_last_pinvoke_error(&self) -> i32;

    fn unsafe_set_last_pinvoke_error(&mut self, value: i32);

    fn unsafe_resolve_runtime_type(
        &self,
        obj: ObjectRef<'gc>,
    ) -> Result<ConcreteType, ExecutionError>;
}

pub(crate) fn ptr_info<'gc, T: dotnet_vm_ops::ops::ExceptionOps<'gc>>(
    ctx: &mut T,
    val: &dotnet_value::StackValue<'gc>,
) -> Result<(PointerOrigin<'gc>, dotnet_utils::ByteOffset), StepResult> {
    match val {
        dotnet_value::StackValue::ObjectRef(o) => Ok((
            o.0.map_or(PointerOrigin::Unmanaged, |h| {
                PointerOrigin::Heap(dotnet_value::object::ObjectRef(Some(h)))
            }),
            dotnet_utils::ByteOffset(0),
        )),
        dotnet_value::StackValue::ManagedPtr(m) => Ok((m.origin().clone(), m.byte_offset())),
        dotnet_value::StackValue::UnmanagedPtr(dotnet_value::pointer::UnmanagedPtr(p)) => Ok((
            PointerOrigin::Unmanaged,
            dotnet_utils::ByteOffset(p.as_ptr() as usize),
        )),
        dotnet_value::StackValue::NativeInt(p) => Ok((
            PointerOrigin::Unmanaged,
            dotnet_utils::ByteOffset(*p as usize),
        )),
        dotnet_value::StackValue::ValueType(obj) => Ok((
            PointerOrigin::new_transient(obj.clone()),
            dotnet_utils::ByteOffset(0),
        )),
        _ => Err(ctx.throw_by_name_with_message(
            "System.InvalidProgramException",
            "Common Language Runtime detected an invalid program.",
        )),
    }
}
