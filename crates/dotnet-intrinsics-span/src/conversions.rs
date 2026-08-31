use crate::{SpanIntrinsicHost, helpers::*};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    WellKnown,
    error::ExecutionError,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
};
use dotnet_utils::ByteOffset;
use dotnet_value::{
    StackValue,
    layout::{HasLayout, LayoutManager},
    object::{HeapStorage, ObjectRef},
    pointer::{ManagedPtr, PointerOrigin},
};
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::SupportSlotOps;
use dotnetdll::prelude::{BaseType, MethodType, ParameterType};
use std::ptr::NonNull;

fn parse_static_array_size(name: &str) -> Result<usize, ExecutionError> {
    let prefix = "__StaticArrayInitTypeSize=";
    let size_str = &name[prefix.len()..];
    let size_end = size_str.find('_').unwrap_or(size_str.len());
    size_str[..size_end].parse::<usize>().map_err(|e| {
        ExecutionError::InternalError(format!("Failed to parse array size: {e}").into())
    })
}

fn pop_nonneg_usize<'gc, T: SpanIntrinsicHost<'gc>>(ctx: &mut T) -> Result<usize, StepResult> {
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

fn make_char_element_type<'gc, T: SpanIntrinsicHost<'gc>>(
    ctx: &mut T,
) -> Result<ConcreteType, StepResult> {
    ctx.make_concrete(&MethodType::Base(Box::new(BaseType::Char)))
        .map_err(|e| StepResult::Error(e.into()))
}

struct SpanSourceData {
    base_ptr: *mut u8,
    total_len: usize,
    element_type: ConcreteType,
    element_size: usize,
}

fn extract_span_source<'gc, T: SpanIntrinsicHost<'gc>>(
    ctx: &mut T,
    source: &StackValue<'gc>,
    generics: &GenericLookup,
) -> Result<SpanSourceData, StepResult> {
    match source {
        StackValue::ObjectRef(ObjectRef(Some(h))) => {
            let heap = h.borrow();
            match &heap.storage {
                HeapStorage::Str(s) => {
                    let element_type = make_char_element_type(ctx)?;

                    Ok(SpanSourceData {
                        // SAFETY: F10.BorrowedStorageStable — `heap` borrow pins the string storage for this scope; we only
                        // use the pointer to build a managed reference with validated bounds below.
                        base_ptr: unsafe { heap.storage.raw_data_ptr() },
                        total_len: s.len(),
                        element_type,
                        element_size: 2, // char is 2 bytes in .NET
                    })
                }
                HeapStorage::Vec(a) => Ok(SpanSourceData {
                    // SAFETY: F10.BorrowedStorageStable — `heap` borrow keeps vector storage alive and we only derive an
                    // offset pointer after explicit start/length range checks.
                    base_ptr: unsafe { a.raw_data_ptr() },
                    total_len: a.layout.length,
                    element_type: a.element.clone(),
                    element_size: a.layout.element_layout.size().as_usize(),
                }),
                _ => Err(StepResult::Error(
                    ExecutionError::NotImplemented(
                        format!(
                            "AsSpan called on non-string/non-array object: {:?}",
                            heap.storage
                        )
                        .into(),
                    )
                    .into(),
                )),
            }
        }
        StackValue::ObjectRef(ObjectRef(None)) => {
            let element_type = if !generics.method_generics.is_empty() {
                generics
                    .cloned_method_arg(0)
                    .map_err(|e| StepResult::Error(e.into()))?
            } else {
                make_char_element_type(ctx)?
            };

            Ok(SpanSourceData {
                base_ptr: std::ptr::null_mut::<u8>(),
                total_len: 0,
                element_type,
                element_size: 2,
            })
        }
        _ => Err(ctx.throw_by_name_with_message(
            "System.ArgumentException",
            "The argument must be a string or an array.",
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
pub fn intrinsic_as_span<'gc, T: SpanIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let param_count = method.signature().parameters.len();

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

    let (origin, mut offset) = match ctx.span_ptr_info(&source) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let h_opt = match origin {
        PointerOrigin::Heap(ObjectRef(Some(h))) => Some(h),
        _ => None,
    };

    let SpanSourceData {
        base_ptr,
        total_len,
        element_type,
        element_size,
    } = match extract_span_source(ctx, &source, generics) {
        Ok(v) => v,
        Err(e) => return e,
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
    let byte_start = start * element_size;
    debug_assert_eq!(
        start.checked_mul(element_size),
        Some(byte_start),
        "AsSpan byte offset overflowed usize during pointer arithmetic"
    );

    let ptr = if base_ptr.is_null() {
        base_ptr
    } else {
        // SAFETY: F10.RawMemoryAccessValid — `start <= total_len` and `actual_length` checks above ensure this computed
        // element offset remains within the source span's backing allocation.
        unsafe { base_ptr.add(byte_start) }
    };
    offset += ByteOffset::new(byte_start);
    let len = actual_length;

    let span_type_concrete = match &method.signature().return_type.1 {
        Some(ParameterType::Value(t)) => {
            dotnet_vm_ops::vm_try!(generics.make_concrete(
                method.resolution(),
                t.clone(),
                ctx.loader().as_ref()
            ))
        }
        Some(_) => {
            return StepResult::Error(
                ExecutionError::InternalError(
                    "AsSpan called on method with ref/typedref return".into(),
                )
                .into(),
            );
        }
        None => {
            return StepResult::Error(
                ExecutionError::InternalError("AsSpan called on method returning void".into())
                    .into(),
            );
        }
    };
    let span_type =
        dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(span_type_concrete.clone()));

    let span = dotnet_vm_ops::vm_try!(
        ctx.span_new_object_with_type_generics(span_type, vec![element_type.clone()],)
    );

    let element_type_desc = dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(element_type));
    let managed = ManagedPtr::new(
        NonNull::new(ptr),
        element_type_desc,
        h_opt.map(|h| ObjectRef(Some(h))),
        false,
        Some(offset),
    );
    ctx.loader()
        .span_or_readonly_span_reference_field(&span.instance_storage, span.description.clone())
        .expect("validated Span<T>/ReadOnlySpan<T> support slot")
        .write(managed);
    ctx.loader()
        .span_or_readonly_span_length_field(&span.instance_storage, span.description.clone())
        .expect("validated Span<T>/ReadOnlySpan<T> support slot")
        .write(len as i32);

    ctx.push_value_type(span);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static System.Span<T> System.Runtime.CompilerServices.RuntimeHelpers::CreateSpan<T>(System.RuntimeFieldHandle)"
)]
pub fn intrinsic_runtime_helpers_create_span<'gc, T: SpanIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let element_type = dotnet_vm_ops::vm_try!(generics.method_arg(0));
    let element_size = dotnet_vm_ops::vm_try!(ctx.span_type_layout(element_type.clone())).size();

    let field_handle = ctx.pop_value_type();

    let (field_desc, lookup) = {
        let obj_ref = ctx
            .loader()
            .rfh_value_field(&field_handle.instance_storage, field_handle.description)
            .expect("validated RuntimeFieldHandle support slot")
            .read();
        dotnet_vm_ops::vm_try!(ctx.span_resolve_runtime_field(obj_ref))
    };
    let field = field_desc.field();
    let field_resolution = field_desc.field_resolution;
    let field_type = dotnet_vm_ops::vm_try!(lookup.make_concrete(
        field_resolution,
        field.return_type.clone(),
        ctx.loader().as_ref(),
    ));
    let field_desc = dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(field_type.clone()));

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
        let array_size =
            dotnet_vm_ops::vm_try!(parse_static_array_size(&field_desc.definition().name));
        let data_slice = &initial_data[..array_size];

        let span_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_wkt(WellKnown::ReadOnlySpan1));
        let span_instance = dotnet_vm_ops::vm_try!(
            ctx.span_new_object_with_type_generics(span_type.clone(), vec![element_type.clone()])
        );

        let element_desc =
            dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(element_type.clone()));
        let managed = ManagedPtr::new(
            NonNull::new(data_slice.as_ptr() as *mut u8),
            element_desc,
            None,
            false,
            None,
        );
        ctx.loader()
            .readonly_span_reference_field(
                &span_instance.instance_storage,
                span_instance.description.clone(),
            )
            .expect("validated Span<T>/ReadOnlySpan<T> support slot")
            .write(managed);

        let element_count = (array_size / element_size.as_usize()) as i32;
        ctx.loader()
            .readonly_span_length_field(
                &span_instance.instance_storage,
                span_instance.description.clone(),
            )
            .expect("validated Span<T>/ReadOnlySpan<T> support slot")
            .write(element_count);

        ctx.push_value_type(span_instance);
        StepResult::Continue
    } else {
        StepResult::Error(
            ExecutionError::NotImplemented(
                format!("initial field data for {:?}", field_desc).into(),
            )
            .into(),
        )
    }
}

