//! String intrinsic handlers and span-backed string host hooks.
use dotnet_types::{
    TypeDescription, error::IntrinsicError, generics::GenericLookup, members::MethodDescription,
};
use dotnet_value::object::Object;
use dotnet_vm_ops::{StepResult, ops::StringIntrinsicHost as VmStringIntrinsicHost};

pub mod accessors;
pub mod constructors;
pub mod operations;
pub mod search;

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
