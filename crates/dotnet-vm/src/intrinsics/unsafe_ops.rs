use crate::{
    context::ResolutionContext,
    layout::type_layout,
    memory::{check_read_safety, RawMemoryAccess},
    pop_args, vm_pop, vm_push, CallStack, StepResult,
};
use dotnet_macros::{dotnet_intrinsic, dotnet_intrinsic_field};
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    layout::{HasLayout, LayoutManager},
    object::{HeapStorage, ObjectRef},
    pointer::ManagedPtr,
    StackValue,
};
use dotnetdll::prelude::{BaseType, MethodType, ParameterType};
use std::ptr;

use super::ReflectionExtensions;

#[dotnet_intrinsic("static int System.Runtime.InteropServices.Marshal::GetLastPInvokeError()")]
#[allow(unused_variables)]
pub fn intrinsic_marshal_get_last_pinvoke_error<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = unsafe { crate::pinvoke::LAST_ERROR };
    vm_push!(stack, gc, Int32(value));
    StepResult::InstructionStepped
}

#[dotnet_intrinsic("static void System.Runtime.InteropServices.Marshal::SetLastPInvokeError(int)")]
#[allow(unused_variables)]
pub fn intrinsic_marshal_set_last_pinvoke_error<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [Int32(value)]);
    unsafe {
        crate::pinvoke::LAST_ERROR = value;
    }
    StepResult::InstructionStepped
}

#[dotnet_intrinsic("static void System.Buffer::Memmove(byte*, byte*, ulong)")]
#[dotnet_intrinsic("static void System.Buffer::Memmove<T>(T&, T&, ulong)")]
#[allow(unused_variables)]
pub fn intrinsic_buffer_memmove<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [NativeInt(len)]);
    let src = vm_pop!(stack, gc).as_ptr();
    let dst = vm_pop!(stack, gc).as_ptr();

    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let total_count = if generics.method_generics.is_empty() {
        len as usize
    } else {
        let target = &generics.method_generics[0];
        let layout = type_layout(target.clone(), &ctx);
        len as usize * layout.size()
    };

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

#[dotnet_intrinsic(
    "static T& System.Runtime.InteropServices.MemoryMarshal::GetArrayDataReference<T>(T[])"
)]
#[allow(unused_variables)]
pub fn intrinsic_memory_marshal_get_array_data_reference<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ObjectRef(obj)]);
    let Some(array_handle) = obj.0 else {
        return stack.throw_by_name(gc, "System.NullReferenceException");
    };

    let data_ptr = match &array_handle.borrow().storage {
        HeapStorage::Vec(v) => v.get().as_ptr() as *mut u8,
        _ => panic!("GetArrayDataReference called on non-array"),
    };

    let element_type = stack
        .loader()
        .find_concrete_type(generics.method_generics[0].clone());
    vm_push!(
        stack,
        gc,
        StackValue::managed_ptr(data_ptr, element_type, false)
    );
    StepResult::InstructionStepped
}

#[dotnet_intrinsic("static int System.Runtime.InteropServices.Marshal::SizeOf(object)")]
#[dotnet_intrinsic("static int System.Runtime.InteropServices.Marshal::SizeOf(System.Type)")]
#[dotnet_intrinsic("static int System.Runtime.InteropServices.Marshal::SizeOf<T>(T)")]
#[dotnet_intrinsic("static int System.Runtime.InteropServices.Marshal::SizeOf<T>()")]
pub fn intrinsic_marshal_size_of<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let concrete_type = if method.method.signature.parameters.is_empty() {
        generics.method_generics[0].clone()
    } else {
        let type_obj = match vm_pop!(stack, gc) {
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

#[dotnet_intrinsic(
    "static IntPtr System.Runtime.InteropServices.Marshal::OffsetOf(System.Type, string)"
)]
#[dotnet_intrinsic("static IntPtr System.Runtime.InteropServices.Marshal::OffsetOf<T>(string)")]
pub fn intrinsic_marshal_offset_of<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    use dotnet_value::with_string;
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let field_name_val = vm_pop!(stack, gc);
    let field_name = with_string!(stack, gc, field_name_val, |s| s.as_string());
    let concrete_type = if method.method.signature.parameters.len() == 1 {
        generics.method_generics[0].clone()
    } else {
        let type_obj = match vm_pop!(stack, gc) {
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

    if let LayoutManager::FieldLayoutManager(flm) = &*layout {
        let td = stack.loader().find_concrete_type(concrete_type.clone());
        if let Some(field) = flm.get_field(td, &field_name) {
            vm_push!(stack, gc, NativeInt(field.position as isize));
        } else {
            panic!("Field {} not found in type {:?}", field_name, concrete_type);
        }
    } else {
        panic!("Type {:?} does not have field layout", concrete_type);
    }
    StepResult::InstructionStepped
}

// System.Runtime.CompilerServices.Unsafe::AsPointer<T>(ref T source)
#[dotnet_intrinsic("static void* System.Runtime.CompilerServices.Unsafe::AsPointer<T>(T&)")]
#[allow(unused_variables)]
pub fn intrinsic_unsafe_as_pointer<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ManagedPtr(ptr)]);
    vm_push!(stack, gc, ManagedPtr(ptr));
    StepResult::InstructionStepped
}