#[dotnet_intrinsic(
    "static T& System.Runtime.CompilerServices.RuntimeHelpers::GetSpanDataFrom<T>(T&, System.Type, int&)"
)]
pub fn intrinsic_runtime_helpers_get_span_data_from<'gc, T: SpanIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let length_ref = ctx.pop_managed_ptr();
    let type_handle = ctx.pop_value_type();
    let field_handle = ctx.pop_value_type();

    // Resolve field
    let (field_desc, _) = {
        let obj_ref = ctx
            .loader()
            .rfh_value_field(&field_handle.instance_storage, field_handle.description)
            .expect("validated RuntimeFieldHandle support slot")
            .read();
        dotnet_vm_ops::vm_try!(ctx.span_resolve_runtime_field(obj_ref))
    };
    let field = field_desc.field();

    // Resolve type
    let element_type_runtime = {
        let obj_ref = ctx
            .loader()
            .rth_value_field(&type_handle.instance_storage, type_handle.description)
            .expect("validated RuntimeTypeHandle support slot")
            .read();
        dotnet_vm_ops::vm_try!(ctx.span_resolve_runtime_type(obj_ref))
    };

    let element_type: ConcreteType = element_type_runtime.to_concrete(ctx.loader().as_ref());

    let element_size = dotnet_vm_ops::vm_try!(ctx.span_type_layout(element_type.clone())).size();

    let Some(initial_data) = &field.initial_value else {
        ctx.push_isize(0);
        return StepResult::Continue;
    };

    if field.name.starts_with("__StaticArrayInitTypeSize=") {
        let array_size = dotnet_vm_ops::vm_try!(parse_static_array_size(&field.name));

        let element_count = (array_size / element_size.as_usize()) as i32;
        dotnet_vm_ops::vm_try!(
            // SAFETY: F10.RawMemoryAccessValid — `length_ref` points to Span `_length` field and we write exactly 4 bytes
            // (`i32`) to that location.
            unsafe {
                ctx.write_bytes(
                    length_ref.origin().clone(),
                    length_ref.byte_offset(),
                    &element_count.to_ne_bytes(),
                )
            }
            .map_err(|e| ExecutionError::InternalError(e.to_string().into()))
        );

        let element_desc =
            dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(element_type.clone()));
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

