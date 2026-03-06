use crate::sync::{Arc, AtomicBool, Ordering, RwLock};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    mem,
};

static VALID_ARENAS: std::sync::LazyLock<RwLock<HashMap<crate::ArenaId, Arc<AtomicBool>>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn register_arena(thread_id: crate::ArenaId, stw_in_progress: Arc<AtomicBool>) {
    VALID_ARENAS.write().insert(thread_id, stw_in_progress);
}

pub fn unregister_arena(thread_id: crate::ArenaId) {
    VALID_ARENAS.write().remove(&thread_id);
}

pub fn is_valid_cross_arena_ref(target_thread_id: crate::ArenaId) -> bool {
    VALID_ARENAS.read().contains_key(&target_thread_id)
}

pub fn reset_arena_registry() {
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
pub fn record_cross_arena_ref(target_thread_id: crate::ArenaId, ptr: usize) {
    if !is_valid_cross_arena_ref(target_thread_id) {
        panic!(
            "Dangling cross-arena reference detected: target_thread_id {:?} is no longer valid (thread exited?)",
            target_thread_id
        );
    }
    FOUND_CROSS_ARENA_REFS.with(|refs| {
        refs.borrow_mut().push((target_thread_id, ptr));
    });
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
