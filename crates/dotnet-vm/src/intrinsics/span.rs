use crate::{
    context::ResolutionContext, layout::type_layout, pop_args, resolution::ValueResolution,
    vm_push, CallStack, StepResult,
};
use dotnet_types::{
    generics::GenericLookup,
    members::{FieldDescription, MethodDescription},
};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    layout::HasLayout,
    object::{HeapStorage, Object, ObjectRef},
    pointer::ManagedPtr,
    StackValue,
};
use dotnetdll::prelude::ParameterType;
use std::{mem::size_of, ptr::NonNull, slice};

use super::ReflectionExtensions;

pub fn span_to_slice<'gc, 'a>(span: Object<'gc>, element_size: usize) -> &'a [u8] {
    let ptr_data = span
        .instance_storage
        .get_field_local(span.description, "_reference");
    let mut len_data = [0u8; size_of::<i32>()];

    // Read only the pointer value (8 bytes) from memory.
    // Managed pointers in Span fields are stored pointer-sized.
    let ptr = ManagedPtr::read_raw_ptr_unsafe(&ptr_data);
    len_data.copy_from_slice(
        &span
            .instance_storage
            .get_field_local(span.description, "_length"),
    );

    let len = i32::from_ne_bytes(len_data) as usize;

    // Defensive check: limit span size to 1GB
    if len > 0x4000_0000 || (len > 0 && element_size > usize::MAX / len) {
        panic!(
            "massive span detected: length={}, element_size={}",
            len, element_size
        );
    }

    if len == 0 {
        return &[];
    }

    let raw_ptr = ptr.map(|p| p.as_ptr()).unwrap_or(std::ptr::null_mut());
    if raw_ptr.is_null() {
        panic!("Null pointer in non-empty span");
    }

    unsafe { slice::from_raw_parts(raw_ptr as *const u8, len * element_size) }
}

use dotnet_macros::dotnet_intrinsic;

#[dotnet_intrinsic("static bool System.MemoryExtensions::Equals(System.ReadOnlySpan<char>, System.ReadOnlySpan<char>, System.StringComparison)")]
pub fn intrinsic_memory_extensions_equals_span_char<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(
        stack,
        gc,
        [ValueType(a), ValueType(b), Int32(_culture_comparison)]
    );

    let a = span_to_slice(*a, 2);
    let b = span_to_slice(*b, 2);

    vm_push!(stack, gc, Int32((a == b) as i32));
    StepResult::InstructionStepped
}

