//! Runtime and cache metric counters used across the VM.
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

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
    pub delegate_dispatch: CacheStat,
    pub static_constrained: CacheStat,
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
        writeln!(f, "  Delegate Dispatch:      {}", self.delegate_dispatch)?;
        writeln!(f, "  Static Constrained:     {}", self.static_constrained)?;
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

#[derive(Debug, Clone, Copy, Default)]
pub struct CacheSize {
    pub entries: usize,
    /// Estimated inline size of stored key/value pairs; excludes backing-store overhead and
    /// heap-allocated content behind `Arc` or `Box`.
    pub pointer_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CacheSizes {
    pub caches: [CacheSize; CacheKind::COUNT],
    pub assembly_type_info: (u64, u64, usize),
    pub assembly_method_info: (u64, u64, usize),
}

macro_rules! count_cache_kinds {
    ($($kind:ident),* $(,)?) => {
        <[()]>::len(&[$(count_cache_kinds!(@one $kind)),*])
    };
    (@one $kind:ident) => {
        ()
    };
}

macro_rules! define_cache_kinds {
    (
        global {
            $($global:ident => { key: $global_key:literal, front: $global_front:literal }),+ $(,)?
        }
        shared {
            $($shared:ident => { key: $shared_key:literal, front: $shared_front:literal }),+ $(,)?
        }
    ) => {
        /// Stable identities for cache metrics and indexed size reports.
        #[repr(usize)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum CacheKind {
            $($global,)+
            $($shared,)+
        }

        impl CacheKind {
            /// VM-global cache kinds in registry/report order.
            pub const GLOBAL: [Self; count_cache_kinds!($($global),+)] =
                [$(Self::$global,)+];

            /// Every cache kind in its stable metric-array order.
            pub const ALL: [Self;
                count_cache_kinds!($($global),+) + count_cache_kinds!($($shared),+)
            ] = [$(Self::$global,)+ $(Self::$shared,)+];

            /// Number of cache kinds represented in metric arrays.
            pub const COUNT: usize = Self::ALL.len();

            /// Returns this kind's stable index in [`Self::ALL`].
            #[inline]
            pub const fn as_index(self) -> usize {
                self as usize
            }

            /// Returns this kind's stable snake-case benchmark-map key.
            #[inline]
            pub const fn as_key(self) -> &'static str {
                match self {
                    $(Self::$global => $global_key,)+
                    $(Self::$shared => $shared_key,)+
                }
            }

            /// Whether benchmark snapshots expose a front-cache tier for this kind.
            #[inline]
            pub const fn has_front_cache(self) -> bool {
                match self {
                    $(Self::$global => $global_front,)+
                    $(Self::$shared => $shared_front,)+
                }
            }
        }
    };
}

