use crate::{
    StepResult,
    error::ExecutionError,
    intrinsics::span::helpers::*,
    layout::type_layout,
    stack::ops::{
        ExceptionOps, LoaderOps, RawMemoryOps, ResolutionOps, TypedStackOps,
    },
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
};
use dotnet_value::{
    StackValue,
    layout::LayoutManager,
    object::ObjectRef,
    pointer::{ManagedPtr, UnmanagedPtr},
};
use std::ptr::NonNull;

#[dotnet_intrinsic("void System.Span<T>::.ctor(void*, int)")]
#[dotnet_intrinsic("void System.ReadOnlySpan<T>::.ctor(void*, int)")]
pub fn intrinsic_span_ctor_from_pointer<
    'gc,
     T: TypedStackOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let length = ctx.pop_i32();
    let ptr_val = ctx.pop();
    let this_ptr = ctx.pop_managed_ptr();

    if length < 0 {
        return ctx.throw_by_name_with_message("System.ArgumentOutOfRangeException", "length");
    }

    let element_type = &generics.type_generics[0];
    let element_desc = vm_try!(ctx.loader().find_concrete_type(element_type.clone()));

    let res_ctx = ctx.current_context();
    let element_layout = vm_try!(type_layout(element_type.clone(), &res_ctx));

    // Span<T>(void*, int) is only valid for types that do not contain references
    if element_layout.is_or_contains_refs() {
        return ctx.throw_by_name_with_message(
            "System.ArgumentException",
            "The type cannot contain references.",
        );
    }

    // The pointer can be a NativeInt, ManagedPtr, or UnmanagedPtr
    let managed = match ptr_val {
        StackValue::NativeInt(ptr) => {
            if ptr == 0 && length > 0 {
                return ctx.throw_by_name_with_message("System.ArgumentOutOfRangeException", "ptr");
            }
            // Create a ManagedPtr from the native pointer
            ManagedPtr::new(
                NonNull::new(ptr as *mut u8),
                element_desc,
                None, // unmanaged pointer
                false,
                None,
            )
        }
        StackValue::UnmanagedPtr(UnmanagedPtr(ptr)) => {
            // Create a ManagedPtr from the unmanaged pointer
            ManagedPtr::new(
                Some(ptr),
                element_desc,
                None, // unmanaged pointer
                false,
                None,
            )
        }
        StackValue::ManagedPtr(m) => {
            if m.is_null() && length > 0 {
                return ctx.throw_by_name_with_message("System.ArgumentOutOfRangeException", "ptr");
            }
            // Already a managed pointer, just use it
            m
        }
        StackValue::ObjectRef(ObjectRef(None)) => {
            if length > 0 {
                return ctx.throw_by_name_with_message("System.ArgumentOutOfRangeException", "ptr");
            }
            ManagedPtr::new(None, element_desc, None, false, None)
        }
        _ => {
            return ctx
                .throw_by_name_with_message("System.ArgumentException", "Invalid pointer value.");
        }
    };

    let span_type = this_ptr.inner_type();
    let res_ctx = ctx.current_context();
    let layout = vm_try!(type_layout(ConcreteType::from(span_type), &res_ctx));

    let LayoutManager::Field(f) = &*layout else {
        return StepResult::Error(
            ExecutionError::NotImplemented("Expected Field layout for Span".to_string()).into(),
        );
    };

    if let Err(e) = write_span_fields(&this_ptr, &managed, length, f, ctx) {
        return StepResult::Error(e.into());
    }

    StepResult::Continue
}
