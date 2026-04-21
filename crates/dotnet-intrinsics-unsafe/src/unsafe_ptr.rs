use crate::{NULL_REF_MSG, UnsafeIntrinsicHost, marshal::offset_ptr, ptr_info};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::{ByteOffset, atomic::validate_atomic_access};
use dotnet_value::{
    StackValue,
    layout::{HasLayout, LayoutManager, Scalar},
    object::ObjectRef,
    pointer::{ManagedPtr, PointerOrigin},
};
use dotnet_vm_ops::{
    StepResult,
    ops::{EvalStackOps, ExceptionOps, LoaderOps, RawMemoryOps, TypedStackOps},
};
use dotnetdll::prelude::{BaseType, MethodType, ParameterType};
use std::ptr::{self, NonNull};

const BULK_OP_CHUNK_SIZE: usize = 1024 * 1024; // 1MB chunks

#[inline]
fn ranges_overlap(src: *const u8, dst: *mut u8, len: usize) -> bool {
    let src_start = src as usize;
    let dst_start = dst as usize;
    let src_end = src_start.saturating_add(len);
    let dst_end = dst_start.saturating_add(len);
    src_start < dst_end && dst_start < src_end
}

fn chunked_copy_with_safe_point<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    src: *const u8,
    dest: *mut u8,
    size: usize,
) -> StepResult {
    if size == 0 || src == dest {
        return StepResult::Continue;
    }

    let overlap = ranges_overlap(src, dest, size);
    let copy_backward = overlap && (dest as usize) > (src as usize);

    if overlap {
        if copy_backward {
            let mut remaining = size;
            while remaining > 0 {
                let current_chunk = std::cmp::min(remaining, BULK_OP_CHUNK_SIZE);
                let start = remaining - current_chunk;
                unsafe {
                    // SAFETY: Pointers are valid for `size` bytes, and backward chunking
                    // preserves whole-range memmove semantics for overlapping ranges.
                    ptr::copy(src.add(start), dest.add(start), current_chunk);
                }
                remaining = start;
                if remaining > 0 && ctx.check_gc_safe_point() {
                    return StepResult::Yield;
                }
            }
            return StepResult::Continue;
        }

        let mut offset = 0usize;
        while offset < size {
            let current_chunk = std::cmp::min(size - offset, BULK_OP_CHUNK_SIZE);
            unsafe {
                // SAFETY: Overlapping ranges require memmove semantics.
                ptr::copy(src.add(offset), dest.add(offset), current_chunk);
            }
            offset += current_chunk;
            if offset < size && ctx.check_gc_safe_point() {
                return StepResult::Yield;
            }
        }
        return StepResult::Continue;
    }

    let mut offset = 0usize;
    while offset < size {
        let current_chunk = std::cmp::min(size - offset, BULK_OP_CHUNK_SIZE);
        unsafe {
            // SAFETY: Chunks are non-overlapping in this branch.
            dotnet_simd::copy_nonoverlapping_raw(dest.add(offset), src.add(offset), current_chunk);
        }
        offset += current_chunk;
        if offset < size && ctx.check_gc_safe_point() {
            return StepResult::Yield;
        }
    }

    StepResult::Continue
}

fn chunked_fill_with_safe_point<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    addr: *mut u8,
    size: usize,
    val: u8,
) -> StepResult {
    let mut offset = 0usize;
    while offset < size {
        let current_chunk = std::cmp::min(size - offset, BULK_OP_CHUNK_SIZE);
        unsafe {
            // SAFETY: Destination pointer is valid for `size` bytes by intrinsic contract.
            dotnet_simd::fill_raw(addr.add(offset), current_chunk, val);
        }
        offset += current_chunk;
        if offset < size && ctx.check_gc_safe_point() {
            return StepResult::Yield;
        }
    }

    StepResult::Continue
}

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

#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::NullRef<T>()")]
#[dotnet_intrinsic("static M0& System.Runtime.CompilerServices.Unsafe::NullRef<M0>()")]
pub fn intrinsic_unsafe_null_ref<'gc, T: TypedStackOps<'gc> + LoaderOps>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let Some(target) = generics.method_generics.first().cloned() else {
        return StepResult::not_implemented(
            "Unsafe.NullRef<T> requires one generic argument".to_string(),
        );
    };
    let target_type = dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(target));
    ctx.push_ptr(std::ptr::null_mut(), target_type, false, None, None);
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.CompilerServices.Unsafe::IsNullRef<T>(T&)")]
#[dotnet_intrinsic("static bool System.Runtime.CompilerServices.Unsafe::IsNullRef<M0>(M0&)")]
pub fn intrinsic_unsafe_is_null_ref<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = ctx.pop();
    ctx.push_i32(value.is_null() as i32);
    StepResult::Continue
}

