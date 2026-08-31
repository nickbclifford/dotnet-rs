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

mod sealed {
    pub trait Sealed {}
}

/// A sealed marker for one of the VM's supported atomic widths.
pub trait AtomicWidth: sealed::Sealed {
    /// The integer representation whose size is exactly this width.
    type Repr: AtomicRepr;
    /// Number of bytes in this width.
    const SIZE: usize;
}

/// One-byte atomic width.
pub struct W1;
/// Two-byte atomic width.
pub struct W2;
/// Four-byte atomic width.
pub struct W4;
/// Eight-byte atomic width.
pub struct W8;

macro_rules! impl_atomic_width {
    ($width:ty, $repr:ty, $size:expr) => {
        impl sealed::Sealed for $width {}
        impl AtomicWidth for $width {
            type Repr = $repr;
            const SIZE: usize = $size;
        }
    };
}

impl_atomic_width!(W1, u8, 1);
impl_atomic_width!(W2, u16, 2);
impl_atomic_width!(W4, u32, 4);
impl_atomic_width!(W8, u64, 8);

/// Internal primitive operations selected by a sealed [`AtomicWidth`].
#[doc(hidden)]
pub trait AtomicRepr: sealed::Sealed + Copy {
    fn from_u64(value: u64) -> Self;
    fn into_u64(self) -> u64;
    fn from_ne_bytes(bytes: &[u8]) -> Self;
    fn to_ne_bytes(self) -> Vec<u8>;

    unsafe fn load(ptr: *const u8, ordering: Ordering) -> Self;
    unsafe fn store(ptr: *mut u8, value: Self, ordering: Ordering);
    unsafe fn compare_exchange(
        ptr: *mut u8,
        expected: Self,
        new: Self,
        success: Ordering,
        failure: Ordering,
    ) -> Result<Self, Self>;
    unsafe fn exchange(ptr: *mut u8, new: Self, ordering: Ordering) -> Self;
    unsafe fn exchange_add(ptr: *mut u8, value: Self, ordering: Ordering) -> Self;
}

