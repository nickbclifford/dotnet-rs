use crate::{
    StepResult,
    error::ExecutionError,
    instructions::objects::get_ptr_info,
    intrinsics::span::helpers::*,
    layout::type_layout,
    resolution::ValueResolution,
    stack::ops::{
        EvalStackOps, ExceptionOps, LoaderOps, MemoryOps, RawMemoryOps, ReflectionOps,
        ResolutionOps, StackOps, TypedStackOps,
    },
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
};
use dotnet_value::{
    ByteOffset, StackValue,
    layout::{HasLayout, LayoutManager},
    object::{HeapStorage, ObjectRef},
    pointer::{ManagedPtr, PointerOrigin},
};
use dotnetdll::prelude::{BaseType, ParameterType};
use std::ptr::NonNull;

fn pop_nonneg_usize<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + ResolutionOps<'gc>
        + LoaderOps
        + ExceptionOps<'gc>
        + RawMemoryOps<'gc>
        + MemoryOps<'gc>
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
) -> Result<usize, StepResult> {
    match ctx.pop() {
        StackValue::Int32(i) => {
            if i < 0 {
                return Err(ctx.throw_by_name_with_message(
                    "System.ArgumentOutOfRangeException",
                    "Specified argument was out of the range of valid values.",
                ));
            }
            Ok(i as usize)
        }
        _ => Err(ctx.throw_by_name_with_message(
            "System.ArgumentException",
            "The argument must be an integer.",
        )),
    }
}

