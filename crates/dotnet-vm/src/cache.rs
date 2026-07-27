use dashmap::DashMap;
use dotnet_metrics::{CacheEvent, CacheKind, CacheSize, RuntimeMetrics};
use dotnet_utils::sync::RwLock;
use lru::LruCache;
use std::{collections::HashMap, hash::Hash, mem::size_of, num::NonZeroUsize, sync::Arc};

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

    /// Records key cloning performed by a cache caller.
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
                bytes: (entries as u64).saturating_mul((size_of::<K>() + size_of::<V>()) as u64),
            },
        )
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
    /// their limit. The victim selection is neither random nor LRU.
    #[inline]
    pub(crate) fn insert(&self, key: K, value: V) {
        let Some(max_entries) = self.capacity else {
            self.store.insert(key, value);
            return;
        };

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
    use std::{mem::size_of, sync::Arc};

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
            size.bytes,
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