macro_rules! impl_atomic_repr {
    ($repr:ty, $atomic:ty) => {
        impl sealed::Sealed for $repr {}
        impl AtomicRepr for $repr {
            fn from_u64(value: u64) -> Self {
                value as Self
            }

            fn into_u64(self) -> u64 {
                self as u64
            }

            fn from_ne_bytes(bytes: &[u8]) -> Self {
                <$repr>::from_ne_bytes(
                    bytes
                        .try_into()
                        .expect("atomic width marker guarantees representation byte length"),
                )
            }

            fn to_ne_bytes(self) -> Vec<u8> {
                <$repr>::to_ne_bytes(self).to_vec()
            }

            unsafe fn load(ptr: *const u8, ordering: Ordering) -> Self {
                #[cfg(feature = "multithreading")]
                {
                    // SAFETY: F4.WidthAligned — `AtomicWidth` fixes this representation's width; the caller proves `ptr` is valid and aligned for it.
                    unsafe { <$atomic>::from_ptr(ptr.cast_mut().cast::<Self>()) }.load(ordering)
                }
                #[cfg(not(feature = "multithreading"))]
                {
                    let _ = ordering;
                    // SAFETY: F10.RawMemoryAccessValid — The caller proves valid readable storage; unaligned access preserves the single-threaded fallback behavior.
                    unsafe { ptr.cast::<Self>().read_unaligned() }
                }
            }

            unsafe fn store(ptr: *mut u8, value: Self, ordering: Ordering) {
                #[cfg(feature = "multithreading")]
                {
                    // SAFETY: F4.WidthAligned — `AtomicWidth` fixes this representation's width; the caller proves `ptr` is valid and aligned for it.
                    unsafe { <$atomic>::from_ptr(ptr.cast::<Self>()) }.store(value, ordering);
                }
                #[cfg(not(feature = "multithreading"))]
                {
                    let _ = ordering;
                    // SAFETY: F10.RawMemoryAccessValid — The caller proves valid writable storage; unaligned access preserves the single-threaded fallback behavior.
                    unsafe { ptr.cast::<Self>().write_unaligned(value) };
                }
            }

            unsafe fn compare_exchange(
                ptr: *mut u8,
                expected: Self,
                new: Self,
                success: Ordering,
                failure: Ordering,
            ) -> Result<Self, Self> {
                #[cfg(feature = "multithreading")]
                {
                    // SAFETY: F4.WidthAligned — `AtomicWidth` fixes this representation's width; the caller proves `ptr` is valid and aligned for it.
                    unsafe { <$atomic>::from_ptr(ptr.cast::<Self>()) }
                        .compare_exchange(expected, new, success, failure)
                }
                #[cfg(not(feature = "multithreading"))]
                {
                    let _ = (success, failure);
                    // SAFETY: F10.RawMemoryAccessValid — The caller proves valid readable storage for this sealed representation.
                    let current = unsafe { Self::load(ptr, Ordering::Relaxed) };
                    if current == expected {
                        // SAFETY: F10.RawMemoryAccessValid — The caller also proves valid writable storage for this sealed representation.
                        unsafe { Self::store(ptr, new, Ordering::Relaxed) };
                        Ok(current)
                    } else {
                        Err(current)
                    }
                }
            }

            unsafe fn exchange(ptr: *mut u8, new: Self, ordering: Ordering) -> Self {
                #[cfg(feature = "multithreading")]
                {
                    // SAFETY: F4.WidthAligned — `AtomicWidth` fixes this representation's width; the caller proves `ptr` is valid and aligned for it.
                    unsafe { <$atomic>::from_ptr(ptr.cast::<Self>()) }.swap(new, ordering)
                }
                #[cfg(not(feature = "multithreading"))]
                {
                    // SAFETY: F10.RawMemoryAccessValid — The caller proves valid readable storage for this sealed representation.
                    let current = unsafe { Self::load(ptr, ordering) };
                    // SAFETY: F10.RawMemoryAccessValid — The caller proves valid writable storage for this sealed representation.
                    unsafe { Self::store(ptr, new, ordering) };
                    current
                }
            }

            unsafe fn exchange_add(ptr: *mut u8, value: Self, ordering: Ordering) -> Self {
                #[cfg(feature = "multithreading")]
                {
                    // SAFETY: F4.WidthAligned — `AtomicWidth` fixes this representation's width; the caller proves `ptr` is valid and aligned for it.
                    unsafe { <$atomic>::from_ptr(ptr.cast::<Self>()) }.fetch_add(value, ordering)
                }
                #[cfg(not(feature = "multithreading"))]
                {
                    // SAFETY: F10.RawMemoryAccessValid — The caller proves valid readable storage for this sealed representation.
                    let current = unsafe { Self::load(ptr, ordering) };
                    // SAFETY: F10.RawMemoryAccessValid — The caller proves valid writable storage for this sealed representation.
                    unsafe { Self::store(ptr, current.wrapping_add(value), ordering) };
                    current
                }
            }
        }
    };
}

impl_atomic_repr!(u8, AtomicU8);
impl_atomic_repr!(u16, AtomicU16);
impl_atomic_repr!(u32, AtomicU32);
impl_atomic_repr!(u64, AtomicU64);

/// Unified, width-typed atomic memory operations.
pub trait AtomicAccess {
    /// # Safety
    /// `ptr` must be valid and aligned for `W::Repr`, and readable for the operation.
    unsafe fn load_atomic<W: AtomicWidth>(ptr: *const u8, ordering: Ordering) -> W::Repr;

    /// # Safety
    /// `ptr` must be valid and aligned for `W::Repr`, and writable for the operation.
    unsafe fn store_atomic<W: AtomicWidth>(ptr: *mut u8, value: W::Repr, ordering: Ordering);

    /// # Safety
    /// `ptr` must be valid and aligned for `W::Repr`, and readable and writable for the operation.
    unsafe fn compare_exchange_atomic<W: AtomicWidth>(
        ptr: *mut u8,
        expected: W::Repr,
        new: W::Repr,
        success: Ordering,
        failure: Ordering,
    ) -> Result<W::Repr, W::Repr>;