#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::Add<T>(T&, int)")]
#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::Add<T>(T&, IntPtr)")]
#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::Add<T>(T&, UIntPtr)")]
#[dotnet_intrinsic("static void* System.Runtime.CompilerServices.Unsafe::Add<T>(void*, int)")]
pub fn intrinsic_unsafe_add<'gc, T: UnsafeIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target = &generics.method_generics[0];
    let layout = dotnet_vm_ops::vm_try!(ctx.unsafe_type_layout(target.clone()));

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
pub fn intrinsic_unsafe_add_byte_offset<'gc, T: EvalStackOps<'gc> + ExceptionOps<'gc>>(
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
pub fn intrinsic_unsafe_subtract<'gc, T: UnsafeIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target = &generics.method_generics[0];
    let layout = dotnet_vm_ops::vm_try!(ctx.unsafe_type_layout(target.clone()));

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
pub fn intrinsic_unsafe_subtract_byte_offset<'gc, T: EvalStackOps<'gc> + ExceptionOps<'gc>>(
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
pub fn intrinsic_unsafe_as_generic<'gc, T: UnsafeIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let src_type_gen = generics.method_generics[0].clone();
    let dest_type_gen = generics.method_generics[1].clone();

    let src_layout = dotnet_vm_ops::vm_try!(ctx.unsafe_type_layout(src_type_gen));
    let dest_layout = dotnet_vm_ops::vm_try!(ctx.unsafe_type_layout(dest_type_gen));

    // Safety Check: Casting ref TFrom to ref TTo
    // TTo must be compatible with TFrom's memory layout regarding References
    dotnet_vm_ops::vm_try!(ctx.unsafe_check_read_safety(&dest_layout, Some(&src_layout), 0));

    let target_type = dotnet_vm_ops::vm_try!(
        ctx.loader()
            .find_concrete_type(generics.method_generics[1].clone())
    );
    let m_val = ctx.pop();
    match m_val {
        StackValue::ManagedPtr(m) => {
            let m = m.with_inner_type(target_type);
            ctx.push(StackValue::ManagedPtr(m.into()));
        }
        StackValue::ObjectRef(o) => {
            ctx.push(StackValue::ManagedPtr(
                ManagedPtr::new(
                    o.with_data(|d| NonNull::new(d.as_ptr() as *mut u8)),
                    target_type,
                    o.0.map(|_| o),
                    false,
                    Some(ByteOffset(0)),
                )
                .into(),
            ));
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

#[dotnet_intrinsic("static TTo System.Runtime.CompilerServices.Unsafe::BitCast<TFrom, TTo>(TFrom)")]
pub fn intrinsic_unsafe_bitcast<'gc, T: UnsafeIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    if generics.method_generics.len() < 2 {
        return StepResult::not_implemented(
            "Unsafe.BitCast<TFrom, TTo> requires two generic arguments".to_string(),
        );
    }

    let src_layout =
        dotnet_vm_ops::vm_try!(ctx.unsafe_type_layout(generics.method_generics[0].clone()));
    let dst_layout =
        dotnet_vm_ops::vm_try!(ctx.unsafe_type_layout(generics.method_generics[1].clone()));
    let src_size = src_layout.size().as_usize();
    let dst_size = dst_layout.size().as_usize();
    if src_size != dst_size || src_size > 16 {
        return StepResult::not_implemented(format!(
            "Unsafe.BitCast<TFrom, TTo> unsupported size cast: {} -> {}",
            src_size, dst_size
        ));
    }

    let value = ctx.pop();
    let mut bytes = [0u8; 16];
    let src_slice = &mut bytes[..src_size];

    let as_i32 = |v: &StackValue<'gc>| match v {
        StackValue::Int32(i) => Some(*i),
        StackValue::Int64(i) => Some(*i as i32),
        StackValue::NativeInt(i) => Some(*i as i32),
        _ => None,
    };
    let as_i64 = |v: &StackValue<'gc>| match v {
        StackValue::Int32(i) => Some(*i as i64),
        StackValue::Int64(i) => Some(*i),
        StackValue::NativeInt(i) => Some(*i as i64),
        _ => None,
    };
    let as_isize = |v: &StackValue<'gc>| match v {
        StackValue::Int32(i) => Some(*i as isize),
        StackValue::Int64(i) => Some(*i as isize),
        StackValue::NativeInt(i) => Some(*i),
        _ => None,
    };

    match src_layout.as_ref() {
        LayoutManager::Scalar(Scalar::Int8 | Scalar::UInt8) => {
            let Some(v) = as_i32(&value) else {
                return StepResult::not_implemented(format!(
                    "Unsafe.BitCast source mismatch: expected 1-byte integer, got {:?}",
                    value
                ));
            };
            src_slice[0] = v as u8;
        }
        LayoutManager::Scalar(Scalar::Int16 | Scalar::UInt16) => {
            let Some(v) = as_i32(&value) else {
                return StepResult::not_implemented(format!(
                    "Unsafe.BitCast source mismatch: expected 2-byte integer, got {:?}",
                    value
                ));
            };
            src_slice.copy_from_slice(&(v as i16).to_ne_bytes());
        }
        LayoutManager::Scalar(Scalar::Int32) => {
            let Some(v) = as_i32(&value) else {
                return StepResult::not_implemented(format!(
                    "Unsafe.BitCast source mismatch: expected Int32, got {:?}",
                    value
                ));
            };
            src_slice.copy_from_slice(&v.to_ne_bytes());
        }
        LayoutManager::Scalar(Scalar::Int64) => {
            let Some(v) = as_i64(&value) else {
                return StepResult::not_implemented(format!(
                    "Unsafe.BitCast source mismatch: expected Int64, got {:?}",
                    value
                ));
            };
            src_slice.copy_from_slice(&v.to_ne_bytes());
        }
        LayoutManager::Scalar(Scalar::NativeInt) => {
            let Some(v) = as_isize(&value) else {
                return StepResult::not_implemented(format!(
                    "Unsafe.BitCast source mismatch: expected NativeInt, got {:?}",
                    value
                ));
            };
            if std::mem::size_of::<isize>() == 8 {
                src_slice.copy_from_slice(&(v as i64).to_ne_bytes());
            } else {
                src_slice.copy_from_slice(&(v as i32).to_ne_bytes());
            }
        }
        LayoutManager::Scalar(Scalar::Float32) => {
            let StackValue::NativeFloat(v) = value else {
                return StepResult::not_implemented(format!(
                    "Unsafe.BitCast source mismatch: expected Float32, got {:?}",
                    value
                ));
            };
            src_slice.copy_from_slice(&(v as f32).to_bits().to_ne_bytes());
        }
        LayoutManager::Scalar(Scalar::Float64) => {
            let StackValue::NativeFloat(v) = value else {
                return StepResult::not_implemented(format!(
                    "Unsafe.BitCast source mismatch: expected Float64, got {:?}",
                    value
                ));
            };
            src_slice.copy_from_slice(&v.to_bits().to_ne_bytes());
        }
        _ => {
            return StepResult::not_implemented(format!(
                "Unsafe.BitCast unsupported source layout: {:?}",
                src_layout
            ));
        }
    }

    let dst_slice = &bytes[..dst_size];
    match dst_layout.as_ref() {
        LayoutManager::Scalar(Scalar::Int8) => {
            ctx.push_i32((dst_slice[0] as i8) as i32);
        }
        LayoutManager::Scalar(Scalar::UInt8) => {
            ctx.push_i32(dst_slice[0] as i32);
        }
        LayoutManager::Scalar(Scalar::Int16) => {
            let mut arr = [0u8; 2];
            arr.copy_from_slice(dst_slice);
            ctx.push_i32(i16::from_ne_bytes(arr) as i32);
        }
        LayoutManager::Scalar(Scalar::UInt16) => {
            let mut arr = [0u8; 2];
            arr.copy_from_slice(dst_slice);
            ctx.push_i32(u16::from_ne_bytes(arr) as i32);
        }
        LayoutManager::Scalar(Scalar::Int32) => {
            let mut arr = [0u8; 4];
            arr.copy_from_slice(dst_slice);
            ctx.push_i32(i32::from_ne_bytes(arr));
        }
        LayoutManager::Scalar(Scalar::Int64) => {
            let mut arr = [0u8; 8];
            arr.copy_from_slice(dst_slice);
            ctx.push_i64(i64::from_ne_bytes(arr));
        }
        LayoutManager::Scalar(Scalar::NativeInt) => {
            if std::mem::size_of::<isize>() == 8 {
                let mut arr = [0u8; 8];
                arr.copy_from_slice(dst_slice);
                ctx.push_isize(i64::from_ne_bytes(arr) as isize);
            } else {
                let mut arr = [0u8; 4];
                arr.copy_from_slice(dst_slice);
                ctx.push_isize(i32::from_ne_bytes(arr) as isize);
            }
        }
        LayoutManager::Scalar(Scalar::Float32) => {
            let mut arr = [0u8; 4];
            arr.copy_from_slice(dst_slice);
            let f = f32::from_bits(u32::from_ne_bytes(arr));
            ctx.push_f64(f as f64);
        }
        LayoutManager::Scalar(Scalar::Float64) => {
            let mut arr = [0u8; 8];
            arr.copy_from_slice(dst_slice);
            let f = f64::from_bits(u64::from_ne_bytes(arr));
            ctx.push_f64(f);
        }
        _ => {
            return StepResult::not_implemented(format!(
                "Unsafe.BitCast unsupported target layout: {:?}",
                dst_layout
            ));
        }
    }

    StepResult::Continue
}

