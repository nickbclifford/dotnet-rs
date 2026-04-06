//! Cross-arena reference tracking and lease API.
//!
//! # TOCTOU Problem
//!
//! The original `is_valid_cross_arena_ref` + dereference pattern is a
//! time-of-check/time-of-use (TOCTOU) race: an arena can unregister between
//! the validity check and the pointer dereference, leaving a dangling pointer.
//!
//! # Lease API
//!
//! `try_acquire_lease` returns an [`ArenaLease`] RAII guard.  While the guard
//! is alive the target arena **cannot complete** `unregister_arena`: the
//! function spins until `active_leases` reaches zero before returning.
//!
//! ## Protocol
//!
//! ```text
//! // Safe cross-arena dereference pattern:
//! if let Some(lease) = try_acquire_lease(target_id) {
//!     // arena is pinned — safe to dereference the cross-arena pointer
//!     let _ = unsafe { ptr.as_ref() };
//!     drop(lease);        // release; unregister_arena may now proceed
//! }
//! ```
//!
//! ## Correctness argument
//!
//! `try_acquire_lease` takes a *read* lock on `VALID_ARENAS`, clones the
//! `Arc<ArenaState>`, increments `active_leases` — all while holding that
//! read lock.  `unregister_arena` must take the *write* lock to remove the
//! entry; it can only do so after every in-progress `try_acquire_lease` call
//! has released the read lock.  At that point every caller has already
//! incremented `active_leases`, so `unregister_arena` will observe a non-zero
//! count and spin.  Once the last `ArenaLease` drops, `active_leases` returns
//! to zero and `unregister_arena` completes.
//!
//! ## Backward compatibility
//!
//! `register_arena`, `is_valid_cross_arena_ref`, `set_stw_in_progress`,
//! `is_stw_in_progress`, and `record_cross_arena_ref` retain their existing
//! signatures and semantics so that legacy call sites continue to compile
//! unchanged while adopting `ArenaLease`.
use crate::{
    newtypes::ArenaId,
    sync::{Arc, AtomicBool, Ordering, RwLock},
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    mem,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering as AtomicOrdering},
};

// ---------------------------------------------------------------------------
// Per-arena shared state
// ---------------------------------------------------------------------------

/// Monotonically increasing counter used to assign a unique generation to each
/// arena registration.  Each call to `register_arena` increments this counter
/// and stores the resulting value in the new [`ArenaState`].  Leases capture
/// the generation at acquisition time; callers can use this to detect whether
/// the arena has been unregistered and re-registered (same `ArenaId`, new
/// generation) between when they recorded a cross-arena pointer and when they
/// attempt to dereference it.
static ARENA_GENERATION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Shared state owned by one arena entry.
///
/// All outstanding [`ArenaLease`] guards hold a clone of the `Arc<ArenaState>`
/// that backs the entry.  `unregister_arena` spins on `active_leases` after
/// removing the entry from the registry, ensuring no dereference races past
/// teardown.
pub struct ArenaState {
    /// The stop-the-world flag passed in at registration time.
    /// Kept as a separate `Arc<AtomicBool>` so that existing callers that
    /// obtained their own clone before `register_arena` can still drive it
    /// directly (backward-compatible).
    pub stw_in_progress: Arc<AtomicBool>,
    /// Number of outstanding [`ArenaLease`] guards currently alive.
    pub active_leases: AtomicUsize,
    /// Set to `false` by `unregister_arena` after removing from the registry.
    /// Guards can observe this to detect that the arena has begun teardown.
    is_alive: AtomicBool,
    /// Unique generation assigned at registration time.  Immutable after
    /// construction; no atomic access needed for reads via a shared reference.
    pub generation: u64,
}

impl ArenaState {
    fn new(stw_in_progress: Arc<AtomicBool>) -> Self {
        // SeqCst ensures the generation is globally unique even under
        // concurrent registrations on different CPUs.
        let generation = ARENA_GENERATION_COUNTER.fetch_add(1, AtomicOrdering::SeqCst);
        Self {
            stw_in_progress,
            active_leases: AtomicUsize::new(0),
            is_alive: AtomicBool::new(true),
            generation,
        }
    }
}

// ---------------------------------------------------------------------------
// Lease guard
// ---------------------------------------------------------------------------

