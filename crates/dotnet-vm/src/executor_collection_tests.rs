//! Multi-arena collection through real `Executor` arenas.
//!
//! Two actual `Executor` arenas are created: arena B allocates an object that has no root in B,
//! and arena A roots a cross-arena reference to it through its heap GC handles. The collection
//! is driven by the production `GCCoordinator` (`begin_collection`, stop-the-world,
//! `collect_all_arenas`: MarkAll -> fixed-point MarkObjects -> Finalize -> Sweep) and the
//! production `THREAD_ARENA`/`execute_gc_command_for_current_thread` command path.
//!
//! The survival witness is a production weak GC handle checked in B after each collection;
//! a missed publication clears the foreign handle before another trace. References cross
//! threads as production-encoded field storage and are decoded through a receiver-owned field.

use super::*;
#[cfg(feature = "multithreading")]
use crate::test_utils::foreign_reference::{decode_reference, encode_reference};
use dotnet_assemblies::AssemblyLoader;
#[cfg(feature = "multithreading")]
use dotnet_utils::{ArenaId, gc::OpaqueObjectPtr};
use dotnet_value::object::{HeapStorage, ObjectRef};
use dotnet_value::string::CLRString;
#[cfg(feature = "multithreading")]
use gc_arena::Gc;

const PAYLOAD: &str = "executor_collection_payload";

fn shared() -> Arc<SharedGlobalState> {
    #[allow(
        clippy::arc_with_non_send_sync,
        reason = "the single-thread configuration keeps shared state on its Executor's thread"
    )]
    Arc::new(SharedGlobalState::new(Arc::new(
        AssemblyLoader::new_bare("executor_collection".to_string()).unwrap(),
    )))
}

/// Allocates the payload string in the executor's arena without rooting it and returns its
/// object pointer. A production *weak* GC handle (slot 0 of the arena's heap handle table) is
/// the survival witness: `HeapManager::finalize_check` zeroes it when the object is swept, so
/// a missed publication is observed as a cleared handle and never as a dangling read.
fn allocate_unrooted(executor: &mut Executor) -> dotnet_value::object::ObjectPtr {
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — engine is the live arena root and the handle
                // stays within this mutation.
                let handle = unsafe { crate::test_utils::test_gc_handle(gc, engine) };
                let obj = ObjectRef::new(handle, HeapStorage::Str(CLRString::from(PAYLOAD)));
                let mut handles = engine.stack.local.heap.gchandles.borrow_mut();
                assert!(handles.is_empty(), "witness must be handle slot 0");
                handles.push(Some((obj, dotnet_utils::gc::GCHandleType::Weak)));
                obj.as_ptr().expect("non-null object")
            })
        })
        .unwrap()
}

/// Reads the payload through the weak-handle witness; `None` means the object was swept.
fn probe(executor: &mut Executor) -> Option<String> {
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let handles = engine.stack.local.heap.gchandles.borrow();
                let (obj, _) = handles[0].as_ref().expect("witness slot");
                obj.0.map(|h| match &h.borrow().storage {
                    HeapStorage::Str(s) => s.as_string(),
                    _ => panic!("unexpected storage"),
                })
            })
        })
        .unwrap()
}

#[cfg(not(feature = "multithreading"))]
#[test]
fn arena_lifetime_vm_collection() {
    // Single-arena no-features control: forced collections keep a GC-handle-rooted object.
    let mut executor = Executor::new(shared());
    allocate_unrooted(&mut executor);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|_gc, engine| {
                let obj = engine.stack.local.heap.gchandles.borrow()[0]
                    .as_ref()
                    .unwrap()
                    .0;
                engine
                    .stack
                    .local
                    .heap
                    .gchandles
                    .borrow_mut()
                    .push(Some((obj, dotnet_utils::gc::GCHandleType::Normal)));
            });
            arena.finish_cycle();
            arena.finish_cycle();
        })
        .unwrap();
    assert_eq!(probe(&mut executor).as_deref(), Some(PAYLOAD));
}

