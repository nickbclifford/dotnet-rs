//! Runtime and cache metric counters used across the VM.
use serde::Serialize;
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

#[cfg(feature = "bench-instrumentation")]
use std::{collections::BTreeMap, sync::Mutex};

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
    pub value_type: CacheStat,
    pub has_finalizer: CacheStat,
    pub overrides: CacheStat,
    pub method_info: CacheStat,
    pub assembly_type: CacheStat,
    pub assembly_method: CacheStat,
    pub shared_runtime_types: CacheStat,
    pub shared_runtime_methods: CacheStat,
    pub shared_runtime_fields: CacheStat,
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
        writeln!(f, "  Value Type Cache:       {}", self.value_type)?;
        writeln!(f, "  Has Finalizer Cache:    {}", self.has_finalizer)?;
        writeln!(f, "  Overrides Cache:        {}", self.overrides)?;
        writeln!(f, "  Method Info Cache:      {}", self.method_info)?;
        writeln!(f, "  Assembly Type Cache:    {}", self.assembly_type)?;
        writeln!(f, "  Assembly Method Cache:  {}", self.assembly_method)?;
        writeln!(f, "  Shared Type Cache:      {}", self.shared_runtime_types)?;
        writeln!(
            f,
            "  Shared Method Cache:    {}",
            self.shared_runtime_methods
        )?;
        writeln!(
            f,
            "  Shared Field Cache:     {}",
            self.shared_runtime_fields
        )?;
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
    pub layout_bytes: u64,
    pub vmt_size: usize,
    pub vmt_bytes: u64,
    pub intrinsic_size: usize,
    pub intrinsic_bytes: u64,
    pub intrinsic_field_size: usize,
    pub intrinsic_field_bytes: u64,
    pub hierarchy_size: usize,
    pub hierarchy_bytes: u64,
    pub static_field_layout_size: usize,
    pub static_field_layout_bytes: u64,
    pub instance_field_layout_size: usize,
    pub instance_field_layout_bytes: u64,
    pub value_type_size: usize,
    pub value_type_bytes: u64,
    pub has_finalizer_size: usize,
    pub has_finalizer_bytes: u64,
    pub overrides_size: usize,
    pub overrides_bytes: u64,
    pub method_info_size: usize,
    pub method_info_bytes: u64,
    pub assembly_type_info: (u64, u64, usize),
    pub assembly_method_info: (u64, u64, usize),
    pub shared_runtime_types_size: usize,
    pub shared_runtime_types_bytes: u64,
    pub shared_runtime_methods_size: usize,
    pub shared_runtime_methods_bytes: u64,
    pub shared_runtime_fields_size: usize,
    pub shared_runtime_fields_bytes: u64,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum OpcodeCategory {
    Arithmetic,
    Calls,
    Comparisons,
    Conversions,
    Exceptions,
    Flow,
    Memory,
    Objects,
    Reflection,
    Stack,
    Other,
}

