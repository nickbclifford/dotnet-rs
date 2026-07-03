//! Helpers for intrinsic stack argument extraction.
//!
//! These helpers intentionally preserve existing failure categories:
//! - Stack underflow and typed-pop mismatches continue to panic by relying on
//!   [`TypedStackOps::pop_obj`] / [`EvalStackOps::pop_multiple`].
//! - Null receiver checks can be mapped to managed `System.NullReferenceException`.
//! - Explicit type checks from untyped [`StackValue`]s return host-side
//!   [`StepResult::Error`] with `ExecutionError::TypeMismatch`.

use crate::{
    NULL_REF_MSG, StepResult,
    ops::{EvalStackOps, ExceptionOps, TypedStackOps},
};
use dotnet_value::{StackValue, object::ObjectRef};

/// Policy for handling `ObjectRef(None)` values while extracting intrinsic args.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ArgPolicy {
    /// Allow `null` object references.
    AllowNull,
    /// Convert `null` object references into a managed
    /// `System.NullReferenceException` throw result.
    ManagedNullNre,
}

/// Builds a host-side type mismatch [`StepResult`] with canonical formatting.
pub fn type_mismatch(expected: &'static str, actual: impl Into<Box<str>>) -> StepResult {
    StepResult::type_error(expected, actual)
}

/// Builds a host-side type mismatch [`StepResult`] from a stack value.
pub fn type_mismatch_stack_value(expected: &'static str, actual: &StackValue<'_>) -> StepResult {
    StepResult::type_error(expected, format!("{actual:?}"))
}

/// Extracts an [`ObjectRef`] from an untyped stack value.
///
/// Returns a host-side `TypeMismatch` [`StepResult`] when `value` is not an
/// object reference.
pub fn expect_stack_object<'gc>(
    value: &StackValue<'gc>,
    expected: &'static str,
) -> Result<ObjectRef<'gc>, StepResult> {
    match value {
        StackValue::ObjectRef(obj) => Ok(*obj),
        actual => Err(type_mismatch_stack_value(expected, actual)),
    }
}

/// Applies [`ArgPolicy`] to an already-extracted object reference.
pub fn apply_object_policy<'gc, T: ExceptionOps<'gc>>(
    ctx: &mut T,
    object: ObjectRef<'gc>,
    policy: ArgPolicy,
) -> Result<ObjectRef<'gc>, StepResult> {
    match policy {
        ArgPolicy::AllowNull => Ok(object),
        ArgPolicy::ManagedNullNre if object.0.is_none() => {
            Err(ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG))
        }
        ArgPolicy::ManagedNullNre => Ok(object),
    }
}

/// Extracts an object reference from an untyped stack value and applies
/// [`ArgPolicy`].
pub fn expect_stack_object_with_policy<'gc, T: ExceptionOps<'gc>>(
    ctx: &mut T,
    value: &StackValue<'gc>,
    expected: &'static str,
    policy: ArgPolicy,
) -> Result<ObjectRef<'gc>, StepResult> {
    let object = expect_stack_object(value, expected)?;
    apply_object_policy(ctx, object, policy)
}

/// Pops an object reference and applies [`ArgPolicy`].
///
/// This preserves the existing panic behavior of [`TypedStackOps::pop_obj`] for
/// stack underflow and non-object stack variants.
pub fn pop_object_ref<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc>>(
    ctx: &mut T,
    policy: ArgPolicy,
) -> Result<ObjectRef<'gc>, StepResult> {
    let object = ctx.pop_obj();
    apply_object_policy(ctx, object, policy)
}

/// Pops `N` call arguments using [`EvalStackOps::pop_multiple`].
///
/// Returned values are in the same stack order as `pop_multiple`, i.e. receiver
/// first for instance calls: `[this, arg0, arg1, ...]`.
#[must_use]
pub fn pop_args<'gc, T: EvalStackOps<'gc>, const N: usize>(ctx: &mut T) -> [StackValue<'gc>; N] {
    ctx.pop_multiple(N)
        .try_into()
        .expect("pop_multiple must return exactly the requested number of values")
}
