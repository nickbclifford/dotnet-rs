use dotnet_types::error::MemoryAccessError;
use dotnet_utils::{ArenaId, gc::GCHandle};
use dotnet_value::object::{HeapStorage, ObjectRef};
use std::{cell::RefCell, marker::PhantomData};

#[cfg(feature = "multithreading")]
use dotnet_value::pointer::ManagedPtr;
#[cfg(feature = "multithreading")]
use std::sync::LazyLock;

#[cfg(feature = "multithreading")]
use dotnet_utils::gc::GcLifetime;
#[cfg(feature = "multithreading")]
use dotnet_value::pointer::{CrossArenaOrigin, PointerOrigin};

#[derive(Copy, Clone)]
pub enum MemoryOwner<'gc> {
    Local(ObjectRef<'gc>),
    #[cfg(feature = "multithreading")]
    CrossArena(CrossArenaOrigin, GcLifetime<'gc>),
}

#[derive(Copy, Clone)]
pub struct HeapWriteTarget<'gc>(pub MemoryOwner<'gc>);

/// A deferred write-barrier entry. Without multithreading no cross-arena reference can exist,
/// so the buffer's entry type is uninhabited and the buffer stays permanently empty.
#[cfg(feature = "multithreading")]
pub type WbEntry = dotnet_utils::gc::CrossArenaRef;
#[cfg(not(feature = "multithreading"))]
pub type WbEntry = std::convert::Infallible;

thread_local! {
    pub(crate) static WB_LOCAL_BUF: RefCell<Vec<WbEntry>> = RefCell::new(Vec::with_capacity(128));
}

#[cfg(feature = "multithreading")]
const DEFAULT_WB_FLUSH_THRESHOLD: usize = 32;

#[cfg(feature = "multithreading")]
static WRITE_BARRIER_FLUSH_THRESHOLD: LazyLock<usize> = LazyLock::new(|| {
    std::env::var("DOTNET_WB_FLUSH_THRESHOLD")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_WB_FLUSH_THRESHOLD)
});

#[cfg(feature = "multithreading")]
fn flush_write_barrier_entries(buffer: &mut Vec<WbEntry>) {
    for reference in buffer.drain(..) {
        dotnet_utils::gc::record_cross_arena_ref(reference);
    }
}

#[cfg(not(feature = "multithreading"))]
fn flush_write_barrier_entries(buffer: &mut Vec<WbEntry>) {
    buffer.clear();
}

#[cfg(feature = "multithreading")]
pub(crate) fn maybe_flush_write_barrier_entries(buffer: &mut Vec<WbEntry>) {
    if buffer.len() >= *WRITE_BARRIER_FLUSH_THRESHOLD {
        flush_write_barrier_entries(buffer);
    }
}

/// Flushes the calling thread's deferred write-barrier buffer.
///
/// This is called at GC safepoint entry so all recorded cross-arena references
/// become visible before mark starts.
pub fn flush_write_barrier_buffer() {
    WB_LOCAL_BUF.with(|buf| {
        flush_write_barrier_entries(&mut buf.borrow_mut());
    });
}

/// RAII guard that flushes deferred write-barrier entries only during panic unwind.
///
/// This preserves panic safety (no leaked buffered references when a write path
/// unwinds) while allowing normal writes to batch until a threshold or safepoint.
pub(crate) struct WriteBarrierPanicFlushGuard;

impl Drop for WriteBarrierPanicFlushGuard {
    fn drop(&mut self) {
        if std::thread::panicking() {
            flush_write_barrier_buffer();
        }
    }
}

#[cfg(feature = "multithreading")]
pub struct WriteBarrierRecorder<'a, 'gc> {
    pub(crate) arena_id: ArenaId,
    pub(crate) buffer: &'a mut Vec<WbEntry>,
    _gc: PhantomData<&'gc ()>,
}

#[cfg(not(feature = "multithreading"))]
pub struct WriteBarrierRecorder<'a, 'gc> {
    _marker: PhantomData<(&'a (), &'gc ())>,
}

#[cfg(feature = "multithreading")]
impl<'a, 'gc> WriteBarrierRecorder<'a, 'gc> {
    pub fn new(arena_id: ArenaId, buffer: &'a mut Vec<WbEntry>) -> Self {
        Self {
            arena_id,
            buffer,
            _gc: PhantomData,
        }
    }

