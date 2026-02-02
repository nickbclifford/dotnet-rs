use crate::instructions::StepResult;
use crate::{CallStack, vm_push, vm_pop};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{StackValue, pointer::UnmanagedPtr};
use dotnet_macros::dotnet_instruction;
use dotnetdll::prelude::*;
use std::ptr;
use dotnet_value::layout::{LayoutManager, Scalar};

#[dotnet_instruction(CopyMemoryBlock)]
pub fn cpblk<'gc, 'm: 'gc>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>) -> StepResult {
    let size = match vm_pop!(stack, gc) {
        StackValue::Int32(i) => i as usize,
        StackValue::NativeInt(i) => i as usize,
        rest => panic!(
            "invalid type for size in cpblk (expected int32 or native int, received {:?})",
            rest
        ),
    };
    let src_val = vm_pop!(stack, gc);
    let src = match &src_val {
        StackValue::NativeInt(i) => *i as *const u8,
        StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr() as *const u8,
        StackValue::ManagedPtr(m) => m.value.map_or(ptr::null(), |p| p.as_ptr()),
        rest => panic!(
            "invalid type for src in cpblk (expected pointer, received {:?})",
            rest
        ),
    };
    let dest_val = vm_pop!(stack, gc);
    let dest = match &dest_val {
        StackValue::NativeInt(i) => *i as *mut u8,
        StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr(),
        StackValue::ManagedPtr(m) => m.value.map_or(ptr::null_mut(), |p| p.as_ptr()),
        rest => panic!(
            "invalid type for dest in cpblk (expected pointer, received {:?})",
            rest
        ),
    };

    // Check GC safe point before large memory block copy operations
    // Threshold: copying more than 4KB of data
    const LARGE_COPY_THRESHOLD: usize = 4096;
    if size > LARGE_COPY_THRESHOLD {
        stack.check_gc_safe_point();
    }

    unsafe {
        ptr::copy(src, dest, size);
    }
    StepResult::InstructionStepped
}

#[dotnet_instruction(InitializeMemoryBlock)]
pub fn initblk<'gc, 'm: 'gc>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>) -> StepResult {
    let size = match vm_pop!(stack, gc) {
        StackValue::Int32(i) => i as usize,
        StackValue::NativeInt(i) => i as usize,
        rest => panic!(
            "invalid type for size in initblk (expected int32 or native int, received {:?})",
            rest
        ),
    };
    let val = match vm_pop!(stack, gc) {
        StackValue::Int32(i) => i as u8,
        StackValue::NativeInt(i) => i as u8,
        rest => panic!(
            "invalid type for val in initblk (expected int32 or native int, received {:?})",
            rest
        ),
    };
    let addr_val = vm_pop!(stack, gc);
    let addr = match &addr_val {
        StackValue::NativeInt(i) => *i as *mut u8,
        StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr(),
        StackValue::ManagedPtr(m) => m.value.map_or(ptr::null_mut(), |p| p.as_ptr()),
        rest => panic!(
            "invalid type for addr in initblk (expected pointer, received {:?})",
            rest
        ),
    };

    // Check GC safe point before large memory block initialization
    if size > 4096 {
        stack.check_gc_safe_point();
    }

    unsafe {
        ptr::write_bytes(addr, val, size);
    }
    StepResult::InstructionStepped
}

#[dotnet_instruction(LocalMemoryAllocate)]
pub fn localloc<'gc, 'm: 'gc>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>) -> StepResult {
    let size = match vm_pop!(stack, gc) {
        StackValue::Int32(i) => {
            if i < 0 {
                return stack.throw_by_name(gc, "System.OverflowException");
            }
            i as usize
        },
        StackValue::NativeInt(i) => {
             if i < 0 {
                return stack.throw_by_name(gc, "System.OverflowException");
            }
            i as usize
        },
        rest => panic!(
            "invalid type for size in localloc (expected int32 or native int, received {:?})",
            rest
        ),
    };

    // Defensive check: limit local allocation to 128MB
    if size > 0x800_0000 {
        return stack.throw_by_name(gc, "System.OutOfMemoryException");
    }

    let ptr = {
        let s = stack.state_mut();
        let loc = s.memory_pool.len();
        s.memory_pool.extend(vec![0; size]);
        s.memory_pool[loc..].as_mut_ptr()
    };
    
    vm_push!(stack, gc, StackValue::UnmanagedPtr(UnmanagedPtr(std::ptr::NonNull::new(ptr).unwrap())));
    StepResult::InstructionStepped
}

#[dotnet_instruction(StoreIndirect)]
pub fn stind<'gc, 'm: 'gc>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>, param0: StoreType) -> StepResult {
    let val = vm_pop!(stack, gc);
    let addr_val = vm_pop!(stack, gc);

    let ptr = match addr_val {
        StackValue::NativeInt(p) => p as *mut u8,
        StackValue::ManagedPtr(m) => m.value.map_or(ptr::null_mut(), |p| p.as_ptr()),
        StackValue::UnmanagedPtr(u) => u.0.as_ptr(),
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

    let heap = &stack.local.heap;
    let mut memory = crate::memory::RawMemoryAccess::new(heap);
    let owner = heap.find_object(ptr as usize);
    match unsafe { memory.write_unaligned(ptr, owner, val, &layout) } {
        Ok(_) => {}
        Err(e) => panic!("StoreIndirect failed: {}", e),
    }
    StepResult::InstructionStepped
}

#[dotnet_instruction(LoadIndirect)]
pub fn ldind<'gc, 'm: 'gc>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>, param0: LoadType) -> StepResult {
    let addr_val = vm_pop!(stack, gc);
    let (ptr, owner) = match addr_val {
        StackValue::NativeInt(p) => (p as *const u8, None),
        StackValue::ManagedPtr(m) => (m.value.map_or(ptr::null(), |p| p.as_ptr() as *const u8), m.owner),
        StackValue::UnmanagedPtr(u) => (u.0.as_ptr() as *const u8, None),
        _ => panic!("LoadIndirect: expected pointer, got {:?}", addr_val),
    };

    let layout = match param0 {
        LoadType::Int8 => LayoutManager::Scalar(Scalar::Int8),
        LoadType::UInt8 => LayoutManager::Scalar(Scalar::Int8),
        LoadType::Int16 => LayoutManager::Scalar(Scalar::Int16),
        LoadType::UInt16 => LayoutManager::Scalar(Scalar::Int16),
        LoadType::Int32 => LayoutManager::Scalar(Scalar::Int32),
        LoadType::UInt32 => LayoutManager::Scalar(Scalar::Int32),
        LoadType::Int64 => LayoutManager::Scalar(Scalar::Int64),
        LoadType::Float32 => LayoutManager::Scalar(Scalar::Float32),
        LoadType::Float64 => LayoutManager::Scalar(Scalar::Float64),
        LoadType::IntPtr => LayoutManager::Scalar(Scalar::NativeInt),
        LoadType::Object => LayoutManager::Scalar(Scalar::ObjectRef),
    };

    let heap = &stack.local.heap;
    let memory = crate::memory::RawMemoryAccess::new(heap);
    let val = unsafe { memory.read_unaligned(ptr, owner, &layout, None).expect("Read failed") };
    vm_push!(stack, gc, val);
    StepResult::InstructionStepped
}