#[dotnet_intrinsic("static System.ReadOnlySpan<char> System.MemoryExtensions::AsSpan(string)")]
#[dotnet_intrinsic("static System.ReadOnlySpan<char> System.MemoryExtensions::AsSpan(string, int)")]
#[dotnet_intrinsic(
    "static System.ReadOnlySpan<char> System.MemoryExtensions::AsSpan(string, int, int)"
)]
#[dotnet_intrinsic("static System.Span<T> System.MemoryExtensions::AsSpan<T>(T[])")]
#[dotnet_intrinsic("static System.Span<T> System.MemoryExtensions::AsSpan<T>(T[], int)")]
#[dotnet_intrinsic("static System.Span<T> System.MemoryExtensions::AsSpan<T>(T[], int, int)")]
#[dotnet_intrinsic("static System.ReadOnlySpan<T> System.MemoryExtensions::AsSpan<T>(T[])")]
#[dotnet_intrinsic("static System.ReadOnlySpan<T> System.MemoryExtensions::AsSpan<T>(T[], int)")]
#[dotnet_intrinsic(
    "static System.ReadOnlySpan<T> System.MemoryExtensions::AsSpan<T>(T[], int, int)"
)]
pub fn intrinsic_as_span<
    'gc,
    T: StackOps<'gc>
        + ResolutionOps<'gc>
        + LoaderOps
        + ExceptionOps<'gc>
        + RawMemoryOps<'gc>
        + MemoryOps<'gc>
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let param_count = method.method().signature.parameters.len();

    // AsSpan can have 1, 2, or 3 parameters:
    // - AsSpan(string) - whole string
    // - AsSpan(string, int start) - substring from start
    // - AsSpan(string, int start, int length) - substring with length
    // - AsSpan(T[]) - whole array
    // - AsSpan(T[], int start) - array slice from start
    // - AsSpan(T[], int start, int length) - array slice with length
    let (start, length_override) = match param_count {
        1 => (0, None),
        2 => {
            let start = match pop_nonneg_usize(ctx) {
                Ok(v) => v,
                Err(e) => return e,
            };
            (start, None)
        }
        3 => {
            let length = match pop_nonneg_usize(ctx) {
                Ok(v) => v,
                Err(e) => return e,
            };
            let start = match pop_nonneg_usize(ctx) {
                Ok(v) => v,
                Err(e) => return e,
            };
            (start, Some(length))
        }
        _ => {
            return ctx.throw_by_name_with_message(
                "System.ArgumentException",
                "Invalid number of arguments.",
            );
        }
    };

    let source = ctx.pop();

    let res_ctx = ctx.with_generics(generics);

    let (origin, mut offset) = match get_ptr_info(ctx, &source) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let h_opt = match origin {
        PointerOrigin::Heap(ObjectRef(Some(h))) => Some(h),
        _ => None,
    };

    let (base_ptr, total_len, element_type, element_size) = match &source {
        StackValue::ObjectRef(ObjectRef(Some(h))) => {
            let heap = h.borrow();
            match &heap.storage {
                HeapStorage::Str(s) => (
                    unsafe { heap.storage.raw_data_ptr() },
                    s.len(),
                    vm_try!(res_ctx.make_concrete(&BaseType::Char)),
                    2, // char is 2 bytes in .NET
                ),
                HeapStorage::Vec(a) => {
                    let elem_type = a.element.clone();
                    let elem_size = a.layout.element_layout.size();
                    (
                        unsafe { a.raw_data_ptr() },
                        a.layout.length,
                        elem_type,
                        elem_size.as_usize(),
                    )
                }
                _ => {
                    return StepResult::Error(
                        ExecutionError::NotImplemented(format!(
                            "AsSpan called on non-string/non-array object: {:?}",
                            heap.storage
                        ))
                        .into(),
                    );
                }
            }
        }
        StackValue::ObjectRef(ObjectRef(None)) => {
            let element_type = if !generics.method_generics.is_empty() {
                generics.method_generics[0].clone()
            } else {
                vm_try!(res_ctx.make_concrete(&BaseType::Char))
            };
            (std::ptr::null_mut(), 0, element_type, 2)
        }
        _ => {
            return ctx.throw_by_name_with_message(
                "System.ArgumentException",
                "The argument must be a string or an array.",
            );
        }
    };

    // Apply start and length_override
    if start > total_len {
        return ctx.throw_by_name_with_message("System.ArgumentOutOfRangeException", "start");
    }
    let actual_length = if let Some(len) = length_override {
        if start + len > total_len {
            return ctx.throw_by_name_with_message("System.ArgumentOutOfRangeException", "length");
        }
        len
    } else {
        total_len - start
    };

    let ptr = if base_ptr.is_null() {
        base_ptr
    } else {
        unsafe { base_ptr.add(start * element_size) }
    };
    offset += dotnet_utils::ByteOffset(start * element_size);
    let len = actual_length;

    let span_type_concrete = match &method.method().signature.return_type.1 {
        Some(ParameterType::Value(t)) => {
            vm_try!(generics.make_concrete(method.resolution(), t.clone(), ctx.loader().as_ref()))
        }
        Some(_) => {
            return StepResult::Error(
                ExecutionError::InternalError(
                    "AsSpan called on method with ref/typedref return".to_string(),
                )
                .into(),
            );
        }
        None => {
            return StepResult::Error(
                ExecutionError::InternalError("AsSpan called on method returning void".to_string())
                    .into(),
            );
        }
    };
    let span_type = vm_try!(ctx.loader().find_concrete_type(span_type_concrete.clone()));

    let layout = vm_try!(type_layout(span_type_concrete.clone(), &res_ctx));

    let (_ref_offset_rel, _length_offset_rel) = match &*layout {
        LayoutManager::Field(f) => {
            let ref_off = vm_try!(f.get_field_by_name("_reference").ok_or_else(|| {
                ExecutionError::InternalError("Span must have _reference field".to_string())
            }))
            .position;
            let len_off = vm_try!(f.get_field_by_name("_length").ok_or_else(|| {
                ExecutionError::InternalError("Span must have _length field".to_string())
            }))
            .position;
            (ref_off, len_off)
        }
        _ => {
            return StepResult::Error(
                ExecutionError::InternalError("Expected Field layout for Span".to_string()).into(),
            );
        }
    };

    let new_lookup = GenericLookup::new(vec![element_type.clone()]);
    let res_ctx_generic = res_ctx.with_generics(&new_lookup);

    let span = vm_try!(res_ctx_generic.new_object(span_type));

    let element_type_desc = vm_try!(ctx.loader().find_concrete_type(element_type));
    let managed = ManagedPtr::new(
        NonNull::new(ptr),
        element_type_desc,
        h_opt.map(|h| ObjectRef(Some(h))),
        false,
        Some(offset),
    );
    span.instance_storage
        .field::<ManagedPtr<'gc>>(span.description.clone(), "_reference")
        .unwrap()
        .write(managed);
    span.instance_storage
        .field::<i32>(span.description.clone(), "_length")
        .unwrap()
        .write(len as i32);

    ctx.push_value_type(span);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static System.Span<T> System.Runtime.CompilerServices.RuntimeHelpers::CreateSpan<T>(System.RuntimeFieldHandle)"
)]
pub fn intrinsic_runtime_helpers_create_span<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + ResolutionOps<'gc>
        + LoaderOps
        + ExceptionOps<'gc>
        + RawMemoryOps<'gc>
        + MemoryOps<'gc>
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let element_type = &generics.method_generics[0];
    let res_ctx = ctx.with_generics(generics);
    let element_size = vm_try!(type_layout(element_type.clone(), &res_ctx)).size();

    let field_handle = ctx.pop_value_type();

    let res_ctx_handle = ctx.current_context();
    let handle_layout = vm_try!(type_layout(
        ConcreteType::from(field_handle.description.clone()),
        &res_ctx_handle
    ));
    let _value_offset = match &*handle_layout {
        LayoutManager::Field(f) => vm_try!(f.get_field_by_name("_value").ok_or_else(|| {
            ExecutionError::InternalError("RuntimeFieldHandle must have _value field".to_string())
        }))
        .position
        .as_usize(),
        _ => {
            return StepResult::Error(
                ExecutionError::InternalError(
                    "Expected Field layout for RuntimeFieldHandle".to_string(),
                )
                .into(),
            );
        }
    };

    let (field_desc, lookup) = {
        let obj_ref = field_handle
            .instance_storage
            .field::<ObjectRef<'gc>>(field_handle.description, "_value")
            .unwrap()
            .read();
        ctx.resolve_runtime_field(obj_ref)
    };
    let field = field_desc.field();
    let field_resolution = field_desc.field_resolution;
    let field_type = vm_try!(lookup.make_concrete(
        field_resolution,
        field.return_type.clone(),
        ctx.loader().as_ref(),
    ));
    let field_desc = vm_try!(ctx.loader().find_concrete_type(field_type.clone()));

    let Some(initial_data) = &field.initial_value else {
        return ctx.throw_by_name_with_message(
            "System.ArgumentException",
            "The field does not have initial data.",
        );
    };

    if field_desc
        .definition()
        .name
        .starts_with("__StaticArrayInitTypeSize=")
    {
        let prefix = "__StaticArrayInitTypeSize=";
        let size_str = &field_desc.definition().name[prefix.len()..];
        let size_end = size_str.find('_').unwrap_or(size_str.len());
        let array_size = vm_try!(size_str[..size_end].parse::<usize>().map_err(|e| {
            ExecutionError::InternalError(format!("Failed to parse array size: {}", e))
        }));
        let data_slice = &initial_data[..array_size];

        let span_type = vm_try!(ctx.loader().corlib_type("System.ReadOnlySpan`1"));
        let span_lookup = GenericLookup::new(vec![element_type.clone()]);
        let span_res_ctx = res_ctx.with_generics(&span_lookup);
        let span_instance = vm_try!(span_res_ctx.new_object(span_type.clone()));

        let layout = vm_try!(type_layout(ConcreteType::from(span_type), &span_res_ctx));
        let (_ref_offset, _length_offset) = match &*layout {
            LayoutManager::Field(f) => {
                let ref_off = vm_try!(f.get_field_by_name("_reference").ok_or_else(|| {
                    ExecutionError::NotImplemented("Span must have _reference field".to_string())
                }))
                .position;
                let len_off = vm_try!(f.get_field_by_name("_length").ok_or_else(|| {
                    ExecutionError::NotImplemented("Span must have _length field".to_string())
                }))
                .position;
                (ref_off, len_off)
            }
            _ => {
                return StepResult::Error(
                    ExecutionError::NotImplemented("Expected Field layout for Span".to_string())
                        .into(),
                );
            }
        };

        let element_desc = vm_try!(ctx.loader().find_concrete_type(element_type.clone()));
        let managed = ManagedPtr::new(
            NonNull::new(data_slice.as_ptr() as *mut u8),
            element_desc,
            None,
            false,
            None,
        );
        span_instance
            .instance_storage
            .field::<ManagedPtr<'gc>>(span_instance.description.clone(), "_reference")
            .unwrap()
            .write(managed);

        let element_count = (array_size / element_size.as_usize()) as i32;
        span_instance
            .instance_storage
            .field::<i32>(span_instance.description.clone(), "_length")
            .unwrap()
            .write(element_count);

        ctx.push_value_type(span_instance);
        StepResult::Continue
    } else {
        todo!("initial field data for {:?}", field_desc);
    }
}