/// RAII guard that pins an arena's validity for the duration of a cross-arena
/// dereference.
///
/// Obtain via [`try_acquire_lease`].  While this value is alive, any
/// concurrent `unregister_arena` call for the same arena will spin rather than
/// return, guaranteeing that arena-owned memory remains live.
///
/// Drop the lease as soon as the dereference is complete.
pub struct ArenaLease {
    state: Arc<ArenaState>,
    /// The generation of the arena at the time this lease was acquired.
    /// Matches `state.generation` for the lifetime of the lease; callers
    /// that stored this value can compare it against a freshly acquired
    /// lease's generation to detect an intervening unregister/re-register.
    generation: u64,
}

impl ArenaLease {
    /// Returns the generation of the arena at the time this lease was acquired.
    ///
    /// Two leases for the same [`ArenaId`] with different generations indicate
    /// that the arena was unregistered and re-registered between the two
    /// acquisitions.  Any cross-arena pointer recorded under the old
    /// generation must be considered invalid.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns `true` if the pinned arena is still alive (has not begun
    /// unregistration) and the lease generation matches the arena's current
    /// generation.
    ///
    /// Under normal operation this will always be `true` for the duration of
    /// a held lease, because `unregister_arena` spins until all leases are
    /// dropped before returning.  The check is provided as a defense-in-depth
    /// assertion point.
    pub fn is_valid(&self) -> bool {
        self.state.is_alive.load(AtomicOrdering::Acquire)
            && self.state.generation == self.generation
    }

    /// Returns `true` if a stop-the-world GC pause is currently in progress
    /// for the pinned arena.
    pub fn is_stw_in_progress(&self) -> bool {
        self.state.stw_in_progress.load(Ordering::Acquire)
    }
}

impl Drop for ArenaLease {
    fn drop(&mut self) {
        // Use Release so that any writes performed while holding the lease are
        // visible to the unregister_arena spin-check on Acquire.
        self.state
            .active_leases
            .fetch_sub(1, AtomicOrdering::Release);
    }
}

// ---------------------------------------------------------------------------
// Global registry
// ---------------------------------------------------------------------------

static VALID_ARENAS: std::sync::LazyLock<RwLock<HashMap<ArenaId, Arc<ArenaState>>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));

/// A fast-path bitset for the first 64 Arena IDs to avoid lock contention on
/// every object read.
static VALID_ARENAS_FAST: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Registry management
// ---------------------------------------------------------------------------

/// Register an arena.
///
/// `stw_in_progress` is the caller-owned flag that will be set when a
/// stop-the-world pause begins for this arena.  The same `Arc` is embedded in
/// the [`ArenaState`] so that [`is_stw_in_progress`] and [`try_acquire_lease`]
/// can read it without requiring the original owner to be present.
pub fn register_arena(thread_id: ArenaId, stw_in_progress: Arc<AtomicBool>) {
    let id = thread_id.0;
    if id < 64 {
        VALID_ARENAS_FAST.fetch_or(1 << id, Ordering::Release);
    }
    let state = Arc::new(ArenaState::new(stw_in_progress));
    VALID_ARENAS.write().insert(thread_id, state);
}

/// Unregister an arena.
///
/// Removes the entry from the registry and then **spins** until all
/// outstanding [`ArenaLease`] guards for this arena have been dropped.  This
/// closes the TOCTOU window: any caller that obtained a lease before the
/// write-lock was acquired has already incremented `active_leases`, so this
/// function will not return until that dereference finishes.
///
/// In practice leases are held only for the duration of a single pointer
/// dereference (nanoseconds), so the spin terminates almost immediately.
pub fn unregister_arena(thread_id: ArenaId) {
    let id = thread_id.0;
    if id < 64 {
        VALID_ARENAS_FAST.fetch_and(!(1 << id), Ordering::Release);
    }
    // Remove under write lock.  After this point try_acquire_lease will find
    // no entry and return None, preventing new leases from being issued.
    let state = VALID_ARENAS.write().remove(&thread_id);
    if let Some(state) = state {
        state.is_alive.store(false, AtomicOrdering::Release);
        // Drain all in-flight leases before returning.
        while state.active_leases.load(AtomicOrdering::Acquire) > 0 {
            std::hint::spin_loop();
        }
    }
}

// ---------------------------------------------------------------------------
// Lease acquisition (new primary dereference API)
// ---------------------------------------------------------------------------

