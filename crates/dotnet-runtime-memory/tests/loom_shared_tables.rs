#![cfg(loom)]

//! Loom models that drive the production VM-wide GC handle table and f-reachable queue.
//!
//! `dotnet-vm` does not compile under `cfg(loom)` with `multithreading`, but this crate does, so
//! `SharedGcHandles` is used directly. `Finalizers` lives in `dotnet-vm` behind the
//! `multithreading` feature; its source file has no other crate-local dependencies, so it is
//! compiled verbatim into this test with `#[path]` instead of being mirrored.
//!
//! Every arena lives on its own loom thread, with a distinct managed thread id as in production.
//! Phases are separated by rendezvous points, standing in for the stop-the-world handshake whose
//! own protocol is modeled in `dotnet-vm/tests/loom_stw.rs`.

use dotnet_runtime_memory::handles::SharedGcHandles;
use dotnet_types::{TypeDescription, generics::GenericLookup};
use dotnet_utils::{
    ArenaId,
    gc::{ArenaHandle, GCHandle, GCHandleType},
    sync::MANAGED_THREAD_ID,
};
use dotnet_value::{
    HeapStorage, Object, ObjectRef,
    layout::{FieldLayoutManager, GcDesc},
    object::ObjectPtr,
    storage::FieldStorage,
};
use gc_arena::{Arena, Collect, Gc, Rootable, collect::Trace};
use loom::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};
use std::collections::HashSet;

#[allow(dead_code)]
#[path = "../../dotnet-vm/src/finalizers.rs"]
mod finalizers;

use finalizers::Finalizers;

fn check(f: impl Fn() + Send + Sync + 'static) {
    let mut builder = loom::model::Builder::new();
    builder.preemption_bound = Some(3);
    builder.max_branches = 20_000;
    builder.max_duration = None;
    builder.max_permutations = None;
    builder.check(f);
}

/// Spins (yielding to the loom scheduler) until `counter` reaches `target`.
fn rendezvous(counter: &AtomicUsize, target: usize) {
    counter.fetch_add(1, Ordering::AcqRel);
    while counter.load(Ordering::Acquire) < target {
        thread::yield_now();
    }
}

/// An arena root whose only content is a shared table traced by the production `Collect` impl.
struct TableRoot<T>(Arc<T>);

// SAFETY: F5.TracesEveryGcRef — delegates to the table's own `Collect` implementation, which
// traces exactly the objects owned by the current collector.
unsafe impl<'gc> Collect<'gc> for TableRoot<SharedGcHandles> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        self.0.trace(cc);
    }
}

// SAFETY: F5.TracesEveryGcRef — as above, for the f-reachable queue.
unsafe impl<'gc> Collect<'gc> for TableRoot<Finalizers> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        self.0.trace(cc);
    }
}

struct ThreadId;

impl ThreadId {
    fn set(id: u16) -> Self {
        MANAGED_THREAD_ID.set(Some(ArenaId::new(id.into())));
        Self
    }
}

impl Drop for ThreadId {
    fn drop(&mut self) {
        MANAGED_THREAD_ID.set(None);
    }
}

/// Runs `f` inside a mutation of `arena`, handing it the arena's `GCHandle`.
fn with_gc<R, T>(
    arena: &Arena<R>,
    owner: &ArenaHandle,
    f: impl for<'gc> FnOnce(GCHandle<'gc>) -> T,
) -> T
where
    R: for<'a> Rootable<'a>,
    for<'a> <R as Rootable<'a>>::Root: Collect<'a>,
{
    // SAFETY: F1.GcHandleRooted — `owner` outlives `arena.mutate`, and the extended reference
    // only builds the `GCHandle` passed to `f` inside that closure.
    let inner = unsafe {
        std::mem::transmute::<&dotnet_utils::gc::ArenaHandleInner, &'static _>(owner.as_inner())
    };
    arena.mutate(|mc, _| f(GCHandle::new(mc, inner)))
}

/// Allocates a small field-less object. Strings are avoided: the string interner is a global
/// `Mutex` that would outlive one Loom execution.
fn alloc_object<'gc>(gc: GCHandle<'gc>) -> ObjectRef<'gc> {
    let layout = dotnet_utils::sync::Arc::new(FieldLayoutManager {
        fields: Default::default(),
        total_size: 8,
        alignment: 8,
        gc_desc: GcDesc::default(),
        has_ref_fields: false,
    });
    ObjectRef::new(
        gc,
        HeapStorage::Obj(Box::new(Object::new(
            TypeDescription::NULL,
            GenericLookup::default(),
            FieldStorage::new(layout, vec![0; 8]),
        ))),
    )
}