// This is the declaration site for metric identity, stable keys, array order, global-cache
// membership, and front-cache snapshot membership. Adding a cache kind must not require parallel
// match arms or counter fields elsewhere in this crate. Append new global kinds after MethodInfo
// so the current CacheStats display order remains a stable prefix.
define_cache_kinds! {
    global {
        Layout => { key: "layout", front: false },
        Vmt => { key: "vmt", front: true },
        Intrinsic => { key: "intrinsic", front: false },
        IntrinsicField => { key: "intrinsic_field", front: false },
        Hierarchy => { key: "hierarchy", front: true },
        StaticFieldLayout => { key: "static_field_layout", front: false },
        InstanceFieldLayout => { key: "instance_field_layout", front: false },
        ValueType => { key: "value_type", front: false },
        HasFinalizer => { key: "has_finalizer", front: false },
        Overrides => { key: "overrides", front: false },
        MethodInfo => { key: "method_info", front: true },
        DelegateDispatch => { key: "delegate_dispatch", front: false },
        StaticConstrained => { key: "static_constrained", front: false },
    }
    shared {
        SharedRuntimeTypes => { key: "shared_runtime_types", front: false },
        SharedRuntimeMethods => { key: "shared_runtime_methods", front: false },
        SharedRuntimeFields => { key: "shared_runtime_fields", front: false },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CacheEvent {
    Hit,
    Miss,
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

/// The runtime operation that charged bytes to an arena's allocation-pressure budget.
///
/// These counters intentionally describe pressure accounting rather than every Rust allocation:
/// their purpose is to make it clear which VM operations can request a collection.
#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AllocationPressureSource {
    Object,
    Vector,
    String,
    StackPush,
    ValueTypeWrite,
}

impl AllocationPressureSource {
    pub const ALL: [Self; 5] = [
        Self::Object,
        Self::Vector,
        Self::String,
        Self::StackPush,
        Self::ValueTypeWrite,
    ];
    pub const COUNT: usize = Self::ALL.len();

    #[inline]
    pub const fn as_index(self) -> usize {
        self as usize
    }

    #[inline]
    pub const fn as_key(self) -> &'static str {
        match self {
            Self::Object => "object",
            Self::Vector => "vector",
            Self::String => "string",
            Self::StackPush => "stack_push",
            Self::ValueTypeWrite => "value_type_write",
        }
    }
}

/// Benchmark-only allocation-pressure counters for one source.
#[cfg(feature = "bench-instrumentation")]
#[derive(Debug, Clone, Copy, Serialize, Default, PartialEq, Eq)]
pub struct AllocationPressureSnapshot {
    /// Number of charges made to the arena pressure budget.
    pub charge_count: u64,
    /// Total bytes charged to the arena pressure budget.
    pub charged_bytes: u64,
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

#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct ArenaGcPressureSnapshot {
    /// Number of times this arena crossed its allocation threshold and requested collection.
    pub collection_trigger_count: u64,
    /// Lifetime high-water mark of bytes allocated between collections for this arena.
    ///
    /// The per-cycle allocation counter is reset after each collection; this metric intentionally
    /// preserves the largest observed counter value so snapshots retain pressure history.
    pub peak_allocation_counter: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeMetricsSnapshot {
    pub gc_pause_total_us: u64,
    pub gc_pause_count: u64,
    pub gc_pause_samples: u64,
    pub gc_pause_p50_us: u64,
    pub gc_pause_p95_us: u64,
    pub gc_pause_p99_us: u64,
    pub gc_pause_max_us: u64,
    pub lock_contention_count: u64,
    pub lock_contention_total_us: u64,
    pub current_gc_allocated: u64,
    pub gc_pressure_by_arena: BTreeMap<String, ArenaGcPressureSnapshot>,
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
    pub frame_pool_hit_count: u64,
    pub frame_pool_miss_count: u64,
    pub frame_pool_recycle_count: u64,
    pub gc_pause_samples: u64,
    pub gc_pause_p50_us: u64,
    pub gc_pause_p95_us: u64,
    pub gc_pause_p99_us: u64,
    pub gc_pause_max_us: u64,
    pub gc_fixed_point_cycle_count: u64,
    pub gc_fixed_point_iteration_total: u64,
    pub gc_fixed_point_max_iterations_per_cycle: u64,
    pub gc_fixed_point_cross_arena_objects_total: u64,
    pub gc_fixed_point_cross_arena_objects_max_per_iteration: u64,
    pub gc_fixed_point_cross_arena_objects_by_iteration: BTreeMap<String, u64>,
    pub gc_trace_root_count_by_root: BTreeMap<String, u64>,
    pub gc_trace_root_total_ns_by_root: BTreeMap<String, u64>,
    pub layout_scan_count_by_path: BTreeMap<String, u64>,
    pub layout_scan_total_ns_by_path: BTreeMap<String, u64>,
    pub opcode_dispatch_total: u64,
    pub opcode_dispatch_by_category: BTreeMap<String, u64>,
    pub intrinsic_call_total: u64,
    pub intrinsic_calls_by_signature: BTreeMap<String, u64>,
    /// Allocation-pressure charges grouped by the operation that issued them.
    pub allocation_pressure_by_source: BTreeMap<String, AllocationPressureSnapshot>,
    pub cache_key_clone_total: u64,
    pub cache_key_clones_by_cache: BTreeMap<String, u64>,
    pub front_cache_hits_by_cache: BTreeMap<String, u64>,
    pub front_cache_misses_by_cache: BTreeMap<String, u64>,
    pub cache_memory_bytes_total: u64,
    pub cache_memory_bytes_by_cache: BTreeMap<String, u64>,
}

const GC_PAUSE_SAMPLE_WINDOW: usize = 1_000;

#[derive(Debug, Default)]
struct CacheCounters {
    hits: AtomicU64,
    misses: AtomicU64,
}

#[cfg(feature = "bench-instrumentation")]
#[derive(Debug, Default)]
struct AllocationPressureCounters {
    charge_count: AtomicU64,
    charged_bytes: AtomicU64,
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
    /// Snapshot of per-arena allocation-pressure counters
    arena_gc_pressure_by_arena: Mutex<BTreeMap<String, ArenaGcPressureSnapshot>>,
    /// Cache hit/miss counters indexed by [`CacheKind`].
    cache_counters: [CacheCounters; CacheKind::COUNT],
    #[cfg(feature = "bench-instrumentation")]
    eval_stack_reallocations: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    eval_stack_pointer_fixup_count: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    eval_stack_pointer_fixup_total_ns: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    frame_pool_hit_count: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    frame_pool_miss_count: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    frame_pool_recycle_count: AtomicU64,
    gc_pause_samples_us: Mutex<Vec<u64>>,
    #[cfg(feature = "bench-instrumentation")]
    gc_fixed_point_cycle_count: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    gc_fixed_point_iteration_total: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    gc_fixed_point_max_iterations_per_cycle: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    gc_fixed_point_cross_arena_objects_total: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    gc_fixed_point_cross_arena_objects_max_per_iteration: AtomicU64,
    #[cfg(feature = "bench-instrumentation")]
    gc_fixed_point_cross_arena_objects_by_iteration: Mutex<BTreeMap<u64, u64>>,
    #[cfg(feature = "bench-instrumentation")]
    gc_trace_root_count_by_root: Mutex<BTreeMap<String, u64>>,
    #[cfg(feature = "bench-instrumentation")]
    gc_trace_root_total_ns_by_root: Mutex<BTreeMap<String, u64>>,
    #[cfg(feature = "bench-instrumentation")]
    layout_scan_count_by_path: Mutex<BTreeMap<String, u64>>,
    #[cfg(feature = "bench-instrumentation")]
    layout_scan_total_ns_by_path: Mutex<BTreeMap<String, u64>>,
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
    allocation_pressure: [AllocationPressureCounters; AllocationPressureSource::COUNT],
    #[cfg(feature = "bench-instrumentation")]
    cache_key_clones: [AtomicU64; CacheKind::COUNT],
    #[cfg(feature = "bench-instrumentation")]
    front_cache_hits: [AtomicU64; CacheKind::COUNT],
    #[cfg(feature = "bench-instrumentation")]
    front_cache_misses: [AtomicU64; CacheKind::COUNT],
}

impl RuntimeMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_gc_pause(&self, duration: Duration) {
        let duration_us = duration.as_micros() as u64;
        self.gc_pause_total_us
            .fetch_add(duration_us, Ordering::Relaxed);
        self.gc_pause_count.fetch_add(1, Ordering::Relaxed);

        let mut samples = self
            .gc_pause_samples_us
            .lock()
            .expect("gc pause samples lock poisoned");
        if samples.len() == GC_PAUSE_SAMPLE_WINDOW {
            samples.remove(0);
        }
        samples.push(duration_us);
    }

    pub fn record_lock_contention(&self, duration: Duration) {
        self.lock_contention_count.fetch_add(1, Ordering::Relaxed);
        self.lock_contention_total_us
            .fetch_add(duration.as_micros() as u64, Ordering::Relaxed);
    }

    pub fn update_gc_metrics(&self, gc_bytes: u64) {
        self.current_gc_allocated.store(gc_bytes, Ordering::Relaxed);
    }

    pub fn update_arena_gc_pressure_metrics(
        &self,
        gc_pressure_by_arena: BTreeMap<String, ArenaGcPressureSnapshot>,
    ) {
        *self
            .arena_gc_pressure_by_arena
            .lock()
            .expect("arena gc pressure metrics lock poisoned") = gc_pressure_by_arena;
    }

    #[inline]
    pub fn record_cache(&self, kind: CacheKind, event: CacheEvent) {
        let counters = &self.cache_counters[kind.as_index()];
        let counter = match event {
            CacheEvent::Hit => &counters.hits,
            CacheEvent::Miss => &counters.misses,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns the current hit and miss counts for one cache kind.
    pub fn cache_event_counts(&self, kind: CacheKind) -> (u64, u64) {
        let counters = &self.cache_counters[kind.as_index()];
        (
            counters.hits.load(Ordering::Relaxed),
            counters.misses.load(Ordering::Relaxed),
        )
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
    pub fn record_frame_pool_hit(&self) {
        self.frame_pool_hit_count.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    #[inline]
    pub fn record_frame_pool_hit(&self) {}

    #[cfg(feature = "bench-instrumentation")]
    #[inline]
    pub fn record_frame_pool_miss(&self) {
        self.frame_pool_miss_count.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    #[inline]
    pub fn record_frame_pool_miss(&self) {}

    #[cfg(feature = "bench-instrumentation")]
    #[inline]
    pub fn record_frame_pool_recycle(&self) {
        self.frame_pool_recycle_count
            .fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    #[inline]
    pub fn record_frame_pool_recycle(&self) {}

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
    pub fn record_allocation_pressure(&self, source: AllocationPressureSource, bytes: usize) {
        let counters = &self.allocation_pressure[source.as_index()];
        counters.charge_count.fetch_add(1, Ordering::Relaxed);
        counters
            .charged_bytes
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    #[inline]
    pub fn record_allocation_pressure(&self, _source: AllocationPressureSource, _bytes: usize) {}

    #[cfg(feature = "bench-instrumentation")]
    pub fn record_gc_fixed_point_iteration(&self, iteration: u64, object_count: u64) {
        self.gc_fixed_point_iteration_total
            .fetch_add(1, Ordering::Relaxed);
        self.gc_fixed_point_cross_arena_objects_total
            .fetch_add(object_count, Ordering::Relaxed);
        update_atomic_max(
            &self.gc_fixed_point_cross_arena_objects_max_per_iteration,
            object_count,
        );

        let mut by_iteration = self
            .gc_fixed_point_cross_arena_objects_by_iteration
            .lock()
            .expect("gc fixed-point-by-iteration lock poisoned");
        *by_iteration.entry(iteration).or_insert(0) += object_count;
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    pub fn record_gc_fixed_point_iteration(&self, _iteration: u64, _object_count: u64) {}

    #[cfg(feature = "bench-instrumentation")]
    pub fn record_gc_fixed_point_cycle(&self, iterations: u64) {
        self.gc_fixed_point_cycle_count
            .fetch_add(1, Ordering::Relaxed);
        update_atomic_max(&self.gc_fixed_point_max_iterations_per_cycle, iterations);
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    pub fn record_gc_fixed_point_cycle(&self, _iterations: u64) {}

    #[cfg(feature = "bench-instrumentation")]
    pub fn record_gc_trace_root_timing(&self, root: impl Into<String>, duration: Duration) {
        let root = root.into();
        let duration_ns = duration.as_nanos() as u64;
        {
            let mut count_by_root = self
                .gc_trace_root_count_by_root
                .lock()
                .expect("gc trace root count lock poisoned");
            *count_by_root.entry(root.clone()).or_insert(0) += 1;
        }
        let mut total_ns_by_root = self
            .gc_trace_root_total_ns_by_root
            .lock()
            .expect("gc trace root timing lock poisoned");
        *total_ns_by_root.entry(root).or_insert(0) += duration_ns;
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    pub fn record_gc_trace_root_timing(&self, _root: impl Into<String>, _duration: Duration) {}

    #[cfg(feature = "bench-instrumentation")]
    pub fn record_layout_scan_timing(&self, path: impl Into<String>, duration: Duration) {
        let path = path.into();
        let duration_ns = duration.as_nanos() as u64;
        {
            let mut count_by_path = self
                .layout_scan_count_by_path
                .lock()
                .expect("layout scan count lock poisoned");
            *count_by_path.entry(path.clone()).or_insert(0) += 1;
        }
        let mut total_ns_by_path = self
            .layout_scan_total_ns_by_path
            .lock()
            .expect("layout scan timing lock poisoned");
        *total_ns_by_path.entry(path).or_insert(0) += duration_ns;
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    pub fn record_layout_scan_timing(&self, _path: impl Into<String>, _duration: Duration) {}

    #[cfg(feature = "bench-instrumentation")]
    #[inline]
    pub fn record_cache_key_clones(&self, kind: CacheKind, count: u64) {
        self.cache_key_clones[kind.as_index()].fetch_add(count, Ordering::Relaxed);
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    #[inline]
    pub fn record_cache_key_clones(&self, _kind: CacheKind, _count: u64) {}

    #[cfg(feature = "bench-instrumentation")]
    #[inline]
    pub fn record_front_cache(&self, kind: CacheKind, event: CacheEvent) {
        let counters = match event {
            CacheEvent::Hit => &self.front_cache_hits,
            CacheEvent::Miss => &self.front_cache_misses,
        };
        counters[kind.as_index()].fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(not(feature = "bench-instrumentation"))]
    #[inline]
    pub fn record_front_cache(&self, _kind: CacheKind, _event: CacheEvent) {}

    pub fn cache_statistics(&self, sizes: CacheSizes) -> CacheStats {
        let caches = sizes.caches;
        let cache_stat = |kind: CacheKind| self.cache_stat(kind, caches[kind.as_index()].entries);
        CacheStats {
            layout: cache_stat(CacheKind::Layout),
            vmt: cache_stat(CacheKind::Vmt),
            intrinsic: cache_stat(CacheKind::Intrinsic),
            intrinsic_field: cache_stat(CacheKind::IntrinsicField),
            hierarchy: cache_stat(CacheKind::Hierarchy),
            static_field_layout: cache_stat(CacheKind::StaticFieldLayout),
            instance_field_layout: cache_stat(CacheKind::InstanceFieldLayout),
            value_type: cache_stat(CacheKind::ValueType),
            has_finalizer: cache_stat(CacheKind::HasFinalizer),
            overrides: cache_stat(CacheKind::Overrides),
            method_info: cache_stat(CacheKind::MethodInfo),
            delegate_dispatch: cache_stat(CacheKind::DelegateDispatch),
            static_constrained: cache_stat(CacheKind::StaticConstrained),
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
            shared_runtime_types: cache_stat(CacheKind::SharedRuntimeTypes),
            shared_runtime_methods: cache_stat(CacheKind::SharedRuntimeMethods),
            shared_runtime_fields: cache_stat(CacheKind::SharedRuntimeFields),
        }
    }

    pub fn snapshot(
        &self,
        cache_stats: CacheStats,
        _cache_sizes: CacheSizes,
    ) -> RuntimeMetricsSnapshot {
        let (gc_pause_samples, gc_pause_p50_us, gc_pause_p95_us, gc_pause_p99_us, gc_pause_max_us) =
            self.gc_pause_histogram_snapshot();

        let gc_pressure_by_arena = self
            .arena_gc_pressure_by_arena
            .lock()
            .expect("arena gc pressure metrics lock poisoned")
            .clone();

        RuntimeMetricsSnapshot {
            gc_pause_total_us: self.gc_pause_total_us.load(Ordering::Relaxed),
            gc_pause_count: self.gc_pause_count.load(Ordering::Relaxed),
            gc_pause_samples,
            gc_pause_p50_us,
            gc_pause_p95_us,
            gc_pause_p99_us,
            gc_pause_max_us,
            lock_contention_count: self.lock_contention_count.load(Ordering::Relaxed),
            lock_contention_total_us: self.lock_contention_total_us.load(Ordering::Relaxed),
            current_gc_allocated: self.current_gc_allocated.load(Ordering::Relaxed),
            gc_pressure_by_arena,
            cache_stats,
            #[cfg(feature = "bench-instrumentation")]
            bench: self.bench_snapshot(_cache_sizes),
        }
    }

    fn gc_pause_histogram_snapshot(&self) -> (u64, u64, u64, u64, u64) {
        let mut gc_pause_samples = self
            .gc_pause_samples_us
            .lock()
            .expect("gc pause samples lock poisoned")
            .clone();
        gc_pause_samples.sort_unstable();
        let gc_pause_sample_count = gc_pause_samples.len() as u64;
        let gc_pause_p50_us = percentile_sorted(&gc_pause_samples, 50);
        let gc_pause_p95_us = percentile_sorted(&gc_pause_samples, 95);
        let gc_pause_p99_us = percentile_sorted(&gc_pause_samples, 99);
        let gc_pause_max_us = gc_pause_samples.last().copied().unwrap_or(0);

        (
            gc_pause_sample_count,
            gc_pause_p50_us,
            gc_pause_p95_us,
            gc_pause_p99_us,
            gc_pause_max_us,
        )
    }

    #[cfg(feature = "bench-instrumentation")]
    pub fn bench_snapshot(&self, cache_sizes: CacheSizes) -> BenchInstrumentationSnapshot {
        let (gc_pause_samples, gc_pause_p50_us, gc_pause_p95_us, gc_pause_p99_us, gc_pause_max_us) =
            self.gc_pause_histogram_snapshot();

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
        let allocation_pressure_by_source = AllocationPressureSource::ALL
            .into_iter()
            .map(|source| {
                let counters = &self.allocation_pressure[source.as_index()];
                (
                    source.as_key().to_string(),
                    AllocationPressureSnapshot {
                        charge_count: counters.charge_count.load(Ordering::Relaxed),
                        charged_bytes: counters.charged_bytes.load(Ordering::Relaxed),
                    },
                )
            })
            .collect();
        let gc_fixed_point_cross_arena_objects_by_iteration = self
            .gc_fixed_point_cross_arena_objects_by_iteration
            .lock()
            .expect("gc fixed-point-by-iteration lock poisoned")
            .iter()
            .map(|(iteration, count)| (iteration.to_string(), *count))
            .collect();
        let gc_trace_root_count_by_root = self
            .gc_trace_root_count_by_root
            .lock()
            .expect("gc trace root count lock poisoned")
            .clone();
        let gc_trace_root_total_ns_by_root = self
            .gc_trace_root_total_ns_by_root
            .lock()
            .expect("gc trace root timing lock poisoned")
            .clone();
        let layout_scan_count_by_path = self
            .layout_scan_count_by_path
            .lock()
            .expect("layout scan count lock poisoned")
            .clone();
        let layout_scan_total_ns_by_path = self
            .layout_scan_total_ns_by_path
            .lock()
            .expect("layout scan timing lock poisoned")
            .clone();
        let mut cache_key_clones_by_cache = BTreeMap::new();
        let mut cache_key_clone_total = 0;
        for kind in CacheKind::ALL {
            let count = self.cache_key_clones[kind.as_index()].load(Ordering::Relaxed);
            cache_key_clones_by_cache.insert(kind.as_key().to_string(), count);
            cache_key_clone_total += count;
        }

        let mut front_cache_hits_by_cache = BTreeMap::new();
        let mut front_cache_misses_by_cache = BTreeMap::new();
        for kind in CacheKind::ALL
            .into_iter()
            .filter(|kind| kind.has_front_cache())
        {
            front_cache_hits_by_cache.insert(
                kind.as_key().to_string(),
                self.front_cache_hits[kind.as_index()].load(Ordering::Relaxed),
            );
            front_cache_misses_by_cache.insert(
                kind.as_key().to_string(),
                self.front_cache_misses[kind.as_index()].load(Ordering::Relaxed),
            );
        }

        let cache_memory_bytes_by_cache = CacheKind::ALL
            .into_iter()
            .map(|kind| {
                (
                    kind.as_key().to_string(),
                    cache_sizes.caches[kind.as_index()].pointer_bytes,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let cache_memory_bytes_total = cache_memory_bytes_by_cache.values().copied().sum();

        BenchInstrumentationSnapshot {
            eval_stack_reallocations: self.eval_stack_reallocations.load(Ordering::Relaxed),
            eval_stack_pointer_fixup_count: self
                .eval_stack_pointer_fixup_count
                .load(Ordering::Relaxed),
            eval_stack_pointer_fixup_total_ns: self
                .eval_stack_pointer_fixup_total_ns
                .load(Ordering::Relaxed),
            frame_pool_hit_count: self.frame_pool_hit_count.load(Ordering::Relaxed),
            frame_pool_miss_count: self.frame_pool_miss_count.load(Ordering::Relaxed),
            frame_pool_recycle_count: self.frame_pool_recycle_count.load(Ordering::Relaxed),
            gc_pause_samples,
            gc_pause_p50_us,
            gc_pause_p95_us,
            gc_pause_p99_us,
            gc_pause_max_us,
            gc_fixed_point_cycle_count: self.gc_fixed_point_cycle_count.load(Ordering::Relaxed),
            gc_fixed_point_iteration_total: self
                .gc_fixed_point_iteration_total
                .load(Ordering::Relaxed),
            gc_fixed_point_max_iterations_per_cycle: self
                .gc_fixed_point_max_iterations_per_cycle
                .load(Ordering::Relaxed),
            gc_fixed_point_cross_arena_objects_total: self
                .gc_fixed_point_cross_arena_objects_total
                .load(Ordering::Relaxed),
            gc_fixed_point_cross_arena_objects_max_per_iteration: self
                .gc_fixed_point_cross_arena_objects_max_per_iteration
                .load(Ordering::Relaxed),
            gc_fixed_point_cross_arena_objects_by_iteration,
            gc_trace_root_count_by_root,
            gc_trace_root_total_ns_by_root,
            layout_scan_count_by_path,
            layout_scan_total_ns_by_path,
            opcode_dispatch_total: self.opcode_dispatch_total.load(Ordering::Relaxed),
            opcode_dispatch_by_category,
            intrinsic_call_total: self.intrinsic_call_total.load(Ordering::Relaxed),
            intrinsic_calls_by_signature,
            allocation_pressure_by_source,
            cache_key_clone_total,
            cache_key_clones_by_cache,
            front_cache_hits_by_cache,
            front_cache_misses_by_cache,
            cache_memory_bytes_total,
            cache_memory_bytes_by_cache,
        }
    }

    fn cache_stat(&self, kind: CacheKind, size: usize) -> CacheStat {
        let (hits, misses) = self.cache_event_counts(kind);
        self.stat(hits, misses, size)
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

#[cfg(feature = "bench-instrumentation")]
pub fn record_active_frame_pool_hit() {
    ACTIVE_RUNTIME_METRICS.with(|slot| {
        if let Some(metrics_ptr) = slot.get() {
            // SAFETY: The guard guarantees the pointed RuntimeMetrics outlives this scope.
            let metrics = unsafe { &*metrics_ptr };
            metrics.record_frame_pool_hit();
        }
    });
}

#[cfg(feature = "bench-instrumentation")]
pub fn record_active_frame_pool_miss() {
    ACTIVE_RUNTIME_METRICS.with(|slot| {
        if let Some(metrics_ptr) = slot.get() {
            // SAFETY: The guard guarantees the pointed RuntimeMetrics outlives this scope.
            let metrics = unsafe { &*metrics_ptr };
            metrics.record_frame_pool_miss();
        }
    });
}

#[cfg(feature = "bench-instrumentation")]
pub fn record_active_frame_pool_recycle() {
    ACTIVE_RUNTIME_METRICS.with(|slot| {
        if let Some(metrics_ptr) = slot.get() {
            // SAFETY: The guard guarantees the pointed RuntimeMetrics outlives this scope.
            let metrics = unsafe { &*metrics_ptr };
            metrics.record_frame_pool_recycle();
        }
    });
}

#[cfg(feature = "bench-instrumentation")]
pub fn record_active_gc_fixed_point_iteration(iteration: u64, object_count: u64) {
    ACTIVE_RUNTIME_METRICS.with(|slot| {
        if let Some(metrics_ptr) = slot.get() {
            // SAFETY: The guard guarantees the pointed RuntimeMetrics outlives this scope.
            let metrics = unsafe { &*metrics_ptr };
            metrics.record_gc_fixed_point_iteration(iteration, object_count);
        }
    });
}

#[cfg(feature = "bench-instrumentation")]
pub fn record_active_gc_fixed_point_cycle(iterations: u64) {
    ACTIVE_RUNTIME_METRICS.with(|slot| {
        if let Some(metrics_ptr) = slot.get() {
            // SAFETY: The guard guarantees the pointed RuntimeMetrics outlives this scope.
            let metrics = unsafe { &*metrics_ptr };
            metrics.record_gc_fixed_point_cycle(iterations);
        }
    });
}

#[cfg(feature = "bench-instrumentation")]
pub fn record_active_gc_trace_root_timing(root: impl Into<String>, duration: Duration) {
    ACTIVE_RUNTIME_METRICS.with(|slot| {
        if let Some(metrics_ptr) = slot.get() {
            // SAFETY: The guard guarantees the pointed RuntimeMetrics outlives this scope.
            let metrics = unsafe { &*metrics_ptr };
            metrics.record_gc_trace_root_timing(root, duration);
        }
    });
}

#[cfg(feature = "bench-instrumentation")]
pub fn record_active_layout_scan_timing(path: impl Into<String>, duration: Duration) {
    ACTIVE_RUNTIME_METRICS.with(|slot| {
        if let Some(metrics_ptr) = slot.get() {
            // SAFETY: The guard guarantees the pointed RuntimeMetrics outlives this scope.
            let metrics = unsafe { &*metrics_ptr };
            metrics.record_layout_scan_timing(path, duration);
        }
    });
}

#[cfg(feature = "bench-instrumentation")]
#[inline]
pub fn record_active_allocation_pressure(source: AllocationPressureSource, bytes: usize) {
    ACTIVE_RUNTIME_METRICS.with(|slot| {
        if let Some(metrics_ptr) = slot.get() {
            // SAFETY: The guard guarantees the pointed RuntimeMetrics outlives this scope.
            let metrics = unsafe { &*metrics_ptr };
            metrics.record_allocation_pressure(source, bytes);
        }
    });
}

#[cfg(feature = "bench-instrumentation")]
fn update_atomic_max(target: &AtomicU64, candidate: u64) {
    let mut current = target.load(Ordering::Relaxed);
    while candidate > current {
        match target.compare_exchange_weak(current, candidate, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => return,
            Err(observed) => current = observed,
        }
    }
}

fn percentile_sorted(samples: &[u64], percentile: u64) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    let idx = ((samples.len() - 1) * percentile as usize) / 100;
    samples[idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_cache_sizes() -> CacheSizes {
        CacheSizes {
            caches: [CacheSize::default(); CacheKind::COUNT],
            assembly_type_info: (0, 0, 0),
            assembly_method_info: (0, 0, 0),
        }
    }

    #[test]
    fn cache_statistics_reads_sizes_by_cache_kind() {
        let metrics = RuntimeMetrics::new();
        let mut sizes = empty_cache_sizes();
        for kind in CacheKind::ALL {
            sizes.caches[kind.as_index()].entries = kind.as_index() + 1;
        }

        let stats = metrics.cache_statistics(sizes);

        assert_eq!(
            stats.layout.size,
            sizes.caches[CacheKind::Layout.as_index()].entries
        );
        assert_eq!(
            stats.vmt.size,
            sizes.caches[CacheKind::Vmt.as_index()].entries
        );
        assert_eq!(
            stats.intrinsic.size,
            sizes.caches[CacheKind::Intrinsic.as_index()].entries
        );
        assert_eq!(
            stats.intrinsic_field.size,
            sizes.caches[CacheKind::IntrinsicField.as_index()].entries
        );
        assert_eq!(
            stats.hierarchy.size,
            sizes.caches[CacheKind::Hierarchy.as_index()].entries
        );
        assert_eq!(
            stats.static_field_layout.size,
            sizes.caches[CacheKind::StaticFieldLayout.as_index()].entries
        );
        assert_eq!(
            stats.instance_field_layout.size,
            sizes.caches[CacheKind::InstanceFieldLayout.as_index()].entries
        );
        assert_eq!(
            stats.value_type.size,
            sizes.caches[CacheKind::ValueType.as_index()].entries
        );
        assert_eq!(
            stats.has_finalizer.size,
            sizes.caches[CacheKind::HasFinalizer.as_index()].entries
        );
        assert_eq!(
            stats.overrides.size,
            sizes.caches[CacheKind::Overrides.as_index()].entries
        );
        assert_eq!(
            stats.method_info.size,
            sizes.caches[CacheKind::MethodInfo.as_index()].entries
        );
        assert_eq!(
            stats.shared_runtime_types.size,
            sizes.caches[CacheKind::SharedRuntimeTypes.as_index()].entries
        );
        assert_eq!(
            stats.shared_runtime_methods.size,
            sizes.caches[CacheKind::SharedRuntimeMethods.as_index()].entries
        );
        assert_eq!(
            stats.shared_runtime_fields.size,
            sizes.caches[CacheKind::SharedRuntimeFields.as_index()].entries
        );
    }

    #[test]
    fn record_cache_updates_cache_statistics_for_selected_kinds() {
        let metrics = RuntimeMetrics::new();

        metrics.record_cache(CacheKind::Layout, CacheEvent::Hit);
        metrics.record_cache(CacheKind::Layout, CacheEvent::Miss);
        metrics.record_cache(CacheKind::MethodInfo, CacheEvent::Hit);
        metrics.record_cache(CacheKind::SharedRuntimeFields, CacheEvent::Miss);

        let stats = metrics.cache_statistics(empty_cache_sizes());

        assert_eq!(stats.layout.hits, 1);
        assert_eq!(stats.layout.misses, 1);
        assert_eq!(stats.method_info.hits, 1);
        assert_eq!(stats.method_info.misses, 0);
        assert_eq!(stats.shared_runtime_fields.hits, 0);
        assert_eq!(stats.shared_runtime_fields.misses, 1);
    }

    #[test]
    fn record_cache_indexes_every_declared_kind() {
        let metrics = RuntimeMetrics::new();

        for kind in CacheKind::ALL {
            metrics.record_cache(kind, CacheEvent::Hit);
            metrics.record_cache(kind, CacheEvent::Miss);
            assert_eq!(metrics.cache_event_counts(kind), (1, 1));
        }
    }

    #[test]
    fn cache_kind_indices_and_keys_are_stable() {
        let stable_keys = [
            (CacheKind::Layout, "layout"),
            (CacheKind::Intrinsic, "intrinsic"),
            (CacheKind::IntrinsicField, "intrinsic_field"),
            (CacheKind::Hierarchy, "hierarchy"),
            (CacheKind::Vmt, "vmt"),
            (CacheKind::StaticFieldLayout, "static_field_layout"),
            (CacheKind::InstanceFieldLayout, "instance_field_layout"),
            (CacheKind::ValueType, "value_type"),
            (CacheKind::HasFinalizer, "has_finalizer"),
            (CacheKind::Overrides, "overrides"),
            (CacheKind::MethodInfo, "method_info"),
            (CacheKind::DelegateDispatch, "delegate_dispatch"),
            (CacheKind::SharedRuntimeTypes, "shared_runtime_types"),
            (CacheKind::SharedRuntimeMethods, "shared_runtime_methods"),
            (CacheKind::SharedRuntimeFields, "shared_runtime_fields"),
        ];

        for (kind, key) in stable_keys {
            assert_eq!(kind.as_key(), key);
        }

        let mut keys = std::collections::BTreeSet::new();
        for (index, kind) in CacheKind::ALL.into_iter().enumerate() {
            assert_eq!(kind.as_index(), index);
            assert!(keys.insert(kind.as_key()));
        }
        assert_eq!(keys.len(), CacheKind::COUNT);
    }

    #[test]
    fn global_kind_order_preserves_legacy_cache_stats_order() {
        let legacy_order = [
            CacheKind::Layout,
            CacheKind::Vmt,
            CacheKind::Intrinsic,
            CacheKind::IntrinsicField,
            CacheKind::Hierarchy,
            CacheKind::StaticFieldLayout,
            CacheKind::InstanceFieldLayout,
            CacheKind::ValueType,
            CacheKind::HasFinalizer,
            CacheKind::Overrides,
            CacheKind::MethodInfo,
            CacheKind::DelegateDispatch,
        ];

        assert!(CacheKind::GLOBAL.starts_with(&legacy_order));
    }

    #[cfg(feature = "bench-instrumentation")]
    #[test]
    fn generic_cache_instrumentation_uses_indexed_counters() {
        let metrics = RuntimeMetrics::new();
        metrics.record_cache_key_clones(CacheKind::MethodInfo, 2);
        metrics.record_cache_key_clones(CacheKind::Vmt, 3);
        metrics.record_cache_key_clones(CacheKind::Layout, 5);
        metrics.record_front_cache(CacheKind::MethodInfo, CacheEvent::Hit);
        metrics.record_front_cache(CacheKind::Vmt, CacheEvent::Miss);
        metrics.record_front_cache(CacheKind::Layout, CacheEvent::Hit);

        let snapshot = metrics.bench_snapshot(empty_cache_sizes());

        assert_eq!(snapshot.cache_key_clone_total, 10);
        assert_eq!(snapshot.cache_key_clones_by_cache.len(), CacheKind::COUNT);
        assert_eq!(
            snapshot.cache_key_clones_by_cache.get("method_info"),
            Some(&2)
        );
        assert_eq!(snapshot.cache_key_clones_by_cache.get("vmt"), Some(&3));
        assert_eq!(snapshot.cache_key_clones_by_cache.get("layout"), Some(&5));
        assert_eq!(snapshot.front_cache_hits_by_cache.len(), 3);
        assert_eq!(snapshot.front_cache_misses_by_cache.len(), 3);
        assert_eq!(
            snapshot.front_cache_hits_by_cache.get("method_info"),
            Some(&1)
        );
        assert_eq!(snapshot.front_cache_misses_by_cache.get("vmt"), Some(&1));
        assert!(!snapshot.front_cache_hits_by_cache.contains_key("layout"));
        assert!(!snapshot.front_cache_misses_by_cache.contains_key("layout"));
    }

    #[cfg(feature = "bench-instrumentation")]
    #[test]
    fn allocation_pressure_instrumentation_reports_every_source() {
        let metrics = RuntimeMetrics::new();
        metrics.record_allocation_pressure(AllocationPressureSource::Vector, 512);
        metrics.record_allocation_pressure(AllocationPressureSource::Vector, 256);
        metrics.record_allocation_pressure(AllocationPressureSource::ValueTypeWrite, 64);

        let snapshot = metrics.bench_snapshot(empty_cache_sizes());

        assert_eq!(
            snapshot.allocation_pressure_by_source.get("vector"),
            Some(&AllocationPressureSnapshot {
                charge_count: 2,
                charged_bytes: 768,
            })
        );
        assert_eq!(
            snapshot
                .allocation_pressure_by_source
                .get("value_type_write"),
            Some(&AllocationPressureSnapshot {
                charge_count: 1,
                charged_bytes: 64,
            })
        );
        assert_eq!(
            snapshot.allocation_pressure_by_source.len(),
            AllocationPressureSource::COUNT
        );
        assert_eq!(
            snapshot.allocation_pressure_by_source.get("stack_push"),
            Some(&AllocationPressureSnapshot::default())
        );
    }

    #[test]
    fn snapshot_includes_always_on_gc_pause_percentiles() {
        let metrics = RuntimeMetrics::new();
        for micros in [10_u64, 20, 30, 40, 50] {
            metrics.record_gc_pause(std::time::Duration::from_micros(micros));
        }

        let cache_sizes = empty_cache_sizes();
        let cache_stats = metrics.cache_statistics(cache_sizes);
        let snapshot = metrics.snapshot(cache_stats, cache_sizes);

        assert_eq!(snapshot.gc_pause_samples, 5);
        assert_eq!(snapshot.gc_pause_p50_us, 30);
        assert_eq!(snapshot.gc_pause_p95_us, 40);
        assert_eq!(snapshot.gc_pause_p99_us, 40);
        assert_eq!(snapshot.gc_pause_max_us, 50);
    }

    #[test]
    fn snapshot_includes_per_arena_gc_pressure_metrics() {
        let metrics = RuntimeMetrics::new();
        let mut by_arena = BTreeMap::new();
        by_arena.insert(
            "7".to_string(),
            ArenaGcPressureSnapshot {
                collection_trigger_count: 3,
                peak_allocation_counter: 8192,
            },
        );
        metrics.update_arena_gc_pressure_metrics(by_arena);

        let cache_sizes = empty_cache_sizes();
        let cache_stats = metrics.cache_statistics(cache_sizes);
        let snapshot = metrics.snapshot(cache_stats, cache_sizes);

        let arena_7 = snapshot
            .gc_pressure_by_arena
            .get("7")
            .expect("expected arena metrics in snapshot");
        assert_eq!(arena_7.collection_trigger_count, 3);
        assert_eq!(arena_7.peak_allocation_counter, 8192);
    }
}
