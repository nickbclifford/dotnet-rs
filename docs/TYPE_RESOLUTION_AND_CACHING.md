# Type Resolution and Caching

This document describes the type/method/field resolution pipeline, the multi-level caching system, and how generic instantiation interacts with layout computation.

## Overview

Resolution converts metadata tokens (from parsed .NET assemblies) into runtime descriptors used by the execution engine. This is a lazy, cached process that spans several modules:

- **`dotnet-runtime-resolver/src/`**: resolver engine implementation with sub-modules for types, methods, layout, and factory
- **`dotnet-vm/src/resolver/mod.rs`**: `VmResolverService` VM-owned adapter wrapper around resolver-owned resolution logic
- **`dotnet-vm/src/context.rs`**: `ResolutionContext` for scoped resolution with generic parameters
- **`dotnet-vm/src/resolution.rs`**: Resolution traits and helpers
- **`dotnet-vm/src/layout.rs`**: free layout functions delegating to resolver-owned layout code
- **`dotnet-value/src/layout.rs`**: `LayoutManager`, `FieldLayoutManager`, `ArrayLayoutManager`
- **`dotnet-vm/src/state.rs`**: `GlobalCaches` and `SharedGlobalState`
- **`dotnet-types/src/`**: Type descriptors, generics, comparer

## Well-Known Type Registry

Runtime code that refers to a fixed core-library or support-library type uses the `WellKnown` enum from `dotnet-types/src/wkt.rs` rather than repeatedly resolving a metadata name. The enum has 59 variants, is represented as a contiguous `usize` index, and provides `WellKnown::name()` for the canonical metadata name. `WellKnown::from_name()` performs the reverse mapping, including both supported spellings of the nested exception dispatch-state type.

`AssemblyLoader` owns `wkt_table: Vec<OnceLock<TypeDescription>>`, initialized with `WellKnown::COUNT` empty cells. A handle's discriminant indexes its cell. `AssemblyLoader::corlib_wkt()` is the one-time resolution seam: on the first successful access to a variant it runs the normal corlib resolution path and stores the resulting descriptor; later accesses clone the cached descriptor. Resolution errors are deliberately not cached, so a type that is not available yet can be retried. Concurrent first accesses may both perform resolution, but `OnceLock` retains one successful result. `WellKnown::ExceptionDispatchState` has a specialized resolver that tries `System.Exception/DispatchState` before `System.Exception+DispatchState`.

The string API, `corlib_type(&str)`, survives as the dynamic fallback. It uses the separate `dynamic_corlib_cache` and does not route names through the well-known table. Its production callers receive names as runtime data or through APIs that accept arbitrary names: a user string passed to `Type.GetType`, the name parameter accepted by the VM's `throw_by_name` helpers, and metadata names encountered while resolving attribute arguments. Fixed runtime-owned names should use `corlib_wkt()` instead.

Core-library availability changes during a loader's lifetime. During `AssemblyLoader` construction, the embedded support assembly is loaded and its stub mappings are populated, so a BCL name such as `System.Delegate` may already resolve to its `DotnetRs` implementation. By contrast, `mscorlib` and `System.Private.CoreLib` entries are loaded lazily by `get_assembly()` when resolution first needs them. The normal lookup order remains stubs, `mscorlib`, `System.Private.CoreLib`, the support assembly, and the legacy fallback. Consequently the well-known table is created empty rather than filled eagerly during construction: forcing every core-library lookup then would defeat lazy loading and could fail in loader configurations where a core library is not yet available. Per-cell lazy initialization also preserves retry behavior when availability changes later.

## Resolution Pipeline

### Descriptor Representation (`dotnet-types/src/`)

Descriptors are owner-tied and index-based:

- `ResolutionS` stores `(Arc<MetadataArena>, NonNull<Resolution<'static>>)` as a single handle.
- `TypeDescription` stores a `TypeIndex` into the owning resolution.
- `MethodDescription` stores a `MethodMemberIndex` (supports ordinary methods plus property/event accessors) rather than raw pointer identity.
- `FieldDescription` stores a field index into the owning type.
- `ConcreteType` stores its resolution source, specialized base type, and an eagerly memoized `u64` hash. Its `Hash` implementation uses that cached value, so hashing a complete type does not revisit its type tree. Its identity compares the source and base type but excludes `BaseType::Type::value_kind`, which is a cached class/value-type annotation rather than type identity.
- `GenericLookup` stores type and method generic arguments plus an eagerly memoized `u64` hash, so generic-instantiation keys hash without revisiting their argument slices.