#[dotnet_intrinsic(
    "static void System.Runtime.CompilerServices.RuntimeHelpers::InitializeArray(System.Array, System.RuntimeFieldHandle)"
)]
pub fn intrinsic_runtime_helpers_initialize_array<'gc, T: SpanIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let field_handle = ctx.pop_value_type();
    let array_ref = ctx.pop_obj();

    let ObjectRef(Some(array_handle)) = array_ref else {
        return ctx
            .throw_by_name_with_message("System.ArgumentNullException", "Value cannot be null.");
    };

    let field_obj_ref = ctx
        .loader()
        .rfh_value_field(
            &field_handle.instance_storage,
            field_handle.description.clone(),
        )
        .expect("validated RuntimeFieldHandle support slot")
        .read();

    if field_obj_ref.0.is_none() {
        return ctx
            .throw_by_name_with_message("System.ArgumentException", "Invalid RuntimeFieldHandle.");
    }

    let (field_desc, _) = dotnet_vm_ops::vm_try!(ctx.span_resolve_runtime_field(field_obj_ref));
    let Some(initial_data) = &field_desc.field().initial_value else {
        return ctx.throw_by_name_with_message(
            "System.ArgumentException",
            "The field does not have initial data.",
        );
    };

    let mut array_heap =
        array_handle.borrow_mut(&ctx.gc_with_token(&ctx.no_active_borrows_token()));
    let HeapStorage::Vec(vector) = &mut array_heap.storage else {
        return ctx.throw_by_name_with_message(
            "System.ArgumentException",
            "Object must be of type Array.",
        );
    };

    let destination = vector.get_mut();
    // RuntimeFieldHandle blobs can include trailing alignment padding.
    // The runtime should only reject when the blob is too small for the destination array.
    if initial_data.len() < destination.len() {
        return ctx.throw_by_name_with_message(
            "System.ArgumentException",
            "Field data is smaller than destination array storage.",
        );
    }
    destination.copy_from_slice(&initial_data[..destination.len()]);
    StepResult::Continue
}

