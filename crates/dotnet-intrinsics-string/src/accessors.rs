use dotnet_macros::{dotnet_intrinsic, dotnet_intrinsic_field};
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
};
use dotnet_value::{ByteOffset, object::HeapStorage, string::CLRString, with_string};
use dotnet_vm_ops::{
    StepResult,
    ops::{
        CallOps, EvalStackOps, ExceptionOps, LoaderOps, MemoryOps, RawMemoryOps, ReflectionOps,
        ResolutionOps, ThreadOps, TypedStackOps,
    },
};
use std::sync::Arc;

use crate::NULL_REF_MSG;

/// System.String::get_Chars(int)
#[dotnet_intrinsic("char System.String::get_Chars(int)")]
pub fn intrinsic_string_get_chars<
    'gc,
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let index = ctx.pop_i32();
    let val = ctx.pop();
    if val.as_object_ref().0.is_none() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }
    let char_opt = with_string!(ctx, val, |s| if index >= 0 && (index as usize) < s.len() {
        Some(s[index as usize])
    } else {
        None
    });
    if let Some(value) = char_opt {
        ctx.push_i32(value as i32);
        StepResult::Continue
    } else {
        ctx.throw_by_name_with_message(
            "System.IndexOutOfRangeException",
            "Index was out of range. Must be non-negative and less than the size of the collection.",
        )
    }
}

/// System.String::get_Length()
#[dotnet_intrinsic("int System.String::get_Length()")]
pub fn intrinsic_string_get_length<
    'gc,
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop();
    let len = with_string!(ctx, val, |s| s.len());
    ctx.push_i32(len as i32);
    StepResult::Continue
}

/// System.String::GetPinnableReference()
/// System.String::GetRawStringData()
#[dotnet_intrinsic("char& System.String::GetPinnableReference()")]
#[dotnet_intrinsic("char& System.String::GetRawStringData()")]
pub fn intrinsic_string_get_raw_data<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc> + LoaderOps>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop();
    let char_type = crate::vm_try!(ctx.loader().corlib_type("System.Char"));

    let obj = val.as_object_ref();
    if let Some(handle) = obj.0 {
        let (ptr, is_str) = {
            let heap = handle.borrow();
            if let HeapStorage::Str(_) = &heap.storage {
                (unsafe { heap.storage.raw_data_ptr() }, true)
            } else {
                (std::ptr::null_mut(), false)
            }
        };

        if is_str {
            ctx.push_ptr(ptr, char_type, false, Some(obj), Some(ByteOffset(0)));
            StepResult::Continue
        } else {
            let heap = handle.borrow();
            StepResult::internal_error(format!(
                "invalid type on stack, expected string, received {:?}",
                heap.storage
            ))
        }
    } else {
        ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG)
    }
}

#[dotnet_intrinsic_field("static string System.String::Empty")]
pub fn intrinsic_field_string_empty<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + MemoryOps<'gc>
        + CallOps<'gc>
        + ResolutionOps<'gc>
        + RawMemoryOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + ThreadOps
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    _field: FieldDescription,
    _type_generics: Arc<[ConcreteType]>,
    _is_address: bool,
) -> StepResult {
    ctx.push_string(CLRString::new(vec![]));
    StepResult::Continue
}

#[dotnet_intrinsic_field("int System.String::_stringLength")]
pub fn intrinsic_field_string_length<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + MemoryOps<'gc>
        + CallOps<'gc>
        + ResolutionOps<'gc>
        + RawMemoryOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + ThreadOps
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    _field: FieldDescription,
    _type_generics: Arc<[ConcreteType]>,
    is_address: bool,
) -> StepResult {
    if is_address {
        return StepResult::not_implemented("taking address of _stringLength is not supported");
    }
    let val = ctx.pop();
    let len = with_string!(ctx, val, |s| s.len());

    ctx.push_i32(len as i32);
    StepResult::Continue
}
