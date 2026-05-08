//! Threading/monitor/interlocked intrinsic handlers and host traits.
//!
//! Atomic and Interlocked intrinsic handlers in this crate call the following
//! [`RawMemoryOps`] methods directly: [`RawMemoryOps::compare_exchange_atomic`],
//! [`RawMemoryOps::exchange_atomic`], [`RawMemoryOps::exchange_add_atomic`],
//! [`RawMemoryOps::load_atomic`], and [`RawMemoryOps::store_atomic`].
use dotnet_utils::{ArenaId, gc::GCHandle};
use dotnet_value::{StackValue, object::ObjectRef};
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
        + RawMemoryOps<'gc>
        + StackSlotWriteHost<'gc>;
}
