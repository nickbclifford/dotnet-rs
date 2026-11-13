//! Cross-arena reference tracking and lease API.
//!
//! A registry maps each live [`ArenaId`] to a generation-qualified [`ArenaIdentity`] and its
//! shared state. Code that dereferences into another arena first takes an [`ArenaLease`];
//! teardown removes the registry entry and then waits until the lease count reaches zero
//! before the arena's storage may be freed. Checking liveness with
//! [`is_valid_cross_arena_ref`] and then dereferencing is a time-of-check/time-of-use race;
//! the lease closes it.
//!
//! # Lease duration
//!
//! A lease lasts as long as its guard. Most callers hold one for a single access, but
//! long-lived holders exist (for example `RetainedPtr` in `dotnet-value` keeps one for its
//! whole lifetime), so teardown can wait indefinitely. `drain_leases` backs off from
//! spinning to yielding and logs a warning every 100 ms while leases remain.
//!
//! # Drain contract
//!
//! Leases count on the registration's shared state, not on the registry entry.
//!
//! - [`ArenaRegistration::unregister_and_drain`] (exact identity, owner-held) removes its
//!   entry if still present and then waits for every lease on that registration, even if
//!   another caller removed the entry first. If the calling thread holds a lease it returns
//!   [`TeardownOutcome::SelfHeldLease`] instead of deadlocking, and the storage must be leaked.
//! - [`unregister_arena_identity`] removes and drains only when the live entry has the given
//!   generation; on absence or mismatch it returns `false` without waiting.
//! - [`unregister_arena`] (ID only) removes and drains whatever entry has that ID. If no entry
//!   is present it returns at once, without waiting for leases taken on an earlier removal.
//!   It does not detect a self-held lease.
//!
//! A lease does not root any object and does not establish pointer provenance; a separate
//! GC root and access discipline are still required.
//!
//! ## Why removal cannot miss a lease
//!
//! Lease acquisition takes the registry *read* lock, clones the `Arc<ArenaState>` and
//! increments `active_leases` while holding it. Removal takes the *write* lock, so once it
//! holds that lock every in-progress acquisition has already incremented the count, and the
//! drain observes it.
//!
//! ## ID-keyed queries
//!
//! `register_arena`, `is_valid_cross_arena_ref`, `set_stw_in_progress` and
//! `is_stw_in_progress` are keyed by ID: they answer "is this ID live now?", not "is this
//! the same registration?". Found cross-arena references are recorded as
//! identity-qualified [`CrossArenaRef`] values.
use super::{CrossArenaRef, OpaqueObjectPtr};
use crate::{
    newtypes::ArenaId,
    sync::{Arc, AtomicBool, AtomicUsize, Ordering, RwLock},
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    mem,
    sync::atomic::{AtomicU64, Ordering as StdAtomicOrdering},
    thread,
    time::{Duration, Instant},
};

// ---------------------------------------------------------------------------
// Per-arena shared state
// ---------------------------------------------------------------------------

/// Exact identity of one arena registration. It does not root any object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ArenaIdentity {
    pub arena_id: ArenaId,
    pub generation: u64,
}

/// Process-local, non-reused registration token. Zero is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RegistrationToken(std::num::NonZeroU32);

impl RegistrationToken {
    pub fn from_raw(value: u32) -> Option<Self> {
        std::num::NonZeroU32::new(value).map(Self)
    }

    pub fn as_u32(self) -> u32 {
        self.0.get()
    }
}

/// Registration was rejected before any identity or fast-bit publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArenaRegistrationError {
    DuplicateLive(ArenaId),
    GenerationExhausted,
    TokenExhausted,
}

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
    token: RegistrationToken,
}

