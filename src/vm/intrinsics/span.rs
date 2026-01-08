use crate::{
    pop_args,
    types::{generics::GenericLookup, members::MethodDescription},
    value::{
        layout::{type_layout, HasLayout},
        object::{Object, ObjectRef},
        pointer::ManagedPtr,
        string::with_string,
    },
    vm::{context::ResolutionContext, CallStack, GCHandle, StepResult},
    vm_pop, vm_push,
};
use std::{mem::size_of, ptr::NonNull, slice};

pub fn span_to_slice<'gc, 'a>(span: Object<'gc>) -> &'a [u8] {
    let ptr_data = span.instance_storage.get_field_local("_reference");
    let mut len_data = [0u8; size_of::<i32>()];

    let ptr = ManagedPtr::read(ptr_data).value;
    len_data.copy_from_slice(span.instance_storage.get_field_local("_length"));

    let len = i32::from_ne_bytes(len_data) as usize;

    unsafe { slice::from_raw_parts(ptr.as_ptr() as *const u8, len) }
}

pub fn intrinsic_memory_extensions_equals_span_char<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(
        stack,
        [Int32(_culture_comparison), ValueType(b), ValueType(a)]
    );

    let a = span_to_slice(*a);
    let b = span_to_slice(*b);

    vm_push!(stack, gc, Int32((a == b) as i32));
    StepResult::InstructionStepped
}

pub fn intrinsic_span_get_item<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, [Int32(index), ManagedPtr(m)]);

    if !m.inner_type.type_name().contains("Span") {
        panic!("invalid type on stack");
    }

    let ctx = ResolutionContext::for_method(method, stack.loader(), generics);
    let span_layout = crate::value::layout::FieldLayoutManager::instance_fields(m.inner_type, &ctx);

    let value_type = &generics.type_generics[0];
    let value_layout = type_layout(value_type.clone(), &ctx);

    let ptr = unsafe {
        m.value
            .add(span_layout.fields["_reference"].position)
            .add(value_layout.size() * index as usize)
    };

    vm_push!(
        stack,
        gc,
        ManagedPtr(ManagedPtr::new(
            ptr,
            stack.loader().find_concrete_type(value_type.clone()),
            m.owner,
            m.pinned
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
    pop_args!(stack, [ManagedPtr(m)]);
    if !m.inner_type.type_name().contains("Span") {
        panic!("invalid type on stack");
    }

    let ctx = ResolutionContext::for_method(method, stack.loader(), generics);
    let layout = crate::value::layout::FieldLayoutManager::instance_fields(m.inner_type, &ctx);
    let value = unsafe {
        let target = m.value.as_ptr().add(layout.fields["_length"].position) as *const i32;
        *target
    };
    vm_push!(stack, gc, Int32(value));
    StepResult::InstructionStepped
}

pub fn intrinsic_string_as_span<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let string_val = vm_pop!(stack);
    let (ptr, len) = with_string!(stack, gc, string_val, |s| (s.as_ptr(), s.len()));

    let ctx = ResolutionContext::for_method(method, stack.loader(), generics);
    let span_type = stack.loader().corlib_type("System.ReadOnlySpan`1");
    let char_type_base = dotnetdll::prelude::BaseType::Char;
    let new_lookup = GenericLookup::new(vec![ctx.make_concrete(&char_type_base)]);
    let ctx = ctx.with_generics(&new_lookup);

    let mut span = Object::new(span_type, &ctx);

    let char_type = stack
        .loader()
        .find_concrete_type(ctx.make_concrete(&char_type_base));
    let managed = ManagedPtr::new(
        NonNull::new(ptr as *mut u8).expect("String pointer should not be null"),
        char_type,
        None,
        false,
    );
    managed.write(span.instance_storage.get_field_mut_local("_reference"));
    span.instance_storage
        .get_field_mut_local("_length")
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
    let ctx = ResolutionContext::for_method(method, stack.loader(), generics);
    let element_size = type_layout(element_type.clone(), &ctx).size();

    pop_args!(stack, [ValueType(field_handle)]);

    // Extract the field index from RuntimeFieldHandle
    let field_index = {
        let mut ptr_buf = [0u8; ObjectRef::SIZE];
        ptr_buf.copy_from_slice(field_handle.instance_storage.get_field_local("_value"));
        let obj_ref = ObjectRef::read(&ptr_buf);
        let handle_obj = obj_ref.0.expect("Null pointer in RuntimeFieldHandle");
        let borrowed = handle_obj.borrow();

        match &borrowed.storage {
            crate::value::object::HeapStorage::Obj(o) => {
                let mut idx_buf = [0u8; size_of::<usize>()];
                idx_buf.copy_from_slice(o.instance_storage.get_field_local("index"));
                usize::from_ne_bytes(idx_buf)
            }
            _ => panic!("RuntimeFieldHandle._value does not point to an object"),
        }
    };

    let (crate::types::members::FieldDescription { field, .. }, lookup) =
        stack.runtime_fields_read()[field_index].clone();
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
        let span_lookup = GenericLookup::new(vec![field_type]);
        let mut span_instance = Object::new(span_type, &ctx.with_generics(&span_lookup));

        let element_desc = stack.loader().find_concrete_type(element_type.clone());
        let managed = ManagedPtr::new(
            NonNull::new(data_slice.as_ptr() as *mut u8)
                .expect("Static data pointer should not be null"),
            element_desc,
            None,
            false,
        );
        managed.write(
            span_instance
                .instance_storage
                .get_field_mut_local("_reference"),
        );

        let element_count = (array_size / element_size) as i32;
        span_instance
            .instance_storage
            .get_field_mut_local("_length")
            .copy_from_slice(&element_count.to_ne_bytes());

        vm_push!(stack, gc, ValueType(Box::new(span_instance)));
        StepResult::InstructionStepped
    } else {
        todo!("initial field data for {:?}", field_desc);
    }
}