#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::Add<T>(T&, int)")]
#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::Add<T>(T&, IntPtr)")]
pub fn intrinsic_unsafe_add<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let target = &generics.method_generics[0];
    let target_type = stack.loader().find_concrete_type(target.clone());
    let layout = type_layout(target.clone(), &ctx);

    let offset_val = vm_pop!(stack, gc);
    let offset = match offset_val {
        StackValue::Int32(i) => i as isize,
        StackValue::NativeInt(i) => i,
        _ => panic!(
            "Unsafe.Add expected Int32 or NativeInt offset, got {:?}",
            offset_val
        ),
    };

    let m_val = vm_pop!(stack, gc);
    let (pinned, owner) = if let StackValue::ManagedPtr(m) = &m_val {
        (m.pinned, m.owner)
    } else {
        (false, None)
    };
    let m = m_val.as_ptr();
    let result_ptr = unsafe { m.offset(offset * layout.size() as isize) };
    vm_push!(
        stack,
        gc,
        StackValue::managed_ptr_with_owner(
            result_ptr,
            target_type,
            owner,
            pinned
        )
    );
    StepResult::InstructionStepped
}

#[dotnet_intrinsic(
    "static T& System.Runtime.CompilerServices.Unsafe::AddByteOffset<T>(T&, IntPtr)"
)]
pub fn intrinsic_unsafe_add_byte_offset<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target = &generics.method_generics[0];
    let target_type = stack.loader().find_concrete_type(target.clone());

    let offset_val = vm_pop!(stack, gc);
    let offset = match offset_val {
        StackValue::Int32(i) => i as isize,
        StackValue::NativeInt(i) => i,
        _ => panic!(
            "Unsafe.AddByteOffset expected Int32 or NativeInt offset, got {:?}",
            offset_val
        ),
    };

    let m_val = vm_pop!(stack, gc);
    let (pinned, owner) = if let StackValue::ManagedPtr(m) = &m_val {
        (m.pinned, m.owner)
    } else {
        (false, None)
    };
    let m = m_val.as_ptr();
    vm_push!(
        stack,
        gc,
        StackValue::managed_ptr_with_owner(unsafe { m.offset(offset) }, target_type, owner, pinned)
    );
    StepResult::InstructionStepped
}

// System.Runtime.CompilerServices.Unsafe::AreSame<T>(ref T left, ref T right)
#[dotnet_intrinsic("static bool System.Runtime.CompilerServices.Unsafe::AreSame<T>(T&, T&)")]
pub fn intrinsic_unsafe_are_same<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let m1 = vm_pop!(stack, gc).as_ptr();
    let m2 = vm_pop!(stack, gc).as_ptr();
    vm_push!(stack, gc, Int32((m1 == m2) as i32));
    StepResult::InstructionStepped
}

// System.Runtime.CompilerServices.Unsafe::As<T>(object value)
// System.Runtime.CompilerServices.Unsafe::AsRef<T>(ref T source)
#[dotnet_intrinsic("static T System.Runtime.CompilerServices.Unsafe::As<T>(object)")]
#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::AsRef<T>(T&)")]
#[allow(unused_variables)]
pub fn intrinsic_unsafe_as<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // Just leave the value on the stack, it's a no-op at runtime
    StepResult::InstructionStepped
}

