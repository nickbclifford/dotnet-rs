//! Non-resizing, zero-initialised raw allocation blocks.
//!
//! Pointers are derived from the retained allocation base without creating
//! references to the payload. The caller must ensure that any use of a pointer
//! remains within the block's lifetime and respects its payload bounds.

use std::{
    alloc::{Layout, alloc_zeroed, dealloc},
    cell::Cell,
    ptr::NonNull,
};

use crate::atomic::remove_atomic_locations_in_range;

thread_local! {
    static LIVE_BLOCKS: Cell<usize> = const { Cell::new(0) };
}

/// Number of live blocks owned by the current thread.
pub fn live_blocks() -> usize {
    LIVE_BLOCKS.with(Cell::get)
}

/// Failure to construct a raw allocation block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawBlockError {
    /// The requested alignment is not a nonzero power of two.
    InvalidAlignment,
    /// The requested size and alignment cannot be represented as a layout.
    LayoutOverflow,
    /// The allocator returned null.
    AllocationFailed,
}

/// An exclusively owned, non-resizing allocation, separate from its payload extent.
#[derive(Debug)]
pub struct RawBlock {
    base: NonNull<u8>,
    payload_len: usize,
    layout: Layout,
}

impl RawBlock {
    /// Allocate a zero-filled block with at least 16-byte alignment.
    ///
    /// A zero-length payload still has a real one-byte allocation.
    pub fn zeroed(payload_len: usize, align: usize) -> Result<Self, RawBlockError> {
        // SAFETY: F10.RawAllocationOwnership — `alloc_zeroed` returns null or a
        // zeroed allocation owned by the global allocator with the given layout.
        unsafe { Self::zeroed_with(payload_len, align, alloc_zeroed) }
    }

    /// Construct a block using an injectable allocation function.
    ///
    /// # Safety
    ///
    /// For the supplied nonzero layout, `alloc_fn` must return null or a
    /// zeroed allocation that is uniquely owned and may be freed using the
    /// global allocator's `dealloc` with that same layout.
    pub(crate) unsafe fn zeroed_with(
        payload_len: usize,
        align: usize,
        alloc_fn: unsafe fn(Layout) -> *mut u8,
    ) -> Result<Self, RawBlockError> {
        if !align.is_power_of_two() {
            return Err(RawBlockError::InvalidAlignment);
        }
        let layout = Layout::from_size_align(payload_len.max(1), align.max(16))
            .map_err(|_| RawBlockError::LayoutOverflow)?;
        // SAFETY: F10.RawMemoryAccessValid — the layout is valid and nonzero;
        // the caller guarantees the allocation function's contract.
        let base = unsafe { alloc_fn(layout) };
        let base = NonNull::new(base).ok_or(RawBlockError::AllocationFailed)?;
        LIVE_BLOCKS.with(|count| count.set(count.get() + 1));
        Ok(Self {
            base,
            payload_len,
            layout,
        })
    }

    /// Return the retained allocation base.
    pub fn base(&self) -> NonNull<u8> {
        self.base
    }

    /// Return the accessible payload extent in bytes.
    pub fn payload_len(&self) -> usize {
        self.payload_len
    }

    /// Return the allocation layout, including the one-byte zero-size minimum.
    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// Return the byte charge used for allocation budgets.
    pub fn accounted_bytes(&self) -> usize {
        self.layout.size().max(64)
    }

    /// Derive a pointer at an offset within or one past the payload.
    pub fn ptr_at(&self, offset: usize) -> Option<NonNull<u8>> {
        if offset > self.payload_len {
            return None;
        }
        // SAFETY: F10.RawMemoryAccessValid — the offset is at most the payload
        // length, which is at most the size of this live allocation.
        Some(unsafe { self.base.add(offset) })
    }
}

impl Drop for RawBlock {
    fn drop(&mut self) {
        remove_atomic_locations_in_range(self.base.as_ptr(), self.layout.size());
        // SAFETY: F10.RawAllocationOwnership — this block uniquely owns the
        // allocation obtained with this layout, and it is freed exactly once.
        unsafe { dealloc(self.base.as_ptr(), self.layout) };
        LIVE_BLOCKS.with(|count| count.set(count.get() - 1));
    }
}

#[cfg(test)]
mod tests {
    use super::{RawBlock, RawBlockError, live_blocks};
    use std::{panic::catch_unwind, ptr};

    static_assertions::assert_not_impl_any!(RawBlock: Clone, Send, Sync);

    #[test]
    fn requested_alignment_is_honoured_for_payload_sizes() {
        let payload_sizes = [0, 1, 15, 16, 17, 4096, 4097];
        let requested_alignments = [1, 8, 16, 32, 64, 4096];

        for payload_len in payload_sizes {
            for requested_align in requested_alignments {
                let block = RawBlock::zeroed(payload_len, requested_align).unwrap();
                let expected_align = requested_align.max(16);

                assert_eq!(block.layout().align(), expected_align);
                assert_eq!(block.base().as_ptr().addr() % expected_align, 0);
            }
        }
    }

