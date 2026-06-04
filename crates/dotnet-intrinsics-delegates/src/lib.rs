//! Delegate intrinsic handlers and delegate-host execution seams.
//!
//! This crate implements runtime handling for delegate methods that do not have
//! CIL bodies (for example `Invoke`, `BeginInvoke`, and `EndInvoke`) and exposes
//! helper operations for delegate equality/combination.
//!
//! The main integration point is [`try_delegate_dispatch`], which allows the VM
//! dispatch loop to intercept delegate method calls and either execute the
//! runtime path (`Some(StepResult)`) or fall back to normal method dispatch
//! (`None`).
//!
//! ## Host Trait
//!
//! VM contexts integrating this crate must implement [`DelegateInvokeHost<'gc>`]
//! for delegate-specific runtime hooks (method lookup, call-frame setup,
//! `MethodInfo` construction, and runtime `MethodInfo` object projection).
//! `try_delegate_dispatch` also requires the broader
//! `dotnet_vm_ops::ops::DelegateIntrinsicHost<'gc>` contract for stack,
//! exception, loader, and frame-state operations used by delegate dispatch.
//!
//! See `docs/DELEGATES_AND_DISPATCH.md` for the delegate model and dispatch
//! behavior, and `docs/BUILD_TIME_CODE_GENERATION.md` for how
//! `#[dotnet_intrinsic]` handlers in this crate are registered into generated
//! intrinsic tables.
use dotnet_types::{
    error::TypeResolutionError, generics::GenericLookup, members::MethodDescription,
};
use dotnet_value::object::ObjectRef;
use dotnet_vm_data::{MethodInfo, StepResult};
use dotnet_vm_ops::NULL_REF_MSG;

pub mod helpers;
pub mod invoke;
pub mod operations;

pub use helpers::try_delegate_dispatch;

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
