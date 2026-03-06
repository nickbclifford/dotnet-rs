use crate::{
    StepResult,
    error::ExecutionError,
    layout::type_layout,
    memory::{MemoryOwner, RawMemoryAccess, check_read_safety},
    stack::ops::{
        EvalStackOps, ExceptionOps, LoaderOps, MemoryOps, RawMemoryOps, ResolutionOps, StackOps,
        TypedStackOps,
    },
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::{ByteOffset, atomic::validate_atomic_access};
use dotnet_value::{
    StackValue,
    layout::HasLayout,
    object::ObjectRef,
    pointer::{ManagedPtr, PointerOrigin},
};
use dotnetdll::prelude::{BaseType, MethodType, ParameterType};
use std::ptr::{self, NonNull};

const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";

use super::marshal::offset_ptr;

// System.Runtime.CompilerServices.Unsafe::AsPointer<T>(ref T source)
#[dotnet_intrinsic("static void* System.Runtime.CompilerServices.Unsafe::AsPointer<T>(T&)")]
#[allow(unused_variables)]
pub fn intrinsic_unsafe_as_pointer<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = ctx.pop();
    let ptr = match val {
        StackValue::ManagedPtr(m) => unsafe { m.with_data(0, |data| data.as_ptr() as *mut u8) },
        StackValue::NativeInt(i) => i as *mut u8,
        StackValue::UnmanagedPtr(p) => p.0.as_ptr(),
        StackValue::ObjectRef(ObjectRef(None)) => ptr::null_mut(),
        _ => {
            return ctx.throw_by_name_with_message(
                "System.ArgumentException",
                "The argument must be a managed pointer or native integer.",
            );
        }
    };
    ctx.push_isize(ptr as isize);
    StepResult::Continue
}

#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::Add<T>(T&, int)")]
#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::Add<T>(T&, IntPtr)")]
#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::Add<T>(T&, UIntPtr)")]
#[dotnet_intrinsic("static void* System.Runtime.CompilerServices.Unsafe::Add<T>(void*, int)")]
pub fn intrinsic_unsafe_add<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + ExceptionOps<'gc> + ResolutionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target = &generics.method_generics[0];
    let layout = vm_try!(type_layout(target.clone(), &ctx.current_context()));

    let offset_val = ctx.pop();
    let offset = match offset_val {
        StackValue::Int32(i) => i as isize,
        StackValue::NativeInt(i) => i,
        _ => {
            return ctx.throw_by_name_with_message(
                "System.ArgumentException",
                "The argument must be an integer.",
            );
        }
    };

    let m_val = ctx.pop();
    let byte_offset = offset * layout.size().as_usize() as isize;
    ctx.push(offset_ptr(m_val, byte_offset));
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static T& System.Runtime.CompilerServices.Unsafe::AddByteOffset<T>(T&, IntPtr)"
)]
#[dotnet_intrinsic(
    "static T& System.Runtime.CompilerServices.Unsafe::AddByteOffset<T>(T&, UIntPtr)"
)]
pub fn intrinsic_unsafe_add_byte_offset<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _target = &generics.method_generics[0];

    let offset_val = ctx.pop();
    let offset = match offset_val {
        StackValue::Int32(i) => i as isize,
        StackValue::NativeInt(i) => i,
        _ => {
            return ctx.throw_by_name_with_message(
                "System.ArgumentException",
                "The argument must be an integer.",
            );
        }
    };

    let m_val = ctx.pop();
    ctx.push(offset_ptr(m_val, offset));
    StepResult::Continue
}

#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::Subtract<T>(T&, int)")]
#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::Subtract<T>(T&, IntPtr)")]
#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::Subtract<T>(T&, UIntPtr)")]
#[dotnet_intrinsic("static void* System.Runtime.CompilerServices.Unsafe::Subtract<T>(void*, int)")]
pub fn intrinsic_unsafe_subtract<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + ExceptionOps<'gc> + ResolutionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target = &generics.method_generics[0];
    let layout = vm_try!(type_layout(target.clone(), &ctx.current_context()));

    let offset_val = ctx.pop();
    let offset = match offset_val {
        StackValue::Int32(i) => i as isize,
        StackValue::NativeInt(i) => i,
        _ => {
            return ctx.throw_by_name_with_message(
                "System.ArgumentException",
                "The argument must be an integer.",
            );
        }
    };

    let m_val = ctx.pop();
    let byte_offset = -(offset * layout.size().as_usize() as isize);
    ctx.push(offset_ptr(m_val, byte_offset));
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static T& System.Runtime.CompilerServices.Unsafe::SubtractByteOffset<T>(T&, IntPtr)"
)]
#[dotnet_intrinsic(
    "static T& System.Runtime.CompilerServices.Unsafe::SubtractByteOffset<T>(T&, UIntPtr)"
)]
pub fn intrinsic_unsafe_subtract_byte_offset<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let offset_val = ctx.pop();
    let offset = match offset_val {
        StackValue::Int32(i) => i as isize,
        StackValue::NativeInt(i) => i,
        _ => {
            return ctx.throw_by_name_with_message(
                "System.ArgumentException",
                "The argument must be an integer.",
            );
        }
    };

    let m_val = ctx.pop();
    ctx.push(offset_ptr(m_val, -offset));
    StepResult::Continue
}

