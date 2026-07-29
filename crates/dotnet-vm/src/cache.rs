use dashmap::DashMap;
use dotnet_metrics::{CacheEvent, CacheKind, CacheSize, RuntimeMetrics};
use dotnet_utils::sync::RwLock;
use lru::LruCache;
use std::{
    cell::RefCell, collections::HashMap, hash::Hash, mem::size_of, num::NonZeroUsize, sync::Arc,
    thread::LocalKey,
};

pub(crate) const DEFAULT_CAPACITY: usize = 128;

mod sealed {
    pub trait Sealed {}
}

/// Storage operations used by a generic cache without dynamic dispatch.
///
/// Cache reads return an owned clone so that a `DashMap` shard guard or `RwLock` guard never
/// escapes the store implementation.
pub(crate) trait CacheStore<K, V>: sealed::Sealed {
    fn get_cloned(&self, key: &K) -> Option<V>
    where
        V: Clone;

    fn contains_key(&self, key: &K) -> bool;

    fn insert(&self, key: K, value: V);

    /// Removes one arbitrary entry and reports whether an entry was removed.
    fn evict_arbitrary(&self) -> bool
    where
        K: Clone;

    fn len(&self) -> usize;
}

/// Configuration for a cache's optional per-thread front-cache companion.
///
/// The [`Cache`] owns this policy and its instrumentation, while a typed
/// [`FrontCache`] owns the actual per-thread entries. Caches without a front
/// cache use `None` for this policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrontCachePolicy {
    enabled: bool,
    capacity: usize,
}

impl FrontCachePolicy {
    pub(crate) const fn new(enabled: bool, capacity: usize) -> Self {
        Self { enabled, capacity }
    }

    #[inline]
    pub(crate) const fn is_enabled(self) -> bool {
        self.enabled
    }

    #[inline]
    pub(crate) const fn capacity(self) -> usize {
        self.capacity
    }
}

/// A metric-instrumented cache over a monomorphized storage backend.
///
/// Bounded caches evict an arbitrary entry selected by the underlying store's
/// iteration order. This is neither random nor LRU: for sharded storage it is
/// biased by shard/hash iteration order and can differ across hash seeds.
/// Capacity is an optional operator limit; unbounded caches do no eviction
/// bookkeeping beyond the predictable `Option` branch.
pub(crate) struct Cache<K, V, S> {
    store: S,
    kind: CacheKind,
    metrics: Arc<RuntimeMetrics>,
    capacity: Option<usize>,
    front_cache: Option<FrontCachePolicy>,
    _types: std::marker::PhantomData<(K, V)>,
}

/// A [`Cache`] backed by sharded storage.
pub(crate) type ShardedCache<K, V> = Cache<K, V, ShardedStore<K, V>>;

/// A [`Cache`] backed by a single read-write lock.
pub(crate) type LockedCache<K, V> = Cache<K, V, LockedStore<K, V>>;