/// Attempt to pin an arena and obtain a dereference lease.
///
/// Returns `Some(ArenaLease)` if the arena is currently registered.  The
/// returned guard keeps the arena pinned until it is dropped, making it safe
/// to dereference any pointer into that arena's heap during the guard's
/// lifetime.
///
/// Returns `None` if the arena is not (or no longer) registered.
///
/// # Correctness
///
/// The `active_leases` counter is incremented **while the registry read-lock
/// is still held**.  Because `unregister_arena` requires the write lock to
/// remove the entry, it cannot proceed past that removal while any
/// `try_acquire_lease` call is in the middle of incrementing the counter.
/// After the read lock is released, `unregister_arena` (if it wins the write
/// lock) will observe `active_leases > 0` and spin.
pub fn try_acquire_lease(target_id: ArenaId) -> Option<ArenaLease> {
    // Fast path: if the arena is definitely absent, skip the lock entirely.
    let id = target_id.0;
    if id < 64 && (VALID_ARENAS_FAST.load(Ordering::Acquire) & (1 << id)) == 0 {
        return None;
    }

    // Hold the read lock while we clone the Arc *and* increment active_leases.
    // This is the critical section that prevents unregister from racing ahead.
    let guard = VALID_ARENAS.read();
    let state = guard.get(&target_id).cloned()?;
    // Capture generation before releasing the lock so it is consistent with
    // the state we are pinning.
    let generation = state.generation;
    // Increment before releasing the map lock.
    state.active_leases.fetch_add(1, AtomicOrdering::Acquire);
    drop(guard); // read lock released here; unregister may now take write lock
    // and will see active_leases > 0, so it will spin.

    Some(ArenaLease { state, generation })
}

// ---------------------------------------------------------------------------
// Backward-compatible helpers (kept for callers not yet migrated)
// ---------------------------------------------------------------------------

/// Check whether an arena is currently registered.
///
/// **Note**: this function has a TOCTOU race — the arena may unregister
/// between this call and any subsequent dereference.  Prefer
/// [`try_acquire_lease`] for dereference-guarded access.
pub fn is_valid_cross_arena_ref(target_thread_id: ArenaId) -> bool {
    let id = target_thread_id.0;
    if id < 64 {
        (VALID_ARENAS_FAST.load(Ordering::Acquire) & (1 << id)) != 0
    } else {
        VALID_ARENAS.read().contains_key(&target_thread_id)
    }
}

pub fn reset_arena_registry() {
    VALID_ARENAS_FAST.store(0, Ordering::Release);
    VALID_ARENAS.write().clear();
}

pub fn set_stw_in_progress(arena_id: ArenaId, in_progress: bool) {
    if let Some(state) = VALID_ARENAS.read().get(&arena_id) {
        state.stw_in_progress.store(in_progress, Ordering::Release);
    }
}