    /// # Safety
    /// `ptr` must be valid and aligned for `W::Repr`, and readable and writable for the operation.
    unsafe fn exchange_atomic<W: AtomicWidth>(
        ptr: *mut u8,
        new: W::Repr,
        ordering: Ordering,
    ) -> W::Repr;

    /// # Safety
    /// `ptr` must be valid and aligned for `W::Repr`, and readable and writable for the operation.
    unsafe fn exchange_add_atomic<W: AtomicWidth>(
        ptr: *mut u8,
        value: W::Repr,
        ordering: Ordering,
    ) -> W::Repr;
}

/// Concrete implementation using `AtomicT::from_ptr`.
pub struct StandardAtomicAccess;

impl AtomicAccess for StandardAtomicAccess {
    unsafe fn load_atomic<W: AtomicWidth>(ptr: *const u8, ordering: Ordering) -> W::Repr {
        validate_atomic_access(ptr, true);
        #[cfg(feature = "multithreading")]
        validate_ordering(ordering, true);
        // SAFETY: F4.WidthAligned — `W` selects the representation and the caller proves `ptr` valid and aligned for that exact representation.
        unsafe { W::Repr::load(ptr, ordering) }
    }

    unsafe fn store_atomic<W: AtomicWidth>(ptr: *mut u8, value: W::Repr, ordering: Ordering) {
        validate_atomic_access(ptr.cast_const(), true);
        #[cfg(feature = "multithreading")]
        validate_ordering(ordering, false);
        // SAFETY: F4.WidthAligned — `W` selects the representation and the caller proves `ptr` valid and aligned for that exact representation.
        unsafe { W::Repr::store(ptr, value, ordering) };
    }

    unsafe fn compare_exchange_atomic<W: AtomicWidth>(
        ptr: *mut u8,
        expected: W::Repr,
        new: W::Repr,
        success: Ordering,
        failure: Ordering,
    ) -> Result<W::Repr, W::Repr> {
        // SAFETY: F4.WidthAligned — `W` selects the representation and the caller proves `ptr` valid and aligned for that exact representation.
        unsafe { W::Repr::compare_exchange(ptr, expected, new, success, failure) }
    }

    unsafe fn exchange_atomic<W: AtomicWidth>(
        ptr: *mut u8,
        new: W::Repr,
        ordering: Ordering,
    ) -> W::Repr {
        // SAFETY: F4.WidthAligned — `W` selects the representation and the caller proves `ptr` valid and aligned for that exact representation.
        unsafe { W::Repr::exchange(ptr, new, ordering) }
    }

    unsafe fn exchange_add_atomic<W: AtomicWidth>(
        ptr: *mut u8,
        value: W::Repr,
        ordering: Ordering,
    ) -> W::Repr {
        // SAFETY: F4.WidthAligned — `W` selects the representation and the caller proves `ptr` valid and aligned for that exact representation.
        unsafe { W::Repr::exchange_add(ptr, value, ordering) }
    }
}

trait AtomicOperation {
    type Output;

    unsafe fn execute<W: AtomicWidth>(self, ptr: *mut u8) -> Self::Output;
}

struct LoadOperation {
    ordering: Ordering,
}

struct LoadBytesOperation {
    ordering: Ordering,
}

impl AtomicOperation for LoadBytesOperation {
    type Output = Vec<u8>;

    unsafe fn execute<W: AtomicWidth>(self, ptr: *mut u8) -> Self::Output {
        // SAFETY: F4.WidthAligned — The width dispatcher selects `W` from the requested supported size, and its caller proves `ptr` valid and aligned for that size.
        unsafe { <StandardAtomicAccess as AtomicAccess>::load_atomic::<W>(ptr, self.ordering) }
            .to_ne_bytes()
    }
}

impl AtomicOperation for LoadOperation {
    type Output = u64;

