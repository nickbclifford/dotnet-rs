use crate::{is_ptr_aligned_to_field, sync::Ordering};
use std::ptr;

#[cfg(feature = "multithreading")]
use std::sync::atomic::{AtomicU8, AtomicU16, AtomicU32, AtomicU64};

#[cfg(feature = "memory-validation")]
use std::{cell::RefCell, collections::HashSet};

#[cfg(feature = "memory-validation")]
thread_local! {
    static ATOMIC_LOCATIONS: RefCell<HashSet<*const u8>> = RefCell::new(HashSet::new());
    static NON_ATOMIC_LOCATIONS: RefCell<HashSet<*const u8>> = RefCell::new(HashSet::new());
}

#[cfg(feature = "memory-validation")]
pub fn validate_atomic_access(ptr: *const u8, is_atomic: bool) {
    if is_atomic {
        NON_ATOMIC_LOCATIONS.with(|locations| {
            if locations.borrow().contains(&ptr) {
                tracing::warn!("Mixed atomic and non-atomic access to the same location detected: {:p}. This may indicate a data race or incorrect synchronization.", ptr);
            }
        });
        ATOMIC_LOCATIONS.with(|locations| {
            locations.borrow_mut().insert(ptr);
        });
    } else {
        ATOMIC_LOCATIONS.with(|locations| {
            if locations.borrow().contains(&ptr) {
                tracing::warn!("Mixed atomic and non-atomic access to the same location detected: {:p}. This may indicate a data race or incorrect synchronization.", ptr);
            }
        });
        NON_ATOMIC_LOCATIONS.with(|locations| {
            locations.borrow_mut().insert(ptr);
        });
    }
}

#[cfg(feature = "multithreading")]
fn validate_ordering(ordering: Ordering, is_load: bool) {
    {
        match (is_load, ordering) {
            (true, Ordering::Release) | (true, Ordering::AcqRel) => {
                // invariant: internal atomic load callers must use load-compatible orderings.
                panic!("Invalid load ordering: {:?}", ordering);
            }
            (false, Ordering::Acquire) | (false, Ordering::AcqRel) => {
                // invariant: internal atomic store callers must use store-compatible orderings.
                panic!("Invalid store ordering: {:?}", ordering);
            }
            _ => {}
        }

        if ordering == Ordering::Relaxed {
            tracing::warn!(
                "Relaxed ordering used for atomic access. Ensure this is intentional (e.g., not for a .NET volatile field)."
            );
        }
    }
}

#[cfg(not(feature = "memory-validation"))]
#[inline(always)]
pub fn validate_atomic_access(_ptr: *const u8, _is_atomic: bool) {}

#[cfg(feature = "memory-validation")]
pub fn remove_atomic_locations_in_range(base_ptr: *const u8, len: usize) {
    if len == 0 {
        return;
    }

    let start = base_ptr.addr();
    let end = start.saturating_add(len);

    // Use try_with so that if ATOMIC_LOCATIONS has already been destroyed during
    // TLS teardown (e.g. when GCArena drops its objects after ATOMIC_LOCATIONS
    // was already destroyed), we silently skip cleanup rather than panicking.
    let _ = ATOMIC_LOCATIONS.try_with(|locations| {
        locations.borrow_mut().retain(|location| {
            let addr = location.addr();
            addr < start || addr >= end
        });
    });
}

#[cfg(not(feature = "memory-validation"))]
#[inline(always)]
pub fn remove_atomic_locations_in_range(_base_ptr: *const u8, _len: usize) {}

/// Unified atomic memory access operations.
///
/// This trait provides a consistent interface for atomic loads and stores
/// on raw memory locations, regardless of the underlying type size.
pub trait AtomicAccess {
    /// Atomically load a value of the specified size from the pointer.
    ///
    /// # Safety
    /// - `ptr` must be valid and aligned for the operation
    /// - The pointed memory must be valid for reads
    unsafe fn load_atomic(ptr: *const u8, size: usize, ordering: Ordering) -> u64;