impl<K, V, S> Cache<K, V, S>
where
    S: CacheStore<K, V> + Default,
{
    pub(crate) fn new(
        kind: CacheKind,
        metrics: Arc<RuntimeMetrics>,
        capacity: Option<usize>,
        front_cache: Option<FrontCachePolicy>,
    ) -> Self {
        assert!(
            capacity.is_none_or(|capacity| capacity > 0),
            "cache capacity must be non-zero when configured"
        );
        Self {
            store: S::default(),
            kind,
            metrics,
            capacity,
            front_cache,
            _types: std::marker::PhantomData,
        }
    }

    /// Looks up an owned value and records the logical cache result.
    ///
    /// Misses intentionally do not coordinate a build: callers build outside cache guards, so
    /// concurrent misses may duplicate work and race to fill the cache. A later insert can replace
    /// an earlier equivalent result. Holding a `DashMap::entry` or the single-threaded `RefCell`
    /// borrow across a build would instead risk re-entrant deadlocks or borrow panics.
    ///
    /// The store returns a clone, so a `DashMap` shard guard or `RwLock` guard
    /// cannot escape this method.
    #[inline]
    pub(crate) fn get(&self, key: &K) -> Option<V>
    where
        V: Clone,
    {
        let value = self.store.get_cloned(key);
        if value.is_some() {
            self.record_hit();
        } else {
            self.record_miss();
        }
        value
    }

    /// Records a logical cache hit, including a hit served by a front cache.
    #[inline]
    pub(crate) fn record_hit(&self) {
        self.metrics.record_cache(self.kind, CacheEvent::Hit);
    }

    /// Records a logical cache miss.
    #[inline]
    fn record_miss(&self) {
        self.metrics.record_cache(self.kind, CacheEvent::Miss);
    }

    /// Records logical key-component clones performed by a cache caller.
    ///
    /// This benchmark-only metric counts descriptor/`Arc` clone work used to construct a key;
    /// it intentionally does not measure the cost of hashing that key.
    #[inline]
    pub(crate) fn record_key_clones(&self, count: u64) {
        self.metrics.record_cache_key_clones(self.kind, count);
    }

    /// Records the result from this cache's front-cache tier.
    #[inline]
    pub(crate) fn record_front_cache(&self, event: CacheEvent) {
        self.metrics.record_front_cache(self.kind, event);
    }

    #[inline]
    pub(crate) fn front_cache_enabled(&self) -> bool {
        self.front_cache.is_some_and(FrontCachePolicy::is_enabled)
    }

    #[inline]
    pub(crate) fn front_cache_capacity(&self) -> Option<usize> {
        self.front_cache.map(FrontCachePolicy::capacity)
    }

    /// Reports this cache's stable metric kind and estimated entry storage.
    #[inline]
    pub(crate) fn size_report(&self) -> (CacheKind, CacheSize) {
        let entries = self.store.len();
        (
            self.kind,
            CacheSize {
                entries,
                pointer_bytes: (entries as u64)
                    .saturating_mul((size_of::<K>() + size_of::<V>()) as u64),
            },
        )
    }
}

impl<K, V, S> Cache<K, V, S>
where
    K: Clone + Eq + Hash,
    V: Clone,
    S: CacheStore<K, V> + Default,
{
    /// Looks up a value through an optional per-thread front cache and promotes L2 hits.
    ///
    /// When the front cache is disabled, this delegates directly to the L2 lookup without
    /// touching TLS.
    ///
    /// The TLS borrow is contained within each `with` closure so it cannot overlap the L2 lookup
    /// or a later front-cache promotion in the single-threaded configuration.
    #[inline]
    pub(crate) fn try_get_with_front(
        &self,
        key: &K,
        tls: &'static LocalKey<RefCell<FrontCache<K, V>>>,
    ) -> Option<V> {
        if !self.front_cache_enabled() {
            return self.get(key);
        }

        let front_cached = tls.with(|cache| cache.borrow_mut().get(key));
        if let Some(front_cached) = front_cached {
            self.record_front_cache(CacheEvent::Hit);
            self.record_hit();
            return Some(front_cached);
        }
        self.record_front_cache(CacheEvent::Miss);

        let cached = self.get(key)?;
        let capacity = self
            .front_cache_capacity()
            .expect("enabled front cache must have a configured capacity");
        tls.with(|cache| {
            cache
                .borrow_mut()
                .insert(key.clone(), cached.clone(), capacity);
        });
        Some(cached)
    }

    /// Inserts into L2 and, when enabled, the per-thread front cache.
    ///
    /// When the front cache is disabled, the key and value move directly into L2 without being
    /// cloned.
    ///
    /// The TLS borrow is contained within the `with` closure so it cannot overlap the L2 insert
    /// in the single-threaded configuration.
    #[inline]
    pub(crate) fn insert_with_front(
        &self,
        key: K,
        value: V,
        tls: &'static LocalKey<RefCell<FrontCache<K, V>>>,
    ) {
        if !self.front_cache_enabled() {
            self.insert(key, value);
            return;
        }

        let capacity = self
            .front_cache_capacity()
            .expect("enabled front cache must have a configured capacity");
        self.insert(key.clone(), value.clone());
        tls.with(|cache| cache.borrow_mut().insert(key, value, capacity));
    }
}

