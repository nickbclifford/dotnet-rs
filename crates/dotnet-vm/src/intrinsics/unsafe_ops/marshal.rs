use crate::{
    StepResult,
    layout::{LayoutFactory, type_layout},
    stack::ops::{
        EvalStackOps, ExceptionOps, LoaderOps, RawMemoryOps, ReflectionOps, ResolutionOps,
        TypedStackOps,
    },
};
use dotnet_macros::{dotnet_intrinsic, dotnet_intrinsic_field};
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
};
use dotnet_value::{
    StackValue,
    layout::{HasLayout, LayoutManager},
};
use std::sync::Arc;

#[allow(dead_code)]
const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";

pub(super) fn offset_ptr<'gc>(val: StackValue<'gc>, byte_offset: isize) -> StackValue<'gc> {
    if let StackValue::ManagedPtr(m) = val {
        let new_m = unsafe { m.offset(byte_offset) };
        StackValue::ManagedPtr(new_m)
    } else {
        let ptr = val.as_ptr();
        let result_ptr = unsafe { ptr.offset(byte_offset) };
        StackValue::NativeInt(result_ptr as isize)
    }
}

#[dotnet_intrinsic("static int System.Runtime.InteropServices.Marshal::GetLastPInvokeError()")]
#[allow(unused_variables)]
pub fn intrinsic_marshal_get_last_pinvoke_error<'gc, T: EvalStackOps<'gc> + TypedStackOps<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = unsafe { crate::pinvoke::LAST_ERROR };
    ctx.push_i32(value);
    StepResult::Continue
}

#[dotnet_intrinsic("static void System.Runtime.InteropServices.Marshal::SetLastPInvokeError(int)")]
#[allow(unused_variables)]
pub fn intrinsic_marshal_set_last_pinvoke_error<'gc, T: EvalStackOps<'gc> + TypedStackOps<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = ctx.pop_i32();
    unsafe {
        crate::pinvoke::LAST_ERROR = value;
    }
    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Runtime.InteropServices.Marshal::SizeOf(object)")]
#[dotnet_intrinsic("static int System.Runtime.InteropServices.Marshal::SizeOf(System.Type)")]
#[dotnet_intrinsic("static int System.Runtime.InteropServices.Marshal::SizeOf<T>(T)")]
#[dotnet_intrinsic("static int System.Runtime.InteropServices.Marshal::SizeOf<T>()")]
pub fn intrinsic_marshal_size_of<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + LoaderOps + ResolutionOps<'gc> + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let concrete_type = if method.method().signature.parameters.is_empty() {
        generics.method_generics[0].clone()
    } else {
        let type_obj = ctx.pop_obj();
        ctx.resolve_runtime_type(type_obj)
            .to_concrete(ctx.loader().as_ref())
    };
    let layout = vm_try!(type_layout(concrete_type, &ctx.current_context()));
    ctx.push_i32(layout.size().as_usize() as i32);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static IntPtr System.Runtime.InteropServices.Marshal::OffsetOf(System.Type, string)"
)]
#[dotnet_intrinsic("static IntPtr System.Runtime.InteropServices.Marshal::OffsetOf<T>(string)")]
pub fn intrinsic_marshal_offset_of<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + ReflectionOps<'gc>
        + ExceptionOps<'gc>
        + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    use dotnet_value::with_string;
    let field_name_val = ctx.pop();
    let field_name = with_string!(ctx, field_name_val, |s| s.as_string());
    let concrete_type = if method.method().signature.parameters.len() == 1 {
        generics.method_generics[0].clone()
    } else {
        let type_obj = ctx.pop_obj();
        ctx.resolve_runtime_type(type_obj)
            .to_concrete(ctx.loader().as_ref())
    };
    let layout = vm_try!(type_layout(concrete_type.clone(), &ctx.current_context()));

    if let LayoutManager::Field(flm) = &*layout {
        if let Some(field) = flm.get_field_by_name(&field_name) {
            ctx.push_isize(field.position.as_usize() as isize);
            return StepResult::Continue;
        } else {
            return ctx.throw_by_name_with_message("System.ArgumentException", "Field not found.");
        }
    }

    if concrete_type.is_class(ctx.loader().as_ref()) {
        let td = vm_try!(ctx.loader().find_concrete_type(concrete_type.clone()));
        let flm = vm_try!(LayoutFactory::instance_field_layout_cached(
            td,
            &ctx.current_context(),
            None
        ));
        if let Some(field) = flm.get_field_by_name(&field_name) {
            ctx.push_isize(field.position.as_usize() as isize);
            return StepResult::Continue;
        } else {
            return ctx.throw_by_name_with_message("System.ArgumentException", "Field not found.");
        }
    }

    ctx.throw_by_name_with_message(
        "System.ArgumentException",
        "Type must be a structure or a class.",
    )
}

#[dotnet_intrinsic_field("static IntPtr System.IntPtr::Zero")]
pub fn intrinsic_field_intptr_zero<'gc, T: EvalStackOps<'gc> + TypedStackOps<'gc>>(
    ctx: &mut T,
    _field: FieldDescription,
    _type_generics: Arc<[ConcreteType]>,
    _is_address: bool,
) -> StepResult {
    ctx.push(StackValue::NativeInt(0));
    StepResult::Continue
}
