use crate::{
    StepResult,
    resolution::ValueResolution,
    stack::ops::{
        EvalStackOps, ExceptionOps, LoaderOps, ReflectionOps, ResolutionOps, TypedStackOps,
    },
};
use dotnet_macros::dotnet_instruction;
use dotnet_value::StackValue;
use dotnetdll::prelude::*;

#[dotnet_instruction(LoadMethodPointer(param0))]
pub fn ldftn<
    'gc,
    'm: 'gc,
    T: ResolutionOps<'gc, 'm> + ReflectionOps<'gc, 'm> + LoaderOps<'m> + TypedStackOps<'gc>,
>(
    ctx: &mut T,
    param0: &MethodSource,
) -> StepResult {
    let (method, lookup) = vm_try!(
        ctx.resolver()
            .find_generic_method(param0, &ctx.current_context())
    );

    let index = ctx.get_runtime_method_index(method, lookup);
    ctx.push_isize(index as isize);
    StepResult::Continue
}

#[dotnet_instruction(LoadVirtualMethodPointer { param0, skip_null_check })]
pub fn ldvirtftn<
    'gc,
    'm: 'gc,
    T: ResolutionOps<'gc, 'm>
        + ReflectionOps<'gc, 'm>
        + LoaderOps<'m>
        + TypedStackOps<'gc>
        + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    param0: &MethodSource,
    skip_null_check: bool,
) -> StepResult {
    let obj = ctx.pop_obj();
    if !skip_null_check && obj.0.is_none() {
        return ctx.throw_by_name_with_message(
            "System.NullReferenceException",
            "Object reference not set to an instance of an object.",
        );
    }

    let (base_method, lookup) = vm_try!(
        ctx.resolver()
            .find_generic_method(param0, &ctx.current_context())
    );

    let this_type = vm_try!(ctx.get_heap_description(obj.0.unwrap()));

    // Virtual dispatch
    let resolved_method = vm_try!(ctx.resolver().resolve_virtual_method(
        base_method,
        this_type,
        &lookup,
        &ctx.current_context()
    ));

    let index = ctx.get_runtime_method_index(resolved_method, lookup);
    ctx.push_isize(index as isize);
    StepResult::Continue
}

#[dotnet_instruction(LoadTokenType(param0))]
pub fn ldtoken_type<
    'gc,
    'm: 'gc,
    T: ResolutionOps<'gc, 'm> + ReflectionOps<'gc, 'm> + LoaderOps<'m> + EvalStackOps<'gc>,
>(
    ctx: &mut T,
    param0: &MethodType,
) -> StepResult {
    let runtime_type = {
        let res_ctx = ctx.current_context();
        ctx.make_runtime_type(&res_ctx, param0)
    };

    let rt_obj = ctx.get_runtime_type(runtime_type);

    let res_ctx = ctx.current_context();
    let rth = vm_try!(ctx.loader().corlib_type("System.RuntimeTypeHandle"));
    let instance = vm_try!(res_ctx.new_object(rth));
    rt_obj.write(&mut instance.instance_storage.get_field_mut_local(rth, "_value"));

    ctx.push(StackValue::ValueType(instance));
    StepResult::Continue
}

#[dotnet_instruction(LoadTokenMethod(param0))]
pub fn ldtoken_method<
    'gc,
    'm: 'gc,
    T: ResolutionOps<'gc, 'm> + ReflectionOps<'gc, 'm> + LoaderOps<'m> + EvalStackOps<'gc>,
>(
    ctx: &mut T,
    param0: &MethodSource,
) -> StepResult {
    let (method, lookup) = vm_try!(
        ctx.resolver()
            .find_generic_method(param0, &ctx.current_context())
    );

    let method_obj = ctx.get_runtime_method_obj(method, lookup);

    let res_ctx = ctx.current_context();
    let rmh = vm_try!(ctx.loader().corlib_type("System.RuntimeMethodHandle"));
    let instance = vm_try!(res_ctx.new_object(rmh));
    method_obj.write(&mut instance.instance_storage.get_field_mut_local(rmh, "_value"));

    ctx.push(StackValue::ValueType(instance));
    StepResult::Continue
}

#[dotnet_instruction(LoadTokenField(param0))]
pub fn ldtoken_field<
    'gc,
    'm: 'gc,
    T: ResolutionOps<'gc, 'm> + ReflectionOps<'gc, 'm> + LoaderOps<'m> + EvalStackOps<'gc>,
>(
    ctx: &mut T,
    param0: &FieldSource,
) -> StepResult {
    let (field, lookup) = vm_try!(ctx.locate_field(*param0));

    let field_obj = ctx.get_runtime_field_obj(field, lookup);

    let res_ctx = ctx.current_context();
    let rfh = vm_try!(ctx.loader().corlib_type("System.RuntimeFieldHandle"));
    let instance = vm_try!(res_ctx.new_object(rfh));
    field_obj.write(&mut instance.instance_storage.get_field_mut_local(rfh, "_value"));

    ctx.push(StackValue::ValueType(instance));
    StepResult::Continue
}
