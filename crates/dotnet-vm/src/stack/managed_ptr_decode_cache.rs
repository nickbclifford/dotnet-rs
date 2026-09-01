use dotnet_value::{
    object::ObjectRef, pointer::HeapManagedPtrDecodeCache as HeapManagedPtrDecodeCacheTrait,
};
use gc_arena::{Collect, collect::Trace};
use hashbrown::HashMap;
use std::collections::VecDeque;

/// Default number of decoded Heap handles retained for one executing CallStack.
const DEFAULT_HEAP_MANAGED_PTR_DECODE_CACHE_CAPACITY: usize = 64;

/// Snapshot of one caller-owned Heap managed-pointer decode cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeapManagedPtrDecodeCacheStats {
    /// Collection epoch in which the current entries were decoded.
    pub epoch: u64,
    /// Number of cache hits since construction.
    pub hits: u64,
    /// Number of cache misses since construction.
    pub misses: u64,
    /// Number of capacity evictions since construction.
    pub evictions: u64,
    /// Entries retained in the current collection epoch.
    pub entries: usize,
}

/// Bounded, GC-traced cache of Heap `ManagedPtr` wire handles for one CallStack.
///
/// It stores an [`ObjectRef`] rather than an object data pointer. Consequently a
/// hit must still borrow current storage through the handle and apply the
/// encoded offset. Entries are traced with the owning CallStack and are cleared
/// by [`super::GCArena::finish_cycle`] after each completed collection cycle.
pub struct HeapManagedPtrDecodeCache<'gc> {
    entries: HashMap<usize, ObjectRef<'gc>>,
    insertion_order: VecDeque<usize>,
    capacity: usize,
    epoch: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
}

impl<'gc> Default for HeapManagedPtrDecodeCache<'gc> {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_HEAP_MANAGED_PTR_DECODE_CACHE_CAPACITY)
    }
}

impl<'gc> HeapManagedPtrDecodeCache<'gc> {
    /// Creates a bounded cache for a caller that will keep it in a traced VM root.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        assert!(
            capacity > 0,
            "Heap managed-pointer decode cache capacity must be non-zero"
        );
        Self {
            entries: HashMap::with_capacity(capacity),
            insertion_order: VecDeque::with_capacity(capacity),
            capacity,
            epoch: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    /// Drops entries without advancing the collection epoch.
    ///
    /// This is useful when a caller ends a decode batch before a collection. VM
    /// collection invalidation must use [`Self::invalidate_after_collection`].
    pub fn clear(&mut self) {
        self.entries.clear();
        self.insertion_order.clear();
    }

    /// Clears serialized-handle mappings after a completed collection cycle.
    ///
    /// The decoded `ObjectRef`s were traced while the collection was active, but
    /// their old wire-address keys must not survive into an epoch where those
    /// addresses could be recycled.
    pub(crate) fn invalidate_after_collection(&mut self) {
        self.entries.clear();
        self.insertion_order.clear();
        self.epoch = self.epoch.wrapping_add(1);
    }

    #[must_use]
    pub fn stats(&self) -> HeapManagedPtrDecodeCacheStats {
        HeapManagedPtrDecodeCacheStats {
            epoch: self.epoch,
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
            entries: self.entries.len(),
        }
    }
}

// SAFETY: F5.TracesEveryGcRef — Every retained Heap ObjectRef is visited while the CallStack root is
// traced. Keys, counters, and insertion-order metadata contain no GC-managed data.
unsafe impl<'gc> Collect<'gc> for HeapManagedPtrDecodeCache<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        for owner in self.entries.values() {
            owner.trace(cc);
        }
    }
}