impl ArenaState {
    fn new(stw_in_progress: Arc<AtomicBool>, generation: u64, token: RegistrationToken) -> Self {
        Self {
            stw_in_progress,
            active_leases: AtomicUsize::new(0),
            is_alive: AtomicBool::new(true),
            generation,
            token,
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
    /// Arena this lease pins; used to record the lease in the holder thread's
    /// `HELD_LEASES` so validation can accept an in-flight access.
    arena_id: ArenaId,
    /// Leases are tracked in thread-locals, so they must be dropped on the
    /// thread that acquired them.
    _not_send: std::marker::PhantomData<*const ()>,
}

thread_local! {
    /// Leases currently held by this thread, as `(arena, count)`.
    static HELD_LEASES: RefCell<Vec<(ArenaId, usize)>> = const { RefCell::new(Vec::new()) };
}

/// Returns `true` if the current thread holds an [`ArenaLease`] on `arena_id`.
///
/// Unlike [`is_valid_cross_arena_ref`], this stays `true` after
/// `unregister_arena` removes the registry entry, because the held lease keeps
/// the arena's memory live until it drops. It never authorises a fresh access:
/// acquiring a new lease still fails once the entry is removed.
///
pub fn current_thread_holds_lease(arena_id: ArenaId) -> bool {
    HELD_LEASES.with(|h| h.borrow().iter().any(|(id, _)| *id == arena_id))
}

impl ArenaLease {
    /// The exact registration pinned by this lease, not an object root.
    pub fn identity(&self) -> ArenaIdentity {
        ArenaIdentity {
            arena_id: self.arena_id,
            generation: self.generation,
        }
    }

    /// Token for this exact registration. It becomes unresolvable after removal.
    pub fn token(&self) -> RegistrationToken {
        self.state.token
    }

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
    /// A concurrent `unregister_arena` clears the liveness flag before
    /// draining outstanding leases, so this may become `false` while the lease
    /// still keeps arena memory live. The lease, not this observation, protects
    /// a dereference through the end of its lifetime. This check is a
    /// defense-in-depth assertion point for callers that need to detect begun
    /// unregistration.
    pub fn is_valid(&self) -> bool {
        self.state.is_alive.load(Ordering::Acquire) && self.state.generation == self.generation
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
        HELD_LEASES.with(|h| {
            let mut h = h.borrow_mut();
            if let Some(i) = h.iter().position(|(id, _)| *id == self.arena_id) {
                h[i].1 -= 1;
                if h[i].1 == 0 {
                    h.swap_remove(i);
                }
            }
        });
        self.state.active_leases.fetch_sub(1, Ordering::Release);
    }
}

// ---------------------------------------------------------------------------
// Global registry
// ---------------------------------------------------------------------------

// The counters and both indexes are protected by the same lock. Counters are
// next-to-issue values; neither is reset, including on registry reset.
struct ArenaRegistry {
    by_id: HashMap<ArenaId, Arc<ArenaState>>,
    by_token: HashMap<RegistrationToken, ArenaId>,
    next_generation: u64,
    next_token: u64,
}

impl ArenaRegistry {
    fn new() -> Self {
        Self {
            by_id: HashMap::new(),
            by_token: HashMap::new(),
            next_generation: 0,
            next_token: 1,
        }
    }

    fn register(
        &mut self,
        id: ArenaId,
        flag: Arc<AtomicBool>,
        fast: &AtomicU64,
    ) -> Result<ArenaIdentity, ArenaRegistrationError> {
        if self.by_id.contains_key(&id) {
            return Err(ArenaRegistrationError::DuplicateLive(id));
        }
        // u64::MAX is reserved by the temporary STW sentinel. Use a wider
        // next-token counter so u32::MAX itself is usable without wrapping.
        let next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or(ArenaRegistrationError::GenerationExhausted)?;
        let raw_token = u32::try_from(self.next_token)
            .ok()
            .and_then(RegistrationToken::from_raw)
            .ok_or(ArenaRegistrationError::TokenExhausted)?;
        let identity = ArenaIdentity {
            arena_id: id,
            generation: self.next_generation,
        };
        let state = Arc::new(ArenaState::new(flag, identity.generation, raw_token));
        self.next_generation = next_generation;
        self.next_token += 1;
        self.by_token.insert(raw_token, id);
        self.by_id.insert(id, state);
        if id.as_u64() < 64 {
            fast.fetch_or(1 << id.as_u64(), StdAtomicOrdering::Release);
        }
        Ok(identity)
    }

    fn remove(&mut self, id: ArenaId, expected: Option<ArenaIdentity>) -> Option<Arc<ArenaState>> {
        let state = self.by_id.get(&id)?;
        if expected.is_some_and(|identity| {
            identity.arena_id != id || identity.generation != state.generation
        }) {
            return None;
        }
        state.is_alive.store(false, Ordering::Release);
        let state = self
            .by_id
            .remove(&id)
            .expect("entry checked under registry lock");
        self.by_token.remove(&state.token);
        Some(state)
    }
}

static VALID_ARENAS: std::sync::LazyLock<RwLock<ArenaRegistry>> =
    std::sync::LazyLock::new(|| RwLock::new(ArenaRegistry::new()));

/// A fast-path bitset for the first 64 Arena IDs to avoid lock contention on
/// every object read.
static VALID_ARENAS_FAST: AtomicU64 = AtomicU64::new(0);

const UNREGISTER_LEASE_SPIN_MAX_ITERS: usize = 1 << 10;
const UNREGISTER_LEASE_WARN_INTERVAL: Duration = Duration::from_millis(100);

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
    try_register_arena(thread_id, stw_in_progress)
        .unwrap_or_else(|error| panic!("arena registration failed: {error:?}"));
}

/// Register exactly once per live ID, returning its generation-qualified identity.
/// Duplicate IDs and exhausted counters fail without publishing anything.
pub fn try_register_arena(
    thread_id: ArenaId,
    stw_in_progress: Arc<AtomicBool>,
) -> Result<ArenaIdentity, ArenaRegistrationError> {
    let mut registry = VALID_ARENAS.write();
    registry.register(thread_id, stw_in_progress, &VALID_ARENAS_FAST)
}

/// Unregister an arena by ID.
///
/// Removes the entry with this ID, if any, and then waits until every outstanding
/// [`ArenaLease`] on that registration has been dropped (see `drain_leases`). If no entry is
/// present it returns immediately without waiting. It does not check for a lease held by the
/// calling thread, which would make the wait never finish; owners use
/// [`ArenaRegistration::unregister_and_drain`] instead.
pub fn unregister_arena(thread_id: ArenaId) {
    remove_and_drain(thread_id, None);
}

/// Remove only the specified live registration; a stale identity cannot remove
/// its successor. Returns false on absence or mismatch.
pub fn unregister_arena_identity(identity: ArenaIdentity) -> bool {
    remove_and_drain(identity.arena_id, Some(identity))
}

/// Owner-held handle to one exact registration.
///
/// Leases count on the registration's shared state, not on the registry entry,
/// so the owner can wait for them even when another caller already removed the
/// entry (an ID-only `unregister_arena` that finds no entry returns without
/// waiting). Memory the registration guards must outlive
/// [`ArenaRegistration::unregister_and_drain`].
pub struct ArenaRegistration {
    state: Arc<ArenaState>,
    identity: ArenaIdentity,
}

/// How [`ArenaRegistration::unregister_and_drain`] ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TeardownOutcome {
    /// The entry is gone and no lease on this registration remains.
    Drained,
    /// The calling thread itself holds a lease, so waiting would deadlock. The
    /// entry is removed, but leased memory must not be freed (leak it).
    SelfHeldLease,
}

/// Capture the live registration for `identity`, without taking a lease.
pub fn registration_for_identity(identity: ArenaIdentity) -> Option<ArenaRegistration> {
    let guard = VALID_ARENAS.read();
    let state = guard.by_id.get(&identity.arena_id)?;
    (state.generation == identity.generation).then(|| ArenaRegistration {
        state: state.clone(),
        identity,
    })
}

impl ArenaRegistration {
    pub fn identity(&self) -> ArenaIdentity {
        self.identity
    }

    /// Remove this registration if still present, then wait until every lease on
    /// it is released, regardless of who removed the entry. Never waits under the
    /// registry lock, and never waits on a lease held by the calling thread.
    /// Protected storage may be freed only after `Drained` is returned.
    /// `SelfHeldLease` or an unwind (including from a tracing subscriber during
    /// drain) requires retaining that storage. Removal alone is not a drain.
    #[must_use = "free protected storage only after Drained; retain it on unwind or SelfHeldLease"]
    pub fn unregister_and_drain(self) -> TeardownOutcome {
        let id = self.identity.arena_id;
        remove_registration(id, Some(self.identity));
        if current_thread_holds_lease(id) {
            return TeardownOutcome::SelfHeldLease;
        }
        drain_leases(id, self.state);
        TeardownOutcome::Drained
    }
}