    unsafe fn execute<W: AtomicWidth>(self, ptr: *mut u8) -> Self::Output {
        // SAFETY: F4.WidthAligned — The width dispatcher selects `W` from the requested supported size, and its caller proves `ptr` valid and aligned for that size.
        unsafe { <StandardAtomicAccess as AtomicAccess>::load_atomic::<W>(ptr, self.ordering) }
            .into_u64()
    }
}

struct StoreOperation {
    value: u64,
    ordering: Ordering,
}

impl AtomicOperation for StoreOperation {
    type Output = ();

    unsafe fn execute<W: AtomicWidth>(self, ptr: *mut u8) {
        // SAFETY: F4.WidthAligned — The width dispatcher selects `W` from the requested supported size, and its caller proves `ptr` valid and aligned for that size.
        unsafe {
            <StandardAtomicAccess as AtomicAccess>::store_atomic::<W>(
                ptr,
                W::Repr::from_u64(self.value),
                self.ordering,
            );
        }
    }
}

struct StoreBytesOperation<'a> {
    value: &'a [u8],
    ordering: Ordering,
}

impl AtomicOperation for StoreBytesOperation<'_> {
    type Output = ();

    unsafe fn execute<W: AtomicWidth>(self, ptr: *mut u8) {
        // SAFETY: F4.WidthAligned — The width dispatcher selects `W` from the requested supported size, and its caller proves `ptr` valid and aligned for that size.
        unsafe {
            <StandardAtomicAccess as AtomicAccess>::store_atomic::<W>(
                ptr,
                W::Repr::from_ne_bytes(self.value),
                self.ordering,
            );
        }
    }
}

struct CompareExchangeOperation {
    expected: u64,
    new: u64,
    success: Ordering,
    failure: Ordering,
}

impl AtomicOperation for CompareExchangeOperation {
    type Output = Result<u64, u64>;

    unsafe fn execute<W: AtomicWidth>(self, ptr: *mut u8) -> Self::Output {
        // SAFETY: F4.WidthAligned — The width dispatcher selects `W` from the requested supported size, and its caller proves `ptr` valid and aligned for that size.
        unsafe {
            <StandardAtomicAccess as AtomicAccess>::compare_exchange_atomic::<W>(
                ptr,
                W::Repr::from_u64(self.expected),
                W::Repr::from_u64(self.new),
                self.success,
                self.failure,
            )
        }
        .map(AtomicRepr::into_u64)
        .map_err(AtomicRepr::into_u64)
    }
}

struct ExchangeOperation {
    new: u64,
    ordering: Ordering,
}

impl AtomicOperation for ExchangeOperation {
    type Output = u64;

    unsafe fn execute<W: AtomicWidth>(self, ptr: *mut u8) -> Self::Output {
        // SAFETY: F4.WidthAligned — The width dispatcher selects `W` from the requested supported size, and its caller proves `ptr` valid and aligned for that size.
        unsafe {
            <StandardAtomicAccess as AtomicAccess>::exchange_atomic::<W>(
                ptr,
                W::Repr::from_u64(self.new),
                self.ordering,
            )
        }
        .into_u64()
    }
}

struct ExchangeAddOperation {
    value: u64,
    ordering: Ordering,
}

impl AtomicOperation for ExchangeAddOperation {
    type Output = u64;

    unsafe fn execute<W: AtomicWidth>(self, ptr: *mut u8) -> Self::Output {
        // SAFETY: F4.WidthAligned — The width dispatcher selects `W` from the requested supported size, and its caller proves `ptr` valid and aligned for that size.
        unsafe {
            <StandardAtomicAccess as AtomicAccess>::exchange_add_atomic::<W>(
                ptr,
                W::Repr::from_u64(self.value),
                self.ordering,
            )
        }
        .into_u64()
    }
}

