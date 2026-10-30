//! Frame-owned, non-moving allocations and their per-frame and per-thread budgets.

use std::{cell::Cell, ptr::NonNull};

use dotnet_utils::raw_block::{RawBlock, RawBlockError};

thread_local! {
    static THREAD_BYTES: Cell<usize> = const { Cell::new(0) };
}

/// Limits for a single request and for cumulative frame and thread allocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameBudget {
    pub max_request: usize,
    pub per_frame: usize,
    pub per_thread: usize,
}

impl FrameBudget {
    /// The standard localloc limits: 128 MiB per request and frame, 512 MiB per thread.
    pub const DEFAULT: Self = Self {
        max_request: 0x800_0000,
        per_frame: 0x800_0000,
        per_thread: 0x2000_0000,
    };

    /// Check the requested payload size, before accounting for the minimum charge.
    pub fn check_request(&self, size: usize) -> Result<(), FrameAllocationError> {
        if size > self.max_request {
            return Err(FrameAllocationError::RequestTooLarge);
        }
        Ok(())
    }
}

impl Default for FrameBudget {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A rejected localloc request or failure to obtain host storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameAllocationError {
    NegativeSize,
    RequestTooLarge,
    FrameBudgetExceeded,
    ThreadBudgetExceeded,
    HostAllocationFailed(RawBlockError),
}

/// Raw blocks retained for the lifetime of a single method frame.
#[derive(Debug, Default)]
pub struct FrameAllocations {
    blocks: Vec<RawBlock>,
    used: usize,
}

impl FrameAllocations {
    /// Convert a signed guest request without accepting negative sizes.
    pub fn request_size(size: isize) -> Result<usize, FrameAllocationError> {
        usize::try_from(size).map_err(|_| FrameAllocationError::NegativeSize)
    }

    /// Allocate under the standard limits.
    pub fn allocate(&mut self, size: usize) -> Result<NonNull<u8>, FrameAllocationError> {
        self.allocate_within(size, &FrameBudget::DEFAULT)
    }

    /// Allocate a zero-filled, at least 16-byte-aligned block under the given limits.
    ///
    /// Even a zero-sized payload has a distinct live allocation. Accounting charges
    /// at least 64 bytes per block, independently of the requested payload size.
    pub fn allocate_within(
        &mut self,
        size: usize,
        budget: &FrameBudget,
    ) -> Result<NonNull<u8>, FrameAllocationError> {
        budget.check_request(size)?;
        // RawBlock's layout size is max(size, 1), without alignment padding.
        let charge = size.max(64);
        let frame_bytes = self
            .used
            .checked_add(charge)
            .filter(|&total| total <= budget.per_frame)
            .ok_or(FrameAllocationError::FrameBudgetExceeded)?;
        let thread_bytes = THREAD_BYTES.with(|bytes| {
            bytes
                .get()
                .checked_add(charge)
                .filter(|&total| total <= budget.per_thread)
                .ok_or(FrameAllocationError::ThreadBudgetExceeded)
        })?;

        let block =
            RawBlock::zeroed(size, 16).map_err(FrameAllocationError::HostAllocationFailed)?;
        debug_assert_eq!(charge, block.accounted_bytes());
        // Reserve before updating either gauge so even metadata allocation failure
        // drops the newly created block and leaves the logical state unchanged.
        self.blocks.try_reserve(1).map_err(|_| {
            FrameAllocationError::HostAllocationFailed(RawBlockError::AllocationFailed)
        })?;
        let base = block.base();
        self.blocks.push(block);
        self.used = frame_bytes;
        THREAD_BYTES.with(|bytes| bytes.set(thread_bytes));
        Ok(base)
    }

    /// Release all blocks and return their charges to the current thread.
    pub fn release_all(&mut self) {
        self.blocks.clear();
        let used = std::mem::take(&mut self.used);
        // A frame may be dropped during thread-local destruction. In that case
        // the gauge is already unavailable and cannot be observed again.
        let _ = THREAD_BYTES.try_with(|bytes| bytes.set(bytes.get() - used));
    }

    /// Number of blocks retained by this frame.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Whether this frame currently retains no blocks.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }
}

impl Drop for FrameAllocations {
    fn drop(&mut self) {
        self.release_all();
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameAllocationError as Error, FrameAllocations, FrameBudget, THREAD_BYTES};
    use dotnet_utils::raw_block::{RawBlockError, live_blocks};

    fn bytes() -> usize {
        THREAD_BYTES.with(|value| value.get())
    }

    fn budget(max_request: usize, per_frame: usize, per_thread: usize) -> FrameBudget {
        FrameBudget {
            max_request,
            per_frame,
            per_thread,
        }
    }

