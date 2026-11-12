use dotnet_vm_data::StepResult;
use std::ptr;

/// Moves `total_count` bytes with memmove semantics in one step. Operations above
/// [`dotnet_vm_ops::effects::BULK_CHUNK_SIZE`] reach the chunked native body instead, so this
/// never reaches a safepoint.
///
/// # Safety
/// Both ranges must be valid for `total_count` bytes, with synchronized reads and exclusive
/// writes and no conflicting live Rust references. Source bytes must be initialized.
/// Unmanaged ranges are guaranteed by the guest unsafe-memory contract; retained owners keep
/// managed allocations alive for the complete call.
pub(crate) unsafe fn copy_one_chunk(
    src: *const u8,
    dst: *mut u8,
    total_count: usize,
) -> StepResult {
    if total_count != 0 && src != dst.cast_const() {
        // SAFETY: F10.RawMemoryAccessValid — the caller proved both ranges; `copy` has memmove
        // semantics for overlapping ranges.
        unsafe { ptr::copy(src, dst, total_count) };
    }
    StepResult::Continue
}

/// Fills `total_count` bytes in one step.
///
/// # Safety
/// The destination must be valid for `total_count` bytes by the intrinsic contract.
pub(crate) unsafe fn fill_one_chunk(dst: *mut u8, total_count: usize, value: u8) -> StepResult {
    if total_count != 0 {
        // SAFETY: F10.RawMemoryAccessValid — the caller proved the destination range.
        unsafe { dotnet_simd::fill_raw(dst, total_count, value) };
    }
    StepResult::Continue
}
