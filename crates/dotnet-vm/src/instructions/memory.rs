use crate::{
    StepResult,
    ops::{ExceptionOps, PoolOps, RawMemoryOps, StackOps, VesOps},
};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::{ByteOffset, atomic::validate_atomic_access};
use dotnet_value::{
    StackValue,
    layout::{LayoutManager, Scalar},
    pointer::{PointerOrigin, UnmanagedPtr},
};
use dotnetdll::prelude::*;
use std::ptr;

#[dotnet_instruction(CopyMemoryBlock { })]
pub fn cpblk<
    'gc,
    'm: 'gc,
    T: StackOps<'gc, 'm> + RawMemoryOps<'gc> + ExceptionOps<'gc> + ?Sized,
>(
    ctx: &mut T,
) -> StepResult {
    let size = vm_pop!(ctx).as_isize() as usize;
    let src = vm_pop!(ctx).as_ptr();
    let dest = vm_pop!(ctx).as_ptr();

    if src.is_null() || dest.is_null() {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    // Check GC safe point before large memory block copy operations
    // Threshold: copying more than 4KB of data
    const LARGE_COPY_THRESHOLD: usize = 4096;
    if size > LARGE_COPY_THRESHOLD {
        // ctx.check_gc_safe_point();
    }

    // SAFETY: The source and destination pointers are obtained from the evaluation stack
    // and are assumed to be valid for the specified size. In .NET, it is the responsibility
    // of the compiler/programmer to ensure these are valid when using cpblk.
    unsafe {
        validate_atomic_access(src, false);
        validate_atomic_access(dest, false);
        ptr::copy(src, dest, size);
    }
    StepResult::Continue
}

#[dotnet_instruction(InitializeMemoryBlock { })]
pub fn initblk<
    'gc,
    'm: 'gc,
    T: StackOps<'gc, 'm> + RawMemoryOps<'gc> + ExceptionOps<'gc> + ?Sized,
>(
    ctx: &mut T,
) -> StepResult {
    let size = vm_pop!(ctx).as_isize() as usize;
    let val = vm_pop!(ctx).as_isize() as u8;
    let addr = vm_pop!(ctx).as_ptr();

    if addr.is_null() {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    // Check GC safe point before large memory block initialization
    if size > 4096 {
        // ctx.check_gc_safe_point();
    }

    // SAFETY: The address and size are obtained from the evaluation stack.
    // Validity of the memory range is the responsibility of the caller in CIL.
    unsafe {
        validate_atomic_access(addr, false);
        ptr::write_bytes(addr, val, size);
    }
    StepResult::Continue
}

#[dotnet_instruction(LocalMemoryAllocate)]
pub fn localloc<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + PoolOps + ExceptionOps<'gc> + ?Sized>(
    ctx: &mut T,
) -> StepResult {
    let size_isize = ctx.pop_isize();
    if size_isize < 0 {
        return ctx.throw_by_name("System.OverflowException");
    }
    let size = size_isize as usize;

    // Defensive check: limit local allocation to 128MB
    if size > 0x800_0000 {
        return ctx.throw_by_name("System.OutOfMemoryException");
    }

    let ptr = ctx.localloc(size);
    if ptr.is_null() {
        return ctx.throw_by_name("System.OutOfMemoryException");
    }

    ctx.push(StackValue::UnmanagedPtr(UnmanagedPtr(
        ptr::NonNull::new(ptr).unwrap(),
    )));
    StepResult::Continue
}

#[dotnet_instruction(StoreIndirect { param0 })]
pub fn stind<'gc, 'm: 'gc>(ctx: &mut dyn VesOps<'gc, 'm>, param0: StoreType) -> StepResult {
    let val = ctx.pop();
    let addr_val = ctx.pop();

    if let StackValue::ManagedPtr(m) = &addr_val
        && let Some((slot_idx, off)) = m.stack_slot_origin()
        && off == ByteOffset::ZERO
    {
        // Direct write to slot - use typed write to maintain StackValue discriminant correctness.
        // Exception: if the slot currently contains a ValueType, we must use raw write to
        // avoid replacing the entire struct with a scalar. Structs on the stack have stable
        // FieldStorage that can be partially overwritten.
        if !matches!(ctx.get_slot_ref(slot_idx), StackValue::ValueType(..)) {
            let typed_val = convert_to_stack_value(val, param0);
            ctx.set_slot(slot_idx, typed_val);
            return StepResult::Continue;
        }
    }

    // Check for null pointer before extracting origin/offset
    if addr_val.is_null() {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    let (origin, offset) = match addr_val {
        StackValue::NativeInt(p) => (PointerOrigin::Unmanaged, ByteOffset(p as usize)),
        StackValue::ManagedPtr(m) => (m.origin, m.offset),
        StackValue::UnmanagedPtr(u) => {
            (PointerOrigin::Unmanaged, ByteOffset(u.0.as_ptr() as usize))
        }
        _ => return ctx.throw_by_name("System.InvalidProgramException"),
    };

    let layout = match param0 {
        StoreType::Int8 => LayoutManager::Scalar(Scalar::Int8),
        StoreType::Int16 => LayoutManager::Scalar(Scalar::Int16),
        StoreType::Int32 => LayoutManager::Scalar(Scalar::Int32),
        StoreType::Int64 => LayoutManager::Scalar(Scalar::Int64),
        StoreType::IntPtr => LayoutManager::Scalar(Scalar::NativeInt),
        StoreType::Float32 => LayoutManager::Scalar(Scalar::Float32),
        StoreType::Float64 => LayoutManager::Scalar(Scalar::Float64),
        StoreType::Object => LayoutManager::Scalar(Scalar::ObjectRef),
    };

    // SAFETY: The pointer and origin are validated by the instruction handler.
    // write_unaligned handles GC-specific write barriers if a heap origin is provided.
    match unsafe { ctx.write_unaligned(origin.clone(), offset, val, &layout) } {
        Ok(_) => {}
        Err(_) => {
            if matches!(origin, PointerOrigin::Unmanaged) && offset.0 == 0 {
                return ctx.throw_by_name("System.NullReferenceException");
            }
            return ctx.throw_by_name("System.AccessViolationException");
        }
    }
    StepResult::Continue
}

#[dotnet_instruction(LoadIndirect { param0 })]
pub fn ldind<'gc, 'm: 'gc>(ctx: &mut dyn VesOps<'gc, 'm>, param0: LoadType) -> StepResult {
    let addr_val = ctx.pop();

    if let StackValue::ManagedPtr(m) = &addr_val
        && let Some((slot_idx, off)) = m.stack_slot_origin()
        && off == ByteOffset::ZERO
        && !matches!(ctx.get_slot_ref(slot_idx), StackValue::ValueType(..))
    {
        let val = convert_from_stack_value(ctx.get_slot(slot_idx), param0);
        ctx.push(val);
        return StepResult::Continue;
    }

    // Check for null pointer before extracting origin/offset
    if addr_val.is_null() {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    let (origin, offset) = match addr_val {
        StackValue::NativeInt(p) => (PointerOrigin::Unmanaged, ByteOffset(p as usize)),
        StackValue::ManagedPtr(m) => (m.origin, m.offset),
        StackValue::UnmanagedPtr(u) => {
            (PointerOrigin::Unmanaged, ByteOffset(u.0.as_ptr() as usize))
        }
        _ => return ctx.throw_by_name("System.InvalidProgramException"),
    };

    let layout = match param0 {
        LoadType::Int8 => LayoutManager::Scalar(Scalar::Int8),
        LoadType::UInt8 => LayoutManager::Scalar(Scalar::UInt8),
        LoadType::Int16 => LayoutManager::Scalar(Scalar::Int16),
        LoadType::UInt16 => LayoutManager::Scalar(Scalar::UInt16),
        LoadType::Int32 => LayoutManager::Scalar(Scalar::Int32),
        LoadType::UInt32 => LayoutManager::Scalar(Scalar::Int32),
        LoadType::Int64 => LayoutManager::Scalar(Scalar::Int64),
        LoadType::Float32 => LayoutManager::Scalar(Scalar::Float32),
        LoadType::Float64 => LayoutManager::Scalar(Scalar::Float64),
        LoadType::IntPtr => LayoutManager::Scalar(Scalar::NativeInt),
        LoadType::Object => LayoutManager::Scalar(Scalar::ObjectRef),
    };

    // SAFETY: The pointer and origin are validated by the instruction handler.
    // read_unaligned handles GC-safe reading from the heap if a heap origin is provided.
    let val = match unsafe { ctx.read_unaligned(origin.clone(), offset, &layout, None) } {
        Ok(v) => v,
        Err(_) => {
            if matches!(origin, PointerOrigin::Unmanaged) && offset.0 == 0 {
                return ctx.throw_by_name("System.NullReferenceException");
            }
            return ctx.throw_by_name("System.AccessViolationException");
        }
    };
    ctx.push(val);
    StepResult::Continue
}

fn convert_to_stack_value(val: StackValue, store_type: StoreType) -> StackValue {
    match store_type {
        StoreType::Int8 | StoreType::Int16 | StoreType::Int32 => StackValue::Int32(val.as_i32()),
        StoreType::Int64 => StackValue::Int64(val.as_i64()),
        StoreType::IntPtr => StackValue::NativeInt(val.as_isize()),
        StoreType::Float32 | StoreType::Float64 => StackValue::NativeFloat(val.as_f64()),
        StoreType::Object => val, // Already correct type
    }
}

fn convert_from_stack_value(val: StackValue, load_type: LoadType) -> StackValue {
    match load_type {
        LoadType::Int8 => StackValue::Int32(val.as_i32() as i8 as i32),
        LoadType::UInt8 => StackValue::Int32(val.as_i32() as u8 as i32),
        LoadType::Int16 => StackValue::Int32(val.as_i32() as i16 as i32),
        LoadType::UInt16 => StackValue::Int32(val.as_i32() as u16 as i32),
        LoadType::Int32 | LoadType::UInt32 => StackValue::Int32(val.as_i32()),
        LoadType::Int64 => StackValue::Int64(val.as_i64()),
        LoadType::Float32 | LoadType::Float64 => StackValue::NativeFloat(val.as_f64()),
        LoadType::IntPtr => StackValue::NativeInt(val.as_isize()),
        LoadType::Object => val,
    }
}