#[dotnet_intrinsic("static System.ReadOnlySpan<char> System.String::op_Implicit(string)")]
#[dotnet_intrinsic("static System.ReadOnlySpan<char> System.MemoryExtensions::AsSpan(string)")]
#[dotnet_intrinsic("static System.ReadOnlySpan<char> System.MemoryExtensions::AsSpan(string, int)")]
#[dotnet_intrinsic(
    "static System.ReadOnlySpan<char> System.MemoryExtensions::AsSpan(string, int, int)"
)]
pub fn intrinsic_as_span<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let param_count = method.method.signature.parameters.len();

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
            let start = match stack.pop_stack(gc) {
                StackValue::Int32(i) => i as usize,
                v => panic!("AsSpan: expected Int32 for start parameter, got {:?}", v),
            };
            (start, None)
        }
        3 => {
            let length = match stack.pop_stack(gc) {
                StackValue::Int32(i) => i as usize,
                v => panic!("AsSpan: expected Int32 for length parameter, got {:?}", v),
            };
            let start = match stack.pop_stack(gc) {
                StackValue::Int32(i) => i as usize,
                v => panic!("AsSpan: expected Int32 for start parameter, got {:?}", v),
            };
            (start, Some(length))
        }
        _ => panic!("AsSpan: unexpected parameter count {}", param_count),
    };

    let obj_val = stack.pop_stack(gc);

    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );

    let (base_ptr, total_len, h_opt, element_type, element_size) = match obj_val {
        StackValue::ObjectRef(ObjectRef(Some(h))) => {
            let heap = h.borrow();
            match &heap.storage {
                HeapStorage::Str(s) => {
                    (
                        s.as_ptr() as *mut u8,
                        s.len(),
                        Some(h),
                        ctx.make_concrete(&dotnetdll::prelude::BaseType::Char),
                        2, // char is 2 bytes in .NET
                    )
                }
                HeapStorage::Vec(a) => {
                    let elem_type = a.element.clone();
                    let elem_size = a.layout.element_layout.size();
                    (
                        a.get().as_ptr() as *mut u8,
                        a.layout.length,
                        Some(h),
                        elem_type,
                        elem_size,
                    )
                }
                _ => panic!(
                    "AsSpan called on non-string/non-array object: {:?}",
                    heap.storage
                ),
            }
        }
        StackValue::ObjectRef(ObjectRef(None)) => {
            let element_type = if !generics.method_generics.is_empty() {
                generics.method_generics[0].clone()
            } else {
                ctx.make_concrete(&dotnetdll::prelude::BaseType::Char)
            };
            (std::ptr::null_mut(), 0, None, element_type, 2)
        }
        _ => panic!("AsSpan called on non-object: {:?}", obj_val),
    };

    // Apply start and length_override
    if start > total_len {
        panic!(
            "AsSpan: start {} is beyond the end of the collection (length {})",
            start, total_len
        );
    }
    let actual_length = if let Some(len) = length_override {
        if start + len > total_len {
            panic!(
                "AsSpan: start {} + length {} is beyond the end of the collection (length {})",
                start, len, total_len
            );
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
    let len = actual_length;

    let span_type_concrete = match &method.method.signature.return_type.1 {
        Some(ParameterType::Value(t)) => ctx.make_concrete(t),
        Some(_) => panic!("AsSpan called on method with ref/typedref return"),
        None => panic!("AsSpan called on method returning void"),
    };
    let span_type = stack.loader().find_concrete_type(span_type_concrete);

    let new_lookup = GenericLookup {
        type_generics: vec![element_type.clone()],
        method_generics: vec![],
    };
    let ctx = ctx.with_generics(&new_lookup);

    let span = ctx.new_object(span_type);

    if let Some(h) = h_opt {
        let element_type_desc = stack.loader().find_concrete_type(element_type);
        let managed = ManagedPtr::new(
            Some(NonNull::new(ptr).expect("Object pointer should not be null")),
            element_type_desc,
            Some(ObjectRef(Some(h))),
            false,
        );
        managed.write(
            &mut span
                .instance_storage
                .get_field_mut_local(span_type, "_reference"),
        );
    }

    span.instance_storage
        .get_field_mut_local(span_type, "_length")
        .copy_from_slice(&(len as i32).to_ne_bytes());

    vm_push!(stack, gc, ValueType(Box::new(span)));
    StepResult::InstructionStepped
}

#[dotnet_intrinsic("static System.Span<T> System.Runtime.CompilerServices.RuntimeHelpers::CreateSpan<T>(System.RuntimeFieldHandle)")]
pub fn intrinsic_runtime_helpers_create_span<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let element_type = &generics.method_generics[0];
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let element_size = type_layout(element_type.clone(), &ctx).size();

    pop_args!(stack, gc, [ValueType(field_handle)]);

    let (FieldDescription { field, .. }, lookup) = {
        let mut ptr_buf = [0u8; ObjectRef::SIZE];
        ptr_buf.copy_from_slice(
            &field_handle
                .instance_storage
                .get_field_local(field_handle.description, "_value"),
        );
        let obj_ref = unsafe { ObjectRef::read_branded(&ptr_buf, gc) };
        stack.resolve_runtime_field(obj_ref)
    };
    let field_type = ctx.with_generics(&lookup).make_concrete(&field.return_type);
    let field_desc = stack.loader().find_concrete_type(field_type.clone());

    let Some(initial_data) = &field.initial_value else {
        return stack.throw_by_name(gc, "System.ArgumentException");
    };

    if field_desc
        .definition()
        .name
        .starts_with("__StaticArrayInitTypeSize=")
    {
        let prefix = "__StaticArrayInitTypeSize=";
        let size_str = &field_desc.definition().name[prefix.len()..];
        let size_end = size_str.find('_').unwrap_or(size_str.len());
        let array_size = size_str[..size_end].parse::<usize>().unwrap();
        let data_slice = &initial_data[..array_size];

        let span_type = stack.loader().corlib_type("System.ReadOnlySpan`1");
        let span_lookup = GenericLookup::new(vec![element_type.clone()]);
        let ctx = ctx.with_generics(&span_lookup);
        let span_instance = ctx.new_object(span_type);

        let element_desc = stack.loader().find_concrete_type(element_type.clone());
        let managed = ManagedPtr::new(
            Some(
                NonNull::new(data_slice.as_ptr() as *mut u8)
                    .expect("Static data pointer should not be null"),
            ),
            element_desc,
            None,
            false,
        );
        managed.write(
            &mut span_instance
                .instance_storage
                .get_field_mut_local(span_type, "_reference"),
        );

        let element_count = (array_size / element_size) as i32;
        span_instance
            .instance_storage
            .get_field_mut_local(span_type, "_length")
            .copy_from_slice(&element_count.to_ne_bytes());

        vm_push!(stack, gc, ValueType(Box::new(span_instance)));
        StepResult::InstructionStepped
    } else {
        todo!("initial field data for {:?}", field_desc);
    }
}