#[dotnet_intrinsic(
    "static T& System.Runtime.CompilerServices.RuntimeHelpers::GetSpanDataFrom<T>(T&, System.Type, int&)"
)]
pub fn intrinsic_runtime_helpers_get_span_data_from<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + ResolutionOps<'gc>
        + LoaderOps
        + ExceptionOps<'gc>
        + RawMemoryOps<'gc>
        + MemoryOps<'gc>
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let length_ref = ctx.pop_managed_ptr();
    let type_handle = ctx.pop_value_type();
    let field_handle = ctx.pop_value_type();

    // Resolve field
    let res_ctx_field = ctx.current_context();
    let field_layout = vm_try!(type_layout(
        ConcreteType::from(field_handle.description.clone()),
        &res_ctx_field
    ));
    let _field_value_offset = match &*field_layout {
        LayoutManager::Field(f) => vm_try!(f.get_field_by_name("_value").ok_or_else(|| {
            ExecutionError::NotImplemented("RuntimeFieldHandle must have _value field".to_string())
        }))
        .position
        .as_usize(),
        _ => {
            return StepResult::Error(
                ExecutionError::NotImplemented(
                    "Expected Field layout for RuntimeFieldHandle".to_string(),
                )
                .into(),
            );
        }
    };

    let (field_desc, _) = {
        let obj_ref = field_handle
            .instance_storage
            .field::<ObjectRef<'gc>>(field_handle.description, "_value")
            .unwrap()
            .read();
        ctx.resolve_runtime_field(obj_ref)
    };
    let field = field_desc.field();

    // Resolve type
    let res_ctx_type = ctx.current_context();
    let runtime_type_layout = vm_try!(type_layout(
        ConcreteType::from(type_handle.description.clone()),
        &res_ctx_type
    ));
    let _type_value_offset = match &*runtime_type_layout {
        LayoutManager::Field(f) => vm_try!(f.get_field_by_name("_value").ok_or_else(|| {
            ExecutionError::NotImplemented("RuntimeTypeHandle must have _value field".to_string())
        }))
        .position
        .as_usize(),
        _ => {
            return StepResult::Error(
                ExecutionError::NotImplemented(
                    "Expected Field layout for RuntimeTypeHandle".to_string(),
                )
                .into(),
            );
        }
    };

    let element_type_runtime = {
        let obj_ref = type_handle
            .instance_storage
            .field::<ObjectRef<'gc>>(type_handle.description, "_value")
            .unwrap()
            .read();
        ctx.resolve_runtime_type(obj_ref)
    };

    let element_type: ConcreteType = element_type_runtime.to_concrete(ctx.loader().as_ref());

    let res_ctx = ctx.with_generics(generics);
    let element_size = vm_try!(type_layout(element_type.clone(), &res_ctx)).size();

    let Some(initial_data) = &field.initial_value else {
        ctx.push_isize(0);
        return StepResult::Continue;
    };

    if field.name.starts_with("__StaticArrayInitTypeSize=") {
        let prefix = "__StaticArrayInitTypeSize=";
        let size_str = &field.name[prefix.len()..];
        let size_end = size_str.find('_').unwrap_or(size_str.len());
        let array_size = vm_try!(size_str[..size_end].parse::<usize>().map_err(|e| {
            ExecutionError::InternalError(format!("Failed to parse array size: {}", e))
        }));

        let element_count = (array_size / element_size.as_usize()) as i32;
        vm_try!(
            unsafe {
                ctx.write_bytes(
                    length_ref.origin().clone(),
                    length_ref.byte_offset(),
                    &element_count.to_ne_bytes(),
                )
            }
            .map_err(|e| ExecutionError::InternalError(e.to_string()))
        );

        let element_desc = vm_try!(ctx.loader().find_concrete_type(element_type.clone()));
        let managed = ManagedPtr::new(
            NonNull::new(initial_data.as_ptr() as *mut u8),
            element_desc,
            None,
            false,
            None,
        );
        ctx.push_managed_ptr(managed);
    } else {
        return ctx.throw_by_name_with_message(
            "System.ArgumentException",
            "The field is not a static array initialization field.",
        );
    }
    StepResult::Continue
}

