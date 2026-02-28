# Type Resolution and Caching

This document describes the type/method/field resolution pipeline, the multi-level caching system, and how generic instantiation interacts with layout computation.

## Overview

Resolution converts metadata tokens (from parsed .NET assemblies) into runtime descriptors used by the execution engine. This is a lazy, cached process that spans several modules:

- **`dotnet-vm/src/resolver/`**: `ResolverService` facade with sub-modules for types, methods, layout, and factory
- **`dotnet-vm/src/context.rs`**: `ResolutionContext` for scoped resolution with generic parameters
- **`dotnet-vm/src/resolution.rs`**: Resolution traits and helpers
- **`dotnet-vm/src/layout.rs`**: `LayoutFactory` for computing object memory layouts (~497 lines)
- **`dotnet-value/src/layout.rs`**: `LayoutManager`, `FieldLayoutManager`, `ArrayLayoutManager` (~438 lines)
- **`dotnet-vm/src/state.rs`**: `GlobalCaches` and `SharedGlobalState`
- **`dotnet-types/src/`**: Type descriptors, generics, comparer

## Resolution Pipeline

### Type Resolution (`resolver/types.rs`)

1. Metadata token (`UserType`: `TypeDefOrRef`, `TypeSpec`, `TypeRef`) arrives from a CIL instruction.
2. The `ResolutionContext` encapsulates the current assembly scope (`ResolutionS`) and generic parameters via `GenericLookup`.
3. The context delegates to `ResolverService::locate_type()`, which uses the `AssemblyLoader` to resolve the token into a `TypeDescription`.
4. For generic types or arrays/pointers, `ResolverService::make_concrete()` substitutes type parameters with concrete arguments using `GenericLookup::make_concrete()`.
5. The result is a `ConcreteType` (representing a specialized type) or a `TypeDescription` (identifying a type definition in a specific assembly).

### Method Resolution (`resolver/methods.rs`)

Multi-phase process:
1. **Token → descriptor**: `ResolverService::locate_method()` maps a metadata token (`UserMethod`) to a `MethodDescription`, applying generic substitutions.
2. **Virtual dispatch**: For `callvirt`, `resolve_virtual_method()` finds the most-derived implementation. It first checks the `vmt_cache` in `GlobalCaches`. If missing, it iterates through ancestors (via `AssemblyLoader::ancestors`) and uses `find_and_cache_method()` to locate the implementation matching the base method's signature.
3. **Generic instantiation**: `find_generic_method()` applies `GenericLookup` from the `ResolutionContext` to specialize the method and its parent type.
4. **Intrinsic check**: `is_intrinsic_cached()` checks if the method has a native implementation by querying `GlobalCaches::intrinsic_cache` and `IntrinsicRegistry`.

### Field Resolution

Fields resolve to `FieldDescription` with computed byte offsets within their containing type. Value type field access requires layout calculation to determine offsets.

### Layout Computation (`resolver/layout.rs`, `layout.rs`)

`LayoutFactory` computes the physical memory layout of objects and value types:
- **Field offsets**: `create_field_layout()` computes offsets respecting alignment requirements of the host architecture. It recursively resolves field types and computes their sub-layouts.
- **Total object size**: Sums field sizes + padding.
- **`GcDesc` generation**: `populate_gc_desc()` creates a descriptor bitmap used by the Stop-The-World (STW) GC to identify which fields contain managed references (`ObjectRef`). It merges the GC descriptors of nested fields into the parent's descriptor based on their computed offsets.

This is a recursive algorithm: computing a struct's layout requires `LayoutFactory::collect_fields()` to recursively compute the layouts of all its field types. Layouts are cached in `GlobalCaches::layout_cache` and `instance_field_layout_cache`.

## Caching Architecture (`state.rs` → `GlobalCaches`)

`GlobalCaches` holds several thread-safe caches wrapped in `Arc` and utilizing `DashMap` for concurrent access:

| Cache                       | Key                                                   | Value                                    | Purpose                                                      |
|-----------------------------|-------------------------------------------------------|------------------------------------------|--------------------------------------------------------------|
| Intrinsic method cache      | `MethodDescription`                                   | `bool`                                   | Is this method natively implemented?                         |
| Intrinsic field cache       | `FieldDescription`                                    | `bool`                                   | Is this field natively implemented?                          |
| Type layout cache           | `ConcreteType`                                        | `Arc<LayoutManager>`                     | Computed memory layout for complete types                    |
| Instance field layout cache | `(TypeDescription, GenericLookup)`                    | `Arc<FieldLayoutManager>`                | Computed instance field layout                               |
| Static field layout cache   | `(TypeDescription, GenericLookup)`                    | `Arc<FieldLayoutManager>`                | Computed static field layout                                 |
| Virtual dispatch cache      | `(MethodDescription, TypeDescription, GenericLookup)` | `MethodDescription`                      | Resolved virtual target                                      |
| Type hierarchy cache        | `(ConcreteType, ConcreteType)`                        | `bool`                                   | Cached result of `is_a` relationship                         |
| Method info cache           | `(MethodDescription, GenericLookup)`                  | `Arc<MethodInfo<'static>>`               | Full resolved method info (instructions, exceptions, locals) |
| Overrides cache             | `(TypeDescription, GenericLookup)`                    | `Arc<HashMap<usize, MethodDescription>>` | Resolved interface/virtual overrides for a type              |

**No eviction**: Caches grow monotonically, bounded by the number of unique types/methods instantiated in loaded assemblies.

### Striped Locking and Concurrent Access

Caches heavily utilize `dashmap::DashMap`, which internally uses sharded/striped read-write locks. This allows multiple threads to read and write to different shards of the cache concurrently without blocking each other. The shard index is computed from the key's hash (e.g., the hash of `ConcreteType` or `(MethodDescription, GenericLookup)`).

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

`GenericLookup` contains two arrays: `type_generics` and `method_generics` (both `Arc<[ConcreteType]>`). When `GenericLookup::make_concrete()` is called with a `MethodType` (e.g., a generic type parameter `!!0`), it indexes into the appropriate array to substitute the parameter with its concrete instantiation. If the input is a base type (e.g., an array of `!0`), it recursively substitutes the inner types to produce a new `ConcreteType`.

### Interaction with Layout
Generic type instantiation affects layout because different type arguments may have different sizes and GC descriptors. Layout computation must be done per-concrete-instantiation.

### Interaction with Caching
Cache keys include `GenericLookup` to distinguish between different instantiations of the same generic type/method.

## Static Field Initialization (`statics.rs`)

`StaticStorageManager` manages static field storage and class constructor (`.cctor`) execution:

- **Sharded storage**: Uses an array of `RwLock<HashMap>` shards indexed by the hash of `(TypeDescription, GenericLookup)`.
- **Init states**: Stored in an `AtomicU8`. States include `INIT_STATE_UNINITIALIZED`, `INIT_STATE_EXECUTING`, `INIT_STATE_INITIALIZED`, and `INIT_STATE_FAILED`.
- **Initialization Protocol**: `init()` checks the state. If uninitialized, it atomically transitions to `EXECUTING` and returns `StaticInitResult::Execute(cctor_method)`. The calling thread is now responsible for running the `.cctor`. Once done, it calls `mark_initialized()`.
- **Cross-thread coordination**: If another thread calls `init()` while the state is `EXECUTING`, it returns `StaticInitResult::Waiting`. The thread then calls `wait_for_init()`, which uses a `Condvar` and `Mutex` to block until the initializing thread finishes and broadcasts a notification.

### Non-Obvious: `.cctor` and GC Interaction
While waiting for another thread's `.cctor` to complete, the waiting thread must remain responsive to GC safe point requests. `wait_for_init` integrates with the GC coordinator for this.

## Non-Obvious Connections

### Resolution ↔ Intrinsics
The resolver checks the intrinsic cache before dispatching a method call. If a method is intrinsic, execution bypasses CIL interpretation entirely. Instead, the `IntrinsicRegistry` uses a Perfect Hash Function (PHF) to map the method signature to a stable `MethodIntrinsicId`. This ID is then passed to a monomorphic dispatcher that calls the native Rust handler. This pipeline ensures that intrinsic lookups are fast and that the final call is a direct, inlinable function call. This check happens in `resolver/methods.rs` and is cached in `GlobalCaches`.

### Resolution ↔ Layout ↔ GC
Layout computation produces `GcDesc` which the GC uses during tracing to know which bytes in an object are managed references. This connects the type system to the garbage collector.

### Resolution ↔ Reflection
`ReflectionRegistry` (in `state.rs`) maps `RuntimeType` → `ObjectRef`, creating heap-allocated reflection objects. These are per-arena (not shared globally) because they contain GC references.

### Two Layout Crates
Layout logic is split across two crates:
- `dotnet-value/src/layout.rs`: `LayoutManager` (runtime data structure for field access)
- `dotnet-vm/src/layout.rs`: `LayoutFactory` (computes layouts using type resolution)

This split exists because `dotnet-value` cannot depend on `dotnet-vm`.