unsafe fn dispatch_atomic_width<O: AtomicOperation>(
    ptr: *mut u8,
    size: usize,
    operation: O,
) -> O::Output {
    match size {
        // SAFETY: F4.WidthAligned — The caller proves `ptr` valid and aligned for `size`; this arm binds that size to its only marker representation.
        1 => unsafe { operation.execute::<W1>(ptr) },
        // SAFETY: F4.WidthAligned — The caller proves `ptr` valid and aligned for `size`; this arm binds that size to its only marker representation.
        2 => unsafe { operation.execute::<W2>(ptr) },
        // SAFETY: F4.WidthAligned — The caller proves `ptr` valid and aligned for `size`; this arm binds that size to its only marker representation.
        4 => unsafe { operation.execute::<W4>(ptr) },
        // SAFETY: F4.WidthAligned — The caller proves `ptr` valid and aligned for `size`; this arm binds that size to its only marker representation.
        8 => unsafe { operation.execute::<W8>(ptr) },
        _ => panic!("Unsupported atomic size: {size}"),
    }
}

impl StandardAtomicAccess {
    /// Dynamic-width bridge for runtime APIs that carry CTS widths as `usize`.
    /// The marker-typed [`AtomicAccess`] methods are the only primitive access path.
    ///
    /// # Safety
    /// `ptr` must be valid, readable, and aligned for the supported `size`.
    pub unsafe fn load_atomic_sized(ptr: *const u8, size: usize, ordering: Ordering) -> u64 {
        // SAFETY: F4.WidthAligned — The caller proves `ptr` valid and aligned for `size`; the dispatcher binds it to the matching marker.
        unsafe { dispatch_atomic_width(ptr.cast_mut(), size, LoadOperation { ordering }) }
    }

    /// # Safety
    /// `ptr` must be valid, readable, and aligned for the supported `size`.
    pub unsafe fn load_atomic_sized_bytes(
        ptr: *const u8,
        size: usize,
        ordering: Ordering,
    ) -> Vec<u8> {
        // SAFETY: F4.WidthAligned — The caller proves `ptr` valid and aligned for `size`; the dispatcher binds it to the matching marker.
        unsafe { dispatch_atomic_width(ptr.cast_mut(), size, LoadBytesOperation { ordering }) }
    }

    /// # Safety
    /// `ptr` must be valid, writable, and aligned for the supported `size`.
    pub unsafe fn store_atomic_sized(ptr: *mut u8, size: usize, value: u64, ordering: Ordering) {
        // SAFETY: F4.WidthAligned — The caller proves `ptr` valid and aligned for `size`; the dispatcher binds it to the matching marker.
        unsafe { dispatch_atomic_width(ptr, size, StoreOperation { value, ordering }) }
    }

    /// # Safety
    /// `ptr` must be valid, writable, and aligned for the supported `value.len()`.
    pub unsafe fn store_atomic_sized_bytes(ptr: *mut u8, value: &[u8], ordering: Ordering) {
        // SAFETY: F4.WidthAligned — The caller proves `ptr` valid and aligned for `value.len()`; the dispatcher binds it to the matching marker.
        unsafe { dispatch_atomic_width(ptr, value.len(), StoreBytesOperation { value, ordering }) }
    }

    /// # Safety
    /// `ptr` must be valid, readable, writable, and aligned for the supported `size`.
    pub unsafe fn compare_exchange_atomic_sized(
        ptr: *mut u8,
        size: usize,
        expected: u64,
        new: u64,
        success: Ordering,
        failure: Ordering,
    ) -> Result<u64, u64> {
        // SAFETY: F4.WidthAligned — The caller proves `ptr` valid and aligned for `size`; the dispatcher binds it to the matching marker.
        unsafe {
            dispatch_atomic_width(
                ptr,
                size,
                CompareExchangeOperation {
                    expected,
                    new,
                    success,
                    failure,
                },
            )
        }
    }

    /// # Safety
    /// `ptr` must be valid, readable, writable, and aligned for the supported `size`.
    pub unsafe fn exchange_atomic_sized(
        ptr: *mut u8,
        size: usize,
        new: u64,
        ordering: Ordering,
    ) -> u64 {
        // SAFETY: F4.WidthAligned — The caller proves `ptr` valid and aligned for `size`; the dispatcher binds it to the matching marker.
        unsafe { dispatch_atomic_width(ptr, size, ExchangeOperation { new, ordering }) }
    }