    #[test]
    fn signed_and_request_limits() {
        assert_eq!(FrameAllocations::request_size(-1), Err(Error::NegativeSize));
        assert_eq!(
            FrameAllocations::request_size(isize::MIN),
            Err(Error::NegativeSize)
        );
        let maximum = FrameAllocations::request_size(isize::MAX).unwrap();
        assert_eq!(maximum, isize::MAX as usize);
        let mut allocations = FrameAllocations::default();
        assert_eq!(allocations.allocate(maximum), Err(Error::RequestTooLarge));
        let limits = budget(65, 256, 256);
        assert!(allocations.allocate_within(65, &limits).is_ok());
        assert_eq!(
            allocations.allocate_within(66, &limits),
            Err(Error::RequestTooLarge)
        );
        assert_eq!(allocations.len(), 1);
    }

    #[test]
    fn frame_budget_and_failure_leave_state_unchanged() {
        let baseline = bytes();
        let blocks = live_blocks();
        let limits = budget(128, 129, baseline + 512);
        let mut allocations = FrameAllocations::default();
        allocations.allocate_within(64, &limits).unwrap();
        let one_short = budget(128, 128, baseline + 512);
        assert_eq!(
            allocations.allocate_within(65, &one_short),
            Err(Error::FrameBudgetExceeded)
        );
        assert_eq!(allocations.len(), 1);
        assert_eq!(allocations.used, 64);
        assert_eq!(bytes(), baseline + 64);
        allocations.allocate_within(65, &limits).unwrap();
        assert_eq!(allocations.used, 129);
        assert_eq!(bytes(), baseline + 129);
        assert_eq!(
            allocations.allocate_within(1, &limits),
            Err(Error::FrameBudgetExceeded)
        );
        assert_eq!(allocations.len(), 2);
        assert_eq!(allocations.used, 129);
        assert_eq!(bytes(), baseline + 129);
        assert_eq!(live_blocks(), blocks + 2);
    }

    #[test]
    fn thread_budget_is_shared_and_restored_on_drop() {
        let baseline = bytes();
        let blocks = live_blocks();
        let limits = budget(128, 256, baseline + 129);
        let mut first = FrameAllocations::default();
        first.allocate_within(64, &limits).unwrap();
        {
            let mut second = FrameAllocations::default();
            let one_short = budget(128, 256, baseline + 128);
            assert_eq!(
                second.allocate_within(65, &one_short),
                Err(Error::ThreadBudgetExceeded)
            );
            assert!(second.is_empty());
            assert_eq!(bytes(), baseline + 64);
            second.allocate_within(65, &limits).unwrap();
            assert_eq!(bytes(), baseline + 129);
            assert_eq!(
                first.allocate_within(1, &limits),
                Err(Error::ThreadBudgetExceeded)
            );
            assert_eq!(first.len(), 1);
            assert_eq!(first.used, 64);
            assert_eq!(live_blocks(), blocks + 2);
        }
        assert_eq!(bytes(), baseline + 64);
        first.allocate_within(65, &limits).unwrap();
        drop(first);
        assert_eq!(bytes(), baseline);
        assert_eq!(live_blocks(), blocks);
    }

    #[test]
    fn zero_size_blocks_are_distinct_and_charged() {
        let baseline = bytes();
        let mut allocations = FrameAllocations::default();
        let limits = budget(0, 128, baseline + 128);
        let first = allocations.allocate_within(0, &limits).unwrap();
        let second = allocations.allocate_within(0, &limits).unwrap();
        assert_ne!(first, second);
        assert_eq!(first.as_ptr().addr() % 16, 0);
        assert_eq!(second.as_ptr().addr() % 16, 0);
        assert_eq!(allocations.used, 128);
        assert_eq!(bytes(), baseline + 128);
        assert_eq!(
            allocations.allocate_within(0, &limits),
            Err(Error::FrameBudgetExceeded)
        );
    }

    #[test]
    fn allocation_order_and_markers_survive_growth() {
        let baseline = bytes();
        let mut allocations = FrameAllocations::default();
        let limits = budget(64, 64 * 40, baseline + 64 * 40);
        let mut pointers = Vec::new();
        for marker in 0..40u8 {
            let pointer = allocations.allocate_within(1, &limits).unwrap();
            // SAFETY: F10.RawMemoryAccessValid — the pointer addresses the first
            // byte of a live one-byte block owned by allocations.
            unsafe { pointer.as_ptr().write(marker) };
            pointers.push(pointer);
        }
        for (index, pointer) in pointers.iter().enumerate() {
            assert_eq!(*pointer, allocations.blocks[index].base());
            // SAFETY: F10.RawMemoryAccessValid — each pointer still addresses
            // the initialized byte of its live block in allocations.
            assert_eq!(unsafe { pointer.as_ptr().read() }, index as u8);
        }
    }