#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.RuntimeHelpers::GetSpanDataFrom<T>(T&, System.Type, int&)")]
pub fn intrinsic_runtime_helpers_get_span_data_from<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    pop_args!(
        stack,
        gc,
        [
            ValueType(field_handle),
            ValueType(type_handle),
            ManagedPtr(length_ref)
        ]
    );

    // Resolve field
    let (FieldDescription { field, .. }, _) = {
        let mut ptr_buf = [0u8; ObjectRef::SIZE];
        ptr_buf.copy_from_slice(
            &field_handle
                .instance_storage
                .get_field_local(field_handle.description, "_value"),
        );
        let obj_ref = unsafe { ObjectRef::read_branded(&ptr_buf, gc) };
        stack.resolve_runtime_field(obj_ref)
    };

    // Resolve type
    let element_type_runtime = {
        let mut ptr_buf = [0u8; ObjectRef::SIZE];
        ptr_buf.copy_from_slice(
            &type_handle
                .instance_storage
                .get_field_local(type_handle.description, "_value"),
        );
        let obj_ref = unsafe { ObjectRef::read_branded(&ptr_buf, gc) };
        stack.resolve_runtime_type(obj_ref)
    };

    let element_type: dotnet_types::generics::ConcreteType =
        element_type_runtime.to_concrete(stack.loader());

    let ctx = ResolutionContext::for_method(
        _method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let element_size = type_layout(element_type, &ctx).size();

    let Some(initial_data) = &field.initial_value else {
        vm_push!(stack, gc, NativeInt(0));
        return StepResult::InstructionStepped;
    };

    if field.name.starts_with("__StaticArrayInitTypeSize=") {
        let prefix = "__StaticArrayInitTypeSize=";
        let size_str = &field.name[prefix.len()..];
        let size_end = size_str.find('_').unwrap_or(size_str.len());
        let array_size = size_str[..size_end].parse::<usize>().unwrap();

        let element_count = (array_size / element_size) as i32;
        unsafe {
            std::ptr::copy_nonoverlapping(
                element_count.to_ne_bytes().as_ptr(),
                length_ref
                    .value
                    .expect("System.NullReferenceException")
                    .as_ptr(),
                size_of::<i32>(),
            );
        }

        let ptr = initial_data.as_ptr() as usize;
        vm_push!(stack, gc, NativeInt(ptr as isize));
    } else {
        vm_push!(stack, gc, NativeInt(0));
    }
    StepResult::InstructionStepped
}

#[dotnet_intrinsic("static byte& DotnetRs.Internal::GetArrayData(System.Array)")]
pub fn intrinsic_internal_get_array_data<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ObjectRef(array_ref)]);

    let element_type = if !generics.method_generics.is_empty() {
        generics.method_generics[0].clone()
    } else {
        panic!("GetArrayData expected generic argument");
    };

    let element_type_desc = stack.loader().find_concrete_type(element_type);

    if let Some(handle) = array_ref.0 {
        let inner = handle.borrow();
        if let HeapStorage::Vec(v) = &inner.storage {
            let ptr = v.get().as_ptr();
            let managed = ManagedPtr::new(
                NonNull::new(ptr as *mut u8),
                element_type_desc,
                Some(array_ref),
                false,
            );
            vm_push!(stack, gc, ManagedPtr(managed));
        } else {
            panic!("GetArrayData called on non-vector object");
        }
    } else {
        let managed = ManagedPtr::new(None, element_type_desc, None, false);
        vm_push!(stack, gc, ManagedPtr(managed));
    }
    StepResult::InstructionStepped
}
