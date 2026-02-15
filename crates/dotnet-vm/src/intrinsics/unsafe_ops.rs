use crate::{
    StepResult,
    layout::type_layout,
    memory::{RawMemoryAccess, check_read_safety},
    stack::ops::VesOps,
};
use dotnet_macros::{dotnet_intrinsic, dotnet_intrinsic_field};
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
};
use dotnet_utils::ByteOffset;
use dotnet_value::{
    StackValue,
    layout::{HasLayout, LayoutManager},
    object::{HeapStorage, ObjectRef},
    pointer::{ManagedPtr, PointerOrigin},
};
use dotnetdll::prelude::{BaseType, MethodType, ParameterType};
use std::{
    ptr::{self, NonNull},
    sync::Arc,
};

#[dotnet_intrinsic("static int System.Runtime.InteropServices.Marshal::GetLastPInvokeError()")]
#[allow(unused_variables)]
pub fn intrinsic_marshal_get_last_pinvoke_error<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = unsafe { crate::pinvoke::LAST_ERROR };
    ctx.push_i32(value);
    StepResult::Continue
}

#[dotnet_intrinsic("static void System.Runtime.InteropServices.Marshal::SetLastPInvokeError(int)")]
#[allow(unused_variables)]
pub fn intrinsic_marshal_set_last_pinvoke_error<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = ctx.pop_i32();
    unsafe {
        crate::pinvoke::LAST_ERROR = value;
    }
    StepResult::Continue
}

#[dotnet_intrinsic("static void System.Buffer::Memmove(byte*, byte*, ulong)")]
#[dotnet_intrinsic("static void System.Buffer::Memmove<T>(T&, T&, ulong)")]
#[allow(unused_variables)]
pub fn intrinsic_buffer_memmove<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let len = ctx.pop_isize();
    let src = ctx.pop_ptr();
    let dst = ctx.pop_ptr();

    let res_ctx = ctx.with_generics(generics);
    let total_count = if generics.method_generics.is_empty() {
        len as usize
    } else {
        let target = &generics.method_generics[0];
        let layout = vm_try!(type_layout(target.clone(), &res_ctx));
        (layout.size() * (len as usize)).as_usize()
    };

    // Check GC safe point before large bulk memory operations
    const LARGE_MEMMOVE_THRESHOLD: usize = 4096;
    if total_count > LARGE_MEMMOVE_THRESHOLD {
        ctx.check_gc_safe_point();
    }

    unsafe {
        ptr::copy(src, dst, total_count);
    }
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static T& System.Runtime.InteropServices.MemoryMarshal::GetArrayDataReference<T>(T[])"
)]
#[allow(unused_variables)]
pub fn intrinsic_memory_marshal_get_array_data_reference<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let Some(array_handle) = obj.0 else {
        return ctx.throw_by_name("System.NullReferenceException");
    };

    let data_ptr = match &array_handle.borrow().storage {
        HeapStorage::Vec(v) => v.get().as_ptr() as *mut u8,
        _ => panic!("GetArrayDataReference called on non-array"),
    };

    let element_type = ctx
        .loader()
        .find_concrete_type(generics.method_generics[0].clone())
        .expect("Element type must exist for GetArrayDataReference");
    ctx.push_ptr(
        data_ptr,
        element_type,
        false,
        Some(obj),
        Some(ByteOffset(0)),
    );
    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Runtime.InteropServices.Marshal::SizeOf(object)")]