#[cfg(feature = "multithreading")]
fn run_vm_collection(root_in_a: bool) -> Vec<Option<String>> {
    use crate::{
        gc::coordinator::GcCycleGuard, test_utils::worker::Worker, threading::ThreadManagerOps,
    };
    use dotnet_utils::gc::GCHandleType;
    use std::{sync::mpsc, thread, time::Duration};

    let state = shared();
    let (ptr_tx, ptr_rx) = mpsc::channel::<(ArenaId, dotnet_value::storage::FieldStorage)>();
    let (collected_tx, collected_rx) = mpsc::channel::<()>();
    let (probe_tx, probe_rx) = mpsc::channel::<Option<String>>();
    let b_state = Arc::clone(&state);

    let owner_b = thread::spawn(move || {
        let mut b = Executor::new(Arc::clone(&b_state));
        let id = b.thread_id;
        allocate_unrooted(&mut b);
        let storage = b
            .with_arena(|arena| {
                arena.mutate(|_, engine| {
                    encode_reference(
                        engine.stack.local.heap.gchandles.borrow()[0]
                            .as_ref()
                            .unwrap()
                            .0,
                    )
                })
            })
            .unwrap();
        ptr_tx.send((id, storage)).unwrap();
        // Mutator loop: park at real safe points so the coordinator can stop this arena.
        // Exits when the driver drops `collected_tx`.
        loop {
            b_state
                .thread_manager
                .safe_point(id, &b_state.gc_coordinator);
            match collected_rx.recv_timeout(Duration::from_millis(1)) {
                Ok(()) => probe_tx.send(probe(&mut b)).unwrap(),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });

    let owner_b = Worker::new(owner_b, collected_tx);
    let mut a = Executor::new(Arc::clone(&state));
    let (b_id, storage) = ptr_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert_ne!(a.thread_id, b_id);
    if root_in_a {
        a.with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live root for this mutation.
                let handle = unsafe { crate::test_utils::test_gc_handle(gc, engine) };
                let obj = decode_reference(storage, handle);
                engine
                    .stack
                    .local
                    .heap
                    .gchandles
                    .borrow_mut()
                    .push(Some((obj, GCHandleType::Normal)));
            });
        })
        .unwrap();
    }

    let mut seen = Vec::new();
    for _ in 0..2 {
        let coordinator = Arc::clone(&state.gc_coordinator);
        let session = coordinator.begin_collection().expect("idle coordinator");
        let stw = state.thread_manager.request_stop_the_world();
        let cycle = GcCycleGuard::new(session, stw);
        cycle.collect_all_arenas(a.thread_id);
        drop(cycle);
        owner_b.resources().send(()).unwrap();
        let observed = probe_rx
            .recv_timeout(Duration::from_secs(30))
            .expect("arena B reported");
        let swept = observed.is_none();
        seen.push(observed);
        if swept && root_in_a {
            // Missed publication: A's slot now dangles. Drop it (Gc is Copy, no deref)
            // and stop; a second trace would read freed memory.
            a.with_arena(|arena| {
                arena.mutate(|_, engine| {
                    engine.stack.local.heap.gchandles.borrow_mut().clear();
                })
            })
            .unwrap();
            break;
        }
    }

    owner_b.finish().unwrap();
    drop(a);
    seen
}

#[cfg(feature = "multithreading")]
#[test]
fn arena_lifetime_vm_collection() {
    let seen = run_vm_collection(true);
    assert_eq!(
        seen,
        vec![Some(PAYLOAD.to_string()), Some(PAYLOAD.to_string())],
        "(a shorter list means a collection missed publication) object owned by arena B and referenced only from arena A must survive both collections"
    );
}

#[cfg(feature = "multithreading")]
#[test]
fn arena_lifetime_vm_collection_unrooted_negative_control() {
    // Without arena A's reference the collection really sweeps B's object (born-black
    // allocation can survive the first cycle, so the second collection is the witness).
    let seen = run_vm_collection(false);
    assert_eq!(
        seen[1], None,
        "unreferenced object must be swept by the second collection"
    );
}