    /// Atomically store a value of the specified size to the pointer.
    ///
    /// # Safety
    /// - `ptr` must be valid and aligned for the operation
    /// - The pointed memory must be valid for writes
    unsafe fn store_atomic(ptr: *mut u8, size: usize, value: u64, ordering: Ordering);

    /// Atomically compare and exchange a value of the specified size.
    ///
    /// # Safety
    /// - `ptr` must be valid and aligned for the operation
    /// - The pointed memory must be valid for reads and writes
    unsafe fn compare_exchange_atomic(
        ptr: *mut u8,
        size: usize,
        expected: u64,
        new: u64,
        success: Ordering,
        failure: Ordering,
    ) -> Result<u64, u64>;

    /// Atomically exchange a value of the specified size.
    ///
    /// # Safety
    /// - `ptr` must be valid and aligned for the operation
    /// - The pointed memory must be valid for reads and writes
    unsafe fn exchange_atomic(ptr: *mut u8, size: usize, new: u64, ordering: Ordering) -> u64;

    /// Atomically add a value to the specified memory location.
    ///
    /// # Safety
    /// - `ptr` must be valid and aligned for the operation
    /// - The pointed memory must be valid for reads and writes
    unsafe fn exchange_add_atomic(ptr: *mut u8, size: usize, value: u64, ordering: Ordering)
    -> u64;
}

/// Concrete implementation using `AtomicT::from_ptr`
pub struct StandardAtomicAccess;

#[cfg(feature = "multithreading")]
impl AtomicAccess for StandardAtomicAccess {
    unsafe fn load_atomic(ptr: *const u8, size: usize, ordering: Ordering) -> u64 {
        validate_atomic_access(ptr, true);
        validate_ordering(ordering, true);
        match size {
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            1 => unsafe { AtomicU8::from_ptr(ptr as *mut u8) }.load(ordering) as u64,
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            2 => unsafe { AtomicU16::from_ptr(ptr as *mut u16) }.load(ordering) as u64,
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            4 => unsafe { AtomicU32::from_ptr(ptr as *mut u32) }.load(ordering) as u64,
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            8 => unsafe { AtomicU64::from_ptr(ptr as *mut u64) }.load(ordering),
            _ => {
                // invariant: VM atomic paths only support 1/2/4/8-byte accesses; this unsafe trait API cannot return Result.
                panic!("Unsupported atomic size: {}", size);
            }
        }
    }

    unsafe fn store_atomic(ptr: *mut u8, size: usize, value: u64, ordering: Ordering) {
        validate_atomic_access(ptr as *const u8, true);
        validate_ordering(ordering, false);
        match size {
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            1 => unsafe { AtomicU8::from_ptr(ptr) }.store(value as u8, ordering),
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            2 => unsafe { AtomicU16::from_ptr(ptr as *mut u16) }.store(value as u16, ordering),
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            4 => unsafe { AtomicU32::from_ptr(ptr as *mut u32) }.store(value as u32, ordering),
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            8 => unsafe { AtomicU64::from_ptr(ptr as *mut u64) }.store(value, ordering),
            _ => {
                // invariant: VM atomic paths only support 1/2/4/8-byte accesses; this unsafe trait API cannot return Result.
                panic!("Unsupported atomic size: {}", size);
            }
        }
    }

    unsafe fn compare_exchange_atomic(
        ptr: *mut u8,
        size: usize,
        expected: u64,
        new: u64,
        success: Ordering,
        failure: Ordering,
    ) -> Result<u64, u64> {
        match size {
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            1 => unsafe { AtomicU8::from_ptr(ptr) }
                .compare_exchange(expected as u8, new as u8, success, failure)
                .map(|x| x as u64)
                .map_err(|x| x as u64),
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            2 => unsafe { AtomicU16::from_ptr(ptr as *mut u16) }
                .compare_exchange(expected as u16, new as u16, success, failure)
                .map(|x| x as u64)
                .map_err(|x| x as u64),
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            4 => unsafe { AtomicU32::from_ptr(ptr as *mut u32) }
                .compare_exchange(expected as u32, new as u32, success, failure)
                .map(|x| x as u64)
                .map_err(|x| x as u64),
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            8 => unsafe { AtomicU64::from_ptr(ptr as *mut u64) }
                .compare_exchange(expected, new, success, failure),
            _ => {
                // invariant: VM atomic paths only support 1/2/4/8-byte accesses; this unsafe trait API cannot return Result.
                panic!("Unsupported atomic size: {}", size);
            }
        }
    }

