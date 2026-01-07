use crate::{
    pop_args,
    types::{
        generics::{ConcreteType, GenericLookup},
        members::{FieldDescription, MethodDescription},
    },
    value::{
        layout::{type_layout, HasLayout, LayoutManager, Scalar},
        object::ObjectRef,
        pointer::ManagedPtr,
        StackValue,
    },
    vm::{context::ResolutionContext, CallStack, GCHandle, StepResult},
    vm_expect_stack, vm_pop, vm_push,
};
use dotnetdll::prelude::{BaseType, MethodType, ParameterType};
use std::ptr;

pub fn intrinsic_marshal_get_last_pinvoke_error<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = unsafe { crate::vm::pinvoke::LAST_ERROR };
    vm_push!(stack, gc, Int32(value));
    StepResult::InstructionStepped
}

pub fn intrinsic_marshal_set_last_pinvoke_error<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, [Int32(value)]);
    unsafe {
        crate::vm::pinvoke::LAST_ERROR = value;
    }
    StepResult::InstructionStepped
}

pub fn intrinsic_buffer_memmove<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, [NativeInt(len)]);
    let src = vm_pop!(stack).as_ptr();
    let dst = vm_pop!(stack).as_ptr();

    let ctx = ResolutionContext::for_method(method, stack.loader(), generics);
    let target = &generics.method_generics[0];
    let layout = type_layout(target.clone(), &ctx);
    let total_count = len as usize * layout.size();

    // Check GC safe point before large bulk memory operations
    const LARGE_MEMMOVE_THRESHOLD: usize = 4096;
    if total_count > LARGE_MEMMOVE_THRESHOLD {
        stack.check_gc_safe_point();
    }

    unsafe {
        ptr::copy(src, dst, total_count);
    }
    StepResult::InstructionStepped
}

pub fn intrinsic_memory_marshal_get_array_data_reference<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, [ObjectRef(obj)]);
    let Some(array_handle) = obj.0 else {
        return stack.throw_by_name(gc, "System.NullReferenceException");
    };

    let data_ptr = match &array_handle.borrow().storage {
        crate::value::object::HeapStorage::Vec(v) => v.get().as_ptr() as *mut u8,
        _ => panic!("GetArrayDataReference called on non-array"),
    };

    let element_type = stack
        .loader()
        .find_concrete_type(generics.method_generics[0].clone());
    vm_push!(
        stack,
        gc,
        StackValue::managed_ptr(data_ptr, element_type, Some(array_handle), false)
    );
    StepResult::InstructionStepped
}

pub fn intrinsic_marshal_size_of<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(method, stack.loader(), generics);
    let concrete_type = if method.method.signature.parameters.is_empty() {
        generics.method_generics[0].clone()
    } else {
        let type_obj = match vm_pop!(stack) {
            StackValue::ObjectRef(o) => o,
            rest => panic!("Marshal.SizeOf(Type) called on non-object: {:?}", rest),
        };
        stack
            .resolve_runtime_type(type_obj)
            .to_concrete(stack.loader())
    };
    let layout = type_layout(concrete_type, &ctx);
    vm_push!(stack, gc, Int32(layout.size() as i32));
    StepResult::InstructionStepped
}

pub fn intrinsic_marshal_offset_of<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    use crate::value::string::with_string;
    let ctx = ResolutionContext::for_method(method, stack.loader(), generics);
    let field_name_val = vm_pop!(stack);
    let field_name = with_string!(stack, gc, field_name_val, |s| s.as_string());
    let concrete_type = if method.method.signature.parameters.len() == 1 {
        generics.method_generics[0].clone()
    } else {
        let type_obj = match vm_pop!(stack) {
            StackValue::ObjectRef(o) => o,
            rest => panic!(
                "Marshal.OffsetOf(Type, string) called on non-object: {:?}",
                rest
            ),
        };
        stack
            .resolve_runtime_type(type_obj)
            .to_concrete(stack.loader())
    };
    let layout = type_layout(concrete_type.clone(), &ctx);

    if let LayoutManager::FieldLayoutManager(flm) = layout {
        if let Some(field) = flm.fields.get(&field_name) {
            vm_push!(stack, gc, NativeInt(field.position as isize));
        } else {
            panic!("Field {} not found in type {:?}", field_name, concrete_type);
        }
    } else {
        panic!("Type {:?} does not have field layout", concrete_type);
    }
    StepResult::InstructionStepped
}