#[dotnet_intrinsic("static byte& DotnetRs.Internal::GetArrayData(System.Array)")]
pub fn intrinsic_internal_get_array_data<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + ResolutionOps<'gc>
        + LoaderOps
        + ExceptionOps<'gc>
        + RawMemoryOps<'gc>
        + MemoryOps<'gc>
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let array_ref = ctx.pop_obj();

    let element_type = if !generics.method_generics.is_empty() {
        generics.method_generics[0].clone()
    } else {
        return StepResult::Error(
            ExecutionError::NotImplemented("GetArrayData expected generic argument".to_string())
                .into(),
        );
    };

    let element_type_desc = vm_try!(ctx.loader().find_concrete_type(element_type));

    if let Some(handle) = array_ref.0 {
        let inner = handle.borrow();
        if let HeapStorage::Vec(v) = &inner.storage {
            let ptr = unsafe { v.raw_data_ptr() };

            // For Vectors, the ManagedPtr offset must be relative to the raw data pointer
            // (returned by raw_data_ptr()), not the Object pointer.
            // Since we are pointing to the start of the data, the offset is 0.
            let offset = 0;

            let managed = ManagedPtr::new(
                NonNull::new(ptr),
                element_type_desc,
                Some(array_ref),
                false,
                Some(ByteOffset(offset)),
            );
            ctx.push_managed_ptr(managed);
        } else {
            return StepResult::Error(
                ExecutionError::NotImplemented(
                    "GetArrayData called on non-vector object".to_string(),
                )
                .into(),
            );
        }
    } else {
        let managed = ManagedPtr::new(None, element_type_desc, None, false, None);
        ctx.push_managed_ptr(managed);
    }
    StepResult::Continue
}