This keeps metadata lifetime explicit while preserving stable, hashable descriptor keys for caches.

Every metadata-backed `ConcreteType` retains a non-null `ResolutionS`, whose `Arc<MetadataArena>`
pins the metadata that the descriptor can dereference. A future compact or interned identifier
therefore cannot replace that ownership with a bare ID unless an equally long-lived owner retains
the corresponding arena.
This refactor does not intern descriptors, so the existing arena-pinning model is unchanged.

`ConcreteType`, `GenericLookup`, `ResolutionS`, `MethodDescription`, and `FieldDescription` use
`static_collect!`: they may own ordinary Rust values such as `Arc` and the memoized `u64`, but must
not contain `gc_arena::Gc`/`GcCell` pointers. They can consequently be carried across collections
without adding GC tracing edges.

### Type Resolution (`dotnet-runtime-resolver/src/types.rs`)

1. Metadata token (`UserType`: `TypeDefOrRef`, `TypeSpec`, `TypeRef`) arrives from a CIL instruction.
2. The `ResolutionContext` encapsulates the current assembly scope (`ResolutionS`) and generic parameters via `GenericLookup`.
3. The context delegates to `VmResolverService::locate_type()`, which uses the `AssemblyLoader` to resolve the token into a `TypeDescription`.
4. For generic types or arrays/pointers, `VmResolverService::make_concrete()` substitutes type parameters with concrete arguments using `GenericLookup::make_concrete()`.
5. The result is a `ConcreteType` (representing a specialized type) or a `TypeDescription` (identifying a type definition in a specific assembly).

### Method Resolution (`dotnet-runtime-resolver/src/methods.rs`)

Multi-phase process:
1. **Token → descriptor**: `VmResolverService::locate_method()` maps a metadata token (`UserMethod`) to a `MethodDescription`, applying generic substitutions.
2. **Virtual dispatch**: For `callvirt`, `resolve_virtual_method()` finds the most-derived implementation. It first checks the `vmt_cache` in `GlobalCaches`. If missing, it iterates through ancestors (via `AssemblyLoader::ancestors`) and uses `find_and_cache_method()` to locate the implementation matching the base method's signature.
3. **Generic instantiation**: `find_generic_method()` applies `GenericLookup` from the `ResolutionContext` to specialize the method and its parent type.
4. **Intrinsic check**: `is_intrinsic_cached()` checks if the method has a native implementation by querying `GlobalCaches::intrinsic_cache` and `IntrinsicRegistry`.

### Field Resolution

Fields resolve to `FieldDescription` with computed byte offsets within their containing type. Value type field access requires layout calculation to determine offsets.

### Layout Computation (`dotnet-runtime-resolver/src/layout.rs`)

The resolver-owned `LayoutFactory` computes the physical memory layout of objects and value types (the VM reaches it through free functions such as `instance_field_layout_cached` in `dotnet-vm/src/layout.rs`):
- **Field offsets**: `LayoutFactory::create_field_layout()` computes offsets respecting alignment requirements of the host architecture. It recursively resolves field types and computes their sub-layouts.
- **Total object size**: Sums field sizes + padding.
- **`GcDesc` generation**: `LayoutFactory::populate_gc_desc()` creates a descriptor bitmap used by the Stop-The-World (STW) GC to identify which fields contain managed references (`ObjectRef`). It merges the GC descriptors of nested fields into the parent's descriptor based on their computed offsets.

This is a recursive algorithm: computing a struct's layout requires `LayoutFactory::collect_fields()` to recursively compute the layouts of all its field types. Layouts are cached in `GlobalCaches::layout_cache` and `instance_field_layout_cache`.

## Caching Architecture (`state.rs` → `GlobalCaches`)

`GlobalCaches` holds eleven instances of the `Cache<K, V, S>` primitive defined in
`dotnet-vm/src/cache.rs`. The primitive owns its monomorphized storage backend,
`CacheKind`, a direct `Arc<RuntimeMetrics>`, an optional capacity, and an optional
front-cache policy. Its sealed `CacheStore<K, V>` contract returns owned values, so
neither a `DashMap` shard guard nor an `RwLock` guard can escape a lookup. The
storage type is a generic parameter rather than a trait object, preserving static
dispatch on cache read paths.

Many cache keys embed `ConcreteType` and `GenericLookup`. Their memoized hashes
make hashing those key components O(1) after descriptor construction, avoiding a
walk of specialized type trees or generic-argument slices on cache lookup.