impl<'gc> HeapManagedPtrDecodeCacheTrait<'gc> for HeapManagedPtrDecodeCache<'gc> {
    fn get_heap_handle(&mut self, serialized_handle: usize) -> Option<ObjectRef<'gc>> {
        match self.entries.get(&serialized_handle).copied() {
            Some(owner) => {
                self.hits += 1;
                Some(owner)
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    fn insert_heap_handle(&mut self, serialized_handle: usize, owner: ObjectRef<'gc>) {
        debug_assert_ne!(serialized_handle, 0, "Heap cache keys must be non-null");
        debug_assert!(
            owner.0.is_some(),
            "Heap cache values must be non-null handles"
        );

        if self.entries.contains_key(&serialized_handle) {
            self.entries.insert(serialized_handle, owner);
            return;
        }

        if self.entries.len() == self.capacity {
            let evicted = self
                .insertion_order
                .pop_front()
                .expect("cache insertion order matches bounded entry count");
            self.entries.remove(&evicted);
            self.evictions += 1;
        }
        self.entries.insert(serialized_handle, owner);
        self.insertion_order.push_back(serialized_handle);
    }
}

#[cfg(test)]
mod tests {
    use super::{HeapManagedPtrDecodeCache, HeapManagedPtrDecodeCacheTrait};
    use crate::{state::SharedGlobalState, sync::Arc, test_utils::new_test_arena};
    use dotnet_assemblies::AssemblyLoader;
    use dotnet_types::TypeDescription;
    use dotnet_utils::{ByteOffset, gc::GCHandle};
    use dotnet_value::{
        ManagedPtr, StackValue,
        object::{HeapStorage, ObjectRef},
        pointer::NoManagedPtrResolver,
        string::CLRString,
    };
    use std::ptr::NonNull;

    #[allow(
        clippy::arc_with_non_send_sync,
        reason = "the bare loader is confined to this single-threaded cache fixture"
    )]
    fn new_test_shared() -> Arc<SharedGlobalState> {
        let loader = AssemblyLoader::new_bare("heap_managed_ptr_decode_cache".to_owned())
            .expect("bare cache-test loader should initialize");
        Arc::new(SharedGlobalState::new(Arc::new(loader)))
    }

    #[test]
    fn cache_evicts_oldest_key_and_invalidates_at_collection_epoch() {
        let shared = new_test_shared();
        let mut arena = new_test_arena(&shared);

        arena.mutate_root(|gc, engine| {
            #[cfg(feature = "memory-validation")]
            let thread_id = dotnet_utils::sync::get_current_thread_id();
            let gc_handle = GCHandle::new(
                gc,
                #[cfg(feature = "multithreading")]
                // SAFETY: F3.StackSlotMatchesView — The CallStack is the root of this test arena and retains
                // its arena handle for the complete mutation closure.
                unsafe {
                    engine.stack.arena_inner_gc()
                },
                #[cfg(feature = "memory-validation")]
                thread_id,
            );
            let object = ObjectRef::new(gc_handle, HeapStorage::Str(CLRString::from("cache root")));
            engine
                .stack
                .execution
                .evaluation_stack
                .stack
                .push(StackValue::ObjectRef(object));

            let mut cache = HeapManagedPtrDecodeCache::with_capacity(2);
            cache.insert_heap_handle(8, object);
            cache.insert_heap_handle(16, object);
            assert_eq!(cache.get_heap_handle(8), Some(object));
            cache.insert_heap_handle(24, object);
            assert_eq!(cache.get_heap_handle(8), None);
            assert_eq!(cache.get_heap_handle(16), Some(object));
            assert_eq!(cache.get_heap_handle(24), Some(object));
            assert_eq!(cache.stats().evictions, 1);

            engine
                .stack
                .heap_managed_ptr_decode_cache
                .insert_heap_handle(8, object);
            assert_eq!(
                engine.stack.heap_managed_ptr_decode_cache.stats().entries,
                1
            );
        });

        // GCArena::finish_cycle is the selected safepoint/collection-epoch hook.
        // The cache remains traceable through CallStack during this cycle, then
        // its raw serialized-handle keys are invalidated before mutators resume.
        arena.finish_cycle();

        arena.mutate_root(|_gc, engine| {
            let stats = engine.stack.heap_managed_ptr_decode_cache.stats();
            assert_eq!(stats.epoch, 1);
            assert_eq!(stats.entries, 0);

            let StackValue::ObjectRef(object) = engine.stack.execution.evaluation_stack.stack[0]
            else {
                panic!("test root should retain the Heap object");
            };
            let offset = ByteOffset::new(2);
            let expected = object.with_data(|data| {
                NonNull::new(data.as_ptr().wrapping_add(offset.as_usize()).cast_mut())
            });
            let ptr = ManagedPtr::new(
                expected,
                TypeDescription::NULL,
                Some(object),
                false,
                Some(offset),
            );
            let mut source = ManagedPtr::serialization_buffer();
            ptr.write(&mut source);

            // SAFETY: F1.GcHandleRooted — `source` was refreshed from the root-retained live object
            // after collection. A cache miss must recover its current storage
            // address rather than retaining any pre-collection data pointer.
            let decoded = unsafe {
                ManagedPtr::read_resolved_with_heap_cache_unchecked(
                    &source,
                    &NoManagedPtrResolver,
                    &mut engine.stack.heap_managed_ptr_decode_cache,
                )
            }
            .expect("post-collection Heap cache miss should decode");
            assert_eq!(decoded.address, expected);
            let refreshed = engine.stack.heap_managed_ptr_decode_cache.stats();
            assert_eq!(refreshed.epoch, 1);
            assert_eq!(refreshed.entries, 1);
            assert_eq!(refreshed.misses, 1);
            assert_eq!(refreshed.hits, 0);
        });
    }
}