    /// # Safety
    /// `ptr` must be valid, readable, writable, and aligned for the supported `size`.
    pub unsafe fn exchange_add_atomic_sized(
        ptr: *mut u8,
        size: usize,
        value: u64,
        ordering: Ordering,
    ) -> u64 {
        // SAFETY: F4.WidthAligned — The caller proves `ptr` valid and aligned for `size`; the dispatcher binds it to the matching marker.
        unsafe { dispatch_atomic_width(ptr, size, ExchangeAddOperation { value, ordering }) }
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
            // SAFETY: F10.RawMemoryAccessValid — The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
            unsafe { StandardAtomicAccess::load_atomic_sized_bytes(ptr, size, ordering) }
        } else {
            validate_atomic_access(ptr, false);
            let mut buf = vec![0u8; size];
            // SAFETY: F10.RawMemoryAccessValid — The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
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
            // SAFETY: F10.RawMemoryAccessValid — The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
            unsafe { StandardAtomicAccess::store_atomic_sized_bytes(ptr, value, ordering) };
        } else {
            validate_atomic_access(ptr as *const u8, false);
            // SAFETY: F10.RawMemoryAccessValid — The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
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

        // SAFETY: F10.RawMemoryAccessValid — `ptr` points into the test's live two-word backing allocation.
        unsafe { StandardAtomicAccess::store_atomic_sized(ptr, 1, 0xAA, Ordering::SeqCst) };
        // SAFETY: F10.RawMemoryAccessValid — `ptr` points into the test's live two-word backing allocation.
        let first = unsafe { StandardAtomicAccess::load_atomic_sized(ptr, 1, Ordering::SeqCst) };
        assert_eq!(first, 0xAA);
        assert_eq!(data[0] as u8, 0xAA);

        // SAFETY: F10.RawMemoryAccessValid — Offset two is within the test's live two-word backing allocation.
        let ptr2 = unsafe { ptr.add(2) };
        // SAFETY: F10.RawMemoryAccessValid — `ptr2` points into the test's live two-word backing allocation.
        unsafe { StandardAtomicAccess::store_atomic_sized(ptr2, 2, 0xBBCC, Ordering::SeqCst) };
        // SAFETY: F10.RawMemoryAccessValid — `ptr2` points into the test's live two-word backing allocation.
        let second = unsafe { StandardAtomicAccess::load_atomic_sized(ptr2, 2, Ordering::SeqCst) };
        assert_eq!(second, 0xBBCC);
        // On little-endian, 0xBBCC at offset 2 in u64:
        // [00 00 CC BB 00 00 00 00]
        assert_eq!((data[0] >> 16) as u16, 0xBBCC);

        // SAFETY: F10.RawMemoryAccessValid — Offset four is within the test's live two-word backing allocation.
        let ptr4 = unsafe { ptr.add(4) };
        // SAFETY: F10.RawMemoryAccessValid — `ptr4` points into the test's live two-word backing allocation.
        unsafe { StandardAtomicAccess::store_atomic_sized(ptr4, 4, 0xDEADBEEF, Ordering::SeqCst) };
        // SAFETY: F10.RawMemoryAccessValid — `ptr4` points into the test's live two-word backing allocation.
        let fourth = unsafe { StandardAtomicAccess::load_atomic_sized(ptr4, 4, Ordering::SeqCst) };
        assert_eq!(fourth, 0xDEADBEEF);
        assert_eq!((data[0] >> 32) as u32, 0xDEADBEEF);

        // SAFETY: F10.RawMemoryAccessValid — Offset eight is the start of the second word in the allocation.
        let ptr8 = unsafe { ptr.add(8) };
        // SAFETY: F10.RawMemoryAccessValid — `ptr8` points into the test's live two-word backing allocation.
        unsafe {
            StandardAtomicAccess::store_atomic_sized(ptr8, 8, 0x0123456789ABCDEF, Ordering::SeqCst)
        };
        // SAFETY: F10.RawMemoryAccessValid — `ptr8` points into the test's live two-word backing allocation.
        let eighth = unsafe { StandardAtomicAccess::load_atomic_sized(ptr8, 8, Ordering::SeqCst) };
        assert_eq!(eighth, 0x0123456789ABCDEF);
        assert_eq!(data[1], 0x0123456789ABCDEF);
    }

