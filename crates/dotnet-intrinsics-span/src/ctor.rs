use crate::{SpanIntrinsicHost, helpers::*};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
};
use dotnet_value::{
    StackValue,
    layout::LayoutManager,
    object::{HeapStorage, ObjectRef},
    pointer::{ManagedPtr, UnmanagedPtr},
};
use dotnet_vm_ops::StepResult;
use std::ptr::NonNull;

#[dotnet_intrinsic("void System.Span<T>::.ctor(void*, int)")]
#[dotnet_intrinsic("void System.ReadOnlySpan<T>::.ctor(void*, int)")]
pub fn intrinsic_span_ctor_from_pointer<'gc, T: SpanIntrinsicHost<'gc>>(
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
    let element_desc =
        dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(element_type.clone()));

    let element_layout = dotnet_vm_ops::vm_try!(ctx.span_type_layout(element_type.clone()));

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
            m.into_inner()
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
    let layout = dotnet_vm_ops::vm_try!(ctx.span_type_layout(ConcreteType::from(span_type)));

    let LayoutManager::Field(f) = &*layout else {
        return StepResult::not_implemented("Expected Field layout for Span");
    };

    if let Err(e) = write_span_fields(&this_ptr, &managed, length, f, ctx) {
        return StepResult::Error(e.into());
    }

    StepResult::Continue
}

#[dotnet_intrinsic("void System.Span<T>::.ctor(T[])")]
#[dotnet_intrinsic("void System.ReadOnlySpan<T>::.ctor(T[])")]
pub fn intrinsic_span_ctor_from_array<'gc, T: SpanIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let array = ctx.pop();
    let this_ptr = ctx.pop_managed_ptr();

    let element_type = &generics.type_generics[0];
    let element_desc =
        dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(element_type.clone()));

    let (managed, length) = match array {
        StackValue::ObjectRef(ObjectRef(None)) => {
            (ManagedPtr::new(None, element_desc, None, false, None), 0)
        }
        // Arity-based intrinsic lookup can route `Span<T>(ref T)` here because
        // both `.ctor(T[])` and `.ctor(ref T)` are instance arity-2 overloads.
        // Accept managed pointers and treat them as a single-element span.
        StackValue::ManagedPtr(ptr) => (ptr.into_inner(), 1),
        StackValue::ObjectRef(ObjectRef(Some(handle))) => {
            let (ptr, len) = {
                let inner = handle.borrow();
                match &inner.storage {
                    HeapStorage::Vec(v) => {
                        // SAFETY: We only take the base pointer while holding `inner` borrow.
                        // The owner handle is stored in `ManagedPtr`, so subsequent accesses are
                        // validated against this vector's lifetime and bounds.
                        (unsafe { v.raw_data_ptr() }, v.layout.length as i32)
                    }
                    _ => {
                        return ctx.throw_by_name_with_message(
                            "System.ArgumentException",
                            "Span<T>(T[]) requires an array argument.",
                        );
                    }
                }
            };

            (
                ManagedPtr::new(
                    NonNull::new(ptr),
                    element_desc,
                    Some(ObjectRef(Some(handle))),
                    false,
                    None,
                ),
                len,
            )
        }
        _ => {
            return ctx.throw_by_name_with_message(
                "System.ArgumentException",
                "Span<T>(T[]) requires an array argument.",
            );
        }
    };

    let span_type = this_ptr.inner_type();
    let layout = dotnet_vm_ops::vm_try!(ctx.span_type_layout(ConcreteType::from(span_type)));
    let LayoutManager::Field(f) = &*layout else {
        return StepResult::not_implemented("Expected Field layout for Span");
    };

    if let Err(e) = write_span_fields(&this_ptr, &managed, length, f, ctx) {
        return StepResult::Error(e.into());
    }

    StepResult::Continue
}