`ShardedStore` is `DashMap`-backed and is used where concurrent writes benefit
from sharding. In particular, VMT and hierarchy entries are created for new
runtime type combinations, while the other sharded caches carry compound keys or
`Arc` values; sharding avoids one global write lock on those paths. `LockedStore`
is `RwLock<HashMap<...>>`-backed for the four descriptor-to-`bool` properties.
Those properties are immutable after resolution, read far more often than they
are first computed, and are naturally bounded by the loaded metadata, so one
read-write lock avoids per-shard overhead without requiring eviction.
`ShardedCache` and `LockedCache` are the corresponding readable aliases of the
generic primitive.

| Cache                       | Store          | Key                                                   | Value                                    | Purpose                                                      |
|-----------------------------|----------------|-------------------------------------------------------|------------------------------------------|--------------------------------------------------------------|
| Type layout cache           | `ShardedStore` | `ConcreteType`                                        | `Arc<LayoutManager>`                     | Computed memory layout for complete types                    |
| Virtual dispatch cache      | `ShardedStore` | `(MethodDescription, TypeDescription, GenericLookup)` | `MethodDescription`                      | Resolved virtual target                                      |
| Intrinsic method cache      | `LockedStore`  | `MethodDescription`                                   | `bool`                                   | Is this method natively implemented?                         |
| Intrinsic field cache       | `LockedStore`  | `FieldDescription`                                    | `bool`                                   | Is this field natively implemented?                          |
| Type hierarchy cache        | `ShardedStore` | `(ConcreteType, ConcreteType)`                        | `bool`                                   | Cached result of `is_a` relationship                         |
| Static field layout cache   | `ShardedStore` | `(TypeDescription, GenericLookup)`                    | `Arc<FieldLayoutManager>`                | Computed static field layout                                 |
| Instance field layout cache | `ShardedStore` | `(TypeDescription, GenericLookup)`                    | `Arc<FieldLayoutManager>`                | Computed instance field layout                               |
| Value type cache            | `LockedStore`  | `TypeDescription`                                     | `bool`                                   | Is this type a value type?                                   |
| Finalizer cache             | `LockedStore`  | `TypeDescription`                                     | `bool`                                   | Does this type have a finalizer?                             |
| Overrides cache             | `ShardedStore` | `(TypeDescription, GenericLookup)`                    | `Arc<HashMap<MethodDescription, MethodDescription>>` | Resolved interface/virtual overrides for a type          |
| Method info cache           | `ShardedStore` | `(MethodDescription, GenericLookup)`                  | `Arc<MethodInfo<'static>>`               | Full resolved method info (instructions, exceptions, locals) |

### Global Cache Registry, Metrics, and Capacity

`define_global_caches!` in `state.rs` is the single eleven-entry registry. From
each entry it generates the `GlobalCaches` field, construction with its
`CacheKind`/capacity/front policy, indexed size-report entry, and, where needed,
a typed TLS companion. The current eleven-entry registry order matches
`CacheKind::GLOBAL` and the fixed legacy `CacheStats` display order. This keeps
declaration, construction, reporting, and front-cache configuration synchronized.
`Cache::size_report()` returns its `CacheKind` with an estimated `CacheSize`;
its `pointer_bytes` value is the entry count multiplied by the inline
`size_of::<K>() + size_of::<V>()` payload. It excludes backing-store overhead and
heap-allocated content behind `Arc` or `Box`. The shared reflection registry's
three reports use the same inline key/value formula. `SharedGlobalState` collects
these reports into the `CacheSizes` array using `CacheKind::as_index()`. For
serialized benchmark compatibility, `BenchInstrumentationSnapshot` retains the
historical `cache_memory_bytes_total` and `cache_memory_bytes_by_cache` field
names; their values are this pointer-footprint estimate, not deep cache memory.
The metric crate's cache-kind declaration generates stable keys, indexes, global
membership, and front-tier membership, while `RuntimeMetrics` stores hit/miss and
optional benchmark counters in `CacheKind`-indexed arrays. The cache primitive
owns logical hit/miss, key-clone, and front-tier instrumentation through its shared
metrics `Arc`, rather than making callers own metrics bookkeeping.

Under `bench-instrumentation`, `cache_key_clone_total` counts the explicit logical
key-component clones recorded by cache callers (principally descriptor and `Arc`
clone work). It deliberately does not measure deep hash traversal. Therefore the
same benchmark workload and cache paths can retain the same count after descriptor
hash memoization; use benchmark time and the hash implementation to evaluate that
optimization, while retaining the counter as a regression guard for key-construction
work.

