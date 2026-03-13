use crate::sync::{Arc, AtomicBool, Ordering, RwLock};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    mem,
};

static VALID_ARENAS: std::sync::LazyLock<RwLock<HashMap<crate::ArenaId, Arc<AtomicBool>>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));

/// A fast-path bitset for the first 64 Arena IDs to avoid lock contention on every object read.
static VALID_ARENAS_FAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn register_arena(thread_id: crate::ArenaId, stw_in_progress: Arc<AtomicBool>) {
    let id = thread_id.0;
    if id < 64 {
        VALID_ARENAS_FAST.fetch_or(1 << id, Ordering::Release);
    }
    VALID_ARENAS.write().insert(thread_id, stw_in_progress);
}

pub fn unregister_arena(thread_id: crate::ArenaId) {
    let id = thread_id.0;
    if id < 64 {
        VALID_ARENAS_FAST.fetch_and(!(1 << id), Ordering::Release);
    }
    VALID_ARENAS.write().remove(&thread_id);
}

pub fn is_valid_cross_arena_ref(target_thread_id: crate::ArenaId) -> bool {
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

pub fn set_stw_in_progress(arena_id: crate::ArenaId, in_progress: bool) {
    if let Some(flag) = VALID_ARENAS.read().get(&arena_id) {
        flag.store(in_progress, Ordering::Release);
    }
}

pub fn is_stw_in_progress(arena_id: crate::ArenaId) -> bool {
    VALID_ARENAS
        .read()
        .get(&arena_id)
        .map(|flag| flag.load(Ordering::Acquire))
        .unwrap_or(false)
}

thread_local! {
    /// Found cross-arena references during the current marking phase.
    static FOUND_CROSS_ARENA_REFS: RefCell<Vec<(crate::ArenaId, usize)>> = const { RefCell::new(Vec::new()) };
    /// The thread ID of the arena currently being traced.
    static CURRENTLY_TRACING_THREAD_ID: Cell<Option<crate::ArenaId>> = const { Cell::new(None) };
}

/// Set the thread ID of the arena currently being traced.
pub fn set_currently_tracing(thread_id: Option<crate::ArenaId>) {
    CURRENTLY_TRACING_THREAD_ID.with(|id| {
        id.set(thread_id);
    });
}

/// Get the thread ID of the arena currently being traced.
pub fn get_currently_tracing() -> Option<crate::ArenaId> {
    CURRENTLY_TRACING_THREAD_ID.get()
}

/// Take all found cross-arena references and clear the local list.
pub fn take_found_cross_arena_refs() -> Vec<(crate::ArenaId, usize)> {
    FOUND_CROSS_ARENA_REFS.with(|refs| {
        let mut r = refs.borrow_mut();
        mem::take(&mut *r)
    })
}

/// Record a cross-arena reference found during marking.
/// Returns true if the target arena is valid, false if it has exited.
pub fn record_cross_arena_ref(target_thread_id: crate::ArenaId, ptr: usize) -> bool {
    if !is_valid_cross_arena_ref(target_thread_id) {
        return false;
    }
    // Only record the reference if we are currently in a GC tracing phase.
    // This prevents the thread-local list from growing infinitely during normal execution.
    if CURRENTLY_TRACING_THREAD_ID.with(|id| id.get().is_some()) {
        FOUND_CROSS_ARENA_REFS.with(|refs| {
            refs.borrow_mut().push((target_thread_id, ptr));
        });
    }
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