/// System.Runtime.CompilerServices.Unsafe::Add<T>(ref T source, nint elementOffset)
pub fn intrinsic_unsafe_add<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(method, stack.loader(), generics);
    let target = &generics.method_generics[0];
    let target_type = stack.loader().find_concrete_type(target.clone());
    let layout = type_layout(target.clone(), &ctx);

    pop_args!(stack, [NativeInt(offset)]);
    let m_val = vm_pop!(stack);
    let (owner, pinned) = if let StackValue::ManagedPtr(m) = &m_val {
        (m.owner, m.pinned)
    } else {
        (None, false)
    };
    let m = m_val.as_ptr();
    vm_push!(
        stack,
        gc,
        StackValue::managed_ptr(
            unsafe { m.offset(offset * layout.size() as isize) },
            target_type,
            owner,
            pinned
        )
    );
    StepResult::InstructionStepped
}

/// System.Runtime.CompilerServices.Unsafe::AreSame<T>(ref T left, ref T right)
pub fn intrinsic_unsafe_are_same<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let m1 = vm_pop!(stack).as_ptr();
    let m2 = vm_pop!(stack).as_ptr();
    vm_push!(stack, gc, Int32((m1 == m2) as i32));
    StepResult::InstructionStepped
}

/// System.Runtime.CompilerServices.Unsafe::As<T>(object value)
/// System.Runtime.CompilerServices.Unsafe::AsRef<T>(ref T source)
pub fn intrinsic_unsafe_as<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    _stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // Just leave the value on the stack, it's a no-op at runtime
    StepResult::InstructionStepped
}

/// System.Runtime.CompilerServices.Unsafe::As<TFrom, TTo>(ref TFrom source)
pub fn intrinsic_unsafe_as_generic<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target_type = stack
        .loader()
        .find_concrete_type(generics.method_generics[1].clone());
    let m_val = vm_pop!(stack);
    let (owner, pinned) = match &m_val {
        StackValue::ManagedPtr(m) => (m.owner, m.pinned),
        StackValue::ObjectRef(ObjectRef(Some(h))) => (Some(*h), false),
        _ => (None, false),
    };
    let m = m_val.as_ptr();
    vm_push!(
        stack,
        gc,
        StackValue::managed_ptr(m, target_type, owner, pinned)
    );
    StepResult::InstructionStepped
}

/// System.Runtime.CompilerServices.Unsafe::AsRef<T>(in T source)
/// System.Runtime.CompilerServices.Unsafe::AsRef<T>(void* ptr)
pub fn intrinsic_unsafe_as_ref_any<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let param_type = &method.method.signature.parameters[0].1;
    if let ParameterType::Value(MethodType::Base(b)) = param_type {
        if matches!(**b, BaseType::ValuePointer(_, _)) {
            return intrinsic_unsafe_as_ref_ptr(gc, stack, method, generics);
        }
    }
    // Otherwise it's the "in T source" (ByRef) overload, which is a no-op
    intrinsic_unsafe_as(gc, stack, method, generics)
}

/// System.Runtime.CompilerServices.Unsafe::AsRef<T>(void* ptr)
pub fn intrinsic_unsafe_as_ref_ptr<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target_type = stack
        .loader()
        .find_concrete_type(generics.method_generics[0].clone());
    vm_expect_stack!(let NativeInt(ptr) = vm_pop!(stack));
    vm_push!(
        stack,
        gc,
        managed_ptr(ptr as *mut u8, target_type, None, false)
    );
    StepResult::InstructionStepped
}