#[dotnet_intrinsic("T& System.Span<T>::GetPinnableReference()")]
#[dotnet_intrinsic("T& System.ReadOnlySpan<T>::GetPinnableReference()")]
pub fn intrinsic_span_get_pinnable_reference<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + ResolutionOps<'gc>
        + LoaderOps
        + ExceptionOps<'gc>
        + RawMemoryOps<'gc>
        + MemoryOps<'gc>
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let span = ctx.pop_managed_ptr();

    let element_type = &generics.type_generics[0];
    let element_desc = vm_try!(ctx.loader().find_concrete_type(element_type.clone()));

    let res_ctx = ctx.current_context();
    let layout = vm_try!(type_layout(ConcreteType::from(span.inner_type()), &res_ctx));

    let LayoutManager::Field(f) = &*layout else {
        return StepResult::Error(
            ExecutionError::NotImplemented("Expected Field layout for Span".to_string()).into(),
        );
    };

    // Read fields using helpers
    let managed_ref = match read_span_reference_from_ptr(&span, f, ctx) {
        Ok(m) => m,
        Err(e) => return StepResult::Error(e.into()),
    };
    let length = match read_span_length_from_ptr(&span, f, ctx) {
        Ok(l) => l,
        Err(e) => return StepResult::Error(e.into()),
    };

    // If the span is empty, return a null reference
    if length == 0 {
        let null_ref = ManagedPtr::new(None, element_desc, None, false, None);
        ctx.push_managed_ptr(null_ref);
    } else {
        // Return a managed pointer to the first element
        let mut managed = managed_ref;
        managed = managed.with_inner_type(element_desc);
        ctx.push_managed_ptr(managed);
    }

    StepResult::Continue
}