fn is_dead_ptr<'gc>(fc: &'gc gc_arena::Finalization<'gc>, ptr: ObjectPtr) -> bool {
    // SAFETY: F1.StwParked — the object's arena is alive and collected by the current thread.
    let object: ObjectRef<'gc> = ObjectRef(Some(unsafe { Gc::from_ptr(ptr.as_ptr().cast()) }));
    Gc::is_dead(fc, object.0.unwrap())
}

type HandlesArena = Arena<Rootable![TableRoot<SharedGcHandles>]>;

struct HandleScenario {
    table: Arc<SharedGcHandles>,
    /// Each arena's live object, published for the other arena's foreign weak handle.
    published: Mutex<[Option<ObjectPtr>; 2]>,
    stage: AtomicUsize,
}

/// One mutator/collector thread of the shared-handle model.
fn handle_thread(id: u16, scenario: Arc<HandleScenario>) {
    let _id = ThreadId::set(id);
    let table = scenario.table.clone();
    let mut arena = HandlesArena::new(|_| TableRoot(table.clone()));
    let owner = ArenaHandle::new(ArenaId::new(id.into()));

    // Mutator phase: concurrent alloc/set/free against the other arena's thread.
    let (live, dead, strong, weak_live, weak_dead, transient) = with_gc(&arena, &owner, |gc| {
        let live = alloc_object(gc);
        let dead = alloc_object(gc);
        let strong = table.alloc(live, GCHandleType::Normal);
        let weak_live = table.alloc(live, GCHandleType::Weak);
        let transient = table.alloc(dead, GCHandleType::Weak);
        // Retargeting and freeing race with the other thread's allocation of the same slots.
        table.set(transient, live);
        table.free(transient);
        let weak_dead = table.alloc(dead, GCHandleType::Weak);
        (
            live.as_ptr().unwrap(),
            dead.as_ptr().unwrap(),
            strong,
            weak_live,
            weak_dead,
            transient,
        )
    });
    let _ = transient;
    scenario.published.lock().unwrap()[usize::from(id) - 1] = Some(live);
    rendezvous(&scenario.stage, 2);

    // A weak handle to the other arena's live object, created by a non-owner.
    let foreign_ptr = scenario.published.lock().unwrap()[usize::from(2 - id)].unwrap();
    let foreign = with_gc(&arena, &owner, |_| {
        // SAFETY: F1.GcHandleRooted — the other thread keeps its arena alive until the final
        // rendezvous below, and the reference only crosses into the handle table.
        let object = ObjectRef(Some(unsafe { Gc::from_ptr(foreign_ptr.as_ptr().cast()) }));
        table.alloc(object, GCHandleType::Weak)
    });
    rendezvous(&scenario.stage, 4);

    // MarkAll on every owner, then ClearWeak on every owner, as in `clear_weak_arena`.
    let marked = arena.finish_marking().expect("fully marked");
    drop(marked);
    rendezvous(&scenario.stage, 6);
    arena
        .finish_marking()
        .expect("still marked")
        .finalize(|fc, root| {
            // SAFETY: F1.StwParked — every owner finished marking before any owner clears.
            unsafe { root.0.clear_dead(fc, GCHandleType::Weak, &HashSet::new()) };
            assert!(is_dead_ptr(fc, dead), "unrooted object must be dead");
            assert!(!is_dead_ptr(fc, live), "strong handle must keep its target");
        });
    rendezvous(&scenario.stage, 8);

    // SAFETY: F1.GcHandleRooted — slot reads copy pointers and never dereference them.
    unsafe {
        let read = |slot: usize| table.get(slot).expect("slot kept").0.as_ptr();
        assert_eq!(read(strong), Some(live));
        assert_eq!(read(weak_live), Some(live));
        assert_eq!(
            read(weak_dead),
            None,
            "dead weak handle is cleared by its owner"
        );
        assert_eq!(
            read(foreign),
            Some(foreign_ptr),
            "a non-owner's clear must not touch a foreign live target"
        );
    }
    rendezvous(&scenario.stage, 10);
}