    #[test]
    fn typed_atomic_operations_cover_every_supported_width() {
        macro_rules! exercise_width {
            ($width:ty, $repr:ty, $initial:expr, $replacement:expr, $increment:expr) => {{
                let mut storage = 0u64;
                let ptr = std::ptr::from_mut(&mut storage).cast::<u8>();
                // SAFETY: F4.WidthAligned — A `u64` local supplies live storage aligned for each supported test width.
                unsafe {
                    <StandardAtomicAccess as AtomicAccess>::store_atomic::<$width>(
                        ptr,
                        $initial as $repr,
                        Ordering::SeqCst,
                    );
                }
                // SAFETY: F4.WidthAligned — The same live local remains aligned for this marker's representation.
                let loaded = unsafe {
                    <StandardAtomicAccess as AtomicAccess>::load_atomic::<$width>(
                        ptr,
                        Ordering::SeqCst,
                    )
                };
                assert_eq!(loaded, $initial as $repr);
                // SAFETY: F4.WidthAligned — The same live local remains aligned for this marker's representation.
                let compared = unsafe {
                    <StandardAtomicAccess as AtomicAccess>::compare_exchange_atomic::<$width>(
                        ptr,
                        $initial as $repr,
                        $replacement as $repr,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    )
                };
                assert_eq!(compared, Ok($initial as $repr));
                // SAFETY: F4.WidthAligned — The same live local remains aligned for this marker's representation.
                let mismatch = unsafe {
                    <StandardAtomicAccess as AtomicAccess>::compare_exchange_atomic::<$width>(
                        ptr,
                        $initial as $repr,
                        0 as $repr,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    )
                };
                assert_eq!(mismatch, Err($replacement as $repr));
                // SAFETY: F4.WidthAligned — The same live local remains aligned for this marker's representation.
                let exchanged = unsafe {
                    <StandardAtomicAccess as AtomicAccess>::exchange_atomic::<$width>(
                        ptr,
                        ($replacement + 1) as $repr,
                        Ordering::SeqCst,
                    )
                };
                assert_eq!(exchanged, $replacement as $repr);
                // SAFETY: F4.WidthAligned — The same live local remains aligned for this marker's representation.
                let added = unsafe {
                    <StandardAtomicAccess as AtomicAccess>::exchange_add_atomic::<$width>(
                        ptr,
                        $increment as $repr,
                        Ordering::SeqCst,
                    )
                };
                assert_eq!(added, ($replacement + 1) as $repr);
                // SAFETY: F4.WidthAligned — The same live local remains aligned for this marker's representation.
                let final_value = unsafe {
                    <StandardAtomicAccess as AtomicAccess>::load_atomic::<$width>(
                        ptr,
                        Ordering::SeqCst,
                    )
                };
                assert_eq!(final_value, ($replacement + 1 + $increment) as $repr);
            }};
        }

        exercise_width!(W1, u8, 10, 20, 3);
        exercise_width!(W2, u16, 100, 200, 30);
        exercise_width!(W4, u32, 1_000, 2_000, 300);
        exercise_width!(W8, u64, 10_000, 20_000, 3_000);
    }

    #[test]
    fn test_orderings() {
        let mut val = 0u64;
        let ptr = std::ptr::from_mut(&mut val).cast::<u8>();

        // Valid load orderings.
        for ord in [Ordering::Relaxed, Ordering::Acquire, Ordering::SeqCst] {
            // SAFETY: F4.WidthAligned — `ptr` points to the live local `u64` used by this test.
            unsafe { StandardAtomicAccess::load_atomic_sized(ptr, 8, ord) };
        }

        // Valid store orderings.
        for ord in [Ordering::Relaxed, Ordering::Release, Ordering::SeqCst] {
            // SAFETY: F4.WidthAligned — `ptr` points to the live local `u64` used by this test.
            unsafe { StandardAtomicAccess::store_atomic_sized(ptr, 8, 42, ord) };
        }
    }

