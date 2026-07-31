//! Helpers for intrinsic stack argument extraction.
//!
//! These helpers intentionally preserve existing failure categories:
//! - Stack underflow and typed-pop mismatches continue to panic: `pop_args`
//!   uses direct [`EvalStackOps::pop`] calls, while object extraction relies on
//!   [`TypedStackOps::pop_obj`].
//! - Null receiver checks can be mapped to managed `System.NullReferenceException`.
//! - Explicit type checks from untyped [`StackValue`]s return host-side
//!   [`StepResult::Error`] with `ExecutionError::TypeMismatch`.

use crate::{
    NULL_REF_MSG,
    ops::{EvalStackOps, ExceptionOps, TypedStackOps},
};
use dotnet_value::{StackValue, object::ObjectRef};
use dotnet_vm_data::StepResult;

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

/// Pops `N` call arguments directly from the evaluation stack without allocating.
///
/// Returned values are in stack order, with the receiver first for instance
/// calls: `[this, arg0, arg1, ...]`.
///
/// # Panics
///
/// Panics if fewer than `N` values are available. Values are removed one at a
/// time, so values already removed before an underflow are not restored.
#[must_use]
pub fn pop_args<'gc, T: EvalStackOps<'gc>, const N: usize>(ctx: &mut T) -> [StackValue<'gc>; N] {
    let mut args = std::array::from_fn(|_| ctx.pop());
    args.reverse();
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotnet_types::error::{IntrinsicError, VmError};
    use dotnet_utils::StackSlotIndex;

    struct TestStack {
        values: Vec<StackValue<'static>>,
    }

    impl TestStack {
        fn from_i32s(values: &[i32]) -> Self {
            Self {
                values: values.iter().copied().map(StackValue::Int32).collect(),
            }
        }
    }

    impl EvalStackOps<'static> for TestStack {
        fn push(&mut self, value: StackValue<'static>) {
            self.values.push(value);
        }

        fn pop(&mut self) -> StackValue<'static> {
            self.pop_safe().expect("test evaluation stack underflow")
        }

        fn pop_safe(&mut self) -> Result<StackValue<'static>, VmError> {
            self.values
                .pop()
                .ok_or(VmError::Intrinsic(IntrinsicError::Static(
                    "test evaluation stack underflow",
                )))
        }

        fn pop_multiple(&mut self, count: usize) -> Vec<StackValue<'static>> {
            let mut values = (0..count).map(|_| self.pop()).collect::<Vec<_>>();
            values.reverse();
            values
        }

        fn peek(&self) -> Option<StackValue<'static>> {
            self.values.last().cloned()
        }

        fn peek_stack(&self) -> StackValue<'static> {
            self.values.last().expect("test stack is empty").clone()
        }

        fn peek_stack_at(&self, offset: usize) -> StackValue<'static> {
            self.values[self.values.len() - 1 - offset].clone()
        }

        fn top_of_stack(&self) -> StackSlotIndex {
            StackSlotIndex::new(self.values.len())
        }
    }

    #[test]
    fn pop_args_returns_values_in_stack_order() {
        let mut stack = TestStack::from_i32s(&[10, 20, 30, 40]);

        let args = pop_args::<_, 3>(&mut stack);

        assert_eq!(args.map(|value| value.as_i32()), [20, 30, 40]);
        assert_eq!(stack.peek_stack().as_i32(), 10);
    }

    #[test]
    fn pop_args_supports_zero_arity() {
        let mut stack = TestStack::from_i32s(&[10]);

        let args = pop_args::<_, 0>(&mut stack);

        assert!(args.is_empty());
        assert_eq!(stack.peek_stack().as_i32(), 10);
    }

    #[test]
    fn pop_args_preserves_partial_removal_on_underflow() {
        let mut stack = TestStack::from_i32s(&[10, 20]);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = pop_args::<_, 3>(&mut stack);
        }));

        assert!(result.is_err());
        assert!(stack.values.is_empty());
    }
}
