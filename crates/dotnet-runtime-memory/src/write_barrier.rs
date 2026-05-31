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
use dotnet_value::{object::ObjectPtr, pointer::PointerOrigin};

#[derive(Copy, Clone)]
pub enum MemoryOwner<'gc> {
    Local(ObjectRef<'gc>),
    #[cfg(feature = "multithreading")]
    CrossArena(ObjectPtr, ArenaId, GcLifetime<'gc>),
}

#[derive(Copy, Clone)]
pub struct HeapWriteTarget<'gc>(pub MemoryOwner<'gc>);

thread_local! {
    pub(crate) static WB_LOCAL_BUF: RefCell<Vec<(ArenaId, usize)>> = RefCell::new(Vec::with_capacity(128));
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
fn flush_write_barrier_entries(buffer: &mut Vec<(ArenaId, usize)>) {
    for (tid, ptr) in buffer.drain(..) {
        dotnet_utils::gc::record_cross_arena_ref(tid, ptr);
    }
}

#[cfg(not(feature = "multithreading"))]
fn flush_write_barrier_entries(buffer: &mut Vec<(ArenaId, usize)>) {
    buffer.clear();
}

#[cfg(feature = "multithreading")]
pub(crate) fn maybe_flush_write_barrier_entries(buffer: &mut Vec<(ArenaId, usize)>) {
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
    pub(crate) buffer: &'a mut Vec<(ArenaId, usize)>,
    _gc: PhantomData<&'gc ()>,
}

#[cfg(not(feature = "multithreading"))]
pub struct WriteBarrierRecorder<'a, 'gc> {
    _marker: PhantomData<(&'a (), &'gc ())>,
}

#[cfg(feature = "multithreading")]
impl<'a, 'gc> WriteBarrierRecorder<'a, 'gc> {
    pub fn new(arena_id: ArenaId, buffer: &'a mut Vec<(ArenaId, usize)>) -> Self {
        Self {
            arena_id,
            buffer,
            _gc: PhantomData,
        }
    }

    pub fn record_ref(&mut self, target: ObjectRef<'gc>) {
        if let Some(h) = target.0 {
            // SAFETY: `h` is a live `Gc` handle; reading immutable `owner_id`
            // does not move or mutate the object.
            let ref_tid = unsafe { (*h.as_ptr()).owner_id() };
            if ref_tid != self.arena_id {
                self.buffer
                    .push((ref_tid, gc_arena::Gc::as_ptr(h) as usize));
                maybe_flush_write_barrier_entries(self.buffer);
            }
        }
    }

    pub fn record_managed_ptr(&mut self, target: &ManagedPtr<'gc>) {
        match target.origin() {
            PointerOrigin::CrossArenaObjectRef(p, ref_tid) if *ref_tid != self.arena_id => {
                self.buffer.push((*ref_tid, p.as_ptr() as usize));
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
    pub fn new(_arena_id: ArenaId, _buffer: &'a mut Vec<(ArenaId, usize)>) -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<'gc> MemoryOwner<'gc> {
    #[cfg(feature = "multithreading")]
    pub fn cross_arena(gc: GCHandle<'gc>, ptr: ObjectPtr, tid: ArenaId) -> Self {
        Self::CrossArena(ptr, tid, gc.lifetime())
    }

    pub fn owner_id(&self) -> ArenaId {
        match self {
            Self::Local(r) => {
                // SAFETY: `h` is a live `Gc` handle; reading immutable `owner_id`
                // does not move or mutate the object.
                r.0.map(|h| unsafe { (*h.as_ptr()).owner_id() })
                    .unwrap_or(ArenaId(0))
            }
            #[cfg(feature = "multithreading")]
            Self::CrossArena(_, tid, _) => *tid,
        }
    }

    pub fn with_data<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        match self {
            Self::Local(r) => r.with_data(f),
            #[cfg(feature = "multithreading")]
            Self::CrossArena(p, _, _) => p.with_data(f),
        }
    }

    pub fn with_data_mut<T>(&self, gc: GCHandle<'gc>, f: impl FnOnce(&mut [u8]) -> T) -> T {
        match self {
            Self::Local(r) => r.with_data_mut(gc, f),
            #[cfg(feature = "multithreading")]
            Self::CrossArena(p, _, _) => p.with_data_mut(gc, f),
        }
    }

    pub fn as_heap_storage<T>(&self, f: impl for<'a> FnOnce(&HeapStorage<'a>) -> T) -> T {
        match self {
            Self::Local(r) => r.as_heap_storage(f),
            #[cfg(feature = "multithreading")]
            Self::CrossArena(p, _, _) => p.as_heap_storage(f),
        }
    }
}