impl<K, V, S> Cache<K, V, S>
where
    K: Clone,
    S: CacheStore<K, V> + Default,
{
    /// Inserts a value, updating an existing key before considering eviction.
    ///
    /// Bounded caches evict arbitrary store-order victims until they are below
    /// their limit. The victim selection is neither random nor LRU. Bounded
    /// capacity is an operator-only escape hatch: the finite metadata universe
    /// makes unbounded caches the correct default, so this eviction loop is not
    /// reached in default runs.
    #[inline]
    pub(crate) fn insert(&self, key: K, value: V) {
        let Some(max_entries) = self.capacity else {
            self.store.insert(key, value);
            return;
        };

        // This lookup distinguishes an update, which needs no eviction, from a new key. Using
        // `DashMap::entry` instead would hold its shard write guard across `evict_arbitrary`,
        // which can deadlock when it visits the same shard.
        if self.store.contains_key(&key) {
            self.store.insert(key, value);
            return;
        }

        while self.store.len() >= max_entries {
            if !self.store.evict_arbitrary() {
                break;
            }
        }

        self.store.insert(key, value);
    }
}

/// A `DashMap`-backed [`CacheStore`] for caches with concurrent write pressure.
pub(crate) struct ShardedStore<K, V> {
    entries: DashMap<K, V>,
}

impl<K, V> Default for ShardedStore<K, V>
where
    K: Eq + Hash,
{
    fn default() -> Self {
        Self {
            entries: DashMap::new(),
        }
    }
}

impl<K, V> sealed::Sealed for ShardedStore<K, V> {}

