use crate::{
    StepResult,
    stack::ops::{ReflectionLookupOps, TypedStackOps, VmCallOps, VmLoaderOps},
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    TypeDescription, generics::GenericLookup, members::MethodDescription, runtime::RuntimeType,
};
use dotnetdll::prelude::MethodMemberIndex;

/// Runs a static method of the support library with the arguments already on the stack.
fn dispatch_support_static<'gc, T: VmCallOps<'gc> + VmLoaderOps>(
    ctx: &mut T,
    type_name: &str,
    method_name: &str,
) -> StepResult {
    let ty = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type(type_name));
    let index = ty
        .definition()
        .methods
        .iter()
        .position(|m| m.name == method_name)
        .unwrap_or_else(|| panic!("support method {type_name}.{method_name}"));
    let method = MethodDescription::new(
        ty.clone(),
        GenericLookup::default(),
        ty.resolution.clone(),
        MethodMemberIndex::Method(index),
    );
    ctx.dispatch_method(method, GenericLookup::default())
}

/// `DelegateHelpers.CanEmitObjectArrayDelegate` is false, the value NativeAOT's trimmer substitutes.
/// The Linq interpreter then never builds its delegate thunks with `DynamicMethod`. (CoreCLR's
/// CoreLib bakes in `true`; the NativeAOT-only fallback it would call instead doesn't exist there,
/// so [`delegate_helpers_create_object_array_delegate`] supplies it.)
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

/// `LambdaExpression.CanCompileToIL` is false: the VM can't compile expression trees to IL, so
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

/// `DefaultJsonTypeInfoResolver.MemberAccessor` is System.Text.Json's `ReflectionMemberAccessor`.
/// The library picks `ReflectionEmitCachingMemberAccessor` whenever
/// `RuntimeFeature.IsDynamicCodeSupported` is true and has no switch of its own, but that value
/// has to stay true for EF Core. The Emit accessors need `DynamicMethod`, which the VM lacks.
#[dotnet_intrinsic(
    "static System.Text.Json.Serialization.Metadata.MemberAccessor System.Text.Json.Serialization.Metadata.DefaultJsonTypeInfoResolver::get_MemberAccessor()"
)]
pub fn json_default_resolver_member_accessor<
    'gc,
    T: VmCallOps<'gc> + VmLoaderOps + ReflectionLookupOps<'gc> + TypedStackOps<'gc>,
>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // Type.GetType(string) isn't available on the VM, so find the accessor type in the library's
    // own metadata and hand the support method its Type object.
    let resolution = method.parent.resolution.clone();
    let accessor = resolution
        .definition()
        .enumerate_type_definitions()
        .find(|(_, ty)| {
            ty.type_name() == "System.Text.Json.Serialization.Metadata.ReflectionMemberAccessor"
        })
        .map(|(index, _)| TypeDescription::new(resolution.clone(), index))
        .expect("System.Text.Json ReflectionMemberAccessor");
    let type_object = ctx.get_runtime_type(RuntimeType::Type(accessor));
    ctx.push_obj(type_object);
    dispatch_support_static(ctx, "DotnetRs.JsonReflectionMemberAccessor", "Get")
}
