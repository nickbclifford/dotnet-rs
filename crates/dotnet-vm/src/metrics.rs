use dotnet_utils::sync::{AtomicU64, Ordering};
use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Serialize, Clone, Copy)]
pub struct CacheStat {
    pub hits: u64,
    pub misses: u64,
    pub hit_rate: f64,
    pub size: usize,
}

#[derive(Debug, Serialize, Clone, Copy)]
pub struct CacheStats {
    pub layout: CacheStat,
    pub vmt: CacheStat,
    pub intrinsic: CacheStat,
    pub intrinsic_field: CacheStat,
    pub hierarchy: CacheStat,
    pub static_field_layout: CacheStat,
    pub instance_field_layout: CacheStat,
    pub assembly_type: CacheStat,
    pub assembly_method: CacheStat,
}

impl std::fmt::Display for CacheStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Cache Statistics:")?;
        writeln!(f, "  Layout Cache:           {}", self.layout)?;
        writeln!(f, "  VMT Cache:              {}", self.vmt)?;
        writeln!(f, "  Intrinsic Cache:        {}", self.intrinsic)?;
        writeln!(f, "  Intrinsic Field Cache:  {}", self.intrinsic_field)?;
        writeln!(f, "  Hierarchy Cache:        {}", self.hierarchy)?;
        writeln!(f, "  Static Field Layout:    {}", self.static_field_layout)?;
        writeln!(
            f,
            "  Instance Field Layout:  {}",
            self.instance_field_layout
        )?;
        writeln!(f, "  Assembly Type Cache:    {}", self.assembly_type)?;
        writeln!(f, "  Assembly Method Cache:  {}", self.assembly_method)?;
        Ok(())
    }
}

impl std::fmt::Display for CacheStat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "hits: {:>8}, misses: {:>8}, hit_rate: {:>6.2}%, size: {:>8}",
            self.hits,
            self.misses,
            self.hit_rate * 100.0,
            self.size
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CacheSizes {
    pub layout_size: usize,
    pub vmt_size: usize,
    pub intrinsic_size: usize,
    pub intrinsic_field_size: usize,
    pub hierarchy_size: usize,
    pub static_field_layout_size: usize,
    pub instance_field_layout_size: usize,
    pub assembly_type_info: (u64, u64, usize),
    pub assembly_method_info: (u64, u64, usize),
}

/// Metrics counters.
///
/// All counters use `Ordering::Relaxed` because they are independent and do not
/// synchronize memory between threads. We only care that they are updated
/// atomically, not when those updates become visible to other threads relative
/// to other memory operations.
#[derive(Debug, Default)]
pub struct RuntimeMetrics {
    /// Total time spent in GC stop-the-world pauses (in microseconds)
    pub gc_pause_total_us: AtomicU64,
    /// Number of full GC cycles performed
    pub gc_pause_count: AtomicU64,
    /// Number of times a thread had to block waiting for a lock
    pub lock_contention_count: AtomicU64,
    /// Total time spent waiting for locks (in microseconds)
    pub lock_contention_total_us: AtomicU64,
    /// Current bytes managed by GC-arena across all threads
    pub current_gc_allocated: AtomicU64,
    /// Current bytes allocated externally but tracked by GC-arena
    pub current_external_allocated: AtomicU64,
    /// Cache hit/miss counters
    pub layout_cache_hits: AtomicU64,
    pub layout_cache_misses: AtomicU64,
    pub intrinsic_cache_hits: AtomicU64,
    pub intrinsic_cache_misses: AtomicU64,
    pub intrinsic_field_cache_hits: AtomicU64,
    pub intrinsic_field_cache_misses: AtomicU64,
    pub hierarchy_cache_hits: AtomicU64,
    pub hierarchy_cache_misses: AtomicU64,
    pub vmt_cache_hits: AtomicU64,
    pub vmt_cache_misses: AtomicU64,
    pub static_field_layout_cache_hits: AtomicU64,
    pub static_field_layout_cache_misses: AtomicU64,
    pub instance_field_layout_cache_hits: AtomicU64,
    pub instance_field_layout_cache_misses: AtomicU64,
}