#[dotnet_intrinsic("static byte& DotnetRs.Internal::GetArrayData(System.Array)")]
pub fn intrinsic_internal_get_array_data<'gc, T: SpanIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let array_ref = ctx.pop_obj();

    let element_type = if !generics.method_generics.is_empty() {
        dotnet_vm_ops::vm_try!(generics.cloned_method_arg(0))
    } else {
        return StepResult::Error(
            ExecutionError::NotImplemented("GetArrayData expected generic argument".into()).into(),
        );
    };

    let element_type_desc = dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(element_type));

    if let Some(handle) = array_ref.0 {
        let inner = handle.borrow();
        if let HeapStorage::Vec(v) = &inner.storage {
            // SAFETY: F10.BorrowedStorageStable — `inner` borrow keeps vector backing storage alive while deriving a pointer to
            // its first element.
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
                Some(ByteOffset::new(offset)),
            );
            ctx.push_managed_ptr(managed);
        } else {
            return StepResult::Error(
                ExecutionError::NotImplemented("GetArrayData called on non-vector object".into())
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
pub fn intrinsic_span_get_pinnable_reference<'gc, T: SpanIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let span = ctx.pop_managed_ptr();

    let element_type = dotnet_vm_ops::vm_try!(generics.type_arg(0));
    let element_desc =
        dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(element_type.clone()));

    let layout =
        dotnet_vm_ops::vm_try!(ctx.span_type_layout(ConcreteType::from(span.inner_type())));

    let LayoutManager::Field(f) = &*layout else {
        return StepResult::Error(
            ExecutionError::NotImplemented("Expected Field layout for Span".into()).into(),
        );
    };

    // Read fields using helpers
    let managed_ref = match read_span_reference_from_ptr(&span, f, ctx.loader().as_ref(), ctx) {
        Ok(m) => m,
        Err(e) => return StepResult::Error(e.into()),
    };
    let length = match read_span_length_from_ptr(&span, f, ctx.loader().as_ref(), ctx) {
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

/// `MemoryMarshal.GetReference` is the static counterpart of a span's pinnable-reference
/// accessor. Framework code uses it for branch-free span traversal, including the globalization
/// path reached while EF Core initializes its diagnostics source.
#[dotnet_intrinsic(
    "static T& System.Runtime.InteropServices.MemoryMarshal::GetReference<T>(System.Span<T>)"
)]
#[dotnet_intrinsic(
    "static T& System.Runtime.InteropServices.MemoryMarshal::GetReference<T>(System.ReadOnlySpan<T>)"
)]
pub fn intrinsic_memory_marshal_get_reference<'gc, T: SpanIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let element_type = dotnet_vm_ops::vm_try!(generics.method_arg(0));
    let element_desc =
        dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(element_type.clone()));

    let managed = match ctx.pop() {
        StackValue::ValueType(span) => {
            let info = match read_span_reference(&span, ctx, ctx.loader().as_ref()) {
                Ok(info) => info,
                Err(err) => return StepResult::Error(err.into()),
            };
            ManagedPtr::from_info_full(info, element_desc, false)
        }
        StackValue::ManagedPtr(span_ptr) => {
            let layout = dotnet_vm_ops::vm_try!(
                ctx.span_type_layout(ConcreteType::from(span_ptr.inner_type(),))
            );
            let LayoutManager::Field(fields) = &*layout else {
                return StepResult::Error(
                    ExecutionError::NotImplemented("Expected Field layout for Span".into()).into(),
                );
            };
            let managed =
                match read_span_reference_from_ptr(&span_ptr, fields, ctx.loader().as_ref(), ctx) {
                    Ok(managed) => managed,
                    Err(err) => return StepResult::Error(err.into()),
                };
            managed.with_inner_type(element_desc)
        }
        other => {
            return StepResult::type_error("Span<T> or ReadOnlySpan<T>", format!("{other:?}"));
        }
    };

    ctx.push_managed_ptr(managed);
    StepResult::Continue
}