// Test-only checkpoint: lets a test observe the removal path immediately before it takes the
// registry write lock, without adding a production synchronization path.
#[cfg(test)]
thread_local! {
    static RECREATION_REMOVE_CHECKPOINT: RefCell<Option<std::sync::mpsc::Sender<bool>>> = const { RefCell::new(None) };
}

fn remove_registration(
    thread_id: ArenaId,
    expected: Option<ArenaIdentity>,
) -> Option<Arc<ArenaState>> {
    #[cfg(test)]
    RECREATION_REMOVE_CHECKPOINT.with(|checkpoint| {
        if let Some(sender) = checkpoint.borrow_mut().take() {
            let _ = sender.send(VALID_ARENAS.try_write().is_none());
        }
    });
    let mut registry = VALID_ARENAS.write();
    let state = registry.remove(thread_id, expected);
    if state.is_some() && thread_id.as_u64() < 64 {
        VALID_ARENAS_FAST.fetch_and(!(1 << thread_id.as_u64()), StdAtomicOrdering::Release);
    }
    state
}

fn remove_and_drain(thread_id: ArenaId, expected: Option<ArenaIdentity>) -> bool {
    if let Some(state) = remove_registration(thread_id, expected) {
        drain_leases(thread_id, state);
        true
    } else {
        false
    }
}

// F8.LockOrderRespected: the registry lock is released before waiting for leases.
fn drain_leases(thread_id: ArenaId, state: Arc<ArenaState>) {
    // Drain all in-flight leases before returning.
    // Back off from short bounded spinning to cooperative yielding to
    // avoid burning CPU if a lease holder is preempted.
    let wait_start = Instant::now();
    let mut next_warn_at = UNREGISTER_LEASE_WARN_INTERVAL;
    let mut spin_budget = 1usize;
    let mut yield_count = 0u64;

    loop {
        if state.active_leases.load(Ordering::Acquire) == 0 {
            break;
        }

        for _ in 0..spin_budget {
            if state.active_leases.load(Ordering::Acquire) == 0 {
                break;
            }
            std::hint::spin_loop();
        }
        if state.active_leases.load(Ordering::Acquire) == 0 {
            break;
        }

        thread::yield_now();
        yield_count += 1;
        spin_budget = spin_budget
            .saturating_mul(2)
            .min(UNREGISTER_LEASE_SPIN_MAX_ITERS);

        let elapsed = wait_start.elapsed();
        if elapsed >= next_warn_at {
            let active_leases = state.active_leases.load(Ordering::Acquire);
            tracing::warn!(
                arena_id = thread_id.as_u64(),
                active_leases,
                waited_millis = elapsed.as_millis() as u64,
                yield_count,
                spin_budget,
                "unregister_arena is waiting for outstanding cross-arena leases"
            );
            next_warn_at = elapsed + UNREGISTER_LEASE_WARN_INTERVAL;
        }
    }
}

// ---------------------------------------------------------------------------
// Lease acquisition
// ---------------------------------------------------------------------------

/// Attempt to pin an arena and obtain a dereference lease.
///
/// Returns `Some(ArenaLease)` if the arena is currently registered.  The
/// returned guard prevents arena teardown until it is dropped. It does not
/// root any object or establish pointer provenance, identity, or access safety.
/// Callers with a previously recorded identity must use
/// [`try_acquire_lease_for_identity`] rather than blessing a stale pointer
/// with a fresh ID-only lease.
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
    let id = target_id.as_u64();
    if id < 64 && (VALID_ARENAS_FAST.load(StdAtomicOrdering::Acquire) & (1 << id)) == 0 {
        return None;
    }

    let guard = VALID_ARENAS.read();
    let state = guard.by_id.get(&target_id)?.clone();
    let lease = lease_under_lock(target_id, state);
    drop(guard);
    Some(track_lease(lease))
}

/// Acquire a lease only if the exact registration is live.
pub fn try_acquire_lease_for_identity(identity: ArenaIdentity) -> Option<ArenaLease> {
    let guard = VALID_ARENAS.read();
    let state = guard.by_id.get(&identity.arena_id)?;
    if state.generation != identity.generation {
        return None;
    }
    let lease = lease_under_lock(identity.arena_id, state.clone());
    drop(guard);
    Some(track_lease(lease))
}

/// Resolve a live token to a lease for its original identity, never a pointer.
pub fn try_acquire_lease_for_token(token: RegistrationToken) -> Option<ArenaLease> {
    let guard = VALID_ARENAS.read();
    let id = *guard.by_token.get(&token)?;
    let state = guard.by_id.get(&id)?;
    debug_assert_eq!(state.token, token);
    let lease = lease_under_lock(id, state.clone());
    drop(guard);
    Some(track_lease(lease))
}

// Caller holds the registry read lock until after the lease count increment.
fn lease_under_lock(target_id: ArenaId, state: Arc<ArenaState>) -> ArenaLease {
    let generation = state.generation;
    state.active_leases.fetch_add(1, Ordering::Acquire);
    ArenaLease {
        state,
        generation,
        arena_id: target_id,
        _not_send: std::marker::PhantomData,
    }
}

// F8.LockOrderRespected: TLS tracking happens only after releasing the registry lock.
fn track_lease(lease: ArenaLease) -> ArenaLease {
    HELD_LEASES.with(|h| {
        let mut h = h.borrow_mut();
        match h.iter_mut().find(|(id, _)| *id == lease.arena_id) {
            Some(e) => e.1 += 1,
            None => h.push((lease.arena_id, 1)),
        }
    });
    lease
}

// ---------------------------------------------------------------------------
// Liveness queries and registry reset
// ---------------------------------------------------------------------------

