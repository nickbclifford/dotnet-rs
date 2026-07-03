//! Helpers for preparing method dispatch arguments.
//!
//! `PreparedCall` centralizes argument ordering for call sites that must push an
//! optional bound receiver (`this`) and then call arguments back onto the
//! evaluation stack before dispatch.

use crate::ops::EvalStackOps;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_value::{StackValue, object::ObjectRef};

/// A resolved method plus generic lookup to dispatch.
#[derive(Clone)]
pub struct CallTarget {
    method: MethodDescription,
    lookup: GenericLookup,
}

impl CallTarget {
    #[must_use]
    pub fn new(method: MethodDescription, lookup: GenericLookup) -> Self {
        Self { method, lookup }
    }

    #[must_use]
    pub fn into_parts(self) -> (MethodDescription, GenericLookup) {
        (self.method, self.lookup)
    }
}

/// A prepared managed call with stack-ready argument material.
#[derive(Clone)]
pub struct PreparedCall<'gc> {
    target: CallTarget,
    bound_this: Option<ObjectRef<'gc>>,
    args: Vec<StackValue<'gc>>,
}

impl<'gc> PreparedCall<'gc> {
    #[must_use]
    pub fn new(
        method: MethodDescription,
        lookup: GenericLookup,
        bound_this: Option<ObjectRef<'gc>>,
        args: Vec<StackValue<'gc>>,
    ) -> Self {
        Self {
            target: CallTarget::new(method, lookup),
            bound_this,
            args,
        }
    }

    /// Builds a call for delegate target invocation.
    ///
    /// For closed static delegates created with a bound target, `_target` is
    /// inserted as the first argument even though the resolved method is static.
    #[must_use]
    pub fn for_delegate_target(
        method: MethodDescription,
        lookup: GenericLookup,
        target: ObjectRef<'gc>,
        args: Vec<StackValue<'gc>>,
    ) -> Self {
        let bound_this = if method.signature().instance || target.0.is_some() {
            Some(target)
        } else {
            None
        };

        Self::new(method, lookup, bound_this, args)
    }

    /// Builds a call for multicast invoke stepping.
    ///
    /// The multicast protocol always dispatches the delegate `Invoke` method
    /// with the next target delegate object as argument zero.
    #[must_use]
    pub fn for_multicast_step(
        invoke_method: MethodDescription,
        lookup: GenericLookup,
        target_delegate: ObjectRef<'gc>,
        args: Vec<StackValue<'gc>>,
    ) -> Self {
        Self::new(invoke_method, lookup, Some(target_delegate), args)
    }

    /// Pushes the prepared call arguments back onto the evaluation stack in
    /// call order and returns the resolved call target.
    #[must_use]
    pub fn push_arguments(self, ctx: &mut impl EvalStackOps<'gc>) -> CallTarget {
        if let Some(bound_this) = self.bound_this {
            ctx.push(StackValue::ObjectRef(bound_this));
        }

        for arg in self.args {
            ctx.push(arg);
        }

        self.target
    }
}
