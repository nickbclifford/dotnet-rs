//! Unsafe and marshalling intrinsic handlers with explicit safety host seams.
use dotnet_types::{
    error::{MemoryAccessError, TypeResolutionError},
    generics::ConcreteType,
};
use dotnet_value::{layout::LayoutManager, object::ObjectRef, pointer::PointerOrigin};
use dotnet_vm_ops::{StepResult, ops::UnsafeIntrinsicHost as VmUnsafeIntrinsicHost};
use std::sync::Arc;

pub mod buffer;
pub mod marshal;
pub mod unsafe_ptr;

pub(crate) const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";

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

    fn unsafe_resolve_runtime_type(&self, obj: ObjectRef<'gc>) -> ConcreteType;
}

macro_rules! vm_try {
    ($expr:expr) => {
        match $expr {
            Ok(v) => v,
            Err(e) => {
                return dotnet_vm_ops::StepResult::Error(dotnet_types::error::VmError::from(e))
            }
        }
    };
}

pub(crate) use vm_try;

pub(crate) fn ptr_info<
    'gc,
    T: dotnet_vm_ops::ops::StackOps<'gc> + dotnet_vm_ops::ops::ExceptionOps<'gc>,
>(
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
            PointerOrigin::Transient(obj.clone()),
            dotnet_utils::ByteOffset(0),
        )),
        _ => Err(ctx.throw_by_name_with_message(
            "System.InvalidProgramException",
            "Common Language Runtime detected an invalid program.",
        )),
    }
}
