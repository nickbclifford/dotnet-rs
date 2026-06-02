use dotnet_vm_ops::{
    StepResult,
    ops::RawMemoryOps,
};
use std::ptr;

const MEM_OP_CHUNK_SIZE: usize = 1024 * 1024; // 1MB chunks

#[inline]
pub(crate) fn ranges_overlap(src: *const u8, dst: *mut u8, len: usize) -> bool {
    let src_start = src as usize;
    let dst_start = dst as usize;
    let src_end = src_start.saturating_add(len);
    let dst_end = dst_start.saturating_add(len);
    src_start < dst_end && dst_start < src_end
}

pub(crate) fn chunked_copy_with_safe_point<'gc, T: RawMemoryOps<'gc>>(
    ctx: &mut T,
    src: *const u8,
    dst: *mut u8,
    total_count: usize,
) -> StepResult {
    if total_count == 0 || src == dst {
        return StepResult::Continue;
    }

    let overlap = ranges_overlap(src, dst, total_count);
    let copy_backward = overlap && (dst as usize) > (src as usize);

    if overlap {
        if copy_backward {
            let mut remaining = total_count;
            while remaining > 0 {
                let current_chunk = std::cmp::min(remaining, MEM_OP_CHUNK_SIZE);
                let start = remaining - current_chunk;
                unsafe {
                    // SAFETY: Pointers are valid for `total_count` bytes, and backward chunking
                    // preserves whole-range memmove semantics for overlapping ranges.
                    ptr::copy(src.add(start), dst.add(start), current_chunk);
                }
                remaining = start;
                if remaining > 0 && ctx.check_gc_safe_point() {
                    return StepResult::Yield;
                }
            }
            return StepResult::Continue;
        }

        let mut offset = 0usize;
        while offset < total_count {
            let current_chunk = std::cmp::min(total_count - offset, MEM_OP_CHUNK_SIZE);
            unsafe {
                // SAFETY: Overlapping ranges require memmove semantics.
                ptr::copy(src.add(offset), dst.add(offset), current_chunk);
            }
            offset += current_chunk;
            if offset < total_count && ctx.check_gc_safe_point() {
                return StepResult::Yield;
            }
        }
        return StepResult::Continue;
    }

    let mut offset = 0usize;
    while offset < total_count {
        let current_chunk = std::cmp::min(total_count - offset, MEM_OP_CHUNK_SIZE);
        unsafe {
            // SAFETY: Chunks are non-overlapping in this branch.
            dotnet_simd::copy_nonoverlapping_raw(dst.add(offset), src.add(offset), current_chunk);
        }
        offset += current_chunk;
        if offset < total_count && ctx.check_gc_safe_point() {
            return StepResult::Yield;
        }
    }

    StepResult::Continue
}

pub(crate) fn chunked_fill_with_safe_point<'gc, T: RawMemoryOps<'gc>>(
    ctx: &mut T,
    dst: *mut u8,
    total_count: usize,
    value: u8,
) -> StepResult {
    let mut offset = 0usize;
    while offset < total_count {
        let current_chunk = std::cmp::min(total_count - offset, MEM_OP_CHUNK_SIZE);
        unsafe {
            // SAFETY: Destination pointer is valid for `total_count` bytes by intrinsic contract.
            dotnet_simd::fill_raw(dst.add(offset), current_chunk, value);
        }
        offset += current_chunk;
        if offset < total_count && ctx.check_gc_safe_point() {
            return StepResult::Yield;
        }
    }

    StepResult::Continue
}