#[dotnet_intrinsic("static int System.Runtime.InteropServices.Marshal::SizeOf(System.Type)")]
#[dotnet_intrinsic("static int System.Runtime.InteropServices.Marshal::SizeOf<T>(T)")]
#[dotnet_intrinsic("static int System.Runtime.InteropServices.Marshal::SizeOf<T>()")]
pub fn intrinsic_marshal_size_of<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let concrete_type = if method.method.signature.parameters.is_empty() {
        generics.method_generics[0].clone()
    } else {
        let type_obj = ctx.pop_obj();
        ctx.resolve_runtime_type(type_obj).to_concrete(ctx.loader())
    };
    let layout = vm_try!(type_layout(concrete_type, &ctx.current_context()));
    ctx.push_i32(layout.size().as_usize() as i32);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static IntPtr System.Runtime.InteropServices.Marshal::OffsetOf(System.Type, string)"
)]
#[dotnet_intrinsic("static IntPtr System.Runtime.InteropServices.Marshal::OffsetOf<T>(string)")]
pub fn intrinsic_marshal_offset_of<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    use dotnet_value::with_string;
    let field_name_val = ctx.pop();
    let field_name = with_string!(ctx, field_name_val, |s| s.as_string());
    let concrete_type = if method.method.signature.parameters.len() == 1 {
        generics.method_generics[0].clone()
    } else {
        let type_obj = ctx.pop_obj();
        ctx.resolve_runtime_type(type_obj).to_concrete(ctx.loader())
    };
    let layout = vm_try!(type_layout(concrete_type.clone(), &ctx.current_context()));

    if let LayoutManager::Field(flm) = &*layout {
        let td = ctx
            .loader()
            .find_concrete_type(concrete_type.clone())
            .expect("Type must exist for Marshal.OffsetOf");
        if let Some(field) = flm.get_field(td, &field_name) {
            ctx.push_isize(field.position.as_usize() as isize);
        } else {
            panic!("Field {} not found in type {:?}", field_name, concrete_type);
        }
    } else {
        panic!("Type {:?} does not have field layout", concrete_type);
    }
    StepResult::Continue
}

// System.Runtime.CompilerServices.Unsafe::AsPointer<T>(ref T source)
#[dotnet_intrinsic("static void* System.Runtime.CompilerServices.Unsafe::AsPointer<T>(T&)")]
#[allow(unused_variables)]
#[allow(deprecated)]
pub fn intrinsic_unsafe_as_pointer<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop();
    let ptr = match val {
        StackValue::ManagedPtr(m) => unsafe { m.with_data(0, |data| data.as_ptr() as *mut u8) },
        StackValue::NativeInt(i) => i as *mut u8,
        StackValue::UnmanagedPtr(p) => p.0.as_ptr(),
        StackValue::ObjectRef(ObjectRef(None)) => ptr::null_mut(),
        _ => panic!("Unsafe.AsPointer expected pointer, got {:?}", val),
    };
    ctx.push_isize(ptr as isize);
    StepResult::Continue
}

#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::Add<T>(T&, int)")]
#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::Add<T>(T&, IntPtr)")]
#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::Add<T>(T&, UIntPtr)")]
#[dotnet_intrinsic("static void* System.Runtime.CompilerServices.Unsafe::Add<T>(void*, int)")]
pub fn intrinsic_unsafe_add<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target = &generics.method_generics[0];
    let layout = vm_try!(type_layout(target.clone(), &ctx.current_context()));

    let offset_val = ctx.pop();
    let offset = match offset_val {
        StackValue::Int32(i) => i as isize,
        StackValue::NativeInt(i) => i,
        _ => panic!(
            "Unsafe.Add expected Int32 or NativeInt offset, got {:?}",
            offset_val
        ),
    };

    let m_val = ctx.pop();
    let byte_offset = offset * layout.size().as_usize() as isize;
    if let StackValue::ManagedPtr(m) = m_val {
        let new_m = unsafe { m.offset(byte_offset) };
        ctx.push(StackValue::ManagedPtr(new_m));
    } else {
        let ptr = m_val.as_ptr();
        let result_ptr = unsafe { ptr.offset(byte_offset) };
        ctx.push(StackValue::NativeInt(result_ptr as isize));
    }
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static T& System.Runtime.CompilerServices.Unsafe::AddByteOffset<T>(T&, IntPtr)"
)]
#[dotnet_intrinsic(
    "static T& System.Runtime.CompilerServices.Unsafe::AddByteOffset<T>(T&, UIntPtr)"
)]
pub fn intrinsic_unsafe_add_byte_offset<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _target = &generics.method_generics[0];

    let offset_val = ctx.pop();
    let offset = match offset_val {
        StackValue::Int32(i) => i as isize,
        StackValue::NativeInt(i) => i,
        _ => panic!(
            "Unsafe.AddByteOffset expected Int32 or NativeInt offset, got {:?}",
            offset_val
        ),
    };

    let m_val = ctx.pop();
    if let StackValue::ManagedPtr(m) = m_val {
        let new_m = unsafe { m.offset(offset) };
        ctx.push(StackValue::ManagedPtr(new_m));
    } else {
        let m = m_val.as_ptr();
        ctx.push(StackValue::NativeInt(unsafe { m.offset(offset) } as isize));
    }
    StepResult::Continue
}