    pub fn record_ref(&mut self, target: ObjectRef<'gc>) {
        if self.arena_id == ArenaId::INVALID {
            return;
        }

        if let Some(h) = target.0 {
            // SAFETY: F1.GcHandleRooted — `h` is a live GC handle, so extracting its raw pointer preserves a valid
            // reference to the object for this synchronous recorder operation.
            let ptr = unsafe { h.as_ptr() };
            // SAFETY: F7.InitializationPublished — `ptr` came from the live handle above; reading immutable `owner_id` neither
            // moves nor mutates the object.
            let ref_tid = unsafe { (*ptr).owner_id() };
            if ref_tid != self.arena_id {
                // SAFETY: F1.GcHandleRooted; F11.ObjectPointerThreadSafe — `h` is a live handle to
                // an object of `ref_tid`'s registration; erased inert, no exposed provenance.
                let opaque = unsafe {
                    dotnet_utils::gc::OpaqueObjectPtr::from_raw(gc_arena::Gc::as_ptr(h).cast())
                };
                // Capture the owner's identity now, under a short lease, so the flush can reject
                // an arena that is unregistered and re-registered before harvest.
                if let Some(lease) = dotnet_utils::gc::try_acquire_lease(ref_tid) {
                    // SAFETY: F1.ArenaGenerationMatch — the lease pins `ref_tid`'s current registration, which owns `h`.
                    let reference =
                        unsafe { dotnet_utils::gc::CrossArenaRef::new(lease.identity(), opaque) };
                    drop(lease);
                    self.buffer.push(reference);
                    maybe_flush_write_barrier_entries(self.buffer);
                }
            }
        }
    }

    pub fn record_managed_ptr(&mut self, target: &ManagedPtr<'gc>) {
        if self.arena_id == ArenaId::INVALID {
            return;
        }

        match target.origin() {
            PointerOrigin::CrossArenaObjectRef(origin)
                if origin.identity().arena_id != self.arena_id =>
            {
                self.buffer.push(origin.cross_arena_ref());
                maybe_flush_write_barrier_entries(self.buffer);
            }
            PointerOrigin::Heap(r) => {
                self.record_ref(*r);
            }
            _ => {}
        }
    }
}

#[cfg(not(feature = "multithreading"))]
impl<'a, 'gc> WriteBarrierRecorder<'a, 'gc> {
    pub fn new(_arena_id: ArenaId, _buffer: &'a mut Vec<WbEntry>) -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<'gc> MemoryOwner<'gc> {
    #[cfg(feature = "multithreading")]
    pub fn cross_arena(gc: GCHandle<'gc>, origin: CrossArenaOrigin) -> Self {
        Self::CrossArena(origin, gc.lifetime())
    }

    pub fn owner_id(&self) -> ArenaId {
        match self {
            Self::Local(r) => {
                r.0.map(|h| {
                    // SAFETY: F1.GcHandleRooted — `h` is a live GC handle, so extracting its raw pointer preserves
                    // a valid reference to the object for this closure.
                    let ptr = unsafe { h.as_ptr() };
                    // SAFETY: F7.InitializationPublished — `ptr` came from the live handle above; reading immutable `owner_id`
                    // neither moves nor mutates its object.
                    unsafe { (*ptr).owner_id() }
                })
                // INVALID sentinel: unowned write; the recorder skips cross-arena tracking.
                .unwrap_or(ArenaId::INVALID)
            }
            #[cfg(feature = "multithreading")]
            Self::CrossArena(origin, _) => origin.identity().arena_id,
        }
    }

    /// A cross-arena lease covers `f`; the closure must not park at a safepoint.
    pub fn with_data<T>(&self, f: impl FnOnce(&[u8]) -> T) -> Result<T, MemoryAccessError> {
        match self {
            Self::Local(r) => Ok(r.with_data(f)),
            #[cfg(feature = "multithreading")]
            Self::CrossArena(origin, _) => {
                let _lease = dotnet_utils::gc::try_acquire_lease_for_identity(origin.identity())
                    .ok_or(MemoryAccessError::CrossArenaViolation)?;
                // SAFETY: F1.ArenaGenerationMatch; F1.GcHandleRooted — `_lease` is held for the origin's exact identity across `f`; rooting rests on the contract `CrossArenaOrigin::new` imposed when the origin was built, which this method cannot recheck.
                let p = unsafe { origin.object_ptr() };
                Ok(p.with_data(f))
            }
        }
    }

    /// A cross-arena lease covers `f`; the closure must not park at a safepoint.
    pub fn with_data_mut<T>(
        &self,
        gc: GCHandle<'gc>,
        f: impl FnOnce(&mut [u8]) -> T,
    ) -> Result<T, MemoryAccessError> {
        match self {
            Self::Local(r) => Ok(r.with_data_mut(gc, f)),
            #[cfg(feature = "multithreading")]
            Self::CrossArena(origin, _) => {
                let _lease = dotnet_utils::gc::try_acquire_lease_for_identity(origin.identity())
                    .ok_or(MemoryAccessError::CrossArenaViolation)?;
                // SAFETY: F1.ArenaGenerationMatch; F1.GcHandleRooted — `_lease` is held for the origin's exact identity across `f`; rooting rests on the `CrossArenaOrigin::new` contract, as in `with_data`.
                let p = unsafe { origin.object_ptr() };
                Ok(p.with_data_mut(gc, f))
            }
        }
    }

    /// A cross-arena lease covers `f`; the closure must not park at a safepoint.
    pub fn as_heap_storage<T>(
        &self,
        f: impl for<'a> FnOnce(&HeapStorage<'a>) -> T,
    ) -> Result<T, MemoryAccessError> {
        match self {
            Self::Local(r) => Ok(r.as_heap_storage(f)),
            #[cfg(feature = "multithreading")]
            Self::CrossArena(origin, _) => {
                let _lease = dotnet_utils::gc::try_acquire_lease_for_identity(origin.identity())
                    .ok_or(MemoryAccessError::CrossArenaViolation)?;
                // SAFETY: F1.ArenaGenerationMatch; F1.GcHandleRooted — `_lease` is held for the origin's exact identity across `f`; rooting rests on the `CrossArenaOrigin::new` contract, as in `with_data`.
                let p = unsafe { origin.object_ptr() };
                Ok(p.as_heap_storage(f))
            }
        }
    }
}