/// The complete real-VM collection contract: three actual `Executor` arenas on three threads, driven through the
/// production coordinator and `THREAD_ARENA` command path.
///
/// * arena A holds the only root: a normal GC handle to vector X owned by arena B;
/// * X's serialized element is the only reference to string Y owned by arena C, so Y is
///   reachable only through the *fixed-point* cross edge A -> B (round 1) -> C (round 2);
/// * both objects must survive two collections, Y's payload must be readable and writable by
///   its owner afterwards, and a third collection must keep the written payload;
/// * once A drops its handle, both objects must be reclaimed within a bounded number of
///   further collections (weak-handle witnesses in the owners; no dangling read is possible).
#[cfg(feature = "multithreading")]
#[test]
fn arena_lifetime_vm_collection_fixed_point_payload_and_reclamation() {
    use crate::{
        gc::coordinator::GcCycleGuard, test_utils::worker::Worker, threading::ThreadManagerOps,
    };
    use dotnet_types::{generics::ConcreteType, resolution::ResolutionS};
    use dotnet_utils::gc::GCHandleType;
    use dotnet_value::{
        layout::{ArrayLayoutManager, LayoutManager, Scalar},
        object::{ObjectPtr, Vector},
    };
    use dotnetdll::prelude::BaseType;
    use std::{sync::mpsc, thread, time::Duration};

    const WRITTEN: &str = "executor_collection_payload_written";
    const VECTOR_MARK: &str = "<vector>";

    type Handoff = dotnet_value::storage::FieldStorage;

    enum Cmd {
        Probe,
        Write(&'static str),
        Link(Handoff),
    }

    /// Allocates an unrooted vector with one null `ObjectRef` element and a weak witness.
    fn allocate_unrooted_vector(executor: &mut Executor) -> ObjectPtr {
        executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — engine is the live arena root and the handle
                    // stays within this mutation.
                    let handle = unsafe { crate::test_utils::test_gc_handle(gc, engine) };
                    let vector = Vector::new(
                        ConcreteType::new(ResolutionS::NULL, BaseType::Object),
                        ArrayLayoutManager {
                            element_layout: Arc::new(LayoutManager::Scalar(Scalar::ObjectRef)),
                            length: 1,
                        },
                        vec![0; ObjectRef::SIZE],
                        vec![1, 1],
                    );
                    let obj = ObjectRef::new(handle, HeapStorage::Vec(Box::new(vector)));
                    let mut handles = engine.stack.local.heap.gchandles.borrow_mut();
                    assert!(handles.is_empty(), "witness must be handle slot 0");
                    handles.push(Some((obj, GCHandleType::Weak)));
                    obj.as_ptr().expect("non-null object")
                })
            })
            .unwrap()
    }

    /// Weak-witness read: payload string, `<vector>` marker, or `None` once swept.
    fn probe_any(executor: &mut Executor) -> Option<String> {
        executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| {
                    let handles = engine.stack.local.heap.gchandles.borrow();
                    let (obj, _) = handles[0].as_ref().expect("witness slot");
                    obj.0.map(|h| match &h.borrow().storage {
                        HeapStorage::Str(s) => s.as_string(),
                        HeapStorage::Vec(_) => VECTOR_MARK.to_string(),
                        _ => panic!("unexpected storage"),
                    })
                })
            })
            .unwrap()
    }

    type OwnerHandoff = (
        ArenaId,
        Handoff,
        Worker<(), mpsc::Sender<Cmd>>,
        mpsc::Receiver<Option<String>>,
    );

    fn spawn_owner(state: &Arc<SharedGlobalState>, vector: bool) -> OwnerHandoff {
        let (ptr_tx, ptr_rx) = mpsc::channel::<(ArenaId, Handoff)>();
        let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
        let (reply_tx, reply_rx) = mpsc::channel::<Option<String>>();
        let owner_state = Arc::clone(state);
        let join = thread::spawn(move || {
            let mut owner = Executor::new(Arc::clone(&owner_state));
            let id = owner.thread_id;
            if vector {
                allocate_unrooted_vector(&mut owner)
            } else {
                allocate_unrooted(&mut owner)
            };
            let storage = owner
                .with_arena(|arena| {
                    arena.mutate(|_, engine| {
                        encode_reference(
                            engine.stack.local.heap.gchandles.borrow()[0]
                                .as_ref()
                                .unwrap()
                                .0,
                        )
                    })
                })
                .unwrap();
            ptr_tx.send((id, storage)).unwrap();
            // Mutator loop: park at real safe points so the coordinator can stop this arena.
            // Exits when the driver drops its command sender.
            loop {
                owner_state
                    .thread_manager
                    .safe_point(id, &owner_state.gc_coordinator);
                match cmd_rx.recv_timeout(Duration::from_millis(1)) {
                    Ok(Cmd::Probe) => reply_tx.send(probe_any(&mut owner)).unwrap(),
                    Ok(Cmd::Write(text)) => {
                        owner
                            .with_arena(|arena| {
                                arena.mutate_root(|gc, engine| {
                                    let handles = engine.stack.local.heap.gchandles.borrow();
                                    let (obj, _) = handles[0].as_ref().expect("witness slot");
                                    let h = obj.0.expect("payload must still be live to write");
                                    dotnet_utils::gc::ThreadSafeLock::borrow_mut(h, gc).storage =
                                        HeapStorage::Str(CLRString::from(text));
                                })
                            })
                            .unwrap();
                        reply_tx.send(None).unwrap();
                    }
                    Ok(Cmd::Link(target)) => {
                        owner
                            .with_arena(|arena| {
                                arena.mutate_root(|gc, engine| {
                                    // SAFETY: F1.GcHandleRooted — live root, handle confined to mutation.
                                    let handle =
                                        unsafe { crate::test_utils::test_gc_handle(gc, engine) };
                                    let target = decode_reference(target, handle);
                                    let handles = engine.stack.local.heap.gchandles.borrow();
                                    let (obj, _) = handles[0].as_ref().expect("witness slot");
                                    assert!(obj.0.is_some(), "vector must still be live to link");
                                    // SAFETY: F1.GcHandleRooted — engine is the live arena root
                                    // and the handle stays within this mutation.
                                    let handle =
                                        unsafe { crate::test_utils::test_gc_handle(gc, engine) };
                                    obj.with_data_mut(handle, |data| target.write(data));
                                })
                            })
                            .unwrap();
                        reply_tx.send(None).unwrap();
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        let worker = Worker::new(join, cmd_tx);
        let (id, ptr) = ptr_rx.recv_timeout(Duration::from_secs(30)).unwrap();
        (id, ptr, worker, reply_rx)
    }

    let state = shared();
    // Y (string) in arena C, X (vector) in arena B, root in arena A (this thread).
    let (c_id, y_storage, c_worker, c_rx) = spawn_owner(&state, false);
    let (b_id, x_storage, b_worker, b_rx) = spawn_owner(&state, true);
    let mut a = Executor::new(Arc::clone(&state));
    assert!(a.thread_id != b_id && a.thread_id != c_id && b_id != c_id);

    let recv = |rx: &mpsc::Receiver<Option<String>>| rx.recv_timeout(Duration::from_secs(30));
    b_worker.resources().send(Cmd::Link(y_storage)).unwrap();
    recv(&b_rx).expect("arena B linked X -> Y");
    a.with_arena(|arena| {
        arena.mutate_root(|gc, engine| {
            // SAFETY: F1.GcHandleRooted — live root, handle confined to mutation.
            let handle = unsafe { crate::test_utils::test_gc_handle(gc, engine) };
            let obj = decode_reference(x_storage, handle);
            engine
                .stack
                .local
                .heap
                .gchandles
                .borrow_mut()
                .push(Some((obj, GCHandleType::Normal)));
        });
    })
    .unwrap();

    let collect = |a: &Executor| {
        let coordinator = Arc::clone(&state.gc_coordinator);
        let session = coordinator.begin_collection().expect("idle coordinator");
        let stw = state.thread_manager.request_stop_the_world();
        let cycle = GcCycleGuard::new(session, stw);
        cycle.collect_all_arenas(a.thread_id);
        drop(cycle);
    };
    let observe = || {
        b_worker.resources().send(Cmd::Probe).unwrap();
        let x = recv(&b_rx).expect("arena B reported");
        c_worker.resources().send(Cmd::Probe).unwrap();
        let y = recv(&c_rx).expect("arena C reported");
        (x, y)
    };
    // Missed publication leaves A's slot dangling: drop it (Gc is Copy, no deref) before
    // any further trace and fail with the observed witnesses.
    let clear_a = |a: &mut Executor| {
        a.with_arena(|arena| {
            arena.mutate(|_, engine| engine.stack.local.heap.gchandles.borrow_mut().clear())
        })
        .unwrap();
    };

    let alive = (Some(VECTOR_MARK.to_string()), Some(PAYLOAD.to_string()));
    let mut seen = Vec::new();
    for _ in 0..2 {
        collect(&a);
        let observed = observe();
        let ok = observed == alive;
        seen.push(observed);
        if !ok {
            clear_a(&mut a);
            break;
        }
    }
    let survived_two = seen.iter().all(|o| *o == alive) && seen.len() == 2;
    let mut written = None;
    if survived_two {
        // Payload read/write after two collections, then a third collection keeps the write.
        c_worker.resources().send(Cmd::Write(WRITTEN)).unwrap();
        recv(&c_rx).expect("arena C wrote payload");
        collect(&a);
        written = Some(observe());
        if written.as_ref().map(|w| w.1.as_deref()) != Some(Some(WRITTEN)) {
            clear_a(&mut a);
        }
    }

    // Eventual reclamation once A releases its only root (bounded: born-black allocation and
    // one fixed-point round per edge may delay the sweep by a cycle or two).
    let mut reclaimed = None;
    if written.as_ref().map(|w| w.1.as_deref()) == Some(Some(WRITTEN)) {
        clear_a(&mut a);
        for _ in 0..4 {
            collect(&a);
            let observed = observe();
            let done = observed == (None, None);
            reclaimed = Some(observed);
            if done {
                break;
            }
        }
    }

    b_worker.finish().unwrap();
    c_worker.finish().unwrap();
    drop(a);

    assert_eq!(
        seen,
        vec![alive.clone(), alive],
        "(vector X in B, payload Y in C) both reachable only through A's handle and the \
         fixed-point cross edge must survive two collections"
    );
    assert_eq!(
        written,
        Some((Some(VECTOR_MARK.to_string()), Some(WRITTEN.to_string()))),
        "payload written after two collections must persist through a third"
    );
    assert_eq!(
        reclaimed,
        Some((None, None)),
        "both objects must be reclaimed once the only root is dropped"
    );
}

/// Keep the original arena
/// and its allocation live while replacing its registry entry. No stale object
/// is freed or dereferenced by the test. A mark command from the old registration
/// must not install that object into the replacement registration's roots.
#[cfg(feature = "multithreading")]
#[test]
fn arena_lifetime_mark_command_rejects_stale_registration() {
    use crate::gc::coordinator::{GCCommand, MarkObjectPointers, MarkPhaseCommand};
    use crate::threading::{ThreadManagerOps, execute_gc_command_for_current_thread};
    use dotnet_utils::gc::{register_arena, try_acquire_lease, unregister_arena};
    use dotnet_utils::sync::AtomicBool;

    let state = shared();
    let mut executor = Executor::new(Arc::clone(&state));
    let id = executor.thread_id;
    let ptr = allocate_unrooted(&mut executor);
    let old_identity = try_acquire_lease(id).unwrap().identity();
    // SAFETY: F1.ArenaGenerationMatch — the object was allocated in this
    // Executor's still-live arena, under the original registration above.
    let old_reference =
        unsafe { dotnet_utils::gc::CrossArenaRef::new(old_identity, ptr.to_opaque()) };
    // A positive control runs the same command while the original owner is live.
    let session = state.gc_coordinator.begin_collection().unwrap();
    let stw = state.thread_manager.request_stop_the_world();
    let mut valid_objects = MarkObjectPointers::new();
    valid_objects.insert(old_reference);
    execute_gc_command_for_current_thread(
        GCCommand::Mark(MarkPhaseCommand::Objects(valid_objects)),
        &state.gc_coordinator,
    );
    let valid_installed = executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let mut roots = engine.stack.local.heap.cross_arena_roots.borrow_mut();
                let installed = roots.contains(&ptr);
                roots.clear();
                installed
            })
        })
        .unwrap();
    session.finish();
    drop(stw);
    assert!(valid_installed, "live owner reference was not installed");

    unregister_arena(id);
    // This test uses a separate registry flag; it denotes the real STW below.
    register_arena(id, Arc::new(AtomicBool::new(true)));
    let new_identity = try_acquire_lease(id).unwrap().identity();
    assert_ne!(old_identity, new_identity);

    // A distinct arena with the replacement identity supplies a valid, live
    // pointer with the same ID but a different generation. It stays allocated
    // throughout the command; it must never enter the old Executor's collector.
    let replacement_arena = gc_arena::Arena::<gc_arena::Rootable![()]>::new(|_| ());
    let replacement_ptr = replacement_arena.mutate(|gc, _| {
        let object = Gc::new(
            gc,
            dotnet_utils::gc::ThreadSafeLock::new(dotnet_value::object::ObjectInner::new(
                HeapStorage::Str(CLRString::from(PAYLOAD)),
                id,
            )),
        );
        // SAFETY: F11.ObjectPointerThreadSafe — this pointer is directly from
        // the live replacement arena's Gc allocation of the concrete pointee.
        unsafe { OpaqueObjectPtr::from_raw(Gc::as_ptr(object).cast()) }
    });
    // SAFETY: F1.ArenaGenerationMatch — this object belongs to the replacement
    // arena under the new registration, which remains live through the command.
    let replacement_reference =
        unsafe { dotnet_utils::gc::CrossArenaRef::new(new_identity, replacement_ptr) };

    // Also carry a valid pointer owned by a different ID to this destination.
    let foreign_id = ArenaId::new(8_001_042);
    register_arena(foreign_id, Arc::new(AtomicBool::new(true)));
    let foreign_identity = try_acquire_lease(foreign_id).unwrap().identity();
    let foreign_arena = gc_arena::Arena::<gc_arena::Rootable![()]>::new(|_| ());
    let foreign_ptr = foreign_arena.mutate(|gc, _| {
        let object = Gc::new(
            gc,
            dotnet_utils::gc::ThreadSafeLock::new(dotnet_value::object::ObjectInner::new(
                HeapStorage::Str(CLRString::from(PAYLOAD)),
                foreign_id,
            )),
        );
        // SAFETY: F11.ObjectPointerThreadSafe — direct pointer to this live
        // foreign arena's concrete Gc allocation; no access is performed.
        unsafe { OpaqueObjectPtr::from_raw(Gc::as_ptr(object).cast()) }
    });
    // SAFETY: F1.ArenaGenerationMatch — this live foreign allocation was made
    // under the foreign registration, which stays live through the command.
    let foreign_reference =
        unsafe { dotnet_utils::gc::CrossArenaRef::new(foreign_identity, foreign_ptr) };

    let session = state.gc_coordinator.begin_collection().unwrap();
    let stw = state.thread_manager.request_stop_the_world();
    let mut objects = MarkObjectPointers::new();
    objects.insert(old_reference);
    objects.insert(replacement_reference);
    objects.insert(foreign_reference);
    execute_gc_command_for_current_thread(
        GCCommand::Mark(MarkPhaseCommand::Objects(objects)),
        &state.gc_coordinator,
    );
    let installed = executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let mut roots = engine.stack.local.heap.cross_arena_roots.borrow_mut();
                let installed = !roots.is_empty();
                roots.clear();
                installed
            })
        })
        .unwrap();
    session.finish();
    drop(stw);
    drop(executor);
    unregister_arena(foreign_id);
    assert!(
        !installed,
        "stale, replacement, or foreign registration installed in the original arena"
    );
    drop(replacement_arena);
    drop(foreign_arena);
}