// System.Runtime.CompilerServices.Unsafe::AsRef<T>(in T source)
// System.Runtime.CompilerServices.Unsafe::AsRef<T>(void* ptr)
#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::AsRef<T>(T&)")] // in T
#[dotnet_intrinsic("static T& System.Runtime.CompilerServices.Unsafe::AsRef<T>(void*)")]
pub fn intrinsic_unsafe_as_ref_any<'gc, T: UnsafeIntrinsicHost<'gc>>(
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
pub fn intrinsic_unsafe_as_ref_ptr<'gc, T: UnsafeIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target_type_gen = generics.method_generics[0].clone();
    let dest_layout = dotnet_vm_ops::vm_try!(ctx.unsafe_type_layout(target_type_gen.clone()));

    let target_type = dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(target_type_gen));

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
    let (src_layout_obj, base_addr) = ctx.unsafe_lookup_owner_layout_and_base(ptr, &origin);

    if let Some(base_addr) = base_addr {
        let ptr_addr = ptr as usize;
        let offset = ptr_addr.wrapping_sub(base_addr);
        dotnet_vm_ops::vm_try!(ctx.unsafe_check_read_safety(
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
    ctx.push(StackValue::ManagedPtr(m.into()));
    StepResult::Continue
}

#[dotnet_intrinsic("static void System.Runtime.CompilerServices.Unsafe::SkipInit<T>(T&)")]
pub fn intrinsic_unsafe_skip_init<'gc, T: EvalStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.pop();
    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Runtime.CompilerServices.Unsafe::SizeOf<T>()")]
#[allow(unused_variables)]
pub fn intrinsic_unsafe_size_of<'gc, T: UnsafeIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target = &generics.method_generics[0];
    let layout = dotnet_vm_ops::vm_try!(ctx.unsafe_type_layout(target.clone()));
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
pub fn intrinsic_unsafe_read_unaligned<'gc, T: UnsafeIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let source = ctx.pop();
    if source.is_null() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    let (origin, offset) = match ptr_info(ctx, &source) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let target = &generics.method_generics[0];
    let layout = dotnet_vm_ops::vm_try!(ctx.unsafe_type_layout(target.clone()));

    let target_type = dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(target.clone()));

    match unsafe { ctx.read_unaligned(origin, offset, &layout, Some(target_type)) } {
        Ok(v) => {
            // If we read a ManagedPtr, we need to supply the target type,
            // because read_unaligned returns ManagedPtr with void/unknown type.
            if let StackValue::ManagedPtr(m) = v {
                let target_type =
                    dotnet_vm_ops::vm_try!(ctx.loader().find_concrete_type(target.clone()));
                let m = m.with_inner_type(target_type);
                ctx.push(StackValue::ManagedPtr(m.into()));
            } else {
                ctx.push(v);
            }
        }
        Err(e) => {
            return StepResult::not_implemented(format!("Unsafe.ReadUnaligned failed: {}", e));
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
pub fn intrinsic_unsafe_write_unaligned<'gc, T: UnsafeIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target = &generics.method_generics[0];
    let layout = dotnet_vm_ops::vm_try!(ctx.unsafe_type_layout(target.clone()));
    let value = ctx.pop();

    let dest = ctx.pop();
    if dest.is_null() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    let (origin, offset) = match ptr_info(ctx, &dest) {
        Ok(v) => v,
        Err(e) => return e,
    };

    match unsafe { ctx.write_unaligned(origin, offset, value, &layout) } {
        Ok(_) => {}
        Err(e) => {
            return StepResult::not_implemented(format!("Unsafe.WriteUnaligned failed: {}", e));
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

    validate_atomic_access(src, false);
    validate_atomic_access(dest, false);

    chunked_copy_with_safe_point(ctx, src, dest, size)
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

    validate_atomic_access(addr, false);

    chunked_fill_with_safe_point(ctx, addr, size, val)
}