impl RuntimeMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_gc_pause(&self, duration: Duration) {
        self.gc_pause_total_us
            .fetch_add(duration.as_micros() as u64, Ordering::Relaxed);
        self.gc_pause_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_lock_contention(&self, duration: Duration) {
        self.lock_contention_count.fetch_add(1, Ordering::Relaxed);
        self.lock_contention_total_us
            .fetch_add(duration.as_micros() as u64, Ordering::Relaxed);
    }

    pub fn update_gc_metrics(&self, gc_bytes: u64, external_bytes: u64) {
        self.current_gc_allocated.store(gc_bytes, Ordering::Relaxed);
        self.current_external_allocated
            .store(external_bytes, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_layout_cache_hit(&self) {
        self.layout_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_layout_cache_miss(&self) {
        self.layout_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_intrinsic_cache_hit(&self) {
        self.intrinsic_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_intrinsic_cache_miss(&self) {
        self.intrinsic_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_intrinsic_field_cache_hit(&self) {
        self.intrinsic_field_cache_hits
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_intrinsic_field_cache_miss(&self) {
        self.intrinsic_field_cache_misses
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_hierarchy_cache_hit(&self) {
        self.hierarchy_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_hierarchy_cache_miss(&self) {
        self.hierarchy_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_vmt_cache_hit(&self) {
        self.vmt_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_vmt_cache_miss(&self) {
        self.vmt_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_static_field_layout_cache_hit(&self) {
        self.static_field_layout_cache_hits
            .fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_static_field_layout_cache_miss(&self) {
        self.static_field_layout_cache_misses
            .fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_instance_field_layout_cache_hit(&self) {
        self.instance_field_layout_cache_hits
            .fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_instance_field_layout_cache_miss(&self) {
        self.instance_field_layout_cache_misses
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn cache_statistics(&self, sizes: CacheSizes) -> CacheStats {
        CacheStats {
            layout: self.stat(
                self.layout_cache_hits.load(Ordering::Relaxed),
                self.layout_cache_misses.load(Ordering::Relaxed),
                sizes.layout_size,
            ),
            vmt: self.stat(
                self.vmt_cache_hits.load(Ordering::Relaxed),
                self.vmt_cache_misses.load(Ordering::Relaxed),
                sizes.vmt_size,
            ),
            intrinsic: self.stat(
                self.intrinsic_cache_hits.load(Ordering::Relaxed),
                self.intrinsic_cache_misses.load(Ordering::Relaxed),
                sizes.intrinsic_size,
            ),
            intrinsic_field: self.stat(
                self.intrinsic_field_cache_hits.load(Ordering::Relaxed),
                self.intrinsic_field_cache_misses.load(Ordering::Relaxed),
                sizes.intrinsic_field_size,
            ),
            hierarchy: self.stat(
                self.hierarchy_cache_hits.load(Ordering::Relaxed),
                self.hierarchy_cache_misses.load(Ordering::Relaxed),
                sizes.hierarchy_size,
            ),
            static_field_layout: self.stat(
                self.static_field_layout_cache_hits.load(Ordering::Relaxed),
                self.static_field_layout_cache_misses
                    .load(Ordering::Relaxed),
                sizes.static_field_layout_size,
            ),
            instance_field_layout: self.stat(
                self.instance_field_layout_cache_hits
                    .load(Ordering::Relaxed),
                self.instance_field_layout_cache_misses
                    .load(Ordering::Relaxed),
                sizes.instance_field_layout_size,
            ),
            assembly_type: self.stat(
                sizes.assembly_type_info.0,
                sizes.assembly_type_info.1,
                sizes.assembly_type_info.2,
            ),
            assembly_method: self.stat(
                sizes.assembly_method_info.0,
                sizes.assembly_method_info.1,
                sizes.assembly_method_info.2,
            ),
        }
    }

    fn stat(&self, hits: u64, misses: u64, size: usize) -> CacheStat {
        let total = hits + misses;
        let hit_rate = if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        };
        CacheStat {
            hits,
            misses,
            hit_rate,
            size,
        }
    }
}