    unsafe fn exchange_atomic(ptr: *mut u8, size: usize, new: u64, ordering: Ordering) -> u64 {
        match size {
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            1 => unsafe { AtomicU8::from_ptr(ptr) }.swap(new as u8, ordering) as u64,
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            2 => unsafe { AtomicU16::from_ptr(ptr as *mut u16) }.swap(new as u16, ordering) as u64,
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            4 => unsafe { AtomicU32::from_ptr(ptr as *mut u32) }.swap(new as u32, ordering) as u64,
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            8 => unsafe { AtomicU64::from_ptr(ptr as *mut u64) }.swap(new, ordering),
            _ => {
                // invariant: VM atomic paths only support 1/2/4/8-byte accesses; this unsafe trait API cannot return Result.
                panic!("Unsupported atomic size: {}", size);
            }
        }
    }

    unsafe fn exchange_add_atomic(
        ptr: *mut u8,
        size: usize,
        value: u64,
        ordering: Ordering,
    ) -> u64 {
        match size {
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            1 => unsafe { AtomicU8::from_ptr(ptr) }.fetch_add(value as u8, ordering) as u64,
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            2 => unsafe { AtomicU16::from_ptr(ptr as *mut u16) }.fetch_add(value as u16, ordering)
                as u64,
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            4 => unsafe { AtomicU32::from_ptr(ptr as *mut u32) }.fetch_add(value as u32, ordering)
                as u64,
            // SAFETY: The caller guarantees `ptr` is valid and aligned for the selected atomic width.
            8 => unsafe { AtomicU64::from_ptr(ptr as *mut u64) }.fetch_add(value, ordering),
            _ => {
                // invariant: VM atomic paths only support 1/2/4/8-byte accesses; this unsafe trait API cannot return Result.
                panic!("Unsupported atomic size: {}", size);
            }
        }
    }
}

#[cfg(not(feature = "multithreading"))]
impl AtomicAccess for StandardAtomicAccess {
    unsafe fn load_atomic(ptr: *const u8, size: usize, _ordering: Ordering) -> u64 {
        validate_atomic_access(ptr, true);
        // In single-threaded mode, we can use simple reads.
        // Although the trait requires alignment, we use unaligned reads
        // during the transition period to avoid UB if alignment is missed.
        match size {
            // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
            1 => unsafe { ptr.read_unaligned() as u64 },
            // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
            2 => unsafe { (ptr as *const u16).read_unaligned() as u64 },
            // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
            4 => unsafe { (ptr as *const u32).read_unaligned() as u64 },
            // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
            8 => unsafe { (ptr as *const u64).read_unaligned() },
            _ => {
                // invariant: VM atomic paths only support 1/2/4/8-byte accesses; this unsafe trait API cannot return Result.
                panic!("Unsupported atomic size: {}", size);
            }
        }
    }

    unsafe fn store_atomic(ptr: *mut u8, size: usize, value: u64, _ordering: Ordering) {
        validate_atomic_access(ptr as *const u8, true);
        // In single-threaded mode, we can use simple writes.
        // Although the trait requires alignment, we use unaligned writes
        // during the transition period to avoid UB if alignment is missed.
        match size {
            // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
            1 => unsafe { ptr.write_unaligned(value as u8) },
            // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
            2 => unsafe { (ptr as *mut u16).write_unaligned(value as u16) },
            // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
            4 => unsafe { (ptr as *mut u32).write_unaligned(value as u32) },
            // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
            8 => unsafe { (ptr as *mut u64).write_unaligned(value) },
            _ => {
                // invariant: VM atomic paths only support 1/2/4/8-byte accesses; this unsafe trait API cannot return Result.
                panic!("Unsupported atomic size: {}", size);
            }
        }
    }

