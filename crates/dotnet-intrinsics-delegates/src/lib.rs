//! Delegate intrinsic handlers and delegate-host execution seams.
use dotnet_types::{
    error::TypeResolutionError, generics::GenericLookup, members::MethodDescription,
};
use dotnet_value::object::ObjectRef;
use dotnet_vm_ops::{MethodInfo, StepResult};

pub mod helpers;
pub mod invoke;
pub mod operations;

pub use helpers::try_delegate_dispatch;

pub(crate) const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";
pub(crate) const BEGIN_END_NOT_SUPPORTED_MSG: &str = "BeginInvoke and EndInvoke are not supported.";

pub trait DelegateInvokeHost<'gc> {
    fn delegate_method_info(
        &self,
        method: MethodDescription,
        lookup: &GenericLookup,
    ) -> Result<MethodInfo<'static>, TypeResolutionError>;

    fn delegate_call_frame(
        &mut self,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
    ) -> Result<(), TypeResolutionError>;

    fn delegate_lookup_method_by_index(&self, index: usize) -> (MethodDescription, GenericLookup);

    fn delegate_runtime_method_obj(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> ObjectRef<'gc>;

    fn delegate_dispatch_method(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult;
}
