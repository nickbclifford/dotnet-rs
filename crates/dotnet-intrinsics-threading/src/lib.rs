//! Threading/monitor/interlocked intrinsic handlers and host traits.
//!
//! Inventory mapping for this host surface is tracked in
//! `docs/p3_s1_trait_inventory.md` (P3.S1).
use dotnet_types::error::{CompareExchangeError, MemoryAccessError};
use dotnet_utils::{ArenaId, ByteOffset, gc::GCHandle, sync::Ordering};
use dotnet_value::{StackValue, object::ObjectRef, pointer::PointerOrigin};
use dotnet_vm_ops::ops::{RawMemoryOps, ThreadingIntrinsicHost as VmThreadingIntrinsicHost};
use std::time::Instant;

pub(crate) mod atomic_dispatch;
pub mod interlocked;
pub mod monitor;
pub mod thread;
pub mod volatile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorLockResult {
    Success,
    Timeout,
    Yield,
}

pub trait MonitorHost<'gc> {
    type SyncBlock: Clone;

    fn monitor_get_sync_block_for_object(&self, object: ObjectRef<'gc>) -> Option<Self::SyncBlock>;

    fn monitor_get_or_create_sync_block_for_object(
        &self,
        object: ObjectRef<'gc>,
        gc: GCHandle<'gc>,
    ) -> Self::SyncBlock;

    fn monitor_try_enter(&self, sync_block: &Self::SyncBlock, thread_id: ArenaId) -> bool;

    fn monitor_enter_safe(
        &self,
        sync_block: &Self::SyncBlock,
        thread_id: ArenaId,
    ) -> MonitorLockResult;

    fn monitor_enter_with_timeout_safe(
        &self,
        sync_block: &Self::SyncBlock,
        thread_id: ArenaId,
        deadline: Instant,
    ) -> MonitorLockResult;

    fn monitor_exit(&self, sync_block: &Self::SyncBlock, thread_id: ArenaId) -> bool;
}

pub trait AtomicMemoryHost<'gc>: RawMemoryOps<'gc> {
    /// # Safety
    ///
    /// Same safety contract as [`RawMemoryOps::compare_exchange_atomic`].
    #[allow(clippy::too_many_arguments)]
    unsafe fn threading_compare_exchange_atomic(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        expected: u64,
        new: u64,
        size: usize,
        success: Ordering,
        failure: Ordering,
    ) -> Result<u64, CompareExchangeError> {
        unsafe {
            RawMemoryOps::compare_exchange_atomic(
                self, origin, offset, expected, new, size, success, failure,
            )
        }
    }

    /// # Safety
    ///
    /// Same safety contract as [`RawMemoryOps::exchange_atomic`].
    unsafe fn threading_exchange_atomic(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        value: u64,
        size: usize,
        ordering: Ordering,
    ) -> Result<u64, MemoryAccessError> {
        unsafe { RawMemoryOps::exchange_atomic(self, origin, offset, value, size, ordering) }
    }

    /// # Safety
    ///
    /// Same safety contract as [`RawMemoryOps::exchange_add_atomic`].
    unsafe fn threading_exchange_add_atomic(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        value: u64,
        size: usize,
        ordering: Ordering,
    ) -> Result<u64, MemoryAccessError> {
        unsafe { RawMemoryOps::exchange_add_atomic(self, origin, offset, value, size, ordering) }
    }

    /// # Safety
    ///
    /// Same safety contract as [`RawMemoryOps::load_atomic`].
    unsafe fn threading_load_atomic(
        &self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        size: usize,
        ordering: Ordering,
    ) -> Result<u64, MemoryAccessError> {
        unsafe { RawMemoryOps::load_atomic(self, origin, offset, size, ordering) }
    }

    /// # Safety
    ///
    /// Same safety contract as [`RawMemoryOps::store_atomic`].
    unsafe fn threading_store_atomic(
        &mut self,
        origin: PointerOrigin<'gc>,
        offset: ByteOffset,
        value: u64,
        size: usize,
        ordering: Ordering,
    ) -> Result<(), MemoryAccessError> {
        unsafe { RawMemoryOps::store_atomic(self, origin, offset, value, size, ordering) }
    }
}

impl<'gc, T: RawMemoryOps<'gc> + ?Sized> AtomicMemoryHost<'gc> for T {}

pub trait StackSlotWriteHost<'gc> {
    fn threading_set_stack_slot(
        &mut self,
        index: dotnet_utils::StackSlotIndex,
        value: StackValue<'gc>,
    );
}

dotnet_vm_ops::trait_alias! {
    pub trait ThreadingIntrinsicHost<'gc> =
        VmThreadingIntrinsicHost<'gc>
        + MonitorHost<'gc>
        + AtomicMemoryHost<'gc>
        + StackSlotWriteHost<'gc>;
}
