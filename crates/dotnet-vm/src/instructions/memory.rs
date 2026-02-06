use crate::{StepResult, stack::VesContext};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    StackValue,
    layout::{LayoutManager, Scalar},
    pointer::UnmanagedPtr,
};
use dotnetdll::prelude::*;
use std::ptr;

#[dotnet_instruction(CopyMemoryBlock { })]
pub fn cpblk<'gc, 'm: 'gc>(ctx: &mut VesContext<'_, 'gc, 'm>, gc: GCHandle<'gc>) -> StepResult {
    let size = ctx.pop_isize(gc) as usize;
    let src_val = ctx.pop(gc);
    let src = match &src_val {
        StackValue::NativeInt(i) => *i as *const u8,
        StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr() as *const u8,
        StackValue::ManagedPtr(m) => m.pointer().map_or(ptr::null(), |p| p.as_ptr()),
        rest => panic!(
            "invalid type for src in cpblk (expected pointer, received {:?})",
            rest
        ),
    };
    let dest_val = ctx.pop(gc);
    let dest = match &dest_val {
        StackValue::NativeInt(i) => *i as *mut u8,
        StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr(),
        StackValue::ManagedPtr(m) => m.pointer().map_or(ptr::null_mut(), |p| p.as_ptr()),
        rest => panic!(
            "invalid type for dest in cpblk (expected pointer, received {:?})",
            rest
        ),
    };

    // Check GC safe point before large memory block copy operations
    // Threshold: copying more than 4KB of data
    const LARGE_COPY_THRESHOLD: usize = 4096;
    if size > LARGE_COPY_THRESHOLD {
        ctx.check_gc_safe_point();
    }

    unsafe {
        ptr::copy(src, dest, size);
    }
    StepResult::Continue
}

#[dotnet_instruction(InitializeMemoryBlock { })]
pub fn initblk<'gc, 'm: 'gc>(ctx: &mut VesContext<'_, 'gc, 'm>, gc: GCHandle<'gc>) -> StepResult {
    let size = ctx.pop_isize(gc) as usize;
    let val = ctx.pop_isize(gc) as u8;
    let addr_val = ctx.pop(gc);
    let addr = match &addr_val {
        StackValue::NativeInt(i) => *i as *mut u8,
        StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr(),
        StackValue::ManagedPtr(m) => m.pointer().map_or(ptr::null_mut(), |p| p.as_ptr()),
        rest => panic!(
            "invalid type for addr in initblk (expected pointer, received {:?})",
            rest
        ),
    };

    // Check GC safe point before large memory block initialization
    if size > 4096 {
        ctx.check_gc_safe_point();
    }

    unsafe {
        ptr::write_bytes(addr, val, size);
    }
    StepResult::Continue
}

#[dotnet_instruction(LocalMemoryAllocate)]
pub fn localloc<'gc, 'm: 'gc>(ctx: &mut VesContext<'_, 'gc, 'm>, gc: GCHandle<'gc>) -> StepResult {
    let size_isize = ctx.pop_isize(gc);
    if size_isize < 0 {
        return ctx.throw_by_name(gc, "System.OverflowException");
    }
    let size = size_isize as usize;

    // Defensive check: limit local allocation to 128MB
    if size > 0x800_0000 {
        return ctx.throw_by_name(gc, "System.OutOfMemoryException");
    }

    let ptr = {
        let s = ctx.state_mut();
        let loc = s.memory_pool.len();
        s.memory_pool.extend(vec![0; size]);
        s.memory_pool[loc..].as_mut_ptr()
    };

    ctx.push(
        gc,
        StackValue::UnmanagedPtr(UnmanagedPtr(std::ptr::NonNull::new(ptr).unwrap())),
    );
    StepResult::Continue
}

#[dotnet_instruction(StoreIndirect { param0 })]
pub fn stind<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    param0: StoreType,
) -> StepResult {
    let val = ctx.pop(gc);
    let addr_val = ctx.pop(gc);

    if let StackValue::ManagedPtr(m) = &addr_val
        && let Some((slot_idx, 0)) = m.stack_slot_origin
    {
        // Direct write to slot - use typed write to maintain StackValue discriminant correctness.
        // Exception: if the slot currently contains a ValueType, we must use raw write to
        // avoid replacing the entire struct with a scalar. Structs on the stack have stable
        // FieldStorage that can be partially overwritten.
        if !matches!(
            ctx.evaluation_stack.get_slot_ref(slot_idx),
            StackValue::ValueType(_)
        ) {
            let typed_val = convert_to_stack_value(val, param0);
            ctx.evaluation_stack.set_slot(gc, slot_idx, typed_val);
            return StepResult::Continue;
        }
    }

    let (ptr, owner) = match addr_val {
        StackValue::NativeInt(p) => (p as *mut u8, None),
        StackValue::ManagedPtr(m) => (m.pointer().map_or(ptr::null_mut(), |p| p.as_ptr()), m.owner),
        StackValue::UnmanagedPtr(u) => (u.0.as_ptr(), None),
        _ => panic!("StoreIndirect: expected pointer, got {:?}", addr_val),
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

    let heap = &ctx.local.heap;
    let mut memory = crate::memory::RawMemoryAccess::new(heap);
    match unsafe { memory.write_unaligned(ptr, owner, val, &layout) } {
        Ok(_) => {}
        Err(e) => panic!("StoreIndirect failed: {}", e),
    }
    StepResult::Continue
}

#[dotnet_instruction(LoadIndirect { param0 })]
pub fn ldind<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    param0: LoadType,
) -> StepResult {
    let addr_val = ctx.pop(gc);

    if let StackValue::ManagedPtr(m) = &addr_val
        && let Some((slot_idx, 0)) = m.stack_slot_origin
        && !matches!(
            ctx.evaluation_stack.get_slot_ref(slot_idx),
            StackValue::ValueType(_)
        )
    {
        let val = convert_from_stack_value(ctx.evaluation_stack.get_slot(slot_idx), param0);
        ctx.push(gc, val);
        return StepResult::Continue;
    }

    let (ptr, owner) = match addr_val {
        StackValue::NativeInt(p) => (p as *const u8, None),
        StackValue::ManagedPtr(m) => (
            m.pointer().map_or(ptr::null(), |p| p.as_ptr() as *const u8),
            m.owner,
        ),
        StackValue::UnmanagedPtr(u) => (u.0.as_ptr() as *const u8, None),
        _ => panic!("LoadIndirect: expected pointer, got {:?}", addr_val),
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

    let heap = &ctx.local.heap;
    let memory = crate::memory::RawMemoryAccess::new(heap);
    let val = unsafe {
        memory
            .read_unaligned(ptr, owner, &layout, None)
            .expect("Read failed")
    };
    ctx.push(gc, val);
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