/// The registry's current
/// generation alone does not identify the particular destination arena.
#[cfg(feature = "multithreading")]
#[test]
fn arena_lifetime_installation_checks_arena_generation() {
    let source = include_str!("threading/basic.rs");
    let command = source
        .split("MarkPhaseCommand::Objects(ptrs) =>")
        .nth(1)
        .unwrap()
        .split("GCCommand::Sweep(sweep_command)")
        .next()
        .unwrap();
    assert!(
        command.contains("c.stack.arena_identity"),
        "mark installation does not compare the destination arena's original generation"
    );
}

#[cfg(feature = "multithreading")]
#[test]
fn collection_request_retries_when_coordinator_is_busy() {
    let state = shared();
    let mut executor = Executor::new(Arc::clone(&state));
    let coordinator = Arc::clone(&state.gc_coordinator);
    let (ready, acquired) = std::sync::mpsc::channel();
    let (release, held) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let session = coordinator.begin_collection().unwrap();
        ready.send(()).unwrap();
        held.recv().unwrap();
        drop(session);
    });
    acquired.recv().unwrap();
    executor.perform_full_gc().unwrap();
    assert_eq!(executor.collection_check().unwrap(), (true, true));
    release.send(()).unwrap();
    thread.join().unwrap();
    executor.perform_full_gc().unwrap();
    assert_eq!(executor.collection_check().unwrap(), (false, false));
}

