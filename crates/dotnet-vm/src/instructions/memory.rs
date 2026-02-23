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

const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";
const INVALID_PROGRAM_MSG: &str = "Common Language Runtime detected an invalid program.";
const OVERFLOW_MSG: &str = "Arithmetic operation resulted in an overflow.";
const OUT_OF_MEMORY_MSG: &str = "Insufficient memory to continue the execution of the program.";
const ACCESS_VIOLATION_MSG: &str = "Attempted to read or write protected memory.";

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
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    // Perform the move in chunks to allow GC safe points if necessary.
    // Since our GC is currently non-moving, raw pointers remain valid across safe points.
    const CHUNK_SIZE: usize = 1024 * 1024; // 1MB chunks
    let mut offset = 0;
    while offset < size {
        let current_chunk = std::cmp::min(size - offset, CHUNK_SIZE);
        unsafe {
            validate_atomic_access(src.add(offset), false);
            validate_atomic_access(dest.add(offset), false);
            ptr::copy(src.add(offset), dest.add(offset), current_chunk);
        }
        offset += current_chunk;
        if offset < size && ctx.check_gc_safe_point() {
            return StepResult::Yield;
        }
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
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    // Perform the initialization in chunks to allow GC safe points if necessary.
    const CHUNK_SIZE: usize = 1024 * 1024; // 1MB chunks
    let mut offset = 0;
    while offset < size {
        let current_chunk = std::cmp::min(size - offset, CHUNK_SIZE);
        unsafe {
            validate_atomic_access(addr.add(offset), false);
            ptr::write_bytes(addr.add(offset), val, current_chunk);
        }
        offset += current_chunk;
        if offset < size && ctx.check_gc_safe_point() {
            return StepResult::Yield;
        }
    }
    StepResult::Continue
}

#[dotnet_instruction(LocalMemoryAllocate)]
pub fn localloc<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + PoolOps + ExceptionOps<'gc> + ?Sized>(
    ctx: &mut T,
) -> StepResult {
    let size_isize = ctx.pop_isize();
    if size_isize < 0 {
        return ctx.throw_by_name_with_message("System.OverflowException", OVERFLOW_MSG);
    }
    let size = size_isize as usize;

    // Defensive check: limit local allocation to 128MB
    if size > 0x800_0000 {
        return ctx.throw_by_name_with_message("System.OutOfMemoryException", OUT_OF_MEMORY_MSG);
    }

    let ptr = ctx.localloc(size);
    if ptr.is_null() {
        return ctx.throw_by_name_with_message("System.OutOfMemoryException", OUT_OF_MEMORY_MSG);
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
        && !matches!(ctx.get_slot_ref(slot_idx), StackValue::ValueType(..))
    {
        let typed_val = convert_to_stack_value(val, param0);
        ctx.set_slot(slot_idx, typed_val);
        return StepResult::Continue;
    }

    // Check for null pointer before extracting origin/offset
    if addr_val.is_null() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    let (origin, offset) = match addr_val {
        StackValue::NativeInt(p) => (PointerOrigin::Unmanaged, ByteOffset(p as usize)),
        StackValue::ManagedPtr(m) => (m.origin, m.offset),
        StackValue::UnmanagedPtr(u) => {
            (PointerOrigin::Unmanaged, ByteOffset(u.0.as_ptr() as usize))
        }
        _ => {
            return ctx
                .throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
        }
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
                return ctx
                    .throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
            }
            return ctx.throw_by_name_with_message(
                "System.AccessViolationException",
                ACCESS_VIOLATION_MSG,
            );
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
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    let (origin, offset) = match addr_val {
        StackValue::NativeInt(p) => (PointerOrigin::Unmanaged, ByteOffset(p as usize)),
        StackValue::ManagedPtr(m) => (m.origin, m.offset),
        StackValue::UnmanagedPtr(u) => {
            (PointerOrigin::Unmanaged, ByteOffset(u.0.as_ptr() as usize))
        }
        _ => {
            return ctx
                .throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
        }
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
                return ctx
                    .throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
            }
            return ctx.throw_by_name_with_message(
                "System.AccessViolationException",
                ACCESS_VIOLATION_MSG,
            );
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