// System.Runtime.CompilerServices.Unsafe::AreSame<T>(ref T left, ref T right)
#[dotnet_intrinsic("static bool System.Runtime.CompilerServices.Unsafe::AreSame<T>(T&, T&)")]
pub fn intrinsic_unsafe_are_same<'gc, T: EvalStackOps<'gc> + TypedStackOps<'gc>>(
    ctx: &mut T,
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
pub fn intrinsic_unsafe_as<T: ?Sized>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // Just leave the value on the stack, it's a no-op at runtime
    StepResult::Continue
}

// System.Runtime.CompilerServices.Unsafe::As<TFrom, TTo>(ref TFrom source)
#[dotnet_intrinsic("static TTo& System.Runtime.CompilerServices.Unsafe::As<TFrom, TTo>(TFrom&)")]
pub fn intrinsic_unsafe_as_generic<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + LoaderOps + ResolutionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let src_type_gen = generics.method_generics[0].clone();
    let dest_type_gen = generics.method_generics[1].clone();

    let src_layout = vm_try!(type_layout(src_type_gen, &ctx.current_context()));
    let dest_layout = vm_try!(type_layout(dest_type_gen, &ctx.current_context()));

    // Safety Check: Casting ref TFrom to ref TTo
    // TTo must be compatible with TFrom's memory layout regarding References
    vm_try!(check_read_safety(&dest_layout, Some(&src_layout), 0));

    let target_type = vm_try!(
        ctx.loader()
            .find_concrete_type(generics.method_generics[1].clone())
    );
    let m_val = ctx.pop();
    match m_val {
        StackValue::ManagedPtr(mut m) => {
            m = m.with_inner_type(target_type);
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
pub fn intrinsic_unsafe_as_ref_any<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + ExceptionOps<'gc>
        + RawMemoryOps<'gc>
        + MemoryOps<'gc>,
>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let param_type = &method.method().signature.parameters[0].1;
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
pub fn intrinsic_unsafe_as_ref_ptr<
    'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + ExceptionOps<'gc>
        + RawMemoryOps<'gc>
        + MemoryOps<'gc>,
>(
    ctx: &mut T,
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
            m.is_pinned(),
            m.origin().clone(),
            Some(m.byte_offset()),
        ),
        StackValue::UnmanagedPtr(p) => (p.0.as_ptr(), false, PointerOrigin::Unmanaged, None),
        _ => {
            return ctx.throw_by_name_with_message(
                "System.ArgumentException",
                "The argument must be a managed pointer or native integer.",
            );
        }
    };

    // Safety Check: Casting ptr to ref T
    let memory = RawMemoryAccess::new(ctx.heap());
    let owner = match &origin {
        PointerOrigin::Heap(o) => Some(*o),
        _ => ctx.heap().find_object(ptr as usize),
    };

    let src_layout_obj = if let Some(owner) = owner {
        memory.get_layout_from_owner(MemoryOwner::Local(owner))
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
        vm_try!(check_read_safety(
            &dest_layout,
            src_layout_obj.as_ref(),
            offset
        ));
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
    m = m.with_origin(origin);
    ctx.push(StackValue::ManagedPtr(m));
    StepResult::Continue
}

#[dotnet_intrinsic("static void System.Runtime.CompilerServices.Unsafe::SkipInit<T>(T&)")]
pub fn intrinsic_unsafe_skip_init<'gc, T: EvalStackOps<'gc> + TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.pop();
    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Runtime.CompilerServices.Unsafe::SizeOf<T>()")]
#[allow(unused_variables)]
pub fn intrinsic_unsafe_size_of<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + ResolutionOps<'gc>,
>(
    ctx: &mut T,
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
pub fn intrinsic_unsafe_byte_offset<'gc, T: EvalStackOps<'gc> + TypedStackOps<'gc>>(
    ctx: &mut T,
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
pub fn intrinsic_unsafe_read_unaligned<
    'gc,
    T: StackOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + ExceptionOps<'gc>
        + RawMemoryOps<'gc>
        + MemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let source = ctx.pop();
    if source.is_null() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    let (origin, offset) = match crate::instructions::objects::get_ptr_info(ctx, &source) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let target = &generics.method_generics[0];
    let layout = vm_try!(type_layout(target.clone(), &ctx.current_context()));

    let target_type = vm_try!(ctx.loader().find_concrete_type(target.clone()));

    match unsafe { ctx.read_unaligned(origin, offset, &layout, Some(target_type)) } {
        Ok(v) => {
            // If we read a ManagedPtr, we need to supply the target type,
            // because read_unaligned returns ManagedPtr with void/unknown type.
            if let StackValue::ManagedPtr(mut m) = v {
                let target_type = vm_try!(ctx.loader().find_concrete_type(target.clone()));
                m = m.with_inner_type(target_type);
                ctx.push(StackValue::ManagedPtr(m));
            } else {
                ctx.push(v);
            }
        }
        Err(e) => {
            return StepResult::Error(
                ExecutionError::NotImplemented(format!("Unsafe.ReadUnaligned failed: {}", e))
                    .into(),
            );
        }
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
pub fn intrinsic_unsafe_write_unaligned<
    'gc,
    T: StackOps<'gc> + ResolutionOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target = &generics.method_generics[0];
    let layout = vm_try!(type_layout(target.clone(), &ctx.current_context()));
    let value = ctx.pop();

    let dest = ctx.pop();
    if dest.is_null() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    let (origin, offset) = match crate::instructions::objects::get_ptr_info(ctx, &dest) {
        Ok(v) => v,
        Err(e) => return e,
    };

    match unsafe { ctx.write_unaligned(origin, offset, value, &layout) } {
        Ok(_) => {}
        Err(e) => {
            return StepResult::Error(
                ExecutionError::NotImplemented(format!("Unsafe.WriteUnaligned failed: {}", e))
                    .into(),
            );
        }
    }

    StepResult::Continue
}

#[dotnet_intrinsic(
    "static void System.Runtime.CompilerServices.Unsafe::CopyBlock(void*, void*, uint)"
)]
#[dotnet_intrinsic(
    "static void System.Runtime.CompilerServices.Unsafe::CopyBlock(byte&, byte&, uint)"
)]
pub fn intrinsic_unsafe_copy_block<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let size = ctx.pop_i32() as u32 as usize;
    let src = ctx.pop_ptr();
    let dest = ctx.pop_ptr();

    if src.is_null() || dest.is_null() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    unsafe {
        validate_atomic_access(src, false);
        validate_atomic_access(dest, false);

        const COPY_BLOCK_CHUNK_SIZE: usize = 1024 * 1024; // 1MB chunks
        let mut offset = 0;
        while offset < size {
            let current_chunk = std::cmp::min(size - offset, COPY_BLOCK_CHUNK_SIZE);
            ptr::copy(src.add(offset), dest.add(offset), current_chunk);
            offset += current_chunk;
            if offset < size && ctx.check_gc_safe_point() {
                return StepResult::Yield;
            }
        }
    }
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static void System.Runtime.CompilerServices.Unsafe::InitBlock(void*, byte, uint)"
)]
#[dotnet_intrinsic(
    "static void System.Runtime.CompilerServices.Unsafe::InitBlock(byte&, byte, uint)"
)]
pub fn intrinsic_unsafe_init_block<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let size = ctx.pop_i32() as u32 as usize;
    let val = ctx.pop_i32() as u8;
    let addr = ctx.pop_ptr();

    if addr.is_null() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    unsafe {
        validate_atomic_access(addr, false);

        const INIT_BLOCK_CHUNK_SIZE: usize = 1024 * 1024; // 1MB chunks
        let mut offset = 0;
        while offset < size {
            let current_chunk = std::cmp::min(size - offset, INIT_BLOCK_CHUNK_SIZE);
            ptr::write_bytes(addr.add(offset), val, current_chunk);
            offset += current_chunk;
            if offset < size && ctx.check_gc_safe_point() {
                return StepResult::Yield;
            }
        }
    }
    StepResult::Continue
}
