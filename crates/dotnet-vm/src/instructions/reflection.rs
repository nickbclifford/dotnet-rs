use crate::{
    StepResult, intrinsics::reflection::ReflectionExtensions, resolution::ValueResolution,
    stack::VesContext,
};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnet_value::StackValue;
use dotnetdll::prelude::*;

#[dotnet_instruction(LoadTokenType(param0))]
pub fn ldtoken_type<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    param0: &MethodType,
) -> StepResult {
    let runtime_type = {
        let res_ctx = ctx.current_context();
        ctx.make_runtime_type(&res_ctx, param0)
    };

    let rt_obj = ctx.get_runtime_type(gc, runtime_type);

    let res_ctx = ctx.current_context();
    let rth = ctx.loader().corlib_type("System.RuntimeTypeHandle");
    let instance = res_ctx.new_object(rth);
    rt_obj.write(&mut instance.instance_storage.get_field_mut_local(rth, "_value"));

    ctx.push(gc, StackValue::ValueType(instance));
    StepResult::Continue
}

#[dotnet_instruction(LoadTokenMethod(param0))]
pub fn ldtoken_method<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    param0: &MethodSource,
) -> StepResult {
    let (method, lookup) = ctx
        .resolver()
        .find_generic_method(param0, &ctx.current_context());

    let method_obj = ctx.get_runtime_method_obj(gc, method, lookup);

    let rmh = ctx.loader().corlib_type("System.RuntimeMethodHandle");
    let instance = ctx.current_context().new_object(rmh);
    method_obj.write(&mut instance.instance_storage.get_field_mut_local(rmh, "_value"));

    ctx.push(gc, StackValue::ValueType(instance));
    StepResult::Continue
}

#[dotnet_instruction(LoadTokenField(param0))]
pub fn ldtoken_field<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    param0: &FieldSource,
) -> StepResult {
    let (field, lookup) = ctx.locate_field(*param0);

    let field_obj = ctx.get_runtime_field_obj(gc, field, lookup);

    let res_ctx = ctx.current_context();
    let rfh = ctx.loader().corlib_type("System.RuntimeFieldHandle");
    let instance = res_ctx.new_object(rfh);
    field_obj.write(&mut instance.instance_storage.get_field_mut_local(rfh, "_value"));

    ctx.push(gc, StackValue::ValueType(instance));
    StepResult::Continue
}