    #[test]
    fn release_all_is_idempotent_and_reusable() {
        let baseline = bytes();
        let blocks = live_blocks();
        let mut allocations = FrameAllocations::default();
        allocations.allocate(1).unwrap();
        allocations.allocate(0).unwrap();
        assert_eq!(live_blocks(), blocks + 2);
        allocations.release_all();
        assert!(allocations.is_empty());
        assert_eq!(allocations.used, 0);
        assert_eq!(bytes(), baseline);
        assert_eq!(live_blocks(), blocks);
        allocations.release_all();
        assert_eq!(bytes(), baseline);
        allocations.allocate(1).unwrap();
        drop(allocations);
        assert_eq!(bytes(), baseline);
        assert_eq!(live_blocks(), blocks);
    }

    #[test]
    fn overflow_and_invalid_layout_leave_state_unchanged() {
        let baseline = bytes();
        let blocks = live_blocks();
        let mut allocations = FrameAllocations::default();
        let unlimited = budget(usize::MAX, usize::MAX, usize::MAX);
        allocations.used = usize::MAX;
        assert_eq!(
            allocations.allocate_within(1, &unlimited),
            Err(Error::FrameBudgetExceeded)
        );
        allocations.used = 0;
        THREAD_BYTES.with(|value| value.set(usize::MAX));
        assert_eq!(
            allocations.allocate_within(1, &unlimited),
            Err(Error::ThreadBudgetExceeded)
        );
        THREAD_BYTES.with(|value| value.set(baseline));
        assert_eq!(
            allocations.allocate_within(isize::MAX as usize, &unlimited),
            Err(Error::HostAllocationFailed(RawBlockError::LayoutOverflow))
        );
        assert_eq!(allocations.len(), 0);
        assert_eq!(allocations.used, 0);
        assert_eq!(bytes(), baseline);
        assert_eq!(live_blocks(), blocks);
    }

    #[test]
    fn frame_blocks_survive_suspension_and_restoration() {
        use crate::{BasePointer, FrameStack, MethodInfo};
        use dotnet_types::{
            TypeDescription, generics::GenericLookup, members::MethodDescription,
            resolution::ResolutionS,
        };
        use dotnetdll::{binary::signature::kinds::CallingConvention, prelude::*};

        static SIGNATURE: ManagedMethod<MethodType> = ManagedMethod {
            instance: true,
            explicit_this: false,
            calling_convention: CallingConvention::Default,
            parameters: Vec::new(),
            return_type: ReturnType(Vec::new(), None),
            varargs: None,
        };
        let method = MethodInfo {
            signature: &SIGNATURE,
            locals: &[],
            max_stack: 0,
            exceptions: Vec::new(),
            instructions: &[],
            // These sentinels suffice: this test moves frames without executing metadata.
            source: MethodDescription::new(
                TypeDescription::NULL,
                GenericLookup::default(),
                ResolutionS::NULL,
                MethodMemberIndex::Method(0),
            ),
            is_cctor: false,
        };
        let baseline = live_blocks();
        let baseline_bytes = bytes();
        let mut frames = FrameStack::new();
        for _ in 0..2 {
            frames.push_frame(
                BasePointer::default(),
                method.clone(),
                Default::default(),
                Default::default(),
            );
        }
        let pointer = frames.frames[1].state.allocations.allocate(1).unwrap();
        // SAFETY: F10.RawMemoryAccessValid — the frame owns this live one-byte block.
        unsafe { pointer.as_ptr().write(0x5a) };

        frames.suspend_above(0);
        assert_eq!(frames.frames.len(), 1);
        assert_eq!(frames.suspended_frames.len(), 1);
        assert_eq!(frames.suspended_frames[0].state.allocations.len(), 1);
        assert_eq!(live_blocks(), baseline + 1);
        frames.frames[0].state.allocations.allocate(4096).unwrap();
        frames.restore_suspended();
        assert!(frames.suspended_frames.is_empty());
        assert_eq!(frames.frames.len(), 2);
        assert_eq!(frames.frames[1].state.allocations.len(), 1);
        // SAFETY: F10.RawMemoryAccessValid — the restored frame still owns the
        // initialized byte; moving frames did not move or release its allocation.
        assert_eq!(unsafe { pointer.as_ptr().read() }, 0x5a);
        assert_eq!(live_blocks(), baseline + 2);
        frames.clear();
        assert_eq!(live_blocks(), baseline);
        assert_eq!(bytes(), baseline_bytes);
    }
}
