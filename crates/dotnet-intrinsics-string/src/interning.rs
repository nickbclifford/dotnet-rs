use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
};
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::ops::{ExceptionOps, TypedStackOps};

/// Public interning methods replace the CoreCLR-specific QCalls used by CoreLib.
#[dotnet_intrinsic("static string System.String::Intern(string)")]
#[dotnet_intrinsic("static string System.String::IsInterned(string)")]
pub fn intrinsic_string_intern<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = ctx.peek_stack();
    let chars = match &value {
        StackValue::ObjectRef(ObjectRef(Some(handle))) => {
            let inner = handle.borrow();
            match &inner.storage {
                HeapStorage::Str(s) => s.to_vec(),
                _ => return StepResult::type_error("string", "non-string object reference"),
            }
        }
        StackValue::ObjectRef(ObjectRef(None)) => {
            return ctx.throw_by_name_with_message(
                "System.ArgumentNullException",
                "Value cannot be null. (Parameter 'str')",
            );
        }
        other => return StepResult::type_error("string", format!("{other:?}")),
    };
    ctx.drop_top(1);
    if method.method().name == "Intern" {
        ctx.push_interned_string_object(&chars, value.as_object_ref());
    } else {
        let object = ctx.find_interned_string(&chars);
        ctx.push_obj(object);
    }
    StepResult::Continue
}
