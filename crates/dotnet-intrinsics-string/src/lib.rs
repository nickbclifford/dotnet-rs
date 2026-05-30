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