// System.Runtime.CompilerServices.Unsafe::As<TFrom, TTo>(ref TFrom source)
#[dotnet_intrinsic("static TTo& System.Runtime.CompilerServices.Unsafe::As<TFrom, TTo>(TFrom&)")]
pub fn intrinsic_unsafe_as_generic<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );

    let src_type_gen = generics.method_generics[0].clone();
    let dest_type_gen = generics.method_generics[1].clone();

    let src_layout = type_layout(src_type_gen, &ctx);
    let dest_layout = type_layout(dest_type_gen, &ctx);

    // Safety Check: Casting ref TFrom to ref TTo
    // TTo must be compatible with TFrom's memory layout regarding References
    check_read_safety(&dest_layout, Some(&src_layout), 0);

    let target_type = stack
        .loader()
        .find_concrete_type(generics.method_generics[1].clone());
    let m_val = vm_pop!(stack, gc);
    let pinned = match &m_val {
        StackValue::ManagedPtr(m) => m.pinned,
        StackValue::ObjectRef(ObjectRef(Some(_))) => false,
        _ => false,
    };
    let m = m_val.as_ptr();
    vm_push!(stack, gc, StackValue::managed_ptr(m, target_type, pinned));
    StepResult::InstructionStepped
}

// System.Runtime.CompilerServices.Unsafe::AsRef<T>(in T source)
// System.Runtime.CompilerServices.Unsafe::AsRef<T>(void* ptr)
#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::AsRef<T>(T&)")] // in T
#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::AsRef<T>(void*)")]
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

// System.Runtime.CompilerServices.Unsafe::AsRef<T>(void* ptr)
// Note: This is a helper, registration is on intrinsic_unsafe_as_ref_any or separate if needed.
// But as_ref_any handles dispatch.
pub fn intrinsic_unsafe_as_ref_ptr<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );

    let target_type_gen = generics.method_generics[0].clone();
    let dest_layout = type_layout(target_type_gen.clone(), &ctx);

    let target_type = stack.loader().find_concrete_type(target_type_gen);

    let val = vm_pop!(stack, gc);
    let (ptr, pinned) = match val {
        StackValue::NativeInt(p) => (p as *mut u8, false),
        StackValue::ManagedPtr(m) => (
            m.pointer().expect("Unsafe.AsRef null managed ptr").as_ptr(),
            m.pinned,
        ),
        StackValue::UnmanagedPtr(p) => (p.0.as_ptr(), false),
        _ => panic!("Unsafe.AsRef expected pointer, got {:?}", val),
    };

    // Safety Check: Casting ptr to ref T
    let memory = RawMemoryAccess::new(&stack.local.heap);
    let owner = stack.local.heap.find_object(ptr as usize);

    let src_layout_obj = if let Some(owner) = owner {
        memory.get_layout_from_owner(owner)
    } else {
        None
    };

    let base_addr = if let Some(owner) = owner {
        let (base, _) = memory.get_storage_base(owner);
        base as usize
    } else {
        0
    };

    if base_addr != 0 {
        let ptr_addr = ptr as usize;
        let offset = ptr_addr.wrapping_sub(base_addr);
        check_read_safety(&dest_layout, src_layout_obj.as_ref(), offset);
    } else if dest_layout.is_or_contains_refs() {
        // panic!("Heap Corruption: Casting unmanaged pointer to Ref type is unsafe");
    }

    vm_push!(stack, gc, StackValue::managed_ptr(ptr, target_type, pinned));
    StepResult::InstructionStepped
}

// System.Runtime.CompilerServices.Unsafe::SizeOf<T>()
#[dotnet_intrinsic("static int System.Runtime.CompilerServices.Unsafe::SizeOf<T>()")]
#[allow(unused_variables)]
pub fn intrinsic_unsafe_size_of<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let target = &generics.method_generics[0];
    let layout = type_layout(target.clone(), &ctx);
    vm_push!(stack, gc, Int32(layout.size() as i32));
    StepResult::InstructionStepped
}

