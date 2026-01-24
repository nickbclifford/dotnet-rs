use crate::{
    pop_args,
    utils::gc::GCHandle,
    value::{
        layout::HasLayout,
        object::{HeapStorage, Object, ObjectRef},
        pointer::{ManagedPtr, ManagedPtrOwner},
        StackValue,
    },
    vm::{
        context::ResolutionContext,
        layout::{type_layout, LayoutFactory},
        resolution::ValueResolution,
        CallStack, StepResult,
    },
    vm_push,
};
use dotnet_types::{
    generics::GenericLookup,
    members::{FieldDescription, MethodDescription},
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
    let ptr = ManagedPtr::read_ptr_only(ptr_data);
    len_data.copy_from_slice(
        span.instance_storage
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

    unsafe { slice::from_raw_parts(ptr.as_ptr() as *const u8, len * element_size) }
}

pub fn intrinsic_memory_extensions_equals_span_char<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(
        stack,
        gc,
        [Int32(_culture_comparison), ValueType(b), ValueType(a)]
    );

    let a = span_to_slice(*a, 2);
    let b = span_to_slice(*b, 2);

    vm_push!(stack, gc, Int32((a == b) as i32));
    StepResult::InstructionStepped
}

pub fn intrinsic_span_get_item<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [Int32(index), ManagedPtr(m)]);

    if !m.inner_type.type_name().contains("Span") {
        panic!("invalid type on stack");
    }

    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let span_layout = LayoutFactory::instance_field_layout_cached(
        m.inner_type,
        &ctx,
        Some(&stack.shared.metrics),
    );

    let value_type = &generics.type_generics[0];
    let value_layout = type_layout(value_type.clone(), &ctx);

    let reference_field = span_layout.get_field_by_name("_reference").unwrap();
    let ptr_data = unsafe { m.value.as_ptr().add(reference_field.position) };
    let base_ptr = ManagedPtr::read_ptr_only(unsafe {
        slice::from_raw_parts(ptr_data, ManagedPtr::MEMORY_SIZE)
    });

    // NOTE: We don't recover metadata from side-table because:
    // 1. The Span's _reference field points into an array (or other memory)
    // 2. The owner we have (m.owner) points to the Span object itself, not the array
    // 3. Trying to lookup side-table metadata causes panics when m.owner is a Vec
    // 4. For Span element access, we just need the raw pointer - the array is
    //    independently GC-rooted, so we don't need owner tracking here.
    // 5. The returned ManagedPtr will have no owner, which is fine for Span elements.

    let ptr = unsafe { base_ptr.add(value_layout.size() * index as usize) };

    vm_push!(
        stack,
        gc,
        ManagedPtr(ManagedPtr::new(
            ptr,
            stack.loader().find_concrete_type(value_type.clone()),
            None,  // No owner needed - array is independently rooted
            false  // Not pinned
        ))
    );
    StepResult::InstructionStepped
}

pub fn intrinsic_span_get_length<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ManagedPtr(m)]);
    if !m.inner_type.type_name().contains("Span") {
        panic!("invalid type on stack");
    }

    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let layout = LayoutFactory::instance_field_layout_cached(
        m.inner_type,
        &ctx,
        Some(&stack.shared.metrics),
    );
    let value = unsafe {
        let target = m
            .value
            .as_ptr()
            .add(layout.get_field_by_name("_length").unwrap().position)
            as *const i32;
        *target
    };
    vm_push!(stack, gc, Int32(value));
    StepResult::InstructionStepped
}

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

    let mut span = ctx.new_object(span_type);

    if let Some(h) = h_opt {
        let element_type_desc = stack.loader().find_concrete_type(element_type);
        let managed = ManagedPtr::new(
            NonNull::new(ptr).expect("Object pointer should not be null"),
            element_type_desc,
            Some(ManagedPtrOwner::Heap(h)),
            false,
        );
        // Write only the pointer value (8 bytes) to memory.
        managed.write_ptr_only(
            span.instance_storage
                .get_field_mut_local(span_type, "_reference"),
        );
        // Register metadata in Span's side-table
        let span_layout = LayoutFactory::instance_field_layout_cached(
            span_type,
            &ctx,
            Some(&stack.shared.metrics),
        );
        let reference_field = span_layout.get_field_by_name("_reference").unwrap();
        span.register_managed_ptr(reference_field.position, &managed, gc);
    }

    span.instance_storage
        .get_field_mut_local(span_type, "_length")
        .copy_from_slice(&(len as i32).to_ne_bytes());

    vm_push!(stack, gc, ValueType(Box::new(span)));
    StepResult::InstructionStepped
}

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
            field_handle
                .instance_storage
                .get_field_local(field_handle.description, "_value"),
        );
        let obj_ref = ObjectRef::read(&ptr_buf);
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
        let mut span_instance = ctx.new_object(span_type);

        let element_desc = stack.loader().find_concrete_type(element_type.clone());
        let managed = ManagedPtr::new(
            NonNull::new(data_slice.as_ptr() as *mut u8)
                .expect("Static data pointer should not be null"),
            element_desc,
            None,
            false,
        );
        // Write only the pointer value (8 bytes) to memory.
        managed.write_ptr_only(
            span_instance
                .instance_storage
                .get_field_mut_local(span_type, "_reference"),
        );

        let element_count = (array_size / element_size) as i32;
        span_instance
            .instance_storage
            .get_field_mut_local(span_type, "_length")
            .copy_from_slice(&element_count.to_ne_bytes());

        // Register metadata in Span's side-table
        let span_layout = LayoutFactory::instance_field_layout_cached(
            span_type,
            &ctx,
            Some(&stack.shared.metrics),
        );
        let reference_field = span_layout.get_field_by_name("_reference").unwrap();
        span_instance.register_managed_ptr(reference_field.position, &managed, gc);

        vm_push!(stack, gc, ValueType(Box::new(span_instance)));
        StepResult::InstructionStepped
    } else {
        todo!("initial field data for {:?}", field_desc);
    }
}