    #[test]
    fn newly_allocated_payload_is_zeroed() {
        let block = RawBlock::zeroed(4097, 16).unwrap();

        for offset in 0..block.payload_len() {
            let byte = block.ptr_at(offset).unwrap();
            // SAFETY: F10.RawMemoryAccessValid — `offset` is within the live,
            // zero-initialised payload and `ptr_at` derives its bounded pointer.
            let value = unsafe { byte.as_ptr().read() };
            assert_eq!(value, 0, "byte {offset} was not zero-initialised");
        }
    }

    #[test]
    fn payload_layout_accounting_and_pointer_bounds_are_distinct() {
        for payload_len in [0, 1, 63, 64, 65] {
            let block = RawBlock::zeroed(payload_len, 16).unwrap();

            assert_eq!(block.payload_len(), payload_len);
            assert_eq!(block.layout().size(), payload_len.max(1));
            assert_eq!(
                block.accounted_bytes(),
                block.layout().size().max(64),
                "D6 charges at least 64 bytes per block"
            );
            assert_eq!(block.ptr_at(0), Some(block.base()));
            assert!(block.ptr_at(payload_len).is_some());
            assert!(block.ptr_at(payload_len + 1).is_none());
        }
    }

    #[test]
    fn earlier_blocks_remain_usable_after_allocating_sixty_four() {
        let mut blocks = Vec::with_capacity(64);
        for index in 0..64 {
            let payload_len = if index == 0 {
                1024 * 1024
            } else {
                (index + 1) * 17
            };
            blocks.push(RawBlock::zeroed(payload_len, 16).unwrap());
        }

        for (index, block) in blocks.iter().enumerate() {
            let first_value = index as u8;
            let last_value = !(index as u8);
            let first = block.ptr_at(0).unwrap();
            let last = block.ptr_at(block.payload_len() - 1).unwrap();

            // SAFETY: F10.RawMemoryAccessValid — `first` is the first byte of
            // this live, nonempty payload and is written exactly once here.
            unsafe { first.as_ptr().write(first_value) };
            // SAFETY: F10.RawMemoryAccessValid — `last` is the last byte of
            // this live, nonempty payload and is written exactly once here.
            unsafe { last.as_ptr().write(last_value) };
        }

        for (index, block) in blocks.iter().enumerate() {
            let first = block.ptr_at(0).unwrap();
            let last = block.ptr_at(block.payload_len() - 1).unwrap();

            // SAFETY: F10.RawMemoryAccessValid — `first` points into this live
            // payload at offset zero, which was initialized above.
            let first_value = unsafe { first.as_ptr().read() };
            // SAFETY: F10.RawMemoryAccessValid — `last` points to the final
            // byte of this live payload, which was initialized above.
            let last_value = unsafe { last.as_ptr().read() };

            assert_eq!(first_value, index as u8);
            assert_eq!(last_value, !(index as u8));
        }
    }

    #[test]
    fn invalid_alignments_and_layout_overflows_do_not_change_live_count() {
        let baseline = live_blocks();

        for alignment in [0, 24] {
            assert_eq!(
                RawBlock::zeroed(1, alignment).unwrap_err(),
                RawBlockError::InvalidAlignment
            );
            assert_eq!(live_blocks(), baseline);
        }

        for payload_len in [usize::MAX, isize::MAX as usize] {
            assert_eq!(
                RawBlock::zeroed(payload_len, 16).unwrap_err(),
                RawBlockError::LayoutOverflow
            );
            assert_eq!(live_blocks(), baseline);
        }

        unsafe fn null_allocator(_: std::alloc::Layout) -> *mut u8 {
            ptr::null_mut()
        }

        // SAFETY: F10.RawAllocationOwnership — this injected allocator always
        // returns null, so it creates no allocation for RawBlock to own.
        let result = unsafe { RawBlock::zeroed_with(1, 16, null_allocator) };
        assert!(matches!(result, Err(RawBlockError::AllocationFailed)));
        assert_eq!(live_blocks(), baseline);
    }

    #[test]
    fn live_count_returns_to_baseline_after_drop() {
        let baseline = live_blocks();
        let block = RawBlock::zeroed(32, 16).unwrap();
        assert_eq!(live_blocks(), baseline + 1);

        drop(block);
        assert_eq!(live_blocks(), baseline);
    }

    #[test]
    fn live_count_returns_to_baseline_after_caught_panic() {
        let baseline = live_blocks();
        let result = catch_unwind(|| {
            let _block = RawBlock::zeroed(32, 16).unwrap();
            assert_eq!(live_blocks(), baseline + 1);
            panic!("exercise RawBlock drop during unwinding");
        });

        assert!(result.is_err());
        assert_eq!(live_blocks(), baseline);
    }
}