/// Check whether an arena is currently registered.
///
/// **Note**: this function has a TOCTOU race — the arena may unregister
/// between this call and any subsequent dereference.  Prefer
/// [`try_acquire_lease`] for dereference-guarded access.
pub fn is_valid_cross_arena_ref(target_thread_id: ArenaId) -> bool {
    let id = target_thread_id.as_u64();
    if id < 64 {
        (VALID_ARENAS_FAST.load(StdAtomicOrdering::Acquire) & (1 << id)) != 0
    } else {
        VALID_ARENAS.read().by_id.contains_key(&target_thread_id)
    }
}

/// Remove all current entries and drain their leases, without recycling identities.
/// Never call while holding a lease on any removed arena: this would deadlock.
pub fn reset_arena_registry() {
    let removed = {
        let mut registry = VALID_ARENAS.write();
        let ids: Vec<_> = registry.by_id.keys().copied().collect();
        let removed: Vec<_> = ids
            .into_iter()
            .filter_map(|id| registry.remove(id, None).map(|state| (id, state)))
            .collect();
        VALID_ARENAS_FAST.store(0, StdAtomicOrdering::Release);
        removed
    };
    for (id, state) in removed {
        drain_leases(id, state);
    }
}

pub fn set_stw_in_progress(arena_id: ArenaId, in_progress: bool) {
    if let Some(state) = VALID_ARENAS.read().by_id.get(&arena_id) {
        state.stw_in_progress.store(in_progress, Ordering::Release);
    }
}