#[dotnet_intrinsic("System.Span<T> System.Span<T>::Slice(int)")]
#[dotnet_intrinsic("System.Span<T> System.Span<T>::Slice(int, int)")]
#[dotnet_intrinsic("System.ReadOnlySpan<T> System.ReadOnlySpan<T>::Slice(int)")]
#[dotnet_intrinsic("System.ReadOnlySpan<T> System.ReadOnlySpan<T>::Slice(int, int)")]
pub fn intrinsic_span_slice<'gc, T: SpanIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let requested_length = match method.signature().parameters.len() {
        1 => None,
        2 => match pop_nonneg_usize(ctx) {
            Ok(length) => Some(length),
            Err(step) => return step,
        },
        _ => {
            return ctx.throw_by_name_with_message(
                "System.ArgumentException",
                "Span.Slice expected one or two arguments.",
            );
        }
    };
    let start = match pop_nonneg_usize(ctx) {
        Ok(start) => start,
        Err(step) => return step,
    };

    let element_type = dotnet_vm_ops::vm_try!(generics.type_arg(0));
    let element_layout = dotnet_vm_ops::vm_try!(ctx.span_type_layout(element_type.clone()));
    let element_size = element_layout.size().as_usize();
    let element_desc =
        dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(element_type.clone()));
    let span_layout =
        dotnet_vm_ops::vm_try!(ctx.span_type_layout(ConcreteType::from(method.parent.clone(),)));

    let (reference, total_length) = match ctx.pop() {
        StackValue::ValueType(span) => {
            let reference = match read_span_reference(&span, ctx, ctx.loader().as_ref()) {
                Ok(info) => ManagedPtr::from_info_full(info, element_desc.clone(), false),
                Err(err) => return StepResult::Error(err.into()),
            };
            let length = match read_span_length(&span, ctx.loader().as_ref()) {
                Ok(length) => length,
                Err(err) => return StepResult::Error(err.into()),
            };
            (reference, length)
        }
        StackValue::ManagedPtr(span_ptr) => {
            let LayoutManager::Field(fields) = &*span_layout else {
                return StepResult::Error(
                    ExecutionError::NotImplemented("Expected Field layout for Span".into()).into(),
                );
            };
            let reference =
                match read_span_reference_from_ptr(&span_ptr, fields, ctx.loader().as_ref(), ctx) {
                    Ok(reference) => reference.with_inner_type(element_desc.clone()),
                    Err(err) => return StepResult::Error(err.into()),
                };
            let length =
                match read_span_length_from_ptr(&span_ptr, fields, ctx.loader().as_ref(), ctx) {
                    Ok(length) => length,
                    Err(err) => return StepResult::Error(err.into()),
                };
            (reference, length)
        }
        other => {
            return StepResult::type_error("Span<T> or ReadOnlySpan<T>", format!("{other:?}"));
        }
    };

    let Ok(total_length) = usize::try_from(total_length) else {
        return StepResult::internal_error("Span had a negative length");
    };
    if start > total_length {
        return ctx.throw_by_name_with_message("System.ArgumentOutOfRangeException", "start");
    }
    let length = requested_length.unwrap_or(total_length - start);
    if length > total_length - start {
        return ctx.throw_by_name_with_message("System.ArgumentOutOfRangeException", "length");
    }

    let Some(byte_offset) = start.checked_mul(element_size) else {
        return ctx.throw_by_name_with_message("System.ArgumentOutOfRangeException", "start");
    };
    let Ok(byte_offset) = isize::try_from(byte_offset) else {
        return ctx.throw_by_name_with_message("System.ArgumentOutOfRangeException", "start");
    };
    let Some(length) = i32::try_from(length).ok() else {
        return ctx.throw_by_name_with_message("System.ArgumentOutOfRangeException", "length");
    };

    // SAFETY: F10.RawMemoryAccessValid — bounds checks above prove the byte adjustment stays within the source span.
    let reference = unsafe { reference.offset(byte_offset) };
    let span = dotnet_vm_ops::vm_try!(
        ctx.span_new_object_with_type_generics(method.parent.clone(), vec![element_type.clone()],)
    );
    ctx.loader()
        .span_or_readonly_span_reference_field(&span.instance_storage, span.description.clone())
        .expect("validated Span<T>/ReadOnlySpan<T> support slot")
        .write(reference);
    ctx.loader()
        .span_or_readonly_span_length_field(&span.instance_storage, span.description.clone())
        .expect("validated Span<T>/ReadOnlySpan<T> support slot")
        .write(length);

    ctx.push_value_type(span);
    StepResult::Continue
}