impl<K, V> CacheStore<K, V> for ShardedStore<K, V>
where
    K: Eq + Hash,
{
    fn get_cloned(&self, key: &K) -> Option<V>
    where
        V: Clone,
    {
        let entry = self.entries.get(key)?;
        Some(entry.value().clone())
    }

    fn contains_key(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    fn insert(&self, key: K, value: V) {
        self.entries.insert(key, value);
    }

    fn evict_arbitrary(&self) -> bool
    where
        K: Clone,
    {
        let victim_key = {
            let entry = match self.entries.iter().next() {
                Some(entry) => entry,
                None => return false,
            };
            entry.key().clone()
        };

        self.entries.remove(&victim_key).is_some()
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// An `RwLock<HashMap<_, _>>`-backed [`CacheStore`] for read-mostly boolean caches.
pub(crate) struct LockedStore<K, V> {
    entries: RwLock<HashMap<K, V>>,
}

impl<K, V> Default for LockedStore<K, V>
where
    K: Eq + Hash,
{
    fn default() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }
}

impl<K, V> sealed::Sealed for LockedStore<K, V> {}

impl<K, V> CacheStore<K, V> for LockedStore<K, V>
where
    K: Eq + Hash,
{
    fn get_cloned(&self, key: &K) -> Option<V>
    where
        V: Clone,
    {
        self.entries.read().get(key).cloned()
    }

    fn contains_key(&self, key: &K) -> bool {
        self.entries.read().contains_key(key)
    }

    fn insert(&self, key: K, value: V) {
        self.entries.write().insert(key, value);
    }

    fn evict_arbitrary(&self) -> bool
    where
        K: Clone,
    {
        let mut entries = self.entries.write();
        let Some(victim_key) = entries.keys().next().cloned() else {
            return false;
        };

        entries.remove(&victim_key).is_some()
    }

    fn len(&self) -> usize {
        self.entries.read().len()
    }
}

/// A small per-thread LRU cache that can be resized or disabled at runtime.
pub(crate) struct FrontCache<K, V> {
    entries: Option<LruCache<K, V>>,
    capacity: usize,
}

impl<K, V> Default for FrontCache<K, V>
where
    K: Eq + Hash,
{
    fn default() -> Self {
        Self {
            entries: Some(Self::new_entries(DEFAULT_CAPACITY)),
            capacity: DEFAULT_CAPACITY,
        }
    }
}

impl<K, V> FrontCache<K, V>
where
    K: Eq + Hash,
{
    fn new_entries(capacity: usize) -> LruCache<K, V> {
        LruCache::new(
            NonZeroUsize::new(capacity)
                .expect("front cache capacity must be non-zero when enabled"),
        )
    }

    /// Gets a cached value and promotes its entry to most-recently-used.
    pub(crate) fn get(&mut self, key: &K) -> Option<V>
    where
        V: Clone,
    {
        self.entries.as_mut()?.get(key).cloned()
    }

    /// Inserts a value after applying the active capacity policy.
    pub(crate) fn insert(&mut self, key: K, value: V, capacity: usize) {
        if !self.ensure_capacity(capacity) {
            return;
        }

        if let Some(entries) = self.entries.as_mut() {
            entries.put(key, value);
        }
    }

    /// Rebuilds the LRU when its capacity changes, or disables it at zero.
    fn ensure_capacity(&mut self, capacity: usize) -> bool {
        if capacity == 0 {
            self.entries = None;
            self.capacity = 0;
            return false;
        }

        if self.capacity != capacity {
            self.entries = Some(Self::new_entries(capacity));
            self.capacity = capacity;
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CacheStore, FrontCache, FrontCachePolicy, LockedCache, LockedStore, ShardedCache,
        ShardedStore,
    };
    use dotnet_metrics::{CacheEvent, CacheKind, RuntimeMetrics};
    use std::{cell::RefCell, mem::size_of, sync::Arc};

    #[derive(Eq, Hash, PartialEq)]
    struct CloneForbidden(u8);

    impl Clone for CloneForbidden {
        fn clone(&self) -> Self {
            panic!("disabled front-cache insertion must not clone its key or value")
        }
    }

    thread_local! {
        static TEST_FRONT_CACHE: RefCell<FrontCache<u8, &'static str>> =
            RefCell::new(FrontCache::default());
        static CLONE_FORBIDDEN_FRONT_CACHE: RefCell<FrontCache<CloneForbidden, CloneForbidden>> =
            RefCell::new(FrontCache::default());
    }

    fn sharded_cache(
        capacity: Option<usize>,
        front_cache: Option<FrontCachePolicy>,
    ) -> (ShardedCache<u8, &'static str>, Arc<RuntimeMetrics>) {
        let metrics = Arc::new(RuntimeMetrics::new());
        let cache = ShardedCache::new(
            CacheKind::Layout,
            Arc::clone(&metrics),
            capacity,
            front_cache,
        );
        (cache, metrics)
    }

    fn assert_store_contract<S>()
    where
        S: CacheStore<u8, String> + Default,
    {
        let store = S::default();
        assert_eq!(store.len(), 0);
        assert!(!store.contains_key(&1));
        assert_eq!(store.get_cloned(&1), None);
        assert!(!store.evict_arbitrary());

        store.insert(1, String::from("one"));
        assert_eq!(store.len(), 1);
        assert!(store.contains_key(&1));

        let mut owned_value = store
            .get_cloned(&1)
            .expect("inserted value must be present");
        owned_value.push_str(" copy");
        store.insert(1, String::from("updated"));
        assert_eq!(owned_value, "one copy");
        assert_eq!(store.get_cloned(&1), Some(String::from("updated")));
        assert_eq!(store.len(), 1);

        store.insert(2, String::from("two"));
        assert_eq!(store.len(), 2);
        assert!(store.evict_arbitrary());
        assert_eq!(store.len(), 1);
        assert!(store.get_cloned(&1).is_none() || store.get_cloned(&2).is_none());
        assert!(store.evict_arbitrary());
        assert_eq!(store.len(), 0);
        assert!(!store.evict_arbitrary());
    }

    #[test]
    fn sharded_store_satisfies_cache_store_contract() {
        assert_store_contract::<ShardedStore<u8, String>>();
    }

    #[test]
    fn locked_store_satisfies_cache_store_contract() {
        assert_store_contract::<LockedStore<u8, String>>();
    }

    #[test]
    fn unbounded_cache_records_results_and_reports_size() {
        let (cache, metrics) = sharded_cache(None, None);

        assert_eq!(cache.get(&1), None);
        cache.insert(1, "one");
        cache.insert(2, "two");
        assert_eq!(cache.get(&1), Some("one"));
        cache.record_hit();
        cache.record_miss();
        cache.record_key_clones(2);
        cache.record_front_cache(CacheEvent::Hit);

        assert_eq!(metrics.cache_event_counts(CacheKind::Layout), (2, 2));

        let (kind, size) = cache.size_report();
        assert_eq!(kind, CacheKind::Layout);
        assert_eq!(size.entries, 2);
        assert_eq!(
            size.pointer_bytes,
            2 * (size_of::<u8>() + size_of::<&'static str>()) as u64
        );
    }

    #[test]
    fn bounded_cache_updates_before_evicting() {
        let (cache, _) = sharded_cache(Some(2), None);
        cache.insert(1, "one");
        cache.insert(2, "two");

        cache.insert(1, "updated");
        assert_eq!(cache.size_report().1.entries, 2);
        assert_eq!(cache.get(&1), Some("updated"));
        assert_eq!(cache.get(&2), Some("two"));

        cache.insert(3, "three");
        assert_eq!(cache.size_report().1.entries, 2);
        assert_eq!(cache.get(&3), Some("three"));
    }

    #[test]
    fn aliases_support_both_stores_and_front_policy_is_optional() {
        let (cache, _) = sharded_cache(Some(1), Some(FrontCachePolicy::new(true, 16)));
        assert!(cache.front_cache_enabled());
        assert_eq!(cache.front_cache_capacity(), Some(16));

        let metrics = Arc::new(RuntimeMetrics::new());
        let locked = LockedCache::new(CacheKind::Intrinsic, metrics, None, None);
        assert!(!locked.front_cache_enabled());
        assert_eq!(locked.front_cache_capacity(), None);
        locked.insert(1, "one");
        assert_eq!(locked.get(&1), Some("one"));
    }

    #[test]
    fn try_get_with_front_hits_l1_promotes_l2_hits_and_records_l2_misses() {
        let (cache, metrics) = sharded_cache(None, Some(FrontCachePolicy::new(true, 2)));
        TEST_FRONT_CACHE.with(|front_cache| *front_cache.borrow_mut() = FrontCache::default());

        cache.insert(1, "l2 value");
        TEST_FRONT_CACHE.with(|front_cache| {
            front_cache.borrow_mut().insert(1, "l1 value", 2);
        });
        assert_eq!(
            cache.try_get_with_front(&1, &TEST_FRONT_CACHE),
            Some("l1 value")
        );
        assert_eq!(metrics.cache_event_counts(CacheKind::Layout), (1, 0));

        cache.insert(2, "l2 value");
        assert_eq!(
            cache.try_get_with_front(&2, &TEST_FRONT_CACHE),
            Some("l2 value")
        );
        TEST_FRONT_CACHE.with(|front_cache| {
            assert_eq!(front_cache.borrow_mut().get(&2), Some("l2 value"));
        });
        assert_eq!(metrics.cache_event_counts(CacheKind::Layout), (2, 0));

        assert_eq!(cache.try_get_with_front(&3, &TEST_FRONT_CACHE), None);
        assert_eq!(metrics.cache_event_counts(CacheKind::Layout), (2, 1));
    }

    #[test]
    fn try_get_with_front_bypasses_a_disabled_front_cache() {
        let (cache, metrics) = sharded_cache(None, Some(FrontCachePolicy::new(false, 2)));
        TEST_FRONT_CACHE.with(|front_cache| {
            *front_cache.borrow_mut() = FrontCache::default();
            front_cache.borrow_mut().insert(1, "l1 value", 2);
        });
        cache.insert(1, "l2 value");

        assert_eq!(
            cache.try_get_with_front(&1, &TEST_FRONT_CACHE),
            Some("l2 value")
        );
        assert_eq!(cache.try_get_with_front(&2, &TEST_FRONT_CACHE), None);
        assert_eq!(metrics.cache_event_counts(CacheKind::Layout), (1, 1));
        TEST_FRONT_CACHE.with(|front_cache| {
            assert_eq!(front_cache.borrow_mut().get(&1), Some("l1 value"));
        });
    }

    #[test]
    fn insert_with_front_updates_l2_and_only_enabled_front_caches() {
        TEST_FRONT_CACHE.with(|front_cache| *front_cache.borrow_mut() = FrontCache::default());

        let (enabled_cache, _) = sharded_cache(None, Some(FrontCachePolicy::new(true, 2)));
        enabled_cache.insert_with_front(1, "enabled", &TEST_FRONT_CACHE);
        assert_eq!(enabled_cache.get(&1), Some("enabled"));
        TEST_FRONT_CACHE.with(|front_cache| {
            assert_eq!(front_cache.borrow_mut().get(&1), Some("enabled"));
        });

        TEST_FRONT_CACHE.with(|front_cache| *front_cache.borrow_mut() = FrontCache::default());
        let (disabled_cache, _) = sharded_cache(None, Some(FrontCachePolicy::new(false, 2)));
        disabled_cache.insert_with_front(2, "disabled", &TEST_FRONT_CACHE);
        assert_eq!(disabled_cache.get(&2), Some("disabled"));
        TEST_FRONT_CACHE.with(|front_cache| {
            assert_eq!(front_cache.borrow_mut().get(&2), None);
        });
    }

    #[test]
    fn insert_with_front_does_not_clone_when_the_front_cache_is_disabled() {
        let metrics = Arc::new(RuntimeMetrics::new());
        let cache = ShardedCache::new(
            CacheKind::Layout,
            metrics,
            None,
            Some(FrontCachePolicy::new(false, 2)),
        );

        cache.insert_with_front(
            CloneForbidden(1),
            CloneForbidden(2),
            &CLONE_FORBIDDEN_FRONT_CACHE,
        );
        assert_eq!(cache.size_report().1.entries, 1);
    }

    #[test]
    fn hit_promotes_entry() {
        let mut cache = FrontCache::default();
        cache.insert("first", 1, 2);
        cache.insert("second", 2, 2);

        assert_eq!(cache.get(&"first"), Some(1));
        cache.insert("third", 3, 2);

        assert_eq!(cache.get(&"first"), Some(1));
        assert_eq!(cache.get(&"second"), None);
        assert_eq!(cache.get(&"third"), Some(3));
    }

    #[test]
    fn insert_evicts_least_recently_used_entry() {
        let mut cache = FrontCache::default();
        cache.insert(1, "one", 2);
        cache.insert(2, "two", 2);
        cache.insert(3, "three", 2);

        assert_eq!(cache.get(&1), None);
        assert_eq!(cache.get(&2), Some("two"));
        assert_eq!(cache.get(&3), Some("three"));
    }

    #[test]
    fn resize_resets_entries() {
        let mut cache = FrontCache::default();
        cache.insert(1, "one", 2);
        cache.insert(2, "two", 2);

        cache.insert(3, "three", 3);

        assert_eq!(cache.get(&1), None);
        assert_eq!(cache.get(&2), None);
        assert_eq!(cache.get(&3), Some("three"));
    }

    #[test]
    fn zero_capacity_disables_and_later_reenables_cache() {
        let mut cache = FrontCache::default();
        cache.insert(1, "one", 2);
        cache.insert(2, "two", 0);

        assert_eq!(cache.get(&1), None);
        assert_eq!(cache.get(&2), None);

        cache.insert(3, "three", 2);

        assert_eq!(cache.get(&1), None);
        assert_eq!(cache.get(&3), Some("three"));
    }

    #[cfg(feature = "multithreading")]
    #[test]
    fn concrete_cache_aliases_are_send_and_sync() {
        fn assert_send_and_sync<T: Send + Sync>() {}

        assert_send_and_sync::<ShardedCache<u8, u8>>();
        assert_send_and_sync::<LockedCache<u8, u8>>();
    }
}