The five cache environment-variable contracts are read once when `GlobalCaches` is
constructed. `DOTNET_CACHE_LIMIT_METHOD_INFO`, `DOTNET_CACHE_LIMIT_VMT`, and
`DOTNET_CACHE_LIMIT_HIERARCHY` each set an optional positive bound for their named
L2 cache; an unset, invalid, or zero value leaves that cache unbounded.
`DOTNET_FRONT_CACHE_ENABLED` controls the three front-cache tiers and defaults to
enabled (the usual `1`/`true`/`yes`/`on` and `0`/`false`/`no`/`off` spellings are
accepted). `DOTNET_FRONT_CACHE_CAPACITY` selects their positive capacity; an unset,
invalid, or zero value uses the default capacity of 128. The three capacity limits,
front enable switch, and front capacity therefore retain their behavior without a
`CachePolicy` aggregate.

**Eviction**: All shared L2 stores are unbounded by default, so eviction is not
triggered in default runs. `DOTNET_CACHE_LIMIT_METHOD_INFO`,
`DOTNET_CACHE_LIMIT_VMT`, and `DOTNET_CACHE_LIMIT_HIERARCHY` enable bounded L2
caches only as an operator-controlled capacity-management escape hatch. An update
to an existing key happens before eviction; for a new key at capacity, `Cache`
removes victims in the store's iteration order until there is room. That selection
is neither random nor LRU. `ShardedStore` is biased by shard/hash iteration, and
`LockedStore` is not LRU either. A true LRU L2 would require
`lru::LruCache::get(&mut self)`, which
would write-lock each `callvirt` hot-path lookup; that cost outweighs its benefit
for the bounded universe of loaded-metadata keys.

### Striped Locking and Concurrent Access

`ShardedStore` uses `dashmap::DashMap`, which internally uses sharded/striped
read-write locks. This allows multiple threads to read and write to different
shards of the cache concurrently without blocking each other. The shard index is
computed from the key's hash (for example, the hash of `ConcreteType` or
`(MethodDescription, GenericLookup)`).

The method-info, VMT, and hierarchy caches add an optional L1 tier in front of
their sharded L2 store. `Cache` owns whether that tier is enabled, its capacity
policy, and its metrics; each typed thread-local `FrontCache<K, V>` owns the actual
per-thread LRU entries. A front-cache lookup records its tier result and, on a hit,
also records the logical cache hit; an L1 miss falls through to the shared L2
cache. Thus the front cache reduces shared-cache traffic without sharing LRU state
between threads.

Additionally, components like `StaticStorageManager` implement their own sharded locks using `[RwLock<HashMap<...>>; NUM_SHARDS]` to manage concurrent `.cctor` initialization without global bottlenecks.

## `ResolutionContext` (`context.rs`)

Provides a scoped view of the resolution state:
- Current generic type parameters and method parameters (`&GenericLookup`)
- Reference to the `AssemblyLoader`
- The current assembly's resolution scope (`ResolutionS`)
- Reference to `GlobalCaches` and optionally `SharedGlobalState`

`ResolutionContext` is created per-frame (via `ResolutionContext::for_method()` during method invocation) or per-type (via `for_type()`). It is passed down through instruction handlers and layout computations to ensure that any type or method tokens encountered are resolved within the correct generic and assembly context. It abstracts away the global state and provides helper methods like `locate_method()` and `make_concrete()`.

## Generic Instantiation

`GenericLookup` (from `dotnet-types/src/generics.rs`) maps generic parameter indices to concrete `ConcreteType` values. It handles:
- Type-level generics (`List<T>` → `List<int>`)
- Method-level generics (`Foo<M>` → `Foo<string>`)
- Nested generics (e.g., `Dictionary<K, List<V>>`)

`GenericLookup` contains two argument arrays, `type_generics` and `method_generics` (both
`Arc<[ConcreteType]>`), plus a private hash computed eagerly from both arrays. Construct lookups
through `new`, `from_type_arc`, or `from_arcs`. Replace an argument array only through
`set_type_generics` or `set_method_generics`, which recompute the hash; direct assignment to the
public fields would desynchronize the cache key. The fields remain public for read compatibility.

Runtime call sites should use the bounded accessors (`type_arg`, `method_arg`, and cloned variants)
when an index comes from metadata or intrinsic binding; missing slots become
`TypeResolutionError::GenericIndexOutOfBounds` instead of unchecked slice panics. When
`GenericLookup::make_concrete()` is called with a `MethodType` (e.g., a generic type parameter
`!!0`), it indexes into the appropriate array to substitute the parameter with its concrete
instantiation and reports the same bounded error for malformed slots. If the input is a base type
(e.g., an array of `!0`), it recursively substitutes the inner types to produce a new
`ConcreteType`.