fn span_reference_and_length<'gc, T: SpanIntrinsicHost<'gc>>(
    ctx: &mut T,
    span: StackValue<'gc>,
    element_desc: &dotnet_types::TypeDescription,
) -> Result<(ManagedPtr<'gc>, i32), StepResult> {
    match span {
        StackValue::ValueType(span) => {
            let reference = read_span_reference(&span, ctx, ctx.loader().as_ref())
                .map(|info| ManagedPtr::from_info_full(info, element_desc.clone(), false))
                .map_err(|err| StepResult::Error(err.into()))?;
            let length = read_span_length(&span, ctx.loader().as_ref())
                .map_err(|err| StepResult::Error(err.into()))?;
            Ok((reference, length))
        }
        StackValue::ManagedPtr(span_ptr) => {
            let layout = ctx
                .span_type_layout(ConcreteType::from(span_ptr.inner_type()))
                .map_err(|err| StepResult::Error(err.into()))?;
            let LayoutManager::Field(fields) = &*layout else {
                return Err(StepResult::Error(
                    ExecutionError::NotImplemented("Expected Field layout for Span".into()).into(),
                ));
            };
            let reference =
                read_span_reference_from_ptr(&span_ptr, fields, ctx.loader().as_ref(), ctx)
                    .map(|reference| reference.with_inner_type(element_desc.clone()))
                    .map_err(|err| StepResult::Error(err.into()))?;
            let length = read_span_length_from_ptr(&span_ptr, fields, ctx.loader().as_ref(), ctx)
                .map_err(|err| StepResult::Error(err.into()))?;
            Ok((reference, length))
        }
        other => Err(StepResult::type_error(
            "Span<T> or ReadOnlySpan<T>",
            format!("{other:?}"),
        )),
    }
}

