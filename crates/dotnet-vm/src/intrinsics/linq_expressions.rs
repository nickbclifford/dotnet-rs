use crate::{
    StepResult,
    stack::ops::{TypedStackOps, VmCallOps, VmLoaderOps},
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{TypeDescription, generics::GenericLookup, members::MethodDescription};
use dotnetdll::prelude::MethodMemberIndex;

/// A VM error for a type or method these substitutions expect but the loaded assemblies lack.
#[cold]
fn missing(what: String) -> StepResult {
    StepResult::Error(
        dotnet_types::error::ExecutionError::NotImplemented(
            format!("Reflection.Emit substitution target not found: {what}").into(),
        )
        .into(),
    )
}

/// Runs a static method of the support library with the arguments already on the stack.
fn dispatch_support_static<'gc, T: VmCallOps<'gc> + VmLoaderOps>(
    ctx: &mut T,
    type_name: &str,
    method_name: &str,
) -> StepResult {
    let ty = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type(type_name));
    let Some(index) = ty
        .definition()
        .methods
        .iter()
        .position(|m| m.name == method_name)
    else {
        return missing(format!("support method {type_name}.{method_name}"));
    };
    let method = MethodDescription::new(
        ty.clone(),
        GenericLookup::default(),
        ty.resolution.clone(),
        MethodMemberIndex::Method(index),
    );
    ctx.dispatch_method(method, GenericLookup::default())
}

/// `DelegateHelpers.CanEmitObjectArrayDelegate` is false, the value NativeAOT's trimmer substitutes.
/// CoreCLR's NativeAOT-only fallback does not exist there, so
/// [`delegate_helpers_create_object_array_delegate`] supplies a table and an Emit fallback.
#[dotnet_intrinsic(
    "static bool System.Dynamic.Utils.DelegateHelpers::get_CanEmitObjectArrayDelegate()"
)]
pub fn delegate_helpers_can_emit_object_array_delegate<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

/// `DelegateHelpers.CreateObjectArrayDelegate(Type, Func<object[], object>)` runs the support
/// library's `DotnetRs.ObjectArrayDelegates.Create`, which has the same signature, so the pending
/// arguments are already in place. It plays the role of NativeAOT's
/// `DynamicDelegateAugments.CreateObjectArrayDelegate`: a delegate of the requested type whose
/// invocation packs its arguments into `object[]` and calls the handler.
#[dotnet_intrinsic(
    "static System.Delegate System.Dynamic.Utils.DelegateHelpers::CreateObjectArrayDelegate(System.Type, System.Func<object[], object>)"
)]
pub fn delegate_helpers_create_object_array_delegate<'gc, T: VmCallOps<'gc> + VmLoaderOps>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    dispatch_support_static(ctx, "DotnetRs.ObjectArrayDelegates", "Create")
}

/// `LambdaExpression.CanCompileToIL` is false: EF InMemory needs unsupported reflection on the
/// IL compiler path, so
/// `Compile()` takes the `LightCompiler` interpreter, as it does under NativeAOT and Mono's
/// interpreter. CoreLib defines it as `RuntimeFeature.IsDynamicCodeSupported`, which has to stay
/// true for EF Core's model-building check.
#[dotnet_intrinsic("static bool System.Linq.Expressions.LambdaExpression::get_CanCompileToIL()")]
pub fn lambda_expression_can_compile_to_il<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

/// Dispatch unsupported generic-thunk shapes to the framework's own Emit implementation.
#[dotnet_intrinsic(
    "static System.Delegate DotnetRs.ObjectArrayDelegates::CreateEmitted(System.Type, System.Func<object[], object>)"
)]
pub fn object_array_delegate_create_emitted<'gc, T: VmCallOps<'gc> + VmLoaderOps>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let resolution = dotnet_vm_ops::vm_try!(ctx.loader().get_assembly("System.Linq.Expressions"));
    let ty = resolution
        .definition()
        .enumerate_type_definitions()
        .find(|(_, ty)| ty.type_name() == "System.Dynamic.Utils.DelegateHelpers")
        .map(|(index, _)| TypeDescription::new(resolution.clone(), index));
    let Some(ty) = ty else {
        return missing("System.Dynamic.Utils.DelegateHelpers".into());
    };
    let Some(index) = ty
        .definition()
        .methods
        .iter()
        .position(|method| method.name == "CreateObjectArrayDelegateRefEmit")
    else {
        return missing("DelegateHelpers.CreateObjectArrayDelegateRefEmit".into());
    };
    let method = MethodDescription::new(
        ty,
        GenericLookup::default(),
        resolution,
        MethodMemberIndex::Method(index),
    );
    ctx.dispatch_method(method, GenericLookup::default())
}
