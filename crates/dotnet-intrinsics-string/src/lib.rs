//! String intrinsic handlers and span-backed string host hooks.
//!
use dotnet_types::{
    TypeDescription, error::IntrinsicError, generics::GenericLookup, members::MethodDescription,
};
use dotnet_value::object::Object;
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::{NULL_REF_MSG, ops::StringIntrinsicHost as VmStringIntrinsicHost};

pub mod accessors;
pub mod constructors;
pub mod operations;
pub mod search;
pub(crate) mod simd;

pub trait IntrinsicStringHost<'gc>: VmStringIntrinsicHost<'gc> {
    fn string_intrinsic_as_span(
        &mut self,
        method: MethodDescription,
        generics: &GenericLookup,
    ) -> StepResult;

    fn string_with_span_data<'span, R, F: FnOnce(&[u8]) -> R>(
        &self,
        span: Object<'span>,
        element_type: TypeDescription,
        element_size: usize,
        f: F,
    ) -> Result<R, IntrinsicError>;
}

#[inline]
pub(crate) fn extend_from_utf16_ne_bytes(dest: &mut Vec<u16>, bytes: &[u8]) {
    dest.extend(
        bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_ne_bytes([chunk[0], chunk[1]])),
    );
}