    unsafe fn compare_exchange_atomic(
        ptr: *mut u8,
        size: usize,
        expected: u64,
        new: u64,
        _success: Ordering,
        _failure: Ordering,
    ) -> Result<u64, u64> {
        // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
        let current = unsafe { Self::load_atomic(ptr, size, Ordering::Relaxed) };
        if current == expected {
            // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
            unsafe { Self::store_atomic(ptr, size, new, Ordering::Relaxed) };
            Ok(current)
        } else {
            Err(current)
        }
    }

    unsafe fn exchange_atomic(ptr: *mut u8, size: usize, new: u64, _ordering: Ordering) -> u64 {
        // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
        let current = unsafe { Self::load_atomic(ptr, size, Ordering::Relaxed) };
        // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
        unsafe { Self::store_atomic(ptr, size, new, Ordering::Relaxed) };
        current
    }

    unsafe fn exchange_add_atomic(
        ptr: *mut u8,
        size: usize,
        value: u64,
        _ordering: Ordering,
    ) -> u64 {
        // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
        let current = unsafe { Self::load_atomic(ptr, size, Ordering::Relaxed) };
        // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
        unsafe { Self::store_atomic(ptr, size, current.wrapping_add(value), Ordering::Relaxed) };
        current
    }
}

pub struct Atomic;

impl Atomic {
    #[inline]
    fn is_atomic_field_access_supported(ptr: *const u8, size: usize) -> bool {
        (size == 1 || size == 2 || size == 4 || size == 8) && is_ptr_aligned_to_field(ptr, size)
    }