// System.Runtime.CompilerServices.Unsafe::ByteOffset<T>(ref T origin, ref T target)
#[dotnet_intrinsic("static IntPtr System.Runtime.CompilerServices.Unsafe::ByteOffset<T>(T&, T&)")]
pub fn intrinsic_unsafe_byte_offset<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let r = vm_pop!(stack, gc).as_ptr();
    let l = vm_pop!(stack, gc).as_ptr();
    let offset = (l as isize) - (r as isize);
    vm_push!(stack, gc, NativeInt(offset));
    StepResult::InstructionStepped
}

// System.Runtime.CompilerServices.Unsafe::ReadUnaligned<T>(void* ptr)
// System.Runtime.CompilerServices.Unsafe::ReadUnaligned<T>(ref byte ptr)
#[dotnet_intrinsic("static T System.Runtime.CompilerServices.Unsafe::ReadUnaligned<T>(void*)")]
#[dotnet_intrinsic("static T System.Runtime.CompilerServices.Unsafe::ReadUnaligned<T>(byte&)")]
pub fn intrinsic_unsafe_read_unaligned<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );

    let source = vm_pop!(stack, gc);
    let (ptr, owner) = match source {
        StackValue::NativeInt(p) => {
            if p == 0 {
                return stack.throw_by_name(gc, "System.NullReferenceException");
            }
            (p as *mut u8, None)
        }
        StackValue::ManagedPtr(m) => (
            m.pointer().expect("Unsafe.ReadUnaligned null").as_ptr(),
            m.owner,
        ),
        rest => panic!("invalid source for read_unchecked unaligned: {:?}", rest),
    };

    let target = &generics.method_generics[0];
    let layout = type_layout(target.clone(), &ctx);

    let target_type = stack.loader().find_concrete_type(target.clone());

    let memory = RawMemoryAccess::new(&stack.local.heap);
    match unsafe { memory.read_unaligned(ptr, owner, &layout, Some(target_type)) } {
        Ok(v) => {
            // If we read a ManagedPtr, we need to supply the target type,
            // because memory.read_unaligned returns ManagedPtr with void/unknown type.
            if let StackValue::ManagedPtr(m) = v {
                let target_type = stack.loader().find_concrete_type(target.clone());
                let new_m = ManagedPtr::new(m.pointer(), target_type, m.owner, m.pinned);
                vm_push!(stack, gc, StackValue::ManagedPtr(new_m));
            } else {
                vm_push!(stack, gc, v);
            }
        }
        Err(e) => panic!("Unsafe.ReadUnaligned failed: {}", e),
    }

    StepResult::InstructionStepped
}

// System.Runtime.CompilerServices.Unsafe::WriteUnaligned<T>(void* ptr, T value)
// System.Runtime.CompilerServices.Unsafe::WriteUnaligned<T>(ref byte ptr, T value)
#[dotnet_intrinsic(
    "static void System.Runtime.CompilerServices.Unsafe::WriteUnaligned<T>(void*, T)"
)]
#[dotnet_intrinsic(
    "static void System.Runtime.CompilerServices.Unsafe::WriteUnaligned<T>(byte&, T)"
)]
pub fn intrinsic_unsafe_write_unaligned<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let target = &generics.method_generics[0];
    let layout = type_layout(target.clone(), &ctx);
    let value = vm_pop!(stack, gc);

    let dest = vm_pop!(stack, gc);
    let (ptr, owner) = match dest {
        StackValue::NativeInt(p) => {
            if p == 0 {
                return stack.throw_by_name(gc, "System.NullReferenceException");
            }
            (p as *mut u8, None)
        }
        StackValue::ManagedPtr(m) => (
            m.pointer().expect("Unsafe.WriteUnaligned null").as_ptr(),
            m.owner,
        ),
        rest => panic!("invalid destination for write unaligned: {:?}", rest),
    };

    let mut memory = RawMemoryAccess::new(&stack.local.heap);

    match unsafe { memory.write_unaligned(ptr, owner, value, &layout) } {
        Ok(_) => {}
        Err(e) => panic!("Unsafe.WriteUnaligned failed: {}", e),
    }

    StepResult::InstructionStepped
}

#[dotnet_intrinsic_field("static IntPtr System.IntPtr::Zero")]
pub fn intrinsic_field_intptr_zero<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _field: FieldDescription,
    _type_generics: Vec<ConcreteType>,
    _is_address: bool,
) -> StepResult {
    stack.push(gc, StackValue::NativeInt(0));
    StepResult::InstructionStepped
}
