#![no_main]
use dotnet_utils::atomic::StandardAtomicAccess;
use libfuzzer_sys::fuzz_target;
use std::sync::atomic::Ordering;

fuzz_target!(|data: (u16, u8, u64, u8)| {
    let (offset, size_idx, value, ord_idx) = data;

    let sizes = [1, 2, 4, 8];
    let size = sizes[(size_idx % 4) as usize];

    let orderings = [
        Ordering::Relaxed,
        Ordering::Acquire,
        Ordering::Release,
        Ordering::AcqRel,
        Ordering::SeqCst,
    ];
    let ordering = orderings[(ord_idx % 5) as usize];

    let mut buffer = [0u64; 128];
    let offset_val = ((offset as usize) % (1024 - 8)) & !(size - 1);

    // SAFETY: F3.InteriorPointerRebased — The rounded offset stays in bounds and preserves alignment for `size`.
    let ptr = unsafe { buffer.as_mut_ptr().cast::<u8>().add(offset_val) };

    // Skip load-incompatible orderings so fuzzing targets memory access rather than invariant panics.
    match ordering {
        Ordering::Release | Ordering::AcqRel => {}
        _ => {
            // SAFETY: F4.WidthAligned — `ptr` is live and aligned for the selected width.
            let _loaded = unsafe { StandardAtomicAccess::load_atomic_sized(ptr, size, ordering) };
        }
    }

    // SAFETY: F4.WidthAligned — `ptr` is live and aligned for the selected width.
    unsafe {
        StandardAtomicAccess::store_atomic_sized(ptr, size, value, Ordering::Relaxed);
    }
    // SAFETY: F4.WidthAligned — `ptr` is live and aligned for the selected width.
    let loaded = unsafe { StandardAtomicAccess::load_atomic_sized(ptr, size, Ordering::Relaxed) };

    let mask = if size == 8 {
        !0u64
    } else {
        (1u64 << (size * 8)) - 1
    };
    assert_eq!(
        loaded,
        value & mask,
        "Value mismatch at size {size} offset {offset_val}"
    );
});
