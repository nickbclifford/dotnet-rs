# dotnet-rs Architecture

`dotnet-rs` is a Rust-based implementation of the .NET Common Language Infrastructure (CLI), specifically the Virtual Execution System (VES), as defined in ECMA-335.

## Panic-vs-Result contribution policy

For contributor rules on when to use `panic!`/`unreachable!`/`debug_assert!`/`expect()` versus returning `VmError` (`Result` / `StepResult::Error`), see [`CONTRIBUTING.md` — Panic-vs-Result policy](../CONTRIBUTING.md#panic-vs-result-policy).

That section also documents the host-error (`VmError`) vs managed-exception (`ManagedException` / `ExceptionState`) boundary.

## Invariants

- `F2.DescriptorMatchesEcmaLayout`: a layout descriptor's offsets, sizes, and field types match the corresponding instantiated ECMA-335 layout. The resolver's layout factory supplies the descriptor, but the specification correspondence is assumed.
- `F3.StackSlotMatchesView`: raw evaluation-stack access reads a slot only through the type used to push it; enum discriminants and instruction stack discipline establish this locally.
- `F3.InteriorPointerRebased`: an interior pointer retains its originating slot association and is re-established by `apply_reallocation_fixup()` after stack-vector reallocation.
- `F10.RawMemoryAccessValid`: raw reads, writes, copies, and pointer derivations stay in live storage with the required range, initialization, alignment, and aliasing conditions. A nearby bounds/integrity check or the immediate unsafe function's caller contract establishes that condition.
- `F10.RawAllocationOwnership`: `Box::into_raw`/`Box::leak` pointers are converted or reclaimed only through their single matching ownership operation; other raw-pointer conversions rely on their documented provenance and lifetime contract.
- `F10.BorrowedStorageStable`: pointers obtained from managed or shared backing storage are used only while the borrow, closure, or lock that stabilizes the allocation remains active.
- `F10.ArchIntrinsicPrecondition`: a target-feature function or architecture intrinsic executes only when its ISA feature is guaranteed by the compilation target or by successful runtime detection at the dispatch site.
- `F11.CliLoadKindMatchesStorage`: an atomic scalar is decoded only by the CLI `LoadType` with its storage bit-width and signedness; the object-handle case relies on its explicit caller contract.
- `F11.PInvokeAbiAgreement`: a native symbol's real calling convention and complete parameter/return ABI agree with the prepared libffi CIF and marshalling storage. Rust can validate its CIF and storage, but agreement with an arbitrary native import is a Plan 06 `ffi-abi` trust candidate.
- `F11.NativeLibraryLoadTrusted`: a dynamic library permitted by the configured host policy has safe initializers/finalizers, and its cached `NativeLibraries` entry remains loaded while selected symbols are used. The cache establishes retention; native initializer/finalizer behavior is a Plan 06 `ffi-abi` trust candidate.

## Crate Responsibilities

The project is divided into several crates, each with a focused responsibility:

- **dotnet-cli**: The entry point. It provides the command-line interface, test harness, and integration tests.
- **dotnet-vm**: The core VM crate. It owns the execution engine, instruction handlers/dispatch generation, GC coordinator, threading/sync runtime, and VM-local intrinsic infrastructure.
- **dotnet-vm-ops**: Foundational VES operation traits (`EvalStackOps`, `TypedStackOps`, `ExceptionOps`, `RawMemoryOps`, etc.). Runtime data types used by those traits are imported directly from `dotnet-vm-data`.
- **dotnet-vm-data**: Shared runtime data model (`StepResult`, `MethodInfo`, `VmContinuation<'gc>`, method/frame state, stack and exception data structures).
- **dotnet-exceptions**: Exception handling logic extracted from `dotnet-vm`. Contains the `ExceptionHandlingSystem` with the two-pass search/unwind state machine. Depends on `dotnet-vm-ops` for base traits and on `dotnet-vm-data` for runtime data types.
- **dotnet-pinvoke**: P/Invoke marshalling extracted from `dotnet-vm`. Uses `libffi` and `libloading` for native interop. Depends on `dotnet-vm-ops` for base traits and on `dotnet-vm-data` for runtime data types.
- **dotnet-runtime-resolver**: Type/method/field resolution services and layout factory implementation, consumed from `dotnet-vm` via adapters.
- **dotnet-runtime-memory**: Runtime memory access/heap services and validation helpers, consumed from `dotnet-vm` via adapters.
- **dotnet-intrinsics-core**: Core intrinsic handlers (`math`, `array_ops`) and conservative `System.Runtime.Intrinsics` capability probes.
- **dotnet-intrinsics-delegates**: Delegate intrinsic handlers and delegate invoke host seams.
- **dotnet-intrinsics-span**: Span/ReadOnlySpan intrinsic handlers and span host seams.
- **dotnet-intrinsics-string**: String intrinsic handlers and string-span host seams.
- **dotnet-intrinsics-threading**: Monitor/interlocked/threading intrinsic handlers and host seams.
- **dotnet-intrinsics-reflection**: Reflection intrinsic handlers and reflection host seams.
- **dotnet-intrinsics-unsafe**: Unsafe/marshalling intrinsic handlers and host seams.
- **dotnet-simd**: Shared SIMD byte-operation helpers with scalar fallback, used by intrinsic crates.
- **dotnet-metrics**: Standalone crate for `RuntimeMetrics` with per-cache hit/miss tracking (`CacheKind`, `CacheEvent`, `CacheStats`, `CacheStat`), serializable via `serde`.
- **dotnet-tracer**: Standalone crate for the `Tracer` subsystem. Provides guarded emit/span helpers, structured logging via the `tracing` crate with configurable levels (`DOTNET_RS_TRACE` env), optional JSON output (`DOTNET_RS_TRACE_FORMAT=json`), and an async flusher thread via `crossbeam-channel`.
- **dotnet-assemblies**: Handles loading and resolving .NET assemblies. It also includes a support library of C# stubs for core types.
- **dotnet-value**: Defines the representation of all .NET values at runtime, including stack values, managed/unmanaged pointers, heap objects, and field storage layouts.
- **dotnet-types**: Implements the .NET type system, including type descriptors, method/field info, generics, and type comparison logic.
- **dotnet-utils**: Contains shared utilities like synchronization primitives, atomic access, GC-related helper types, and strongly-typed newtypes.
- **dotnet-build-tools**: Shared helpers used by crate build scripts (`build.rs`) for deterministic input scanning/caching.
- **dotnet-benchmarks**: Criterion benchmark harness and fixture pipeline for end-to-end runtime performance measurement.
- **dotnet-macros** & **dotnet-macros-core**: Procedural macros used to define instructions and intrinsics concisely.

### Crate Dependency Hierarchy

```
dotnet-cli
  ├── dotnet-build-tools (build dependency)
  └── dotnet-vm
      ├── dotnet-build-tools (build dependency)
      ├── dotnet-vm-ops
      │   └── dotnet-vm-data
      ├── dotnet-exceptions
      ├── dotnet-pinvoke
      ├── dotnet-runtime-resolver
      ├── dotnet-runtime-memory
      ├── dotnet-intrinsics-core
      ├── dotnet-intrinsics-delegates
      ├── dotnet-intrinsics-span
      ├── dotnet-intrinsics-string
      ├── dotnet-intrinsics-threading
      ├── dotnet-intrinsics-reflection
      ├── dotnet-intrinsics-unsafe
      ├── dotnet-assemblies
      ├── dotnet-value
      ├── dotnet-types
      ├── dotnet-utils
      ├── dotnet-metrics
      ├── dotnet-tracer
      ├── dotnet-simd
      ├── dotnet-macros
      └── dotnet-macros-core

dotnet-benchmarks
  └── dotnet-vm
```

## Data Flow

1. **Initialization**: `dotnet-cli` initializes the runtime, creating a `SharedGlobalState` which holds caches, the assembly loader, and the intrinsic registry.
2. **Assembly Loading**: The `AssemblyLoader` (in `dotnet-assemblies`) parses DLL files into a structured metadata format (using the `dotnetdll` crate).
3. **Execution Entry**: The `Executor` starts execution at the entry point of the main assembly.
4. **Main Loop**: The executor runs a loop that:
    - Fetches the next CIL instruction based on the Instruction Pointer (IP).
    - Dispatches the instruction to its handler.
    - Updates the `EvaluationStack` with results.
    - Handles flow control (jumps, calls, returns).
5. **Instruction Set and Dispatch**: CIL instructions are categorized (arithmetic, flow, objects, etc.) and handled in `crates/dotnet-vm/src/instructions/`. The `dispatch/` subsystem manages instruction execution with an auto-generated dispatch table in `dispatch/registry.rs` and a `dispatch/ring_buffer.rs` instruction trace buffer.
6. **Method Calls and Intrinsics**:
    - Static calls resolve the target method and push a new `StackFrame`.
    - Virtual calls use the object's vtable (computed via the layout system) to find the correct method implementation.
    - Tail calls (`tail.`) are supported in a guarded manner: when the prefix is present and the call is in a valid tail position (immediately followed by `ret`, eval stack otherwise empty, not inside an exception region, etc.), the VM replaces the current frame before dispatching the callee; otherwise it falls back to a normal call.
    - Intrinsic calls are intercepted and handled by native Rust code split across `dotnet-intrinsics-*` crates and VM-local handler modules, with registry/metadata code in `crates/dotnet-vm/src/intrinsics/`. Similar to instructions, intrinsics use a monomorphic ID-based dispatch system.
    
    (See [Delegates and Dispatch](DELEGATES_AND_DISPATCH.md) for more details on invocation paths).

## Memory and Garbage Collection

`dotnet-rs` uses a Stop-The-World (STW) garbage collector based on the `gc-arena` crate. (See [GC and Memory Safety](GC_AND_MEMORY_SAFETY.md) for an in-depth look).

- **Heap Management**: `HeapManager` handles the allocation of objects. Each thread typically has its own arena for allocation to minimize contention.
- **GC Roots**: The evaluation stack, local variables, and static fields serve as the primary roots for GC.
- **STW Coordination**: When a GC is triggered, all threads are brought to a "Safe Point" (e.g., at a loop back-edge or method call). Once all threads are paused, the collector traces all reachable objects across all arenas.
- **GcScopeGuard**: To prevent deadlocks during STW, `GcScopeGuard` must be used when holding a reference to heap-allocated data. It informs the GC that the thread is currently "busy" and cannot safely pause until the guard is dropped.
- **Collect Trait**: Every type stored on the heap or containing GC references must implement the `Collect` trait to allow the tracer to find nested references.

## Threading Model

The VM supports multi-threading (feature-gated via `multithreading`). For detailed mechanics, see [Threading and Synchronization](THREADING_AND_SYNCHRONIZATION.md):

- **Thread Manager**: Manages the lifecycle of managed threads and coordinates STW pauses.
- **Safe Points**: Execution periodically checks if a GC or suspension has been requested via `ctx.check_gc_safe_point()`.
- **Synchronization**: .NET `Monitor` (lock/unlock) is implemented using `SyncBlockManager`, providing thread-safe access to objects with monitor-style semantics.

Each executor's `CallStack` owns `ArenaLocalState` for arena/thread-private heap, reflection, and cached P/Invoke last-error state; cross-thread services remain in `SharedGlobalState`.

## Exception Handling

`dotnet-rs` implements the ECMA-335 structured exception handling (SEH) model using a two-pass approach.
- **State Machine**: Exception processing is modeled as a state machine (`Throwing` → `Searching` → `Filtering` → `Unwinding` → `ExecutingHandler`).
- **Filter Clauses**: Support for dynamic `filter` blocks that run user CIL code during the search phase.
- **Unwinding**: The `leave` instruction and exception unwinding properly execute `finally` and `fault` blocks.
- **Extracted Crate**: The exception handling system (`ExceptionHandlingSystem`) lives in `dotnet-exceptions`, while the exception/stack data model lives in `dotnet-vm-data` and is imported from there directly.

See [Exception Handling](EXCEPTION_HANDLING.md) for full details on the state machine and unwinding process.

## Type System and Layout

For more details on caching and resolution pipelines, see [Type Resolution and Caching](TYPE_RESOLUTION_AND_CACHING.md).

- **Type Resolution**: Types are resolved lazily. `ResolutionContext` manages the scope of resolution, including generic parameters.
- **Layout Calculation**: The resolver-owned `LayoutFactory` computes the physical memory layout of objects and value types, including field offsets and GC descriptors (which fields are references).
- **VM Ownership Boundary**: `dotnet-vm` uses `VmResolverService` as its VM-owned resolution facade; the free functions in `dotnet-vm/src/layout.rs` delegate layout requests to the resolver-owned layout engine.
- **Generics**: Generic types and methods are instantiated on-demand, with metadata specialized for the specific type arguments.

### Support-assembly slot annotation policy

The embedded C# support assembly has three intentionally separate field/metadata concepts:

1. **Ordinary managed fields** implement normal C# state (including async/task state). They may
   carry `[UsedImplicitly]`, but Rust neither assigns them a storage-ABI ID nor accesses them
   through the support-slot registry.
2. **Stub attribute schema** is metadata used to map a support type to its BCL name. Its
   `[Stub(InPlaceOf = "...")]` named argument is validated as the exact `StubAttribute` schema
   and is not a storage slot.
3. **Runtime storage slots** are the fixed set of 21 fields Rust reads or writes. Each has both
   `[RuntimeSlot(RuntimeSlotId.<SemanticId>)]` and `[UsedImplicitly]` annotations. The implication
   is one-way: every runtime slot must be marked `UsedImplicitly`, while an ordinary managed field
   need not be a runtime slot.

`crates/dotnet-assemblies/support_slots.def` is the single declarative storage-ABI contract. It
assigns stable, dense semantic `RuntimeSlotId` values (existing IDs must never be renumbered),
declaring `WellKnown` owners, field names/staticness, exact ECMA-335 signatures, storage shapes,
and the allowed typed or layout-only views. The loader validates the generated-ID constructor,
each ID's unique presence, placement, staticness, and exact signature before registering a slot.
It then binds every ID to its exact resolved `TypeDescription`; downstream access is by ID and
descriptor identity, not a type/field-name lookup. A same-named user type therefore cannot obtain
support-assembly behavior.

`SupportSlotOps`, generated from that contract, is the only Rust accessor surface for these
fields. The three runtime-handle `_value` IDs have their documented `ObjectRef`/`usize` views;
the Span and ReadOnlySpan fields expose separate generated IDs and layout-only accessors, with
bounded convenience helpers only where code accepts either span representation. This contract
does not apply to metadata-driven BCL or user-assembly layout probes. `[UsedImplicitly]` methods
remain runtime entry points rather than storage slots.

### Descriptor Ownership Model

Metadata descriptors are index-based and owner-tied:

- `ResolutionS` carries both a raw metadata pointer and an `Arc<MetadataArena>` owner.
- `TypeDescription` stores `(ResolutionS, TypeIndex)`.
- `MethodDescription` stores `(parent, parent_generics, method_resolution, MethodMemberIndex)` so accessors/events are represented without raw pointer identity tricks.
- `FieldDescription` stores `(parent, field_resolution, index)`.

This model avoids publishing naked `'static` metadata references while preserving fast descriptor hashing and cache keys.

## Trait Architecture

The core VES trait tower is split between `dotnet-vm-ops` and `dotnet-vm` to
avoid circular dependencies. Intrinsic crates extend the lower layer with
crate-local composite host traits: they depend on `dotnet-vm-ops`, never on
`dotnet-vm`.

### Base Traits (`dotnet-vm-ops/src/ops.rs`)
Foundational traits that instruction handlers and intrinsics can target without depending on `dotnet-vm`:
- `EvalStackOps`, `TypedStackOps`, `LocalOps`, `ArgumentOps`, `VariableOps`
- `ExceptionOps`, `RawMemoryOps`, `ThreadOps`, `LoaderOps`
- `MemoryOps`, `ResolutionOps`, `ReflectionOps`, `TypeLayoutOps`, `StaticsOps`
- `VesBaseOps`, `VesInternals`, `ExceptionContext`, `PInvokeContext`
- `SpanBaseOps`, `ThreadingBaseOps`, `ReflectionBaseOps`, `UnsafeBaseOps` (base capability aliases extended by their corresponding intrinsic crates)

### Extended Traits (`dotnet-vm/src/stack/ops.rs`)
VM-specific extensions (all `Vm`-prefixed) that add resolver, shared state, and reflection capabilities on top of the base traits:
- `VmStackOps` (extends `StackOps` with frame access and slot operations)
- `VmRawMemoryOps` (extends `RawMemoryOps` with address resolution and `localloc`)
- `VmResolutionOps` (extends `ResolutionOps` with `ResolutionContext`)
- `VmReflectionOps` (extends `ReflectionOps` + `IntrinsicDispatchOps` + `ReflectionLookupOps`)
- `VmLoaderOps` (extends `LoaderOps` with `VmResolverService` and `SharedGlobalState`)
- `VmStaticsOps` (extends `StaticsOps` with `StaticStorageManager` access)
- `VmCallOps` (VM-local frame construction and method dispatch)
- `VmExceptionContext` / `VmPInvokeContext` (VM-side extensions of `ExceptionContext` / `PInvokeContext`)
- `VesOps`: The unified trait combining `PInvokeContext + VmStackOps + VmRawMemoryOps + VmResolutionOps + VmReflectionOps + VmLoaderOps + VmStaticsOps + ThreadOps + VmCallOps` plus the composite host traits defined by the `dotnet-intrinsics-*` crates. These crate-level composites—such as `SpanIntrinsicHost`, `ThreadingIntrinsicHost`, `ReflectionIntrinsicHost`, and `UnsafeIntrinsicHost`—are the names bound directly in `VesOps`; the corresponding `*BaseOps` aliases remain in `dotnet-vm-ops`. `VesOps` is the primary generic bound for instruction handlers.

### Usage Pattern
```rust
pub fn handle_instruction<'gc, T: VesOps<'gc>>(
    ctx: &mut T,
    instr: &Instruction,
) -> StepResult {
    let value = ctx.pop_i32();
    ctx.push_i32(value * 2);
    StepResult::Continue
}
```

## Assembly Isolation and Load Contexts

As of the current implementation, `dotnet-rs` utilizes a single global **Assembly Load Context** (represented by the `AssemblyLoader` in `SharedGlobalState`).

- **Single Namespace**: All assemblies are loaded into a single global namespace. Loading multiple versions of the same assembly name is not supported and will result in a version mismatch error (if strict versioning is enabled) or a warning.
- **Global Statics**: Static fields are managed by a single `StaticStorageManager` in `SharedGlobalState`, meaning they are shared across all threads and assemblies.
- **Application Domain**: The current architecture effectively implements a single, process-wide Application Domain (the "Default Domain").

### Future Considerations

If isolation between multiple applications or plugins is required:
1. **Multiple Loaders**: The `AssemblyLoader` can be instantiated multiple times to create distinct load contexts.
2. **Context Association**: The VM would need to be updated to associate each `StackFrame` or `Thread` with a specific load context to correctly resolve cross-assembly references.
3. **Static Isolation**: To support full ECMA-335 AppDomains, `StaticStorageManager` would need to be moved from `SharedGlobalState` to a per-domain structure.