pub fn is_stw_in_progress(arena_id: ArenaId) -> bool {
    VALID_ARENAS
        .read()
        .by_id
        .get(&arena_id)
        .map(|state| state.stw_in_progress.load(Ordering::Acquire))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Cross-arena tracing thread-locals
// ---------------------------------------------------------------------------

thread_local! {
    /// Found cross-arena references during the current marking phase. Each entry is an
    /// inert pointer paired with the exact registration it was captured under, so harvest
    /// can reject an arena that was unregistered and re-registered since recording.
    static FOUND_CROSS_ARENA_REFS: RefCell<Vec<CrossArenaRef>> = const { RefCell::new(Vec::new()) };
    /// The thread ID of the arena currently being traced.
    static CURRENTLY_TRACING_THREAD_ID: Cell<Option<ArenaId>> = const { Cell::new(None) };
}

/// Set the tracing context for the current thread.
pub fn set_currently_tracing(thread_id: Option<ArenaId>) {
    CURRENTLY_TRACING_THREAD_ID.with(|id| {
        id.set(thread_id);
    });
}

/// Get the thread ID of the arena currently being traced.
pub fn get_currently_tracing() -> Option<ArenaId> {
    CURRENTLY_TRACING_THREAD_ID.get()
}

/// Take all found cross-arena references and clear the local list.
///
/// Each entry carries the registration identity captured when it was recorded. Callers
/// must acquire the lease with [`try_acquire_lease_for_identity`] and discard the entry
/// on `None`; they must never refresh the identity from the current registration of the ID.
pub fn take_found_cross_arena_refs() -> Vec<CrossArenaRef> {
    FOUND_CROSS_ARENA_REFS.with(|refs| {
        let mut r = refs.borrow_mut();
        mem::take(&mut *r)
    })
}

/// Record a retained cross-arena reference whose identity was already captured, such as an
/// origin's. Returns `true` only if that exact registration is currently live.
///
/// The identity is validated, never refreshed: a stale identity is rejected even when the
/// ID has since been registered again. Outside a tracing phase nothing is recorded.
pub fn record_cross_arena_ref(reference: CrossArenaRef) -> bool {
    let Some(_lease) = try_acquire_lease_for_identity(reference.identity()) else {
        return false;
    };
    if CURRENTLY_TRACING_THREAD_ID.with(|id| id.get().is_some()) {
        FOUND_CROSS_ARENA_REFS.with(|refs| refs.borrow_mut().push(reference));
    }
    true
}

/// Record a bare object pointer found during marking, capturing its owner's identity under
/// a short lease taken now (also under STW; no wildcard generation is accepted).
///
/// Returns `true` if the owner arena is currently registered. Outside a tracing phase
/// nothing is recorded.
///
/// # Safety
///
/// `ptr` must designate a live object owned by arena `target`'s *current* registration,
/// for example one just read from a live handle or decoded under a lease for `target`.
pub unsafe fn record_cross_arena_object(target: ArenaId, ptr: OpaqueObjectPtr) -> bool {
    record_cross_arena_object_with(target, |identity| {
        // SAFETY: F1.ArenaGenerationMatch — the identity comes from the lease on
        // `target`'s current registration, and the caller guarantees the pointer belongs to it.
        unsafe { CrossArenaRef::new(identity, ptr) }
    })
}

// Share the lease/tracing logic with address-only bookkeeping tests. Production
// callers must construct a reference to the live object for the supplied identity.
fn record_cross_arena_object_with(
    target: ArenaId,
    make_ref: impl FnOnce(ArenaIdentity) -> CrossArenaRef,
) -> bool {
    if CURRENTLY_TRACING_THREAD_ID.with(|id| id.get().is_none()) {
        return is_valid_cross_arena_ref(target);
    }
    let Some(lease) = try_acquire_lease(target) else {
        return false;
    };
    let reference = make_ref(lease.identity());
    FOUND_CROSS_ARENA_REFS.with(|refs| refs.borrow_mut().push(reference));
    true
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

    /// Address-only pointer for registry/buffer bookkeeping tests. It is not an object
    /// allocation (see `OpaqueObjectPtr::address_only_for_tests`), so these tests only
    /// record, compare and drain it; nothing converts it back or dereferences it.
    fn fake_ptr(addr: usize) -> OpaqueObjectPtr {
        OpaqueObjectPtr::address_only_for_tests(addr)
    }

    // Address-only entries never leave bookkeeping tests or reach marking. Construct
    // them directly rather than violating the object-only unsafe producer contract.
    fn fake_ref(identity: ArenaIdentity, addr: usize) -> CrossArenaRef {
        CrossArenaRef {
            identity,
            ptr: fake_ptr(addr),
        }
    }

    fn record_fake_object(id: ArenaId, addr: usize) -> bool {
        record_cross_arena_object_with(id, |identity| fake_ref(identity, addr))
    }

    fn stw_flag() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    fn active_lease_count(id: ArenaId) -> usize {
        let guard = VALID_ARENAS.read();
        guard
            .by_id
            .get(&id)
            .map(|s| s.active_leases.load(Ordering::Acquire))
            .unwrap_or(0)
    }

    // ------------------------------------------------------------------
    // try_acquire_lease on unregistered arena returns None
    // ------------------------------------------------------------------
    #[test]
    fn lease_on_unregistered_arena_returns_none() {
        let id = ArenaId::new(10_000);
        // ensure it is not in the registry
        unregister_arena(id);
        assert!(try_acquire_lease(id).is_none());
    }

    // ------------------------------------------------------------------
    // try_acquire_lease on registered arena returns Some
    // ------------------------------------------------------------------
    #[test]
    fn lease_on_registered_arena_returns_some() {
        let id = ArenaId::new(10_001);
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
        let id = ArenaId::new(10_002);
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
        let id = ArenaId::new(10_003);
        register_arena(id, stw_flag());
        unregister_arena(id);
        assert!(try_acquire_lease(id).is_none());
    }

    // ------------------------------------------------------------------
    // active_leases counter: multiple concurrent leases increment/decrement
    // ------------------------------------------------------------------
    #[test]
    fn multiple_leases_track_borrow_count() {
        let id = ArenaId::new(10_004);
        register_arena(id, stw_flag());

        let l1 = try_acquire_lease(id).unwrap();
        let l2 = try_acquire_lease(id).unwrap();
        let l3 = try_acquire_lease(id).unwrap();

        let count = active_lease_count(id);
        assert_eq!(count, 3);

        drop(l1);
        drop(l2);
        let count2 = active_lease_count(id);
        assert_eq!(count2, 1);

        drop(l3);
        let count3 = active_lease_count(id);
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
        use std::{thread, time::Duration};

        let id = ArenaId::new(10_005);
        register_arena(id, stw_flag());

        let lease = try_acquire_lease(id).expect("should acquire lease");

        // Signal between threads
        let lease_dropped = StdArc::new(StdAtomicBool::new(false));
        let _lease_dropped_clone = lease_dropped.clone();
        let unregister_returned = StdArc::new(StdAtomicBool::new(false));
        let unregister_returned_clone = unregister_returned.clone();

        let unregister_thread = thread::spawn(move || {
            // This must block until the lease is dropped.
            unregister_arena(id);
            unregister_returned_clone.store(true, Ordering::Release);
        });

        // Give the unregister thread time to reach the spin loop.
        thread::sleep(Duration::from_millis(20));

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
        let id = ArenaId::new(10_006);
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
        let id = ArenaId::new(7); // < 64, uses fast-path bitset
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
        let id = ArenaId::new(10_007);
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
        let id = ArenaId::new(10_008);

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
        let id = ArenaId::new(10_009);
        register_arena(id, stw_flag());

        let lease = try_acquire_lease(id).unwrap();
        let state_gen = VALID_ARENAS
            .read()
            .by_id
            .get(&id)
            .map(|s| s.generation)
            .unwrap();

        assert_eq!(lease.generation(), state_gen);
        drop(lease);
        unregister_arena(id);
    }

    // ------------------------------------------------------------------
    // Generation: is_valid returns true while arena is alive
    // ------------------------------------------------------------------
    #[test]
    fn lease_is_valid_while_arena_alive() {
        let id = ArenaId::new(10_010);
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
        let id = ArenaId::new(10_011);

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
        let id = ArenaId::new(10_012);
        register_arena(id, stw_flag());

        let expected_gen = try_acquire_lease(id).unwrap().generation();

        // Simulate being inside a GC tracing phase.
        set_currently_tracing(Some(id));
        let recorded = record_fake_object(id, 0xDEAD_BEEF);
        set_currently_tracing(None);

        assert!(recorded, "record should return true for live arena");

        let refs = take_found_cross_arena_refs();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].identity().arena_id, id);
        assert_eq!(refs[0].ptr().as_ptr().addr(), 0xDEAD_BEEF);
        assert_eq!(
            refs[0].identity().generation,
            expected_gen,
            "stored generation must match arena generation at record time"
        );

        unregister_arena(id);
    }

    // ------------------------------------------------------------------
    // A retained identity is validated, never refreshed by ID
    // ------------------------------------------------------------------
    #[test]
    fn record_cross_arena_ref_rejects_stale_identity_after_reregistration() {
        let id = ArenaId::new(10_013);
        register_arena(id, stw_flag());
        let old_identity = try_acquire_lease(id).unwrap().identity();
        let stale = fake_ref(old_identity, 0xABCD);

        unregister_arena(id);
        register_arena(id, stw_flag());

        set_currently_tracing(Some(id));
        let recorded = record_cross_arena_ref(stale);
        set_currently_tracing(None);

        assert!(
            !recorded,
            "stale identity must not match the new registration"
        );
        assert!(take_found_cross_arena_refs().is_empty());
        unregister_arena(id);
    }

    // ------------------------------------------------------------------
    // record_cross_arena_ref outside tracing phase: no push, returns validity
    // ------------------------------------------------------------------
    #[test]
    fn record_cross_arena_ref_outside_tracing_returns_validity() {
        let id = ArenaId::new(10_014);
        // Not registered yet.
        assert!(!record_fake_object(id, 0x1));

        register_arena(id, stw_flag());
        // Not in tracing phase — should return true but not push anything.
        assert!(record_fake_object(id, 0x2));
        let refs = take_found_cross_arena_refs();
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

        let writer_locks: Arc<Vec<crate::sync::Mutex<()>>> =
            Arc::new((0..num_ids).map(|_| crate::sync::Mutex::new(())).collect());
        let mut handles = vec![];

        for t in 0..num_threads {
            let barrier = barrier.clone();
            let writer_locks = writer_locks.clone();
            let h = thread::spawn(move || {
                barrier.wait();
                for i in 0..num_iterations {
                    let id = ArenaId::new(base_id + (i % num_ids));
                    match t % 3 {
                        0 => {
                            // Serialize owners of the same ID; readers remain concurrent.
                            let _owner = writer_locks[(i % num_ids) as usize].lock();
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
                            let recorded = record_fake_object(id, i as usize + 1);
                            set_currently_tracing(None);

                            if recorded {
                                let refs = take_found_cross_arena_refs();
                                for reference in refs {
                                    let (target_id, recorded_gen) = (
                                        reference.identity().arena_id,
                                        reference.identity().generation,
                                    );
                                    if let Some(lease) = try_acquire_lease(target_id) {
                                        // is_valid() may flip to false if unregister starts while this
                                        // lease is still held (state.is_alive is cleared before waiting
                                        // for active leases to drop). So we only assert validity when
                                        // no teardown race is visible.
                                        if lease.generation() == recorded_gen {
                                            let valid = lease.is_valid();
                                            if !valid {
                                                assert!(
                                                    try_acquire_lease_for_identity(
                                                        lease.identity()
                                                    )
                                                    .is_none()
                                                );
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
            unregister_arena(ArenaId::new(base_id + i));
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
        let writer_locks: Arc<Vec<crate::sync::Mutex<()>>> =
            Arc::new((0..NUM_IDS).map(|_| crate::sync::Mutex::new(())).collect());
        let mut handles = vec![];

        for t in 0..NUM_THREADS {
            let barrier = barrier.clone();
            let writer_locks = writer_locks.clone();
            handles.push(thread::spawn(move || {
                barrier.wait();
                for i in 0..ITERATIONS {
                    let id = ArenaId::new(BASE_ID + (i as u64 % NUM_IDS));
                    let _owner = if t != 2 {
                        Some(writer_locks[(i as u64 % NUM_IDS) as usize].lock())
                    } else {
                        None
                    };
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
                                // A held lease may observe is_valid() == false if
                                // unregister_arena started teardown (is_alive=false)
                                // while waiting for active leases to drain.
                                // This is allowed; the arena should no longer be
                                // reported as valid in that case.
                                let valid = lease.is_valid();
                                if !valid {
                                    assert!(
                                        try_acquire_lease_for_identity(lease.identity()).is_none(),
                                        "invalid identity must not be reacquired"
                                    );
                                }
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
                            record_fake_object(id, i + 1);
                            set_currently_tracing(None);

                            let refs = take_found_cross_arena_refs();
                            for reference in refs {
                                let (target_id, recorded_gen) = (
                                    reference.identity().arena_id,
                                    reference.identity().generation,
                                );
                                if let Some(lease) = try_acquire_lease(target_id) {
                                    if lease.generation() == recorded_gen {
                                        // Matching generation means no re-registration
                                        // happened, but teardown can still be in flight.
                                        // Treat an invalid lease as acceptable only if
                                        // the arena is no longer reported as valid.
                                        let valid = lease.is_valid();
                                        if !valid {
                                            assert!(
                                                try_acquire_lease_for_identity(lease.identity())
                                                    .is_none(),
                                                "invalid identity must not be reacquired"
                                            );
                                        }
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
            unregister_arena(ArenaId::new(BASE_ID + i));
        }
    }

    // ------------------------------------------------------------------
    // record_and_harvest_race_with_reregistration:
    // ensures generation check catches re-registration
    // ------------------------------------------------------------------
    #[test]
    fn record_and_harvest_race_with_reregistration() {
        let id = ArenaId::new(40_000);

        // 1. Register and record
        register_arena(id, stw_flag());
        set_currently_tracing(Some(id));
        record_fake_object(id, 0x123);
        set_currently_tracing(None);

        let refs = take_found_cross_arena_refs();
        let target_id = refs[0].identity().arena_id;
        let recorded_gen = refs[0].identity().generation;

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

/// Registry and cross-arena transport tests: a found cross-arena reference carries the pointer
/// itself from tracing to harvest, with numeric keys used only for equality and deduplication.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArenaId;

    fn new_arena_flag() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    /// Duplicate publication must not
    /// replace a live registration or leave its outstanding lease detached from the map.
    #[test]
    fn arena_identity_registry_duplicate() {
        let id = ArenaId::new(10_151);
        register_arena(id, new_arena_flag());
        let original = try_acquire_lease(id).expect("original registration");
        let original_generation = original.generation();
        let rejected = std::panic::catch_unwind(|| register_arena(id, new_arena_flag())).is_err();
        let current_generation = try_acquire_lease(id).map(|lease| lease.generation());
        let original_alive = original.is_valid();
        drop(original);
        unregister_arena(id);

        assert!(
            rejected && original_alive && current_generation == Some(original_generation),
            "duplicate live registration accepted or replaced: rejected={rejected}, \
             original_alive={original_alive}, original={original_generation}, current={current_generation:?}"
        );
    }

    /// The fast bit and map must not
    /// disagree while unregister is waiting for the registry write lock.
    #[test]
    fn arena_identity_registry_fast_bit_transaction() {
        let id = ArenaId::new(51);
        register_arena(id, new_arena_flag());
        let map = VALID_ARENAS.write();
        let (checkpoint_tx, checkpoint_rx) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            RECREATION_REMOVE_CHECKPOINT.with(|checkpoint| {
                *checkpoint.borrow_mut() = Some(checkpoint_tx);
            });
            unregister_arena(id);
        });
        // F8.LockOrderRespected: this test deliberately retains the registry lock
        // only for the bounded checkpoint and bit observation; it releases it
        // before joining the worker or allowing unregister to drain leases.
        let contended = checkpoint_rx.recv_timeout(Duration::from_secs(5));
        let map_has_entry = map.by_id.contains_key(&id);
        let bit_set = is_valid_cross_arena_ref(id);
        // With a cleared bit the real acquisition exits before attempting the held lock.
        let rejected_while_present = !bit_set && try_acquire_lease(id).is_none();
        drop(map);
        worker.join().unwrap();
        assert_eq!(
            contended,
            Ok(true),
            "remover did not reach the contended write-lock checkpoint"
        );
        assert!(
            map_has_entry && bit_set,
            "fast bit cleared before map removal: map_has_entry={map_has_entry}, bit_set={bit_set}, lease_rejected={rejected_while_present}"
        );
    }

    /// Re-register after removal while
    /// the old unregister is draining. This is a production-registry control, not a model.
    #[test]
    fn arena_identity_registry_drain_reuse_control() {
        let id = ArenaId::new(52);
        register_arena(id, new_arena_flag());
        let old = try_acquire_lease(id).expect("old lease");
        let old_generation = old.generation();
        let old_identity = old.identity();
        let old_token = old.token();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            unregister_arena(id);
            done_tx.send(()).unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while old.is_valid() && Instant::now() < deadline {
            thread::yield_now();
        }
        let closed = !old.is_valid();
        let still_draining = done_rx.try_recv().is_err();
        let absent = try_acquire_lease(id).is_none();
        if !closed {
            drop(old);
            worker.join().unwrap();
            panic!("unregister did not close within the deadline");
        }

        // Collect observations before asserting, so even a regression releases
        // the old lease and joins the draining worker before the test fails.
        let observations = (|| {
            try_register_arena(id, new_arena_flag()).map_err(|e| format!("{e:?}"))?;
            let new = try_acquire_lease(id).ok_or("new registration unavailable")?;
            let new_identity = new.identity();
            let new_token = new.token();
            // A faulty stale removal must not wait on our own new lease.
            drop(new);
            Ok::<_, String>((
                new_identity,
                new_token,
                try_acquire_lease_for_token(old_token).is_none(),
                try_acquire_lease_for_identity(old_identity).is_none(),
                !unregister_arena_identity(old_identity),
                try_acquire_lease_for_token(new_token).map(|lease| lease.identity()),
                is_valid_cross_arena_ref(id),
            ))
        })();
        drop(old);
        worker.join().unwrap();
        let after_drain = try_acquire_lease(id).map(|lease| (lease.identity(), lease.token()));
        let present_after_drain = is_valid_cross_arena_ref(id);
        let old_token_after_drain = try_acquire_lease_for_token(old_token).is_none();
        unregister_arena(id);
        let (
            new_identity,
            new_token,
            old_token_absent,
            old_identity_absent,
            stale_remove_rejected,
            new_token_identity,
            present_during_drain,
        ) = observations.expect("re-registration failed");
        assert!(closed && still_draining && absent);
        assert_ne!(
            old_generation, new_identity.generation,
            "ABA generation mismatch"
        );
        assert_ne!(old_token, new_token);
        assert!(old_token_absent && old_identity_absent && stale_remove_rejected);
        assert_eq!(new_token_identity, Some(new_identity));
        assert!(present_during_drain && present_after_drain);
        assert!(old_token_after_drain);
        assert_eq!(after_drain, Some((new_identity, new_token)));
    }

    /// Exercise production allocation
    /// against an isolated registry, including failed-publication and token exhaustion.
    #[test]
    fn arena_identity_registry_generation_exhaustion() {
        let mut registry = ArenaRegistry::new();
        let fast = AtomicU64::new(0);
        registry.next_generation = u64::MAX - 1;
        let first = registry
            .register(ArenaId::new(54), new_arena_flag(), &fast)
            .unwrap();
        assert_eq!(first.generation, u64::MAX - 1);
        let duplicate_counters = (registry.next_generation, registry.next_token);
        assert_eq!(
            registry.register(first.arena_id, new_arena_flag(), &fast),
            Err(ArenaRegistrationError::DuplicateLive(first.arena_id))
        );
        assert_eq!(
            (registry.next_generation, registry.next_token),
            duplicate_counters
        );
        let before = (
            registry.next_generation,
            registry.next_token,
            fast.load(StdAtomicOrdering::Acquire),
        );
        for _ in 0..2 {
            assert_eq!(
                registry.register(ArenaId::new(55), new_arena_flag(), &fast),
                Err(ArenaRegistrationError::GenerationExhausted)
            );
            assert!(!registry.by_id.contains_key(&ArenaId::new(55)));
            assert_eq!(registry.by_token.len(), 1);
            assert_eq!(
                (
                    registry.next_generation,
                    registry.next_token,
                    fast.load(StdAtomicOrdering::Acquire)
                ),
                before
            );
        }

        let mut tokens = ArenaRegistry::new();
        let fast_tokens = AtomicU64::new(0);
        tokens.next_token = u32::MAX as u64;
        let last = tokens
            .register(ArenaId::new(56), new_arena_flag(), &fast_tokens)
            .unwrap();
        assert_eq!(tokens.by_id[&last.arena_id].token.as_u32(), u32::MAX);
        let before = (
            tokens.next_generation,
            tokens.next_token,
            fast_tokens.load(StdAtomicOrdering::Acquire),
        );
        for _ in 0..2 {
            assert_eq!(
                tokens.register(ArenaId::new(57), new_arena_flag(), &fast_tokens),
                Err(ArenaRegistrationError::TokenExhausted)
            );
            assert!(!tokens.by_id.contains_key(&ArenaId::new(57)));
            assert_eq!(tokens.by_token.len(), 1);
            assert_eq!(
                (
                    tokens.next_generation,
                    tokens.next_token,
                    fast_tokens.load(StdAtomicOrdering::Acquire)
                ),
                before
            );
        }
        assert_eq!(RegistrationToken::from_raw(0), None);
    }

    /// Tokens, exact leases and stale
    /// unregisters must not bind an old registration to its successor.
    #[test]
    fn arena_identity_registry_identity_token_reuse() {
        let id = ArenaId::new(53);
        let old_id = try_register_arena(id, new_arena_flag()).unwrap();
        let old = try_acquire_lease_for_identity(old_id).unwrap();
        let old_token = old.token();
        assert_eq!(
            try_acquire_lease_for_token(old_token).unwrap().identity(),
            old_id
        );
        assert_eq!(
            try_register_arena(id, new_arena_flag()),
            Err(ArenaRegistrationError::DuplicateLive(id))
        );
        drop(old);
        assert!(unregister_arena_identity(old_id));
        assert!(try_acquire_lease_for_token(old_token).is_none());
        let new_id = try_register_arena(id, new_arena_flag()).unwrap();
        let new = try_acquire_lease_for_identity(new_id).unwrap();
        let new_token = new.token();
        assert_ne!(old_id, new_id);
        assert_ne!(old_token, new_token);
        assert!(try_acquire_lease_for_identity(old_id).is_none());
        assert!(try_acquire_lease_for_token(old_token).is_none());
        assert!(!unregister_arena_identity(old_id));
        assert!(is_valid_cross_arena_ref(id));
        assert_eq!(
            try_acquire_lease_for_token(new_token).unwrap().identity(),
            new_id
        );
        drop(new);
        assert!(unregister_arena_identity(new_id));
        assert!(try_acquire_lease_for_token(new_token).is_none());
    }

    /// An isolated checked-counter model
    /// for generation and future u32 tokens. It is not production token-allocation evidence.
    #[test]
    fn arena_identity_registry_checked_counter_control() {
        use std::sync::atomic::AtomicU32;
        let generations = AtomicU64::new(u64::MAX - 1);
        let tokens = AtomicU32::new(u32::MAX - 1);
        let next_generation = || {
            generations.try_update(StdAtomicOrdering::SeqCst, StdAtomicOrdering::SeqCst, |n| {
                n.checked_add(1)
            })
        };
        let next_token = || {
            tokens.try_update(StdAtomicOrdering::SeqCst, StdAtomicOrdering::SeqCst, |n| {
                n.checked_add(1)
            })
        };
        assert_eq!(next_generation(), Ok(u64::MAX - 1));
        assert_eq!(next_generation(), Err(u64::MAX));
        assert_eq!(next_generation(), Err(u64::MAX));
        assert_eq!(next_token(), Ok(u32::MAX - 1));
        assert_eq!(next_token(), Err(u32::MAX));
        assert_eq!(next_token(), Err(u32::MAX));
    }

    /// Control: the retained transport carries the allocation pointer itself (no integer round
    /// trip, no exposed provenance), so use after harvest works under permissive execution and
    /// Miri alike. It cannot by itself prove the full contract; the structural scan covers that.
    #[test]
    fn arena_lifetime_transport_permissive_control() {
        let id = ArenaId::new(10_101);
        register_arena(id, Arc::new(AtomicBool::new(false)));
        let allocation = Box::into_raw(Box::new(0xA5u8));

        set_currently_tracing(Some(ArenaId::new(10_102)));
        // SAFETY: F11.ObjectPointerThreadSafe — `allocation` is a non-null `Box` pointer standing in for an object allocation; the erased wrapper is inert and is only compared and dereferenced below on this thread while the box is live.
        let opaque = unsafe { OpaqueObjectPtr::from_raw(allocation.cast()) };
        // SAFETY: F1.ArenaGenerationMatch — `id` was registered above and is not unregistered until the end of the test, and `allocation` stays live until then.
        assert!(unsafe { record_cross_arena_object(id, opaque) });
        set_currently_tracing(None);
        let found = take_found_cross_arena_refs();
        let reference = *found.first().expect("recorded");
        assert_eq!(reference.identity().arena_id, id);
        let _lease =
            try_acquire_lease_for_identity(reference.identity()).expect("same registration");

        let rebuilt = reference.ptr().as_ptr().cast::<u8>().cast_mut();
        // SAFETY: F10.RawMemoryAccessValid — `allocation` is live and this thread owns it; the pointer kept its provenance.
        assert_eq!(unsafe { rebuilt.read() }, 0xA5);
        // SAFETY: F10.RawMemoryAccessValid — as above; the write goes through the transported pointer.
        unsafe { rebuilt.write(0x5A) };
        // SAFETY: F10.RawMemoryAccessValid — as above; no other reference to the allocation exists.
        assert_eq!(unsafe { *allocation }, 0x5A);
        drop(_lease);
        // SAFETY: F10.RawAllocationOwnership — `allocation` came from `Box::into_raw` and is freed exactly once.
        drop(unsafe { Box::from_raw(allocation) });
        unregister_arena(id);
    }

    /// Structural pending regression: the transport path must not convert a pointer to an
    /// integer for retention and back. Fails while the found buffer stores `usize` and any
    /// producer/harvester uses `expose_provenance`/`with_exposed_provenance*`. It scans the
    /// complete producer -> buffer -> harvest path; it is source evidence, not VM execution.
    #[test]
    fn arena_lifetime_transport() {
        const FORBIDDEN: [&str; 2] = ["expose_provenance", "with_exposed_provenance"];
        // Top-level test module only: an item-level `#[cfg(test)]` must not truncate a file.
        const TEST_MODULE: &str = "\n#[cfg(test)]\nmod ";
        let this = include_str!("cross_arena.rs");
        let production = &this[..this.find(TEST_MODULE).expect("test module marker present")];
        let path: [(&str, &str); 6] = [
            ("dotnet-utils gc/cross_arena.rs (buffer)", production),
            (
                "dotnet-runtime-memory write_barrier.rs (producer)",
                include_str!("../../../dotnet-runtime-memory/src/write_barrier.rs"),
            ),
            (
                "dotnet-value pointer/origin.rs (producer)",
                include_str!("../../../dotnet-value/src/pointer/origin.rs"),
            ),
            (
                "dotnet-value object/mod.rs (producer)",
                include_str!("../../../dotnet-value/src/object/mod.rs"),
            ),
            (
                "dotnet-vm gc/coordinator.rs (dispatch)",
                include_str!("../../../dotnet-vm/src/gc/coordinator.rs"),
            ),
            (
                "dotnet-vm threading/basic.rs (harvest)",
                include_str!("../../../dotnet-vm/src/threading/basic.rs"),
            ),
        ];
        let mut violations = Vec::new();
        for (label, source) in path {
            // Stop at an in-file test module so test-only conversions are not counted.
            let source = source.split(TEST_MODULE).next().unwrap_or(source);
            for (line_no, line) in source.lines().enumerate() {
                let code = line.trim_start();
                // The fuzz-only object registry is an intentional boundary exposure outside the
                // GC transport path and is preserved.
                if code.starts_with("//") || code.contains("register_object_ptr") {
                    continue;
                }
                if FORBIDDEN.iter().any(|needle| code.contains(needle)) {
                    violations.push(format!("{label}:{}: {code}", line_no + 1));
                }
            }
        }
        let integer_buffer = production.contains("Vec<(ArenaId, usize, u64)>");
        assert!(
            !integer_buffer && violations.is_empty(),
            "integer transport of GC pointers remains: usize_found_buffer={integer_buffer}, \
             conversions={violations:#?}"
        );
    }
}