impl OpcodeCategory {
    pub fn as_key(self) -> &'static str {
        match self {
            Self::Arithmetic => "arithmetic",
            Self::Calls => "calls",
            Self::Comparisons => "comparisons",
            Self::Conversions => "conversions",
            Self::Exceptions => "exceptions",
            Self::Flow => "flow",
            Self::Memory => "memory",
            Self::Objects => "objects",
            Self::Reflection => "reflection",
            Self::Stack => "stack",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeMetricsSnapshot {
    pub gc_pause_total_us: u64,
    pub gc_pause_count: u64,
    pub lock_contention_count: u64,
    pub lock_contention_total_us: u64,
    pub current_gc_allocated: u64,
    pub current_external_allocated: u64,
    pub cache_stats: CacheStats,
    #[cfg(feature = "bench-instrumentation")]
    pub bench: BenchInstrumentationSnapshot,
}

#[cfg(feature = "bench-instrumentation")]
#[derive(Debug, Clone, Serialize)]
pub struct BenchInstrumentationSnapshot {
    pub eval_stack_reallocations: u64,
    pub eval_stack_pointer_fixup_count: u64,
    pub eval_stack_pointer_fixup_total_ns: u64,
    pub opcode_dispatch_total: u64,
    pub opcode_dispatch_by_category: BTreeMap<String, u64>,
    pub intrinsic_call_total: u64,
    pub intrinsic_calls_by_signature: BTreeMap<String, u64>,
    pub cache_key_clone_total: u64,
    pub cache_key_clones_by_cache: BTreeMap<String, u64>,
    pub front_cache_hits_by_cache: BTreeMap<String, u64>,
    pub front_cache_misses_by_cache: BTreeMap<String, u64>,
    pub cache_memory_bytes_total: u64,
    pub cache_memory_bytes_by_cache: BTreeMap<String, u64>,
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
    pub value_type_cache_hits: AtomicU64,
    pub value_type_cache_misses: AtomicU64,
    pub has_finalizer_cache_hits: AtomicU64,
    pub has_finalizer_cache_misses: AtomicU64,
    pub overrides_cache_hits: AtomicU64,
    pub overrides_cache_misses: AtomicU64,
    pub method_info_cache_hits: AtomicU64,
    pub method_info_cache_misses: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    eval_stack_reallocations: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    eval_stack_pointer_fixup_count: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    eval_stack_pointer_fixup_total_ns: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    opcode_dispatch_total: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    opcode_dispatch_arithmetic: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    opcode_dispatch_calls: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    opcode_dispatch_comparisons: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    opcode_dispatch_conversions: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    opcode_dispatch_exceptions: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    opcode_dispatch_flow: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    opcode_dispatch_memory: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    opcode_dispatch_objects: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    opcode_dispatch_reflection: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    opcode_dispatch_stack: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    opcode_dispatch_other: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    intrinsic_call_total: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    intrinsic_calls_by_signature: Mutex<BTreeMap<String, u64>>,
    #[cfg(feature = "bench-instrumentation")]
    method_info_key_clones: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    vmt_key_clones: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    hierarchy_key_clones: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    method_info_front_cache_hits: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    method_info_front_cache_misses: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    vmt_front_cache_hits: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    vmt_front_cache_misses: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    hierarchy_front_cache_hits: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    hierarchy_front_cache_misses: AtomicU64,
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

    #[inline]
    pub fn record_value_type_cache_hit(&self) {
        self.value_type_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_value_type_cache_miss(&self) {
        self.value_type_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_has_finalizer_cache_hit(&self) {
        self.has_finalizer_cache_hits
            .fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_has_finalizer_cache_miss(&self) {
        self.has_finalizer_cache_misses
            .fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_overrides_cache_hit(&self) {
        self.overrides_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_overrides_cache_miss(&self) {
        self.overrides_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_method_info_cache_hit(&self) {
        self.method_info_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_method_info_cache_miss(&self) {
        self.method_info_cache_misses
            .fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(feature = "bench-instrumentation")]
    #[inline]
    pub fn record_eval_stack_reallocation(&self, pointer_fixup_duration: Duration) {
        self.eval_stack_reallocations
            .fetch_add(1, Ordering::Relaxed);
        self.eval_stack_pointer_fixup_count
            .fetch_add(1, Ordering::Relaxed);
        self.eval_stack_pointer_fixup_total_ns
            .fetch_add(pointer_fixup_duration.as_nanos() as u64, Ordering::Relaxed);
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    #[inline]
    pub fn record_eval_stack_reallocation(&self, _pointer_fixup_duration: Duration) {}

    #[cfg(feature = "bench-instrumentation")]
    #[inline]
    pub fn record_opcode_dispatch(&self, category: OpcodeCategory) {
        self.opcode_dispatch_total.fetch_add(1, Ordering::Relaxed);
        match category {
            OpcodeCategory::Arithmetic => {
                self.opcode_dispatch_arithmetic
                    .fetch_add(1, Ordering::Relaxed);
            }
            OpcodeCategory::Calls => {
                self.opcode_dispatch_calls.fetch_add(1, Ordering::Relaxed);
            }
            OpcodeCategory::Comparisons => {
                self.opcode_dispatch_comparisons
                    .fetch_add(1, Ordering::Relaxed);
            }
            OpcodeCategory::Conversions => {
                self.opcode_dispatch_conversions
                    .fetch_add(1, Ordering::Relaxed);
            }
            OpcodeCategory::Exceptions => {
                self.opcode_dispatch_exceptions
                    .fetch_add(1, Ordering::Relaxed);
            }
            OpcodeCategory::Flow => {
                self.opcode_dispatch_flow.fetch_add(1, Ordering::Relaxed);
            }
            OpcodeCategory::Memory => {
                self.opcode_dispatch_memory.fetch_add(1, Ordering::Relaxed);
            }
            OpcodeCategory::Objects => {
                self.opcode_dispatch_objects.fetch_add(1, Ordering::Relaxed);
            }
            OpcodeCategory::Reflection => {
                self.opcode_dispatch_reflection
                    .fetch_add(1, Ordering::Relaxed);
            }
            OpcodeCategory::Stack => {
                self.opcode_dispatch_stack.fetch_add(1, Ordering::Relaxed);
            }
            OpcodeCategory::Other => {
                self.opcode_dispatch_other.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    #[inline]
    pub fn record_opcode_dispatch(&self, _category: OpcodeCategory) {}

    #[cfg(feature = "bench-instrumentation")]
    pub fn record_intrinsic_signature_call(&self, signature: impl Into<String>) {
        self.intrinsic_call_total.fetch_add(1, Ordering::Relaxed);
        let mut map = self
            .intrinsic_calls_by_signature
            .lock()
            .expect("intrinsic metric lock poisoned");
        *map.entry(signature.into()).or_insert(0) += 1;
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    pub fn record_intrinsic_signature_call(&self, _signature: impl Into<String>) {}

    #[cfg(feature = "bench-instrumentation")]
    #[inline]
    pub fn record_method_info_key_clones(&self, count: u64) {
        self.method_info_key_clones
            .fetch_add(count, Ordering::Relaxed);
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    #[inline]
    pub fn record_method_info_key_clones(&self, _count: u64) {}

    #[cfg(feature = "bench-instrumentation")]
    #[inline]
    pub fn record_vmt_key_clones(&self, count: u64) {
        self.vmt_key_clones.fetch_add(count, Ordering::Relaxed);
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    #[inline]
    pub fn record_vmt_key_clones(&self, _count: u64) {}

    #[cfg(feature = "bench-instrumentation")]
    #[inline]
    pub fn record_hierarchy_key_clones(&self, count: u64) {
        self.hierarchy_key_clones
            .fetch_add(count, Ordering::Relaxed);
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    #[inline]
    pub fn record_hierarchy_key_clones(&self, _count: u64) {}

    #[cfg(feature = "bench-instrumentation")]
    #[inline]
    pub fn record_method_info_front_cache_hit(&self) {
        self.method_info_front_cache_hits
            .fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    #[inline]
    pub fn record_method_info_front_cache_hit(&self) {}

    #[cfg(feature = "bench-instrumentation")]
    #[inline]
    pub fn record_method_info_front_cache_miss(&self) {
        self.method_info_front_cache_misses
            .fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    #[inline]
    pub fn record_method_info_front_cache_miss(&self) {}

    #[cfg(feature = "bench-instrumentation")]
    #[inline]
    pub fn record_vmt_front_cache_hit(&self) {
        self.vmt_front_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    #[inline]
    pub fn record_vmt_front_cache_hit(&self) {}

    #[cfg(feature = "bench-instrumentation")]
    #[inline]
    pub fn record_vmt_front_cache_miss(&self) {
        self.vmt_front_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    #[inline]
    pub fn record_vmt_front_cache_miss(&self) {}

    #[cfg(feature = "bench-instrumentation")]
    #[inline]
    pub fn record_hierarchy_front_cache_hit(&self) {
        self.hierarchy_front_cache_hits
            .fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    #[inline]
    pub fn record_hierarchy_front_cache_hit(&self) {}

    #[cfg(feature = "bench-instrumentation")]
    #[inline]
    pub fn record_hierarchy_front_cache_miss(&self) {
        self.hierarchy_front_cache_misses
            .fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    #[inline]
    pub fn record_hierarchy_front_cache_miss(&self) {}

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
            value_type: self.stat(
                self.value_type_cache_hits.load(Ordering::Relaxed),
                self.value_type_cache_misses.load(Ordering::Relaxed),
                sizes.value_type_size,
            ),
            has_finalizer: self.stat(
                self.has_finalizer_cache_hits.load(Ordering::Relaxed),
                self.has_finalizer_cache_misses.load(Ordering::Relaxed),
                sizes.has_finalizer_size,
            ),
            overrides: self.stat(
                self.overrides_cache_hits.load(Ordering::Relaxed),
                self.overrides_cache_misses.load(Ordering::Relaxed),
                sizes.overrides_size,
            ),
            method_info: self.stat(
                self.method_info_cache_hits.load(Ordering::Relaxed),
                self.method_info_cache_misses.load(Ordering::Relaxed),
                sizes.method_info_size,
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
            shared_runtime_types: self.stat(0, 0, sizes.shared_runtime_types_size),
            shared_runtime_methods: self.stat(0, 0, sizes.shared_runtime_methods_size),
            shared_runtime_fields: self.stat(0, 0, sizes.shared_runtime_fields_size),
        }
    }

    pub fn snapshot(
        &self,
        cache_stats: CacheStats,
        _cache_sizes: CacheSizes,
    ) -> RuntimeMetricsSnapshot {
        RuntimeMetricsSnapshot {
            gc_pause_total_us: self.gc_pause_total_us.load(Ordering::Relaxed),
            gc_pause_count: self.gc_pause_count.load(Ordering::Relaxed),
            lock_contention_count: self.lock_contention_count.load(Ordering::Relaxed),
            lock_contention_total_us: self.lock_contention_total_us.load(Ordering::Relaxed),
            current_gc_allocated: self.current_gc_allocated.load(Ordering::Relaxed),
            current_external_allocated: self.current_external_allocated.load(Ordering::Relaxed),
            cache_stats,
            #[cfg(feature = "bench-instrumentation")]
            bench: self.bench_snapshot(_cache_sizes),
        }
    }

    #[cfg(feature = "bench-instrumentation")]
    pub fn bench_snapshot(&self, cache_sizes: CacheSizes) -> BenchInstrumentationSnapshot {
        let mut opcode_dispatch_by_category = BTreeMap::new();
        opcode_dispatch_by_category.insert(
            OpcodeCategory::Arithmetic.as_key().to_string(),
            self.opcode_dispatch_arithmetic.load(Ordering::Relaxed),
        );
        opcode_dispatch_by_category.insert(
            OpcodeCategory::Calls.as_key().to_string(),
            self.opcode_dispatch_calls.load(Ordering::Relaxed),
        );
        opcode_dispatch_by_category.insert(
            OpcodeCategory::Comparisons.as_key().to_string(),
            self.opcode_dispatch_comparisons.load(Ordering::Relaxed),
        );
        opcode_dispatch_by_category.insert(
            OpcodeCategory::Conversions.as_key().to_string(),
            self.opcode_dispatch_conversions.load(Ordering::Relaxed),
        );
        opcode_dispatch_by_category.insert(
            OpcodeCategory::Exceptions.as_key().to_string(),
            self.opcode_dispatch_exceptions.load(Ordering::Relaxed),
        );
        opcode_dispatch_by_category.insert(
            OpcodeCategory::Flow.as_key().to_string(),
            self.opcode_dispatch_flow.load(Ordering::Relaxed),
        );
        opcode_dispatch_by_category.insert(
            OpcodeCategory::Memory.as_key().to_string(),
            self.opcode_dispatch_memory.load(Ordering::Relaxed),
        );
        opcode_dispatch_by_category.insert(
            OpcodeCategory::Objects.as_key().to_string(),
            self.opcode_dispatch_objects.load(Ordering::Relaxed),
        );
        opcode_dispatch_by_category.insert(
            OpcodeCategory::Reflection.as_key().to_string(),
            self.opcode_dispatch_reflection.load(Ordering::Relaxed),
        );
        opcode_dispatch_by_category.insert(
            OpcodeCategory::Stack.as_key().to_string(),
            self.opcode_dispatch_stack.load(Ordering::Relaxed),
        );
        opcode_dispatch_by_category.insert(
            OpcodeCategory::Other.as_key().to_string(),
            self.opcode_dispatch_other.load(Ordering::Relaxed),
        );
        let intrinsic_calls_by_signature = self
            .intrinsic_calls_by_signature
            .lock()
            .expect("intrinsic metric lock poisoned")
            .clone();
        let mut cache_key_clones_by_cache = BTreeMap::new();
        let method_info_key_clones = self.method_info_key_clones.load(Ordering::Relaxed);
        cache_key_clones_by_cache.insert("method_info".to_string(), method_info_key_clones);
        let vmt_key_clones = self.vmt_key_clones.load(Ordering::Relaxed);
        cache_key_clones_by_cache.insert("vmt".to_string(), vmt_key_clones);
        let hierarchy_key_clones = self.hierarchy_key_clones.load(Ordering::Relaxed);
        cache_key_clones_by_cache.insert("hierarchy".to_string(), hierarchy_key_clones);
        let cache_key_clone_total = method_info_key_clones + vmt_key_clones + hierarchy_key_clones;

        let mut front_cache_hits_by_cache = BTreeMap::new();
        front_cache_hits_by_cache.insert(
            "method_info".to_string(),
            self.method_info_front_cache_hits.load(Ordering::Relaxed),
        );
        front_cache_hits_by_cache.insert(
            "vmt".to_string(),
            self.vmt_front_cache_hits.load(Ordering::Relaxed),
        );
        front_cache_hits_by_cache.insert(
            "hierarchy".to_string(),
            self.hierarchy_front_cache_hits.load(Ordering::Relaxed),
        );

        let mut front_cache_misses_by_cache = BTreeMap::new();
        front_cache_misses_by_cache.insert(
            "method_info".to_string(),
            self.method_info_front_cache_misses.load(Ordering::Relaxed),
        );
        front_cache_misses_by_cache.insert(
            "vmt".to_string(),
            self.vmt_front_cache_misses.load(Ordering::Relaxed),
        );
        front_cache_misses_by_cache.insert(
            "hierarchy".to_string(),
            self.hierarchy_front_cache_misses.load(Ordering::Relaxed),
        );

        let mut cache_memory_bytes_by_cache = BTreeMap::new();
        cache_memory_bytes_by_cache.insert("layout".to_string(), cache_sizes.layout_bytes);
        cache_memory_bytes_by_cache.insert("vmt".to_string(), cache_sizes.vmt_bytes);
        cache_memory_bytes_by_cache.insert("intrinsic".to_string(), cache_sizes.intrinsic_bytes);
        cache_memory_bytes_by_cache.insert(
            "intrinsic_field".to_string(),
            cache_sizes.intrinsic_field_bytes,
        );
        cache_memory_bytes_by_cache.insert("hierarchy".to_string(), cache_sizes.hierarchy_bytes);
        cache_memory_bytes_by_cache.insert(
            "static_field_layout".to_string(),
            cache_sizes.static_field_layout_bytes,
        );
        cache_memory_bytes_by_cache.insert(
            "instance_field_layout".to_string(),
            cache_sizes.instance_field_layout_bytes,
        );
        cache_memory_bytes_by_cache.insert("value_type".to_string(), cache_sizes.value_type_bytes);
        cache_memory_bytes_by_cache
            .insert("has_finalizer".to_string(), cache_sizes.has_finalizer_bytes);
        cache_memory_bytes_by_cache.insert("overrides".to_string(), cache_sizes.overrides_bytes);
        cache_memory_bytes_by_cache
            .insert("method_info".to_string(), cache_sizes.method_info_bytes);
        cache_memory_bytes_by_cache.insert(
            "shared_runtime_types".to_string(),
            cache_sizes.shared_runtime_types_bytes,
        );
        cache_memory_bytes_by_cache.insert(
            "shared_runtime_methods".to_string(),
            cache_sizes.shared_runtime_methods_bytes,
        );
        cache_memory_bytes_by_cache.insert(
            "shared_runtime_fields".to_string(),
            cache_sizes.shared_runtime_fields_bytes,
        );
        let cache_memory_bytes_total = cache_memory_bytes_by_cache.values().copied().sum();

        BenchInstrumentationSnapshot {
            eval_stack_reallocations: self.eval_stack_reallocations.load(Ordering::Relaxed),
            eval_stack_pointer_fixup_count: self
                .eval_stack_pointer_fixup_count
                .load(Ordering::Relaxed),
            eval_stack_pointer_fixup_total_ns: self
                .eval_stack_pointer_fixup_total_ns
                .load(Ordering::Relaxed),
            opcode_dispatch_total: self.opcode_dispatch_total.load(Ordering::Relaxed),
            opcode_dispatch_by_category,
            intrinsic_call_total: self.intrinsic_call_total.load(Ordering::Relaxed),
            intrinsic_calls_by_signature,
            cache_key_clone_total,
            cache_key_clones_by_cache,
            front_cache_hits_by_cache,
            front_cache_misses_by_cache,
            cache_memory_bytes_total,
            cache_memory_bytes_by_cache,
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

#[cfg(feature = "bench-instrumentation")]
std::thread_local! {
    static ACTIVE_RUNTIME_METRICS: std::cell::Cell<Option<*const RuntimeMetrics>> = const { std::cell::Cell::new(None) };
}

#[cfg(feature = "bench-instrumentation")]
pub struct ActiveRuntimeMetricsGuard {
    previous: Option<*const RuntimeMetrics>,
}

#[cfg(feature = "bench-instrumentation")]
impl ActiveRuntimeMetricsGuard {
    pub fn enter(metrics: &RuntimeMetrics) -> Self {
        let previous = ACTIVE_RUNTIME_METRICS.with(|slot| {
            let prev = slot.get();
            slot.set(Some(metrics as *const RuntimeMetrics));
            prev
        });
        Self { previous }
    }
}

#[cfg(feature = "bench-instrumentation")]
impl Drop for ActiveRuntimeMetricsGuard {
    fn drop(&mut self) {
        ACTIVE_RUNTIME_METRICS.with(|slot| slot.set(self.previous));
    }
}

#[cfg(feature = "bench-instrumentation")]
pub fn record_active_eval_stack_reallocation(pointer_fixup_duration: Duration) {
    ACTIVE_RUNTIME_METRICS.with(|slot| {
        if let Some(metrics_ptr) = slot.get() {
            // SAFETY: The guard guarantees the pointed RuntimeMetrics outlives this scope.
            let metrics = unsafe { &*metrics_ptr };
            metrics.record_eval_stack_reallocation(pointer_fixup_duration);
        }
    });
}