#[cfg(all(test, feature = "multithreading"))]
mod tests {
    use super::*;
    use dotnet_types::TypeDescription;
    use dotnet_utils::{ByteOffset, gc::ThreadSafeLock};
    use dotnet_value::{
        CLRString,
        object::{ObjectInner, ObjectPtr},
    };

    #[test]
    fn invalid_recorder_skips_cross_arena_managed_ptrs() {
        let target_id = ArenaId::new(900_001);
        let lock = Box::new(ThreadSafeLock::new(ObjectInner::new(
            HeapStorage::Str(CLRString::from("invalid-recorder-target")),
            target_id,
        )));
        let raw: *mut ThreadSafeLock<ObjectInner<'static>> = Box::into_raw(lock);
        // SAFETY: F10.RawAllocationOwnership — `raw` came from `Box::into_raw`, is non-null, and remains allocated until the
        // managed pointer and recorder have been dropped below.
        let ptr = unsafe { ObjectPtr::from_raw(raw) }.expect("boxed object pointer is non-null");
        dotnet_utils::gc::register_arena(
            target_id,
            dotnet_utils::sync::Arc::new(dotnet_utils::sync::AtomicBool::new(false)),
        );
        // SAFETY: F1.ArenaGenerationMatch; F10.RawAllocationOwnership — the test owns this live box and original registration.
        let managed = unsafe {
            ManagedPtr::new_cross_arena(
                None,
                TypeDescription::NULL,
                ptr,
                &dotnet_utils::gc::try_acquire_lease(target_id).unwrap(),
                ByteOffset::ZERO,
            )
        };
        let mut buffer = Vec::new();

        {
            let mut recorder = WriteBarrierRecorder::new(ArenaId::INVALID, &mut buffer);
            recorder.record_managed_ptr(&managed);
        }

        assert!(
            buffer.is_empty(),
            "an unowned recorder must skip every cross-arena reference kind"
        );
        drop(managed);
        dotnet_utils::gc::unregister_arena(target_id);
        // SAFETY: F10.RawAllocationOwnership — `raw` was produced by `Box::into_raw` above, no aliases are used after this
        // point, and reconstructing the box releases the allocation exactly once.
        unsafe {
            drop(Box::from_raw(raw));
        }
    }
}