### Interaction with Layout
Generic type instantiation affects layout because different type arguments may have different sizes and GC descriptors. Layout computation must be done per-concrete-instantiation.

### Interaction with Caching
Cache keys include `GenericLookup` to distinguish between different instantiations of the same
generic type or method. Its `Hash` implementation writes only the memoized `u64`, so lookup-time
hashing is O(1); equality still compares both argument arrays to preserve structural identity in
the event of a hash collision.

### Finite instantiation closure (no expanding-cycle detection)

ECMA-335 requires that each generic type definition generates a **finite instantiation closure** and describes a validation algorithm based on detecting **expanding-cycles** in the inheritance/interface instantiation graph (§II.9.2).

`dotnet-rs` currently does **not** implement expanding-cycle detection / finite-closure validation during load or resolution.

**Rationale:** This project primarily targets **trusted assemblies** (similar to production runtimes, which typically do not proactively validate this property for already-trusted inputs). Adding the full §II.9.2 analysis would increase loader/resolution complexity and cost for a class of malformed metadata that is not expected in supported workloads.

**Implication:** If an assembly contains an expanding-cycle, type resolution and/or layout computation may recurse indefinitely or otherwise fail at runtime. If `dotnet-rs` is extended to run untrusted or adversarial assemblies, this validation should be added as a loader-level check.

## Static Field Initialization (`statics.rs`)

`StaticStorageManager` manages static field storage and class constructor (`.cctor`) execution:

- **Sharded storage**: Uses an array of `RwLock<HashMap>` shards indexed by the hash of `(TypeDescription, GenericLookup)`.
- **Init states**: Stored in an `AtomicU8`. States include `INIT_STATE_UNINITIALIZED`, `INIT_STATE_INITIALIZING`, `INIT_STATE_INITIALIZED`, and `INIT_STATE_FAILED`.
- **Initialization Protocol**: `init()` checks the state and returns a `StaticInitResult` — one of `Execute(cctor_method)`, `Initialized`, `Recursive`, `Failed`, or `Waiting`. If uninitialized, it atomically transitions to `INITIALIZING` and returns `StaticInitResult::Execute(cctor_method)`. The calling thread is now responsible for running the `.cctor`. Once done, it calls `mark_initialized()`.
- **Cross-thread coordination**: With `multithreading`, if another thread calls `init()` while the state is `INITIALIZING`, it returns `StaticInitResult::Waiting`. The thread then calls `wait_for_init()`, which uses a `Condvar` and `Mutex` to block until the initializing thread finishes and broadcasts a notification. A single-threaded build cannot produce `Waiting`, so `wait_for_init()` has no valid caller there and its wait arm panics if that invariant is violated.

### Non-Obvious: `.cctor` and GC Interaction
While waiting for another thread's `.cctor` to complete, the waiting thread must remain responsive to GC safe point requests. `wait_for_init` integrates with the GC coordinator for this.

## Non-Obvious Connections

### Resolution ↔ Intrinsics
The resolver checks the intrinsic cache before dispatching a method call. If a method is intrinsic, execution bypasses CIL interpretation entirely. Instead, the `IntrinsicRegistry` uses a Perfect Hash Function (PHF) to map the method signature to a stable `MethodIntrinsicId`. This ID is then passed to a monomorphic dispatcher that calls the native Rust handler. This pipeline ensures that intrinsic lookups are fast and that the final call is a direct, inlinable function call. This check happens in `dotnet-runtime-resolver/src/methods.rs` and is cached in `GlobalCaches`.

### Resolution ↔ Layout ↔ GC
Layout computation produces `GcDesc` which the GC uses during tracing to know which bytes in an object are managed references. This connects the type system to the garbage collector.

### Resolution ↔ Reflection
`ReflectionRegistry` (in `state.rs`) maps `RuntimeType` → `ObjectRef`, creating heap-allocated reflection objects. These are per-arena (not shared globally) because they contain GC references.

### Layout Ownership
Layout representation and layout computation are split:
- `dotnet-value/src/layout.rs`: runtime layout data model (`LayoutManager`, `FieldLayoutManager`, `ArrayLayoutManager`)
- `dotnet-runtime-resolver/src/layout.rs`: resolver-owned layout algorithms used by the VM's layout functions
- `dotnet-vm/src/layout.rs`: free-function delegation surface over resolver-owned layout code

This split keeps layout algorithms reusable outside `dotnet-vm` while preserving existing VM call sites.
