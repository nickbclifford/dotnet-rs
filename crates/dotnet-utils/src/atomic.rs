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

#[cfg(feature = "memory-validation")]
fn validate_ordering(ordering: Ordering, is_load: bool) {
    match (is_load, ordering) {
        (true, Ordering::Release) | (true, Ordering::AcqRel) => {
            panic!("Invalid load ordering: {:?}", ordering);
        }
        (false, Ordering::Acquire) | (false, Ordering::AcqRel) => {
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

#[cfg(not(feature = "memory-validation"))]
#[inline(always)]
#[allow(dead_code)]
pub fn validate_atomic_access(_ptr: *const u8, _is_atomic: bool) {}

#[cfg(not(feature = "memory-validation"))]
#[inline(always)]
#[allow(dead_code)]
fn validate_ordering(_ordering: Ordering, _is_load: bool) {}

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
}

/// Concrete implementation using `AtomicT::from_ptr`
pub struct StandardAtomicAccess;

#[cfg(feature = "multithreading")]
impl AtomicAccess for StandardAtomicAccess {
    unsafe fn load_atomic(ptr: *const u8, size: usize, ordering: Ordering) -> u64 {
        validate_atomic_access(ptr, true);
        validate_ordering(ordering, true);
        match size {
            1 => unsafe { AtomicU8::from_ptr(ptr as *mut u8) }.load(ordering) as u64,
            2 => unsafe { AtomicU16::from_ptr(ptr as *mut u16) }.load(ordering) as u64,
            4 => unsafe { AtomicU32::from_ptr(ptr as *mut u32) }.load(ordering) as u64,
            8 => unsafe { AtomicU64::from_ptr(ptr as *mut u64) }.load(ordering),
            _ => panic!("Unsupported atomic size: {}", size),
        }
    }

    unsafe fn store_atomic(ptr: *mut u8, size: usize, value: u64, ordering: Ordering) {
        validate_atomic_access(ptr as *const u8, true);
        validate_ordering(ordering, false);
        match size {
            1 => unsafe { AtomicU8::from_ptr(ptr) }.store(value as u8, ordering),
            2 => unsafe { AtomicU16::from_ptr(ptr as *mut u16) }.store(value as u16, ordering),
            4 => unsafe { AtomicU32::from_ptr(ptr as *mut u32) }.store(value as u32, ordering),
            8 => unsafe { AtomicU64::from_ptr(ptr as *mut u64) }.store(value, ordering),
            _ => panic!("Unsupported atomic size: {}", size),
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
            1 => unsafe { AtomicU8::from_ptr(ptr) }
                .compare_exchange(expected as u8, new as u8, success, failure)
                .map(|x| x as u64)
                .map_err(|x| x as u64),
            2 => unsafe { AtomicU16::from_ptr(ptr as *mut u16) }
                .compare_exchange(expected as u16, new as u16, success, failure)
                .map(|x| x as u64)
                .map_err(|x| x as u64),
            4 => unsafe { AtomicU32::from_ptr(ptr as *mut u32) }
                .compare_exchange(expected as u32, new as u32, success, failure)
                .map(|x| x as u64)
                .map_err(|x| x as u64),
            8 => unsafe { AtomicU64::from_ptr(ptr as *mut u64) }
                .compare_exchange(expected, new, success, failure),
            _ => panic!("Unsupported atomic size: {}", size),
        }
    }

    unsafe fn exchange_atomic(ptr: *mut u8, size: usize, new: u64, ordering: Ordering) -> u64 {
        match size {
            1 => unsafe { AtomicU8::from_ptr(ptr) }.swap(new as u8, ordering) as u64,
            2 => unsafe { AtomicU16::from_ptr(ptr as *mut u16) }.swap(new as u16, ordering) as u64,
            4 => unsafe { AtomicU32::from_ptr(ptr as *mut u32) }.swap(new as u32, ordering) as u64,
            8 => unsafe { AtomicU64::from_ptr(ptr as *mut u64) }.swap(new, ordering),
            _ => panic!("Unsupported atomic size: {}", size),
        }
    }
}

#[cfg(not(feature = "multithreading"))]
impl AtomicAccess for StandardAtomicAccess {
    unsafe fn load_atomic(ptr: *const u8, size: usize, _ordering: Ordering) -> u64 {
        validate_atomic_access(ptr, true);
        // In single-threaded mode, we can use simple reads.
        // Since the trait requires alignment, we can use ptr::read for all supported sizes.
        match size {
            1 => unsafe { ptr::read(ptr) as u64 },
            2 => unsafe { ptr::read(ptr as *const u16) as u64 },
            4 => unsafe { ptr::read(ptr as *const u32) as u64 },
            8 => unsafe { ptr::read(ptr as *const u64) },
            _ => panic!("Unsupported atomic size: {}", size),
        }
    }

    unsafe fn store_atomic(ptr: *mut u8, size: usize, value: u64, _ordering: Ordering) {
        validate_atomic_access(ptr as *const u8, true);
        // In single-threaded mode, we can use simple writes.
        // Since the trait requires alignment, we can use ptr::write for all supported sizes.
        match size {
            1 => unsafe { ptr::write(ptr, value as u8) },
            2 => unsafe { ptr::write(ptr as *mut u16, value as u16) },
            4 => unsafe { ptr::write(ptr as *mut u32, value as u32) },
            8 => unsafe { ptr::write(ptr as *mut u64, value) },
            _ => panic!("Unsupported atomic size: {}", size),
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
        let current = unsafe { Self::load_atomic(ptr, size, Ordering::Relaxed) };
        if current == expected {
            unsafe { Self::store_atomic(ptr, size, new, Ordering::Relaxed) };
            Ok(current)
        } else {
            Err(current)
        }
    }

    unsafe fn exchange_atomic(ptr: *mut u8, size: usize, new: u64, _ordering: Ordering) -> u64 {
        let current = unsafe { Self::load_atomic(ptr, size, Ordering::Relaxed) };
        unsafe { Self::store_atomic(ptr, size, new, Ordering::Relaxed) };
        current
    }
}

pub struct Atomic;

impl Atomic {
    /// # Safety
    /// Caller must ensure `ptr` is valid for `size` bytes.
    pub unsafe fn load_field(ptr: *const u8, size: usize, ordering: Ordering) -> Vec<u8> {
        if (size == 1 || size == 2 || size == 4 || size == 8) && is_ptr_aligned_to_field(ptr, size)
        {
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
            unsafe { ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), size) };
            buf
        }
    }

    /// # Safety
    /// Caller must ensure `ptr` is valid for `value.len()` bytes.
    pub unsafe fn store_field(ptr: *mut u8, value: &[u8], ordering: Ordering) {
        let size = value.len();
        if (size == 1 || size == 2 || size == 4 || size == 8) && is_ptr_aligned_to_field(ptr, size)
        {
            let val = match size {
                1 => u8::from_ne_bytes(value.try_into().unwrap()) as u64,
                2 => u16::from_ne_bytes(value.try_into().unwrap()) as u64,
                4 => u32::from_ne_bytes(value.try_into().unwrap()) as u64,
                8 => u64::from_ne_bytes(value.try_into().unwrap()),
                _ => unreachable!(),
            };
            unsafe { StandardAtomicAccess::store_atomic(ptr, size, val, ordering) };
        } else {
            validate_atomic_access(ptr as *const u8, false);
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

        unsafe {
            StandardAtomicAccess::store_atomic(ptr, 1, 0xAA, Ordering::SeqCst);
            assert_eq!(
                StandardAtomicAccess::load_atomic(ptr, 1, Ordering::SeqCst),
                0xAA
            );
            assert_eq!(data[0] as u8, 0xAA);

            StandardAtomicAccess::store_atomic(ptr.add(2), 2, 0xBBCC, Ordering::SeqCst);
            assert_eq!(
                StandardAtomicAccess::load_atomic(ptr.add(2), 2, Ordering::SeqCst),
                0xBBCC
            );
            // On little-endian, 0xBBCC at offset 2 in u64:
            // [00 00 CC BB 00 00 00 00]
            assert_eq!((data[0] >> 16) as u16, 0xBBCC);

            StandardAtomicAccess::store_atomic(ptr.add(4), 4, 0xDEADBEEF, Ordering::SeqCst);
            assert_eq!(
                StandardAtomicAccess::load_atomic(ptr.add(4), 4, Ordering::SeqCst),
                0xDEADBEEF
            );
            assert_eq!((data[0] >> 32) as u32, 0xDEADBEEF);

            StandardAtomicAccess::store_atomic(ptr.add(8), 8, 0x0123456789ABCDEF, Ordering::SeqCst);
            assert_eq!(
                StandardAtomicAccess::load_atomic(ptr.add(8), 8, Ordering::SeqCst),
                0x0123456789ABCDEF
            );
            assert_eq!(data[1], 0x0123456789ABCDEF);
        }
    }

    #[test]
    fn test_orderings() {
        let mut val = 0u64;
        let ptr = &mut val as *mut u64 as *mut u8;

        unsafe {
            // Valid load orderings
            for ord in [Ordering::Relaxed, Ordering::Acquire, Ordering::SeqCst] {
                StandardAtomicAccess::load_atomic(ptr, 8, ord);
            }

            // Valid store orderings
            for ord in [Ordering::Relaxed, Ordering::Release, Ordering::SeqCst] {
                StandardAtomicAccess::store_atomic(ptr, 8, 42, ord);
            }
        }
    }

    #[test]
    #[cfg(feature = "memory-validation")]
    #[should_panic(expected = "Invalid load ordering")]
    fn test_invalid_load_ordering() {
        let val = 0u64;
        let ptr = &val as *const u64 as *const u8;
        unsafe {
            StandardAtomicAccess::load_atomic(ptr, 8, Ordering::Release);
        }
    }

    #[test]
    #[cfg(feature = "memory-validation")]
    #[should_panic(expected = "Invalid store ordering")]
    fn test_invalid_store_ordering() {
        let mut val = 0u64;
        let ptr = &mut val as *mut u64 as *mut u8;
        unsafe {
            StandardAtomicAccess::store_atomic(ptr, 8, 42, Ordering::Acquire);
        }
    }

    #[test]
    #[cfg(feature = "memory-validation")]
    fn test_mixed_access_validation() {
        let mut val = 0u64;
        let ptr = &mut val as *mut u64 as *mut u8;
        unsafe {
            // This should not panic, but it will populate ATOMIC_LOCATIONS
            StandardAtomicAccess::store_atomic(ptr, 8, 42, Ordering::SeqCst);
            // This should trigger a warning (not a panic)
            validate_atomic_access(ptr, false);
        }
    }
}