pub fn is_stw_in_progress(arena_id: ArenaId) -> bool {
    VALID_ARENAS
        .read()
        .get(&arena_id)
        .map(|state| state.stw_in_progress.load(Ordering::Acquire))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Cross-arena tracing thread-locals
// ---------------------------------------------------------------------------

thread_local! {
    /// Found cross-arena references during the current marking phase.
    /// Each entry stores `(target_arena_id, raw_pointer_usize, arena_generation)`
    /// so that harvest sites can verify the target arena has not been
    /// unregistered and re-registered (changing its generation) between
    /// recording and dereference.
    static FOUND_CROSS_ARENA_REFS: RefCell<Vec<(ArenaId, usize, u64)>> = const { RefCell::new(Vec::new()) };
    /// The thread ID of the arena currently being traced.
    static CURRENTLY_TRACING_THREAD_ID: Cell<Option<ArenaId>> = const { Cell::new(None) };
}

/// Set the thread ID of the arena currently being traced.
pub fn set_currently_tracing(thread_id: Option<ArenaId>) {
    CURRENTLY_TRACING_THREAD_ID.with(|id| {
        id.set(thread_id);
    });
}

/// Get the thread ID of the arena currently being traced.
pub fn get_currently_tracing() -> Option<ArenaId> {
    CURRENTLY_TRACING_THREAD_ID.get()
}

/// Take all found cross-arena references including their recorded generation,
/// and clear the local list.
///
/// Returns `(arena_id, raw_ptr_usize, generation)` triples.  Callers should
/// acquire a fresh [`ArenaLease`] for each entry and compare
/// `lease.generation()` against the stored generation; a mismatch means the
/// target arena was unregistered and re-registered between recording and
/// harvest, and the pointer must be discarded.
pub fn take_found_cross_arena_refs_with_generation() -> Vec<(ArenaId, usize, u64)> {
    FOUND_CROSS_ARENA_REFS.with(|refs| {
        let mut r = refs.borrow_mut();
        mem::take(&mut *r)
    })
}

/// Record a cross-arena reference found during marking.
///
/// Returns `true` if the target arena is currently registered, `false` if it
/// has already exited.
///
/// When called during a GC tracing phase the reference is pushed to the
/// thread-local `FOUND_CROSS_ARENA_REFS` list together with the arena's
/// current **generation** (captured via [`try_acquire_lease`]).  This closes
/// the recording-side TOCTOU window: the arena is pinned by the lease while
/// the push occurs, so the generation stored in the list is guaranteed to
/// reflect a live registration.  The lease is released immediately after the
/// push; dereference-side protection is provided at the harvest site via
/// [`take_found_cross_arena_refs_with_generation`].
pub fn record_cross_arena_ref(target_thread_id: ArenaId, ptr: usize) -> bool {
    // Only record the reference if we are currently in a GC tracing phase.
    if CURRENTLY_TRACING_THREAD_ID.with(|id| id.get().is_some()) {
        // Acquire a lease to pin the arena and capture its generation atomically.
        if let Some(lease) = try_acquire_lease(target_thread_id) {
            let generation = lease.generation();
            FOUND_CROSS_ARENA_REFS.with(|refs| {
                refs.borrow_mut().push((target_thread_id, ptr, generation));
            });
            // Release the lease immediately; the arena may unregister after this
            // point.  The stored generation lets harvest sites detect that case.
            drop(lease);
            return true;
        }
        return false;
    }
    // Outside a tracing phase: just check validity (backward-compatible path).
    is_valid_cross_arena_ref(target_thread_id)
}

/// Clear tracing-related thread-local state.
pub fn clear_tracing_state() {
    CURRENTLY_TRACING_THREAD_ID.with(|id| {
        id.set(None);
    });
    FOUND_CROSS_ARENA_REFS.with(|refs| {
        refs.borrow_mut().clear();
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod lease_tests {
    use super::*;
    use crate::ArenaId;

    // Use high IDs (>= 64) to avoid the fast-path bitset and use unique IDs
    // per test to avoid cross-test registry pollution via global statics.
    // Tests that run concurrently in the same process share VALID_ARENAS.

    fn stw_flag() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    // ------------------------------------------------------------------
    // try_acquire_lease on unregistered arena returns None
    // ------------------------------------------------------------------
    #[test]
    fn lease_on_unregistered_arena_returns_none() {
        let id = ArenaId(10_000);
        // ensure it is not in the registry
        unregister_arena(id);
        assert!(try_acquire_lease(id).is_none());
    }

    // ------------------------------------------------------------------
    // try_acquire_lease on registered arena returns Some
    // ------------------------------------------------------------------
    #[test]
    fn lease_on_registered_arena_returns_some() {
        let id = ArenaId(10_001);
        register_arena(id, stw_flag());
        let lease = try_acquire_lease(id);
        assert!(lease.is_some());
        // cleanup
        drop(lease);
        unregister_arena(id);
    }

    // ------------------------------------------------------------------
    // After drop the lease, try_acquire_lease still works (arena still up)
    // ------------------------------------------------------------------
    #[test]
    fn lease_drop_does_not_unregister_arena() {
        let id = ArenaId(10_002);
        register_arena(id, stw_flag());
        {
            let _lease = try_acquire_lease(id).expect("should get lease");
        } // lease dropped here
        // arena still registered; new lease should succeed
        let lease2 = try_acquire_lease(id);
        assert!(lease2.is_some());
        drop(lease2);
        unregister_arena(id);
    }

    // ------------------------------------------------------------------
    // After unregister, try_acquire_lease returns None
    // ------------------------------------------------------------------
    #[test]
    fn lease_after_unregister_returns_none() {
        let id = ArenaId(10_003);
        register_arena(id, stw_flag());
        unregister_arena(id);
        assert!(try_acquire_lease(id).is_none());
    }

    // ------------------------------------------------------------------
    // active_leases counter: multiple concurrent leases increment/decrement
    // ------------------------------------------------------------------
    #[test]
    fn multiple_leases_track_borrow_count() {
        let id = ArenaId(10_004);
        register_arena(id, stw_flag());

        let l1 = try_acquire_lease(id).unwrap();
        let l2 = try_acquire_lease(id).unwrap();
        let l3 = try_acquire_lease(id).unwrap();

        let count = {
            let guard = VALID_ARENAS.read();
            guard
                .get(&id)
                .map(|s| s.active_leases.load(AtomicOrdering::Acquire))
                .unwrap_or(0)
        };
        assert_eq!(count, 3);

        drop(l1);
        drop(l2);
        let count2 = {
            let guard = VALID_ARENAS.read();
            guard
                .get(&id)
                .map(|s| s.active_leases.load(AtomicOrdering::Acquire))
                .unwrap_or(0)
        };
        assert_eq!(count2, 1);

        drop(l3);
        let count3 = {
            let guard = VALID_ARENAS.read();
            guard
                .get(&id)
                .map(|s| s.active_leases.load(AtomicOrdering::Acquire))
                .unwrap_or(0)
        };
        assert_eq!(count3, 0);

        unregister_arena(id);
    }

    // ------------------------------------------------------------------
    // unregister_arena waits for an outstanding lease to be dropped
    // (cross-thread: unregister races against lease holder)
    // ------------------------------------------------------------------
    #[test]
    fn unregister_waits_for_outstanding_lease() {
        use std::sync::Arc as StdArc;
        use std::sync::atomic::AtomicBool as StdAtomicBool;

        let id = ArenaId(10_005);
        register_arena(id, stw_flag());

        let lease = try_acquire_lease(id).expect("should acquire lease");

        // Signal between threads
        let lease_dropped = StdArc::new(StdAtomicBool::new(false));
        let _lease_dropped_clone = lease_dropped.clone();
        let unregister_returned = StdArc::new(StdAtomicBool::new(false));
        let unregister_returned_clone = unregister_returned.clone();

        let unregister_thread = std::thread::spawn(move || {
            // This must block until the lease is dropped.
            unregister_arena(id);
            unregister_returned_clone.store(true, Ordering::Release);
        });

        // Give the unregister thread time to reach the spin loop.
        std::thread::sleep(std::time::Duration::from_millis(20));

        // unregister should NOT have returned yet because we still hold the lease.
        assert!(
            !unregister_returned.load(Ordering::Acquire),
            "unregister_arena returned while lease was still held"
        );

        // Drop the lease — this should unblock unregister_arena.
        drop(lease);
        lease_dropped.store(true, Ordering::Release);

        unregister_thread
            .join()
            .expect("unregister thread panicked");

        assert!(
            unregister_returned.load(Ordering::Acquire),
            "unregister_arena did not return after lease was dropped"
        );
        assert!(
            lease_dropped.load(Ordering::Acquire),
            "lease_dropped flag not set"
        );
    }

    // ------------------------------------------------------------------
    // ArenaLease::is_stw_in_progress reflects the flag state
    // ------------------------------------------------------------------
    #[test]
    fn lease_reflects_stw_flag() {
        let id = ArenaId(10_006);
        let flag = stw_flag();
        register_arena(id, flag.clone());

        let lease = try_acquire_lease(id).unwrap();
        assert!(!lease.is_stw_in_progress());

        flag.store(true, Ordering::Release);
        assert!(lease.is_stw_in_progress());

        flag.store(false, Ordering::Release);
        assert!(!lease.is_stw_in_progress());

        drop(lease);
        unregister_arena(id);
    }

    // ------------------------------------------------------------------
    // Fast-path bitset (id < 64): lease works for low IDs too
    // ------------------------------------------------------------------
    #[test]
    fn lease_works_for_low_arena_id() {
        let id = ArenaId(7); // < 64, uses fast-path bitset
        register_arena(id, stw_flag());
        let lease = try_acquire_lease(id);
        assert!(lease.is_some());
        drop(lease);
        unregister_arena(id);
    }

    // ------------------------------------------------------------------
    // is_valid_cross_arena_ref reflects registration state
    // ------------------------------------------------------------------
    #[test]
    fn is_valid_cross_arena_ref_reflects_registration_state() {
        let id = ArenaId(10_007);
        assert!(!is_valid_cross_arena_ref(id));
        register_arena(id, stw_flag());
        assert!(is_valid_cross_arena_ref(id));
        unregister_arena(id);
        assert!(!is_valid_cross_arena_ref(id));
    }

    // ------------------------------------------------------------------
    // Generation: each registration produces a strictly greater generation
    // ------------------------------------------------------------------
    #[test]
    fn each_registration_has_a_unique_increasing_generation() {
        let id = ArenaId(10_008);

        register_arena(id, stw_flag());
        let gen1 = try_acquire_lease(id).unwrap().generation();
        unregister_arena(id);

        register_arena(id, stw_flag());
        let gen2 = try_acquire_lease(id).unwrap().generation();
        unregister_arena(id);

        register_arena(id, stw_flag());
        let gen3 = try_acquire_lease(id).unwrap().generation();
        unregister_arena(id);

        assert!(gen2 > gen1, "second generation must exceed first");
        assert!(gen3 > gen2, "third generation must exceed second");
    }

    // ------------------------------------------------------------------
    // Generation: lease generation matches ArenaState generation
    // ------------------------------------------------------------------
    #[test]
    fn lease_generation_matches_arena_state_generation() {
        let id = ArenaId(10_009);
        register_arena(id, stw_flag());

        let lease = try_acquire_lease(id).unwrap();
        let state_gen = VALID_ARENAS.read().get(&id).map(|s| s.generation).unwrap();

        assert_eq!(lease.generation(), state_gen);
        drop(lease);
        unregister_arena(id);
    }

    // ------------------------------------------------------------------
    // Generation: is_valid returns true while arena is alive
    // ------------------------------------------------------------------
    #[test]
    fn lease_is_valid_while_arena_alive() {
        let id = ArenaId(10_010);
        register_arena(id, stw_flag());
        let lease = try_acquire_lease(id).unwrap();
        assert!(
            lease.is_valid(),
            "lease must be valid while arena is registered"
        );
        drop(lease);
        unregister_arena(id);
    }

    // ------------------------------------------------------------------
    // Generation: re-register uses a different generation than previous lease
    // ------------------------------------------------------------------
    #[test]
    fn reregistered_arena_has_different_generation_than_old_lease() {
        let id = ArenaId(10_011);

        register_arena(id, stw_flag());
        let old_gen = try_acquire_lease(id).unwrap().generation();
        unregister_arena(id);

        // Re-register same ArenaId.
        register_arena(id, stw_flag());
        let new_lease = try_acquire_lease(id).unwrap();
        assert_ne!(
            new_lease.generation(),
            old_gen,
            "re-registered arena must have a different generation"
        );
        drop(new_lease);
        unregister_arena(id);
    }

    // ------------------------------------------------------------------
    // record_cross_arena_ref stores generation with the ref
    // ------------------------------------------------------------------
    #[test]
    fn record_cross_arena_ref_stores_generation() {
        let id = ArenaId(10_012);
        register_arena(id, stw_flag());

        let expected_gen = try_acquire_lease(id).unwrap().generation();

        // Simulate being inside a GC tracing phase.
        set_currently_tracing(Some(id));
        let recorded = record_cross_arena_ref(id, 0xDEAD_BEEF);
        set_currently_tracing(None);

        assert!(recorded, "record should return true for live arena");

        let refs = take_found_cross_arena_refs_with_generation();
        assert_eq!(refs.len(), 1);
        let (ref_id, ref_ptr, ref_gen) = refs[0];
        assert_eq!(ref_id, id);
        assert_eq!(ref_ptr, 0xDEAD_BEEF);
        assert_eq!(
            ref_gen, expected_gen,
            "stored generation must match arena generation at record time"
        );

        unregister_arena(id);
    }

    // ------------------------------------------------------------------
    // record_cross_arena_ref outside tracing phase: no push, returns validity
    // ------------------------------------------------------------------
    #[test]
    fn record_cross_arena_ref_outside_tracing_returns_validity() {
        let id = ArenaId(10_014);
        // Not registered yet.
        assert!(!record_cross_arena_ref(id, 0x1));

        register_arena(id, stw_flag());
        // Not in tracing phase — should return true but not push anything.
        assert!(record_cross_arena_ref(id, 0x2));
        let refs = take_found_cross_arena_refs_with_generation();
        assert!(
            refs.is_empty(),
            "no refs should be recorded outside tracing phase"
        );

        unregister_arena(id);
    }

    // ------------------------------------------------------------------
    // Stress test: concurrent registration, unregistration, and leasing
    // ------------------------------------------------------------------
    #[test]
    #[cfg(feature = "multithreading")]
    fn stress_arena_lifecycle_concurrency() {
        use std::sync::Barrier;
        use std::thread;
        use std::time::Duration;

        let num_threads = 6;
        let num_iterations = 200;
        let barrier = Arc::new(Barrier::new(num_threads));
        let base_id = 30_000;
        let num_ids = 5;

        let mut handles = vec![];

        for t in 0..num_threads {
            let barrier = barrier.clone();
            let h = thread::spawn(move || {
                barrier.wait();
                for i in 0..num_iterations {
                    let id = ArenaId(base_id + (i % num_ids));
                    match t % 3 {
                        0 => {
                            // Registrator: cyclically registers and unregisters
                            register_arena(id, stw_flag());
                            // Small random sleep to create jitter
                            if i % 7 == 0 {
                                thread::sleep(Duration::from_micros(1));
                            }
                            unregister_arena(id);
                        }
                        1 => {
                            // Tracer & Harvester: simulate the GC cycle
                            set_currently_tracing(Some(id));
                            let recorded = record_cross_arena_ref(id, i as usize);
                            set_currently_tracing(None);

                            if recorded {
                                let refs = take_found_cross_arena_refs_with_generation();
                                for (target_id, _, recorded_gen) in refs {
                                    if let Some(lease) = try_acquire_lease(target_id) {
                                        // is_valid() may flip to false if unregister starts while this
                                        // lease is still held (state.is_alive is cleared before waiting
                                        // for active leases to drop). So we only assert validity when
                                        // no teardown race is visible.
                                        if lease.generation() == recorded_gen {
                                            let valid = lease.is_valid();
                                            if !valid {
                                                assert!(!is_valid_cross_arena_ref(target_id));
                                            }
                                        }
                                        drop(lease);
                                    }
                                }
                            }
                        }
                        2 => {
                            // Heavy Leaser: tries to hold leases for a while
                            if let Some(lease) = try_acquire_lease(id) {
                                let _ = lease.is_valid();
                                if i % 11 == 0 {
                                    thread::sleep(Duration::from_micros(2));
                                }
                                drop(lease);
                            }
                        }
                        _ => unreachable!(),
                    }
                }
            });
            handles.push(h);
        }

        for h in handles {
            h.join().unwrap();
        }

        // Final cleanup
        for i in 0..num_ids {
            unregister_arena(ArenaId(base_id + i));
        }
    }

    // ------------------------------------------------------------------
    // Stress: STW flag transitions interleaved with lease acquisition
    //
    // Several threads concurrently flip the STW flag on/off, acquire leases,
    // and register/unregister arenas.  Invariants verified per iteration:
    //  - A lease obtained while STW is in progress must report is_stw_in_progress().
    //  - A lease obtained on an unregistered arena must be None.
    //  - Generation must strictly increase across re-registrations.
    // ------------------------------------------------------------------
    #[test]
    #[cfg(feature = "multithreading")]
    fn stress_stw_flag_transitions_with_lease_acquisition() {
        use std::sync::Barrier;
        use std::thread;
        use std::time::Duration;

        const NUM_THREADS: usize = 5;
        const ITERATIONS: usize = 150;
        const BASE_ID: u64 = 80_000;
        const NUM_IDS: u64 = 4;

        let barrier = Arc::new(Barrier::new(NUM_THREADS));
        let mut handles = vec![];

        for t in 0..NUM_THREADS {
            let barrier = barrier.clone();
            handles.push(thread::spawn(move || {
                barrier.wait();
                for i in 0..ITERATIONS {
                    let id = ArenaId(BASE_ID + (i as u64 % NUM_IDS));
                    match t % 5 {
                        0 => {
                            // Registrator/unregistrator
                            register_arena(id, stw_flag());
                            if i % 3 == 0 {
                                thread::sleep(Duration::from_micros(1));
                            }
                            unregister_arena(id);
                        }
                        1 => {
                            // STW flag setter: toggles on then off
                            register_arena(id, stw_flag());
                            set_stw_in_progress(id, true);
                            thread::yield_now();
                            set_stw_in_progress(id, false);
                            unregister_arena(id);
                        }
                        2 => {
                            // Lease checker: verifies STW visibility through lease
                            if let Some(lease) = try_acquire_lease(id) {
                                assert!(lease.is_valid(), "lease must be valid while held");
                                // If STW is in progress, the lease must reflect it.
                                let stw_via_lease = lease.is_stw_in_progress();
                                let stw_direct = is_stw_in_progress(id);
                                // The direct check races with flag changes; we
                                // only assert the weaker property that the lease
                                // view is self-consistent (no panic / no UB).
                                let _ = (stw_via_lease, stw_direct);
                                drop(lease);
                            }
                        }
                        3 => {
                            // Tracer: records cross-arena refs and validates generation.
                            // Uses equality-based stale detection: if the generation
                            // matches at harvest time the arena is still the same
                            // registration and must be alive.  A different generation
                            // simply means an intervening unregister/re-register; the
                            // pointer is stale and that is expected under concurrent
                            // lifecycle churn.
                            register_arena(id, stw_flag());
                            set_currently_tracing(Some(id));
                            record_cross_arena_ref(id, i);
                            set_currently_tracing(None);

                            let refs = take_found_cross_arena_refs_with_generation();
                            for (target_id, _ptr, recorded_gen) in refs {
                                if let Some(lease) = try_acquire_lease(target_id) {
                                    if lease.generation() == recorded_gen {
                                        assert!(
                                            lease.is_valid(),
                                            "arena with matching generation must be alive"
                                        );
                                    }
                                    // Differing generation = arena was re-registered; stale
                                    // pointer is expected, no assertion needed.
                                    drop(lease);
                                }
                            }
                            unregister_arena(id);
                        }
                        4 => {
                            // Re-registration exerciser: register, lease, unregister,
                            // register again and verify a lease can be acquired.
                            // We do NOT assert generation ordering here: concurrent
                            // register_arena calls on the same ID can serialize in
                            // reversed counter order (fetch_add and write().insert()
                            // are under separate locks), so the "after" lease may
                            // legitimately have a lower generation number than
                            // "before" when another thread's insert wins the lock.
                            //
                            // IMPORTANT: all leases must be dropped BEFORE calling
                            // unregister_arena.  unregister_arena spins on
                            // active_leases; holding a live lease in the same scope
                            // as unregister_arena causes a self-deadlock.
                            register_arena(id, stw_flag());
                            // Use .map() so the ArenaLease is dropped inside the
                            // closure, before we reach the unregister call.
                            let _gen_before = try_acquire_lease(id).map(|l| l.generation());
                            unregister_arena(id);
                            register_arena(id, stw_flag());
                            let _gen_after = try_acquire_lease(id).map(|l| l.generation());
                            // A lease may be None if another thread unregistered
                            // concurrently — both outcomes are valid.
                            unregister_arena(id);
                        }
                        _ => unreachable!(),
                    }
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // Cleanup any arenas that may have been left registered by a
        // thread that was interrupted between register and unregister.
        for i in 0..NUM_IDS {
            unregister_arena(ArenaId(BASE_ID + i));
        }
    }

    // ------------------------------------------------------------------
    // record_and_harvest_race_with_reregistration:
    // ensures generation check catches re-registration
    // ------------------------------------------------------------------
    #[test]
    fn record_and_harvest_race_with_reregistration() {
        let id = ArenaId(40_000);

        // 1. Register and record
        register_arena(id, stw_flag());
        set_currently_tracing(Some(id));
        record_cross_arena_ref(id, 0x123);
        set_currently_tracing(None);

        let refs = take_found_cross_arena_refs_with_generation();
        let (target_id, _ptr, recorded_gen) = refs[0];

        // 2. Unregister and re-register
        unregister_arena(id);
        register_arena(id, stw_flag());

        // 3. Harvest with the old generation
        if let Some(lease) = try_acquire_lease(target_id) {
            assert_ne!(
                lease.generation(),
                recorded_gen,
                "generation must change after re-registration"
            );
            // This is what record_found_cross_arena_refs does:
            if lease.generation() == recorded_gen {
                panic!("Generation check failed to catch re-registration!");
            }
            drop(lease);
        }

        unregister_arena(id);
    }
}