#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::Subtract<T>(T&, int)")]
#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::Subtract<T>(T&, IntPtr)")]
#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::Subtract<T>(T&, UIntPtr)")]
#[dotnet_intrinsic("static void* System.Runtime.CompilerServices.Unsafe::Subtract<T>(void*, int)")]
pub fn intrinsic_unsafe_subtract<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target = &generics.method_generics[0];
    let layout = vm_try!(type_layout(target.clone(), &ctx.current_context()));

    let offset_val = ctx.pop();
    let offset = match offset_val {
        StackValue::Int32(i) => i as isize,
        StackValue::NativeInt(i) => i,
        _ => panic!(
            "Unsafe.Subtract expected Int32 or NativeInt offset, got {:?}",
            offset_val
        ),
    };

    let m_val = ctx.pop();
    let byte_offset = -(offset * layout.size().as_usize() as isize);
    if let StackValue::ManagedPtr(m) = m_val {
        let new_m = unsafe { m.offset(byte_offset) };
        ctx.push(StackValue::ManagedPtr(new_m));
    } else {
        let ptr = m_val.as_ptr();
        let result_ptr = unsafe { ptr.offset(byte_offset) };
        ctx.push(StackValue::NativeInt(result_ptr as isize));
    }
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static T& System.Runtime.CompilerServices.Unsafe::SubtractByteOffset<T>(T&, IntPtr)"
)]
#[dotnet_intrinsic(
    "static T& System.Runtime.CompilerServices.Unsafe::SubtractByteOffset<T>(T&, UIntPtr)"
)]
pub fn intrinsic_unsafe_subtract_byte_offset<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let offset_val = ctx.pop();
    let offset = match offset_val {
        StackValue::Int32(i) => i as isize,
        StackValue::NativeInt(i) => i,
        _ => panic!(
            "Unsafe.SubtractByteOffset expected Int32 or NativeInt offset, got {:?}",
            offset_val
        ),
    };

    let m_val = ctx.pop();
    if let StackValue::ManagedPtr(m) = m_val {
        let new_m = unsafe { m.offset(-offset) };
        ctx.push(StackValue::ManagedPtr(new_m));
    } else {
        let m = m_val.as_ptr();
        ctx.push(StackValue::NativeInt(unsafe { m.offset(-offset) } as isize));
    }
    StepResult::Continue
}

// System.Runtime.CompilerServices.Unsafe::AreSame<T>(ref T left, ref T right)
#[dotnet_intrinsic("static bool System.Runtime.CompilerServices.Unsafe::AreSame<T>(T&, T&)")]
pub fn intrinsic_unsafe_are_same<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let m1 = ctx.pop_ptr();
    let m2 = ctx.pop_ptr();
    ctx.push_i32((m1 == m2) as i32);
    StepResult::Continue
}

// System.Runtime.CompilerServices.Unsafe::As<T>(object value)
// System.Runtime.CompilerServices.Unsafe::AsRef<T>(ref T source)
#[dotnet_intrinsic("static T System.Runtime.CompilerServices.Unsafe::As<T>(object)")]
#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::AsRef<T>(T&)")]
#[allow(unused_variables)]
pub fn intrinsic_unsafe_as<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // Just leave the value on the stack, it's a no-op at runtime
    StepResult::Continue
}

