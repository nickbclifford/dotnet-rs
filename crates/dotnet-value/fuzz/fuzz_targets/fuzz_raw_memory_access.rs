#![no_main]
use dotnet_utils::atomic::{AtomicAccess, StandardAtomicAccess};
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
    
    let mut buffer = [0u8; 1024];
    let offset_val = (offset as usize) % (1024 - 8);
    
    let ptr = unsafe { buffer.as_mut_ptr().add(offset_val) };
    
    // Note: store_atomic/load_atomic might panic on invalid orderings
    // (e.g. Release for load, Acquire for store).
    // Let's filter those out to avoid uninteresting panics.
    
    match ordering {
        Ordering::Release | Ordering::AcqRel if true => {
            // Invalid for load
        }
        _ => {
             let _loaded = unsafe { StandardAtomicAccess::load_atomic(ptr, size, ordering) };
        }
    }
    
    unsafe {
        StandardAtomicAccess::store_atomic(ptr, size, value, Ordering::Relaxed);
    }
    let loaded = unsafe { StandardAtomicAccess::load_atomic(ptr, size, Ordering::Relaxed) };
    
    let mask = if size == 8 { !0u64 } else { (1u64 << (size * 8)) - 1 };
    assert_eq!(loaded, value & mask, "Value mismatch at size {} offset {}", size, offset_val);
});
