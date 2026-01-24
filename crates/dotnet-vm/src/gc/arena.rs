use crate::stack::GCArena;
use std::cell::RefCell;

thread_local! {
    /// Per-thread storage for the GC arena.
    /// Each thread manages its own GC heap in this arena.
    pub static THREAD_ARENA: RefCell<Option<Box<GCArena>>> = const { RefCell::new(None) };
}