#[test]
fn shared_handles_publish_and_clear_by_owner_partition() {
    check(|| {
        let scenario = Arc::new(HandleScenario {
            table: Arc::new(SharedGcHandles::default()),
            published: Mutex::new([None, None]),
            stage: AtomicUsize::new(0),
        });
        let other = {
            let scenario = scenario.clone();
            thread::spawn(move || handle_thread(2, scenario))
        };
        handle_thread(1, scenario.clone());
        other.join().unwrap();
        // Every slot published by the two threads is distinct and still present.
        assert_eq!(scenario.table.len(), 8 - 0);
    });
}

type QueueArena = Arena<Rootable![TableRoot<Finalizers>]>;

struct QueueScenario {
    queue: Arc<Finalizers>,
    stop_requested: AtomicBool,
    parked: AtomicUsize,
    worker_done: AtomicBool,
}

impl QueueScenario {
    /// The finalizer thread's safepoint: park while a stop-the-world is requested.
    fn safepoint(&self) {
        if self.stop_requested.load(Ordering::Acquire) {
            self.parked.fetch_add(1, Ordering::AcqRel);
            while self.stop_requested.load(Ordering::Acquire) {
                thread::yield_now();
            }
            self.parked.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

/// The finalizer thread: claim, park mid-call at a safepoint, complete; FIFO.
fn finalizer_worker(scenario: Arc<QueueScenario>, expected: Arc<Mutex<Vec<ObjectPtr>>>) {
    let _id = ThreadId::set(2);
    let mut finished = 0;
    while finished < 2 {
        scenario.safepoint();
        if let Some(ptr) = scenario.queue.claim() {
            // Parked here, the claimed object is `active` and must stay rooted.
            scenario.safepoint();
            assert_eq!(Some(ptr), expected.lock().unwrap().get(finished).copied());
            scenario.queue.complete();
            finished += 1;
        } else {
            thread::yield_now();
        }
    }
    scenario.worker_done.store(true, Ordering::Release);
}

#[test]
fn finalizer_queue_roots_claimed_and_pending_objects_while_worker_is_parked() {
    check(|| {
        let scenario = Arc::new(QueueScenario {
            queue: Arc::new(Finalizers::default()),
            stop_requested: AtomicBool::new(false),
            parked: AtomicUsize::new(0),
            worker_done: AtomicBool::new(false),
        });
        let expected = Arc::new(Mutex::new(Vec::new()));
        let _id = ThreadId::set(1);
        let queue = scenario.queue.clone();
        let mut arena = QueueArena::new(|_| TableRoot(queue.clone()));
        let owner = ArenaHandle::new(ArenaId::new(1));

        let worker = {
            let (scenario, expected) = (scenario.clone(), expected.clone());
            thread::spawn(move || finalizer_worker(scenario, expected))
        };

        // The worker may claim the first entry before the second is enqueued.
        let objects = with_gc(&arena, &owner, |gc| {
            let first = alloc_object(gc).as_ptr().unwrap();
            let second = alloc_object(gc).as_ptr().unwrap();
            expected.lock().unwrap().extend([first, second]);
            for ptr in [first, second] {
                // SAFETY: F1.GcHandleRooted — both objects are live in this mutation.
                let object = ObjectRef(Some(unsafe { Gc::from_ptr(ptr.as_ptr().cast()) }));
                scenario.queue.enqueue(object);
            }
            [first, second]
        });

        // Stop the world: wait for the finalizer thread to park or to finish.
        scenario.stop_requested.store(true, Ordering::Release);
        while scenario.parked.load(Ordering::Acquire) == 0
            && !scenario.worker_done.load(Ordering::Acquire)
        {
            thread::yield_now();
        }
        let completed = (0..=2u64)
            .rev()
            .find(|generation| scenario.queue.completed(*generation))
            .unwrap();
        arena
            .finish_marking()
            .expect("fully marked")
            .finalize(|fc, _| {
                for ptr in &objects[completed as usize..] {
                    assert!(
                        !is_dead_ptr(fc, *ptr),
                        "queued or active entries must stay rooted while the worker is parked"
                    );
                }
            });
        scenario.stop_requested.store(false, Ordering::Release);

        worker.join().unwrap();
        assert!(scenario.queue.completed(2));
        assert!(!scenario.queue.is_pending());
    });
}