    #[test]
    #[cfg(all(feature = "memory-validation", feature = "multithreading"))]
    #[should_panic(expected = "Invalid load ordering")]
    fn test_invalid_load_ordering() {
        let val = 0u64;
        let ptr = std::ptr::from_ref(&val).cast::<u8>();
        // SAFETY: F4.WidthAligned — `ptr` points to the live local `u64` used by this test.
        unsafe {
            StandardAtomicAccess::load_atomic_sized(ptr, 8, Ordering::Release);
        }
    }

    #[test]
    #[cfg(all(feature = "memory-validation", feature = "multithreading"))]
    #[should_panic(expected = "Invalid store ordering")]
    fn test_invalid_store_ordering() {
        let mut val = 0u64;
        let ptr = std::ptr::from_mut(&mut val).cast::<u8>();
        // SAFETY: F4.WidthAligned — `ptr` points to the live local `u64` used by this test.
        unsafe {
            StandardAtomicAccess::store_atomic_sized(ptr, 8, 42, Ordering::Acquire);
        }
    }

    #[test]
    #[cfg(feature = "memory-validation")]
    fn test_mixed_access_validation() {
        let mut val = 0u64;
        let ptr = std::ptr::from_mut(&mut val).cast::<u8>();
        // SAFETY: F4.WidthAligned — `ptr` points to the live local `u64` used by this test.
        unsafe {
            // This should not panic, but it will populate ATOMIC_LOCATIONS
            StandardAtomicAccess::store_atomic_sized(ptr, 8, 42, Ordering::SeqCst);
            // This trigger a warning (not a panic)
            validate_atomic_access(ptr, false);
        }
    }

    #[test]
    #[cfg(feature = "memory-validation")]
    #[should_panic(expected = "Alignment violation")]
    fn test_validate_alignment_rejects_misaligned_pointer() {
        let data = [0u8; 16];
        // SAFETY: F10.RawMemoryAccessValid — Offset one remains within the live sixteen-byte allocation.
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
        // SAFETY: F10.RawMemoryAccessValid — The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
        let misaligned_ptr = unsafe { ptr.add(1) };
        // SAFETY: F10.RawMemoryAccessValid — The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
        let loaded = unsafe { Atomic::load_field(misaligned_ptr, 8, Ordering::SeqCst) };
        assert_eq!(loaded, 0x0102030405060708u64.to_ne_bytes());
    }

    #[test]
    #[cfg(feature = "multithreading")]
    fn test_store_field_large_size_falls_back_in_mt() {
        // 3-byte store: not atomically expressible, falls back to memcpy.
        // Caller must hold an external lock; here we just verify the bytes are written.
        let mut data = [0u8; 8];
        // SAFETY: F10.RawMemoryAccessValid — The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
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
        // SAFETY: F10.RawMemoryAccessValid — The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
        let misaligned_ptr = unsafe { ptr.add(1) };
        // SAFETY: F10.RawMemoryAccessValid — The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
        let loaded = unsafe { Atomic::load_field(misaligned_ptr, 4, Ordering::SeqCst) };
        assert_eq!(loaded, vec![0x11, 0x22, 0x33, 0x44]);
    }

    #[test]
    #[cfg(not(feature = "multithreading"))]
    fn test_store_field_unsupported_size_falls_back_non_mt() {
        let mut data = [0u8; 8];
        // SAFETY: F10.RawMemoryAccessValid — The valid backing storage and this API's documented unsafe contract satisfy the pointer operation's preconditions.
        unsafe {
            Atomic::store_field(data.as_mut_ptr(), &[0xAA, 0xBB, 0xCC], Ordering::SeqCst);
        }
        assert_eq!(&data[..3], &[0xAA, 0xBB, 0xCC]);
    }
}