// System.Runtime.CompilerServices.Unsafe::As<TFrom, TTo>(ref TFrom source)
#[dotnet_intrinsic("static TTo& System.Runtime.CompilerServices.Unsafe::As<TFrom, TTo>(TFrom&)")]
pub fn intrinsic_unsafe_as_generic<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let src_type_gen = generics.method_generics[0].clone();
    let dest_type_gen = generics.method_generics[1].clone();

    let src_layout = vm_try!(type_layout(src_type_gen, &ctx.current_context()));
    let dest_layout = vm_try!(type_layout(dest_type_gen, &ctx.current_context()));

    // Safety Check: Casting ref TFrom to ref TTo
    // TTo must be compatible with TFrom's memory layout regarding References
    check_read_safety(&dest_layout, Some(&src_layout), 0);

    let target_type = ctx
        .loader()
        .find_concrete_type(generics.method_generics[1].clone())
        .expect("Type must exist for Unsafe.As");
    let m_val = ctx.pop();
    match m_val {
        StackValue::ManagedPtr(mut m) => {
            m.inner_type = target_type;
            ctx.push(StackValue::ManagedPtr(m));
        }
        StackValue::ObjectRef(o) => {
            ctx.push(StackValue::ManagedPtr(ManagedPtr::new(
                o.with_data(|d| NonNull::new(d.as_ptr() as *mut u8)),
                target_type,
                o.0.map(|_| o),
                false,
                Some(ByteOffset(0)),
            )));
        }
        rest => {
            let m = rest.as_ptr();
            ctx.push(StackValue::managed_ptr(
                m,
                target_type,
                false,
                Some(ByteOffset(m as usize)),
            ));
        }
    }
    StepResult::Continue
}

// System.Runtime.CompilerServices.Unsafe::AsRef<T>(in T source)
// System.Runtime.CompilerServices.Unsafe::AsRef<T>(void* ptr)
#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::AsRef<T>(T&)")] // in T
#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::AsRef<T>(void*)")]
pub fn intrinsic_unsafe_as_ref_any<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let param_type = &method.method.signature.parameters[0].1;
    if let ParameterType::Value(MethodType::Base(b)) = param_type
        && matches!(**b, BaseType::ValuePointer(_, _))
    {
        return intrinsic_unsafe_as_ref_ptr(ctx, method, generics);
    }
    // Otherwise it's the "in T source" (ByRef) overload, which is a no-op
    intrinsic_unsafe_as(ctx, method, generics)
}

// System.Runtime.CompilerServices.Unsafe::AsRef<T>(void* ptr)
// Note: This is a helper, registration is on intrinsic_unsafe_as_ref_any or separate if needed.
// But as_ref_any handles dispatch.
#[allow(deprecated)]
pub fn intrinsic_unsafe_as_ref_ptr<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target_type_gen = generics.method_generics[0].clone();
    let dest_layout = vm_try!(type_layout(target_type_gen.clone(), &ctx.current_context()));

    let target_type = vm_try!(ctx.loader().find_concrete_type(target_type_gen));

    let val = ctx.pop();
    let (ptr, pinned, origin, offset_base) = match val {
        StackValue::NativeInt(p) => (p as *mut u8, false, PointerOrigin::Unmanaged, None),
        StackValue::ManagedPtr(m) => (
            unsafe { m.with_data(0, |data| data.as_ptr() as *mut u8) },
            m.pinned,
            m.origin.clone(),
            Some(m.offset),
        ),
        StackValue::UnmanagedPtr(p) => (p.0.as_ptr(), false, PointerOrigin::Unmanaged, None),
        _ => panic!("Unsafe.AsRef expected pointer, got {:?}", val),
    };

    // Safety Check: Casting ptr to ref T
    let memory = RawMemoryAccess::new(ctx.heap());
    let owner = match &origin {
        PointerOrigin::Heap(o) => Some(*o),
        _ => ctx.heap().find_object(ptr as usize),
    };

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

    let mut m = ManagedPtr::new(
        NonNull::new(ptr),
        target_type,
        None, // We will set the origin manually to preserve non-heap origins
        pinned,
        offset_base,
    );
    m.origin = origin;
    ctx.push(StackValue::ManagedPtr(m));
    StepResult::Continue
}