    /// # Safety
    /// Caller must ensure `ptr` is valid for `size` bytes.
    /// For sizes > 8 or misaligned pointers, the caller must hold an external lock;
    /// this falls back to a non-atomic memcpy guarded by that lock.
    pub unsafe fn load_field(ptr: *const u8, size: usize, ordering: Ordering) -> Vec<u8> {
        if Self::is_atomic_field_access_supported(ptr, size) {
            // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
            let val = unsafe { StandardAtomicAccess::load_atomic(ptr, size, ordering) };
            match size {
                1 => (val as u8).to_ne_bytes().to_vec(),
                2 => (val as u16).to_ne_bytes().to_vec(),
                4 => (val as u32).to_ne_bytes().to_vec(),
                8 => val.to_ne_bytes().to_vec(),
                _ => unreachable!(),
            }
        } else {
            validate_atomic_access(ptr, false);
            let mut buf = vec![0u8; size];
            // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
            unsafe { ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), size) };
            buf
        }
    }

    /// # Safety
    /// Caller must ensure `ptr` is valid for `value.len()` bytes.
    /// For sizes > 8 or misaligned pointers, the caller must hold an external lock;
    /// this falls back to a non-atomic memcpy guarded by that lock.
    pub unsafe fn store_field(ptr: *mut u8, value: &[u8], ordering: Ordering) {
        let size = value.len();
        if Self::is_atomic_field_access_supported(ptr as *const u8, size) {
            let val = match size {
                1 => u8::from_ne_bytes(
                    value
                        .try_into()
                        .expect("size == 1 guarantees value is 1 byte"),
                ) as u64,
                2 => u16::from_ne_bytes(
                    value
                        .try_into()
                        .expect("size == 2 guarantees value is 2 bytes"),
                ) as u64,
                4 => u32::from_ne_bytes(
                    value
                        .try_into()
                        .expect("size == 4 guarantees value is 4 bytes"),
                ) as u64,
                8 => u64::from_ne_bytes(
                    value
                        .try_into()
                        .expect("size == 8 guarantees value is 8 bytes"),
                ),
                _ => unreachable!(),
            };
            // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
            unsafe { StandardAtomicAccess::store_atomic(ptr, size, val, ordering) };
        } else {
            validate_atomic_access(ptr as *const u8, false);
            // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
            unsafe { ptr::copy_nonoverlapping(value.as_ptr(), ptr, size) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn test_sizes() {
        let mut data = [0u64; 2];
        let ptr = data.as_mut_ptr() as *mut u8;

        // SAFETY: `ptr` points into the test's live two-word backing allocation.
        unsafe { StandardAtomicAccess::store_atomic(ptr, 1, 0xAA, Ordering::SeqCst) };
        // SAFETY: `ptr` points into the test's live two-word backing allocation.
        let first = unsafe { StandardAtomicAccess::load_atomic(ptr, 1, Ordering::SeqCst) };
        assert_eq!(first, 0xAA);
        assert_eq!(data[0] as u8, 0xAA);

        // SAFETY: Offset two is within the test's live two-word backing allocation.
        let ptr2 = unsafe { ptr.add(2) };
        // SAFETY: `ptr2` points into the test's live two-word backing allocation.
        unsafe { StandardAtomicAccess::store_atomic(ptr2, 2, 0xBBCC, Ordering::SeqCst) };
        // SAFETY: `ptr2` points into the test's live two-word backing allocation.
        let second = unsafe { StandardAtomicAccess::load_atomic(ptr2, 2, Ordering::SeqCst) };
        assert_eq!(second, 0xBBCC);
        // On little-endian, 0xBBCC at offset 2 in u64:
        // [00 00 CC BB 00 00 00 00]
        assert_eq!((data[0] >> 16) as u16, 0xBBCC);

        // SAFETY: Offset four is within the test's live two-word backing allocation.
        let ptr4 = unsafe { ptr.add(4) };
        // SAFETY: `ptr4` points into the test's live two-word backing allocation.
        unsafe { StandardAtomicAccess::store_atomic(ptr4, 4, 0xDEADBEEF, Ordering::SeqCst) };
        // SAFETY: `ptr4` points into the test's live two-word backing allocation.
        let fourth = unsafe { StandardAtomicAccess::load_atomic(ptr4, 4, Ordering::SeqCst) };
        assert_eq!(fourth, 0xDEADBEEF);
        assert_eq!((data[0] >> 32) as u32, 0xDEADBEEF);

        // SAFETY: Offset eight is the start of the second word in the allocation.
        let ptr8 = unsafe { ptr.add(8) };
        // SAFETY: `ptr8` points into the test's live two-word backing allocation.
        unsafe {
            StandardAtomicAccess::store_atomic(ptr8, 8, 0x0123456789ABCDEF, Ordering::SeqCst)
        };
        // SAFETY: `ptr8` points into the test's live two-word backing allocation.
        let eighth = unsafe { StandardAtomicAccess::load_atomic(ptr8, 8, Ordering::SeqCst) };
        assert_eq!(eighth, 0x0123456789ABCDEF);
        assert_eq!(data[1], 0x0123456789ABCDEF);
    }

    #[test]
    fn test_orderings() {
        let mut val = 0u64;
        let ptr = std::ptr::from_mut(&mut val).cast::<u8>();

        // Valid load orderings.
        for ord in [Ordering::Relaxed, Ordering::Acquire, Ordering::SeqCst] {
            // SAFETY: `ptr` points to the live local `u64` used by this test.
            unsafe { StandardAtomicAccess::load_atomic(ptr, 8, ord) };
        }

        // Valid store orderings.
        for ord in [Ordering::Relaxed, Ordering::Release, Ordering::SeqCst] {
            // SAFETY: `ptr` points to the live local `u64` used by this test.
            unsafe { StandardAtomicAccess::store_atomic(ptr, 8, 42, ord) };
        }
    }

    #[test]
    #[cfg(all(feature = "memory-validation", feature = "multithreading"))]
    #[should_panic(expected = "Invalid load ordering")]
    fn test_invalid_load_ordering() {
        let val = 0u64;
        let ptr = std::ptr::from_ref(&val).cast::<u8>();
        // SAFETY: `ptr` points to the live local `u64` used by this test.
        unsafe {
            StandardAtomicAccess::load_atomic(ptr, 8, Ordering::Release);
        }
    }

    #[test]
    #[cfg(all(feature = "memory-validation", feature = "multithreading"))]
    #[should_panic(expected = "Invalid store ordering")]
    fn test_invalid_store_ordering() {
        let mut val = 0u64;
        let ptr = std::ptr::from_mut(&mut val).cast::<u8>();
        // SAFETY: `ptr` points to the live local `u64` used by this test.
        unsafe {
            StandardAtomicAccess::store_atomic(ptr, 8, 42, Ordering::Acquire);
        }
    }

    #[test]
    #[cfg(feature = "memory-validation")]
    fn test_mixed_access_validation() {
        let mut val = 0u64;
        let ptr = std::ptr::from_mut(&mut val).cast::<u8>();
        // SAFETY: `ptr` points to the live local `u64` used by this test.
        unsafe {
            // This should not panic, but it will populate ATOMIC_LOCATIONS
            StandardAtomicAccess::store_atomic(ptr, 8, 42, Ordering::SeqCst);
            // This trigger a warning (not a panic)
            validate_atomic_access(ptr, false);
        }
    }

    #[test]
    #[cfg(feature = "memory-validation")]
    #[should_panic(expected = "Alignment violation")]
    fn test_validate_alignment_rejects_misaligned_pointer() {
        let data = [0u8; 16];
        // SAFETY: Offset one remains within the live sixteen-byte allocation.
        let misaligned_ptr = unsafe { data.as_ptr().add(1) };
        crate::validate_alignment(misaligned_ptr, 4);
    }

    #[test]
    #[cfg(feature = "multithreading")]
    fn test_load_field_misaligned_falls_back_in_mt() {
        // Misaligned 8-byte load: not atomically expressible, falls back to memcpy.
        // Caller must hold an external lock; here we just verify correct data is returned.
        let mut data = [0u8; 16];
        data[1..9].copy_from_slice(&0x0102030405060708u64.to_ne_bytes());
        let ptr = data.as_ptr();
        // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
        let misaligned_ptr = unsafe { ptr.add(1) };
        // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
        let loaded = unsafe { Atomic::load_field(misaligned_ptr, 8, Ordering::SeqCst) };
        assert_eq!(loaded, 0x0102030405060708u64.to_ne_bytes());
    }

    #[test]
    #[cfg(feature = "multithreading")]
    fn test_store_field_large_size_falls_back_in_mt() {
        // 3-byte store: not atomically expressible, falls back to memcpy.
        // Caller must hold an external lock; here we just verify the bytes are written.
        let mut data = [0u8; 8];
        // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
        unsafe {
            Atomic::store_field(data.as_mut_ptr(), &[1, 2, 3], Ordering::SeqCst);
        }
        assert_eq!(&data[..3], &[1, 2, 3]);
    }

    #[test]
    #[cfg(not(feature = "multithreading"))]
    fn test_load_field_misaligned_falls_back_non_mt() {
        let mut data = [0u8; 8];
        data[1..5].copy_from_slice(&[0x11, 0x22, 0x33, 0x44]);
        let ptr = data.as_ptr();
        // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
        let misaligned_ptr = unsafe { ptr.add(1) };
        // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
        let loaded = unsafe { Atomic::load_field(misaligned_ptr, 4, Ordering::SeqCst) };
        assert_eq!(loaded, vec![0x11, 0x22, 0x33, 0x44]);
    }

    #[test]
    #[cfg(not(feature = "multithreading"))]
    fn test_store_field_unsupported_size_falls_back_non_mt() {
        let mut data = [0u8; 8];
        // SAFETY: The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
        unsafe {
            Atomic::store_field(data.as_mut_ptr(), &[0xAA, 0xBB, 0xCC], Ordering::SeqCst);
        }
        assert_eq!(&data[..3], &[0xAA, 0xBB, 0xCC]);
    }
}