#[cfg(feature = "multithreading")]
#[test]
fn dependent_handle_chain_survives_and_clears() {
    use crate::{gc::coordinator::GcCycleGuard, threading::ThreadManagerOps};
    use dotnet_utils::gc::GCHandleType;

    let state = shared();
    let mut executor = Executor::new(Arc::clone(&state));
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: The objects and handles stay within this live arena.
                let handle = unsafe { crate::test_utils::test_gc_handle(gc, engine) };
                let key = ObjectRef::new(handle, HeapStorage::Str(CLRString::from("key")));
                let middle = ObjectRef::new(handle, HeapStorage::Str(CLRString::from("middle")));
                let tail = ObjectRef::new(handle, HeapStorage::Str(CLRString::from("tail")));
                let heap = &engine.stack.local.heap;
                heap.gchandles
                    .borrow_mut()
                    .push(Some((key, GCHandleType::Normal)));
                heap.gchandles
                    .borrow_mut()
                    .push(Some((tail, GCHandleType::Weak)));
                // Reverse order requires another mark round after the first dependent activates.
                heap.dependent_handles
                    .borrow_mut()
                    .extend([Some((middle, tail)), Some((key, middle))]);
            })
        })
        .unwrap();
    let thread_id = executor.thread_id;
    let collect = || {
        let session = state.gc_coordinator.begin_collection().unwrap();
        let stw = state.thread_manager.request_stop_the_world();
        let cycle = GcCycleGuard::new(session, stw);
        cycle.collect_all_arenas(thread_id);
    };
    collect();
    executor
        .with_arena(|arena| {
            arena.mutate_root(|_, engine| {
                let heap = &engine.stack.local.heap;
                assert!(heap.gchandles.borrow()[1].as_ref().unwrap().0.0.is_some());
                heap.gchandles.borrow_mut()[0] = None;
            })
        })
        .unwrap();
    collect();
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let heap = &engine.stack.local.heap;
                assert!(heap.gchandles.borrow()[1].as_ref().unwrap().0.0.is_none());
                for (key, value) in heap.dependent_handles.borrow().iter().flatten() {
                    assert!(key.0.is_none() && value.0.is_none());
                }
            })
        })
        .unwrap();
}