#[dotnet_intrinsic("static System.ReadOnlySpan<T> System.Span<T>::op_Implicit(System.Span<T>)")]
pub fn intrinsic_span_to_readonly_span<'gc, T: SpanIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let source = ctx.pop();
    let element_type = dotnet_vm_ops::vm_try!(generics.type_arg(0));
    let element_desc =
        dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(element_type.clone()));
    let (reference, length) = match source {
        StackValue::ObjectRef(ObjectRef(None)) => (
            ManagedPtr::new(None, element_desc.clone(), None, false, None),
            0,
        ),
        StackValue::ObjectRef(ObjectRef(Some(handle))) => {
            let (ptr, length) = {
                let object = handle.borrow();
                let HeapStorage::Vec(vector) = &object.storage else {
                    return StepResult::type_error(
                        "array or Span<T>",
                        format!("{:?}", object.storage),
                    );
                };
                // SAFETY: F10.BorrowedStorageStable — the owning handle is retained by the ManagedPtr below, and the vector
                // borrow keeps its backing storage live while deriving the data pointer.
                (
                    unsafe { vector.raw_data_ptr() },
                    vector.layout.length as i32,
                )
            };
            (
                ManagedPtr::new(
                    NonNull::new(ptr),
                    element_desc.clone(),
                    Some(ObjectRef(Some(handle))),
                    false,
                    None,
                ),
                length,
            )
        }
        other => match span_reference_and_length(ctx, other, &element_desc) {
            Ok(data) => data,
            Err(step) => return step,
        },
    };

    let Some(ParameterType::Value(return_type)) = &method.signature().return_type.1 else {
        return StepResult::internal_error("Span conversion must return a value type");
    };
    let return_concrete = dotnet_vm_ops::vm_try!(generics.make_concrete(
        method.resolution(),
        return_type.clone(),
        ctx.loader().as_ref(),
    ));
    let return_span = dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(return_concrete));
    let span = dotnet_vm_ops::vm_try!(
        ctx.span_new_object_with_type_generics(return_span, vec![element_type.clone()],)
    );
    ctx.loader()
        .span_or_readonly_span_reference_field(&span.instance_storage, span.description.clone())
        .expect("validated ReadOnlySpan<T> support slot")
        .write(reference);
    ctx.loader()
        .span_or_readonly_span_length_field(&span.instance_storage, span.description.clone())
        .expect("validated ReadOnlySpan<T> support slot")
        .write(length);

    ctx.push_value_type(span);
    StepResult::Continue
}

#[dotnet_intrinsic("void System.Span<T>::CopyTo(System.Span<T>)")]
#[dotnet_intrinsic("void System.ReadOnlySpan<T>::CopyTo(System.Span<T>)")]
pub fn intrinsic_span_copy_to<'gc, T: SpanIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let destination = ctx.pop();
    let source = ctx.pop();

    let element_type = dotnet_vm_ops::vm_try!(generics.type_arg(0));
    let element_layout = dotnet_vm_ops::vm_try!(ctx.span_type_layout(element_type.clone()));
    let element_desc =
        dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(element_type.clone()));
    let element_size = element_layout.size().as_usize();

    let (source_reference, source_length) =
        match span_reference_and_length(ctx, source, &element_desc) {
            Ok(data) => data,
            Err(step) => return step,
        };
    let (destination_reference, destination_length) =
        match span_reference_and_length(ctx, destination, &element_desc) {
            Ok(data) => data,
            Err(step) => return step,
        };
    let Ok(source_length) = usize::try_from(source_length) else {
        return StepResult::internal_error("Span had a negative length");
    };
    let Ok(destination_length) = usize::try_from(destination_length) else {
        return StepResult::internal_error("Span had a negative length");
    };
    if destination_length < source_length {
        return ctx
            .throw_by_name_with_message("System.ArgumentException", "Destination is too short.");
    }

    // Materialize before writing so overlapping source/destination spans retain CopyTo's
    // memmove semantics and reference-type elements use the normal typed write path.
    let mut values = Vec::with_capacity(source_length);
    for index in 0..source_length {
        let offset = ByteOffset::new(index * element_size);
        // SAFETY: F10.RawMemoryAccessValid — source span metadata supplied the managed origin and base offset; the length
        // check above and element-size stride keep this read within the source span.
        let value = unsafe {
            ctx.read_unaligned(
                source_reference.origin().clone(),
                source_reference.byte_offset() + offset,
                &element_layout,
                Some(element_desc.clone()),
            )
        };
        match value {
            Ok(value) => values.push(value),
            Err(err) => {
                return StepResult::Error(
                    ExecutionError::InternalError(err.to_string().into()).into(),
                );
            }
        }
    }

    for (index, value) in values.into_iter().enumerate() {
        let offset = ByteOffset::new(index * element_size);
        // SAFETY: F10.RawMemoryAccessValid — destination span metadata supplied the managed origin and base offset; its
        // validated length is at least the source length, so this write is in bounds.
        let result = unsafe {
            ctx.write_unaligned(
                destination_reference.origin().clone(),
                destination_reference.byte_offset() + offset,
                value,
                &element_layout,
            )
        };
        if let Err(err) = result {
            return StepResult::Error(ExecutionError::InternalError(err.to_string().into()).into());
        }
    }

    StepResult::Continue
}