/// System.Runtime.CompilerServices.Unsafe::SizeOf<T>()
pub fn intrinsic_unsafe_size_of<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(method, stack.loader(), generics);
    let target = &generics.method_generics[0];
    let layout = type_layout(target.clone(), &ctx);
    vm_push!(stack, gc, Int32(layout.size() as i32));
    StepResult::InstructionStepped
}

/// System.Runtime.CompilerServices.Unsafe::ByteOffset<T>(ref T origin, ref T target)
pub fn intrinsic_unsafe_byte_offset<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let r = vm_pop!(stack).as_ptr();
    let l = vm_pop!(stack).as_ptr();
    let offset = (l as isize) - (r as isize);
    vm_push!(stack, gc, NativeInt(offset));
    StepResult::InstructionStepped
}

/// System.Runtime.CompilerServices.Unsafe::ReadUnaligned<T>(void* ptr)
pub fn intrinsic_unsafe_read_unaligned<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(method, stack.loader(), generics);
    vm_expect_stack!(let NativeInt(ptr) = vm_pop!(stack));
    let target = &generics.method_generics[0];
    let layout = type_layout(target.clone(), &ctx);

    macro_rules! read_ua {
        ($t:ty) => {
            unsafe { ptr::read_unaligned(ptr as *const $t) }
        };
    }

    let v = match layout {
        LayoutManager::Scalar(s) => match s {
            Scalar::ObjectRef => StackValue::ObjectRef(read_ua!(ObjectRef)),
            Scalar::Int8 => StackValue::Int32(read_ua!(i8) as i32),
            Scalar::Int16 => StackValue::Int32(read_ua!(i16) as i32),
            Scalar::Int32 => StackValue::Int32(read_ua!(i32)),
            Scalar::Int64 => StackValue::Int64(read_ua!(i64)),
            Scalar::NativeInt => StackValue::NativeInt(read_ua!(isize)),
            Scalar::Float32 => StackValue::NativeFloat(read_ua!(f32) as f64),
            Scalar::Float64 => StackValue::NativeFloat(read_ua!(f64)),
            Scalar::ManagedPtr => StackValue::ManagedPtr(read_ua!(ManagedPtr<'gc>)),
        },
        _ => panic!("unsupported layout for read unaligned"),
    };
    vm_push!(stack, gc, v);
    StepResult::InstructionStepped
}

/// System.Runtime.CompilerServices.Unsafe::WriteUnaligned<T>(void* ptr, T value)
pub fn intrinsic_unsafe_write_unaligned<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(method, stack.loader(), generics);
    let target = &generics.method_generics[0];
    let layout = type_layout(target.clone(), &ctx);
    let value = vm_pop!(stack);
    vm_expect_stack!(let NativeInt(ptr) = vm_pop!(stack));

    macro_rules! write_ua {
        ($variant:ident, $t:ty) => {{
            vm_expect_stack!(let $variant(v) = value);
            unsafe {
                ptr::write_unaligned(ptr as *mut _, v as $t);
            }
        }};
    }

    match layout {
        LayoutManager::Scalar(s) => match s {
            Scalar::ObjectRef => write_ua!(ObjectRef, ObjectRef),
            Scalar::Int8 => write_ua!(Int32, i8),
            Scalar::Int16 => write_ua!(Int32, i16),
            Scalar::Int32 => write_ua!(Int32, i32),
            Scalar::Int64 => write_ua!(Int64, i64),
            Scalar::NativeInt => write_ua!(NativeInt, isize),
            Scalar::Float32 => write_ua!(NativeFloat, f32),
            Scalar::Float64 => write_ua!(NativeFloat, f64),
            Scalar::ManagedPtr => write_ua!(ManagedPtr, ManagedPtr<'gc>),
        },
        _ => panic!("unsupported layout for write unaligned"),
    }
    StepResult::InstructionStepped
}

pub fn intrinsic_field_intptr_zero<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _field: FieldDescription,
    _type_generics: Vec<ConcreteType>,
) {
    stack.push_stack(_gc, StackValue::NativeInt(0));
}
