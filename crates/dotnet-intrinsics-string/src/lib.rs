//! String intrinsic handlers and span-backed string host hooks.
//!
//! Inventory mapping for this host surface is tracked in
//! `docs/p3_s1_trait_inventory.md` (P3.S1).
use dotnet_types::{
    TypeDescription, error::IntrinsicError, generics::GenericLookup, members::MethodDescription,
};
use dotnet_value::object::Object;
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::ops::StringIntrinsicHost as VmStringIntrinsicHost;

pub mod accessors;
pub mod constructors;
pub mod operations;
pub mod search;
pub(crate) mod simd;

pub(crate) const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";

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