#[dotnet_intrinsic("static void System.Runtime.CompilerServices.Unsafe::SkipInit<T>(T&)")]
pub fn intrinsic_unsafe_skip_init<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.pop();
    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Runtime.CompilerServices.Unsafe::SizeOf<T>()")]
#[allow(unused_variables)]
pub fn intrinsic_unsafe_size_of<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let res_ctx = ctx.with_generics(generics);
    let target = &generics.method_generics[0];
    let layout = vm_try!(type_layout(target.clone(), &res_ctx));
    ctx.push_i32(layout.size().as_usize() as i32);
    StepResult::Continue
}

// System.Runtime.CompilerServices.Unsafe::ByteOffset<T>(ref T origin, ref T target)
#[dotnet_intrinsic("static IntPtr System.Runtime.CompilerServices.Unsafe::ByteOffset<T>(T&, T&)")]
pub fn intrinsic_unsafe_byte_offset<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let r = ctx.pop_ptr();
    let l = ctx.pop_ptr();
    let offset = (l as isize) - (r as isize);
    ctx.push_isize(offset);
    StepResult::Continue
}

// System.Runtime.CompilerServices.Unsafe::ReadUnaligned<T>(void* ptr)
// System.Runtime.CompilerServices.Unsafe::ReadUnaligned<T>(ref byte ptr)
#[dotnet_intrinsic("static T System.Runtime.CompilerServices.Unsafe::ReadUnaligned<T>(void*)")]
#[dotnet_intrinsic("static T System.Runtime.CompilerServices.Unsafe::ReadUnaligned<T>(byte&)")]
#[allow(deprecated)]
pub fn intrinsic_unsafe_read_unaligned<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let source = ctx.pop();
    if let StackValue::ManagedPtr(m) = &source {
        if m.is_null() {
            return ctx.throw_by_name("System.NullReferenceException");
        }
    } else if let StackValue::NativeInt(0) = &source {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    let (origin, offset) = crate::instructions::objects::get_ptr_info(ctx, &source);

    let target = &generics.method_generics[0];
    let layout = vm_try!(type_layout(target.clone(), &ctx.current_context()));

    let target_type = vm_try!(ctx.loader().find_concrete_type(target.clone()));

    match unsafe { ctx.read_unaligned(origin, offset, &layout, Some(target_type)) } {
        Ok(v) => {
            // If we read a ManagedPtr, we need to supply the target type,
            // because read_unaligned returns ManagedPtr with void/unknown type.
            if let StackValue::ManagedPtr(mut m) = v {
                let target_type = ctx
                    .loader()
                    .find_concrete_type(target.clone())
                    .expect("Type must exist for Unsafe.ReadUnaligned ManagedPtr");
                m.inner_type = target_type;
                ctx.push(StackValue::ManagedPtr(m));
            } else {
                ctx.push(v);
            }
        }
        Err(e) => panic!("Unsafe.ReadUnaligned failed: {}", e),
    }

    StepResult::Continue
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
    ctx: &mut dyn VesOps<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target = &generics.method_generics[0];
    let layout = vm_try!(type_layout(target.clone(), &ctx.current_context()));
    let value = ctx.pop();

    let dest = ctx.pop();
    if let StackValue::ManagedPtr(m) = &dest {
        if m.is_null() {
            return ctx.throw_by_name("System.NullReferenceException");
        }
    } else if let StackValue::NativeInt(0) = &dest {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    let (origin, offset) = crate::instructions::objects::get_ptr_info(ctx, &dest);

    match unsafe { ctx.write_unaligned(origin, offset, value, &layout) } {
        Ok(_) => {}
        Err(e) => panic!("Unsafe.WriteUnaligned failed: {}", e),
    }

    StepResult::Continue
}

#[dotnet_intrinsic_field("static IntPtr System.IntPtr::Zero")]
pub fn intrinsic_field_intptr_zero<'gc, 'm: 'gc>(
    ctx: &mut dyn VesOps<'gc, 'm>,
    _field: FieldDescription,
    _type_generics: Arc<[ConcreteType]>,
    _is_address: bool,
) -> StepResult {
    ctx.push(StackValue::NativeInt(0));
    StepResult::Continue
}
