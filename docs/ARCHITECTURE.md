# dotnet-rs Architecture

`dotnet-rs` is a Rust-based implementation of the .NET Common Language Infrastructure (CLI), specifically the Virtual Execution System (VES), as defined in ECMA-335.

## Crate Responsibilities

The project is divided into several crates, each with a focused responsibility:

- **dotnet-cli**: The entry point. It provides the command-line interface, test harness, and integration tests.
- **dotnet-vm**: The core of the virtual machine. It includes the execution engine (executor), instruction handlers, native intrinsics (BCL), memory management (heap and GC), and threading support. Re-exports several extracted sub-crates for backward compatibility.
- **dotnet-vm-ops**: Foundational VES operation traits (`EvalStackOps`, `TypedStackOps`, `ExceptionOps`, `RawMemoryOps`, etc.), execution data types (`StepResult`, `MethodInfo`, `MethodState`), exception data types (`ExceptionState`, `ProtectedSection`, `Handler`, etc.), and stack types (`EvaluationStack`, `FrameStack`, `StackFrame`). Depends only on `dotnet-value`, `dotnet-types`, `dotnet-utils`, and `dotnetdll` — no circular dependency on `dotnet-vm`.
- **dotnet-exceptions**: Exception handling logic extracted from `dotnet-vm`. Contains the `ExceptionHandlingSystem` with the two-pass search/unwind state machine. Depends on `dotnet-vm-ops` for base traits and types.
- **dotnet-pinvoke**: P/Invoke marshalling extracted from `dotnet-vm`. Uses `libffi` and `libloading` for native interop. Depends on `dotnet-vm-ops` for base traits.
- **dotnet-metrics**: Standalone crate for `RuntimeMetrics` with per-cache hit/miss tracking (`CacheStats`, `CacheStat`), serializable via `serde`.
- **dotnet-tracer**: Standalone crate for the `Tracer` subsystem. Provides structured logging via the `tracing` crate with configurable levels (`DOTNET_LOG` env), a `LogEntry` enum for structured trace events, and an async flusher thread via `crossbeam-channel`.
- **dotnet-assemblies**: Handles loading and resolving .NET assemblies. It also includes a support library of C# stubs for core types.
- **dotnet-value**: Defines the representation of all .NET values at runtime, including stack values, managed/unmanaged pointers, heap objects, and field storage layouts.
- **dotnet-types**: Implements the .NET type system, including type descriptors, method/field info, generics, and type comparison logic.
- **dotnet-utils**: Contains shared utilities like synchronization primitives, atomic access, GC-related helper types, and strongly-typed newtypes.
- **dotnet-macros** & **dotnet-macros-core**: Procedural macros used to define instructions and intrinsics concisely.

### Crate Dependency Hierarchy

```
dotnet-cli               # CLI entry point, TestHarness, integration tests, feature config tests
    └── dotnet-vm            # Core VM: Executor, Instruction Set, Intrinsics, GC, Threading
        ├── dotnet-exceptions    # Exception handling system (ExceptionHandlingSystem, two-pass SEH)
        ├── dotnet-pinvoke       # P/Invoke marshalling (libffi, libloading)
        ├── dotnet-vm-ops        # Base VES traits, StepResult, MethodInfo, EvaluationStack, ExceptionState
        │   ├── dotnet-assemblies    # Assembly loading, resolution, bundled support library (.cs stubs)
        │   ├── dotnet-value         # StackValue, Managed/Unmanaged Pointers, Heap Objects, FieldStorage, Layout
        │   ├── dotnet-types         # Type/Method/Field descriptors, Generics, TypeComparer, RuntimeType, Error types
        │   ├── dotnet-utils         # GC utilities, ThreadSafeLock, AtomicAccess, Sync primitives, Newtypes, BorrowGuard
        │   ├── dotnet-metrics       # RuntimeMetrics, CacheStats (standalone, serde)
        │   └── dotnet-tracer        # Tracer, LogEntry, structured logging (standalone)
        ├── dotnet-macros        # Proc-macros: #[dotnet_intrinsic], #[dotnet_instruction], #[dotnet_intrinsic_field]
        └── dotnet-macros-core   # Shared logic for macro expansion
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
5. **Instruction Set and Dispatch**: CIL instructions are categorized (arithmetic, flow, objects, etc.) and handled in `crates/dotnet-vm/src/instructions/`. The `dispatch/` subsystem manages instruction execution, including an auto-generated dispatch table in `registry.rs` and a high-performance `ring_buffer.rs` for execution tracing.
6. **Method Calls and Intrinsics**:
    - Static calls resolve the target method and push a new `StackFrame`.
    - Virtual calls use the object's vtable (computed via the layout system) to find the correct method implementation.
    - Intrinsic calls are intercepted and handled by native Rust code in `crates/dotnet-vm/src/intrinsics/`, including core BCL logic and `constants.rs` for intrinsic metadata. Similar to instructions, intrinsics use a monomorphic ID-based dispatch system to ensure high performance.
    
    (See [Delegates and Dispatch](DELEGATES_AND_DISPATCH.md) for more details on invocation paths).

## Memory and Garbage Collection

`dotnet-rs` uses a Stop-The-World (STW) garbage collector based on the `gc-arena` crate. (See [GC and Memory Safety](GC_AND_MEMORY_SAFETY.md) for an in-depth look).

- **Heap Management**: `HeapManager` handles the allocation of objects. Each thread typically has its own arena for allocation to minimize contention.
- **GC Roots**: The evaluation stack, local variables, and static fields serve as the primary roots for GC.
- **STW Coordination**: When a GC is triggered, all threads are brought to a "Safe Point" (e.g., at a loop back-edge or method call). Once all threads are paused, the collector traces all reachable objects across all arenas.
- **BorrowGuard**: To prevent deadlocks during STW, `BorrowGuard` must be used when holding a reference to heap-allocated data. It informs the GC that the thread is currently "busy" and cannot safely pause until the guard is dropped.
- **Collect Trait**: Every type stored on the heap or containing GC references must implement the `Collect` trait to allow the tracer to find nested references.

## Threading Model

The VM supports multi-threading (feature-gated via `multithreading`). For detailed mechanics, see [Threading and Synchronization](THREADING_AND_SYNCHRONIZATION.md):

- **Thread Manager**: Manages the lifecycle of managed threads and coordinates STW pauses.
- **Safe Points**: Execution periodically checks if a GC or suspension has been requested via `ctx.check_gc_safe_point()`.
- **Synchronization**: .NET `Monitor` (lock/unlock) is implemented using `SyncBlockManager`, providing thread-safe access to objects with monitor-style semantics.

## Exception Handling

`dotnet-rs` implements the ECMA-335 structured exception handling (SEH) model using a two-pass approach.
- **State Machine**: Exception processing is modeled as a state machine (`Throwing` → `Searching` → `Unwinding` → `ExecutingHandler`).
- **Filter Clauses**: Support for dynamic `filter` blocks that run user CIL code during the search phase.
- **Unwinding**: The `leave` instruction and exception unwinding properly execute `finally` and `fault` blocks.
- **Extracted Crate**: The exception handling system (`ExceptionHandlingSystem`) lives in `dotnet-exceptions`, while the exception data types (`ExceptionState`, `ProtectedSection`, `Handler`, etc.) live in `dotnet-vm-ops/src/exceptions.rs`.

See [Exception Handling](EXCEPTION_HANDLING.md) for full details on the state machine and unwinding process.

## Type System and Layout

For more details on caching and resolution pipelines, see [Type Resolution and Caching](TYPE_RESOLUTION_AND_CACHING.md).

- **Type Resolution**: Types are resolved lazily. `ResolutionContext` manages the scope of resolution, including generic parameters.
- **Layout Calculation**: `LayoutFactory` computes the physical memory layout of objects and value types, including field offsets and GC descriptors (which fields are references).
- **Generics**: Generic types and methods are instantiated on-demand, with metadata specialized for the specific type arguments.

## Trait Architecture

The VES trait system is split across two crates to avoid circular dependencies:

### Base Traits (`dotnet-vm-ops/src/ops.rs`)
Foundational traits that instruction handlers and intrinsics can target without depending on `dotnet-vm`:
- `EvalStackOps`, `TypedStackOps`, `LocalOps`, `ArgumentOps`, `VariableOps`, `AllStackOps`
- `ExceptionOps`, `RawMemoryOps`, `ThreadOps`, `CallOps`, `LoaderOps`
- `MemoryOps`, `ResolutionOps`, `ReflectionOps`, `StaticsOps`
- `VesBaseOps`, `VesInternals`, `ExceptionContext`, `PInvokeContext`

### Extended Traits (`dotnet-vm/src/stack/ops.rs`)
VM-specific extensions that add resolver, shared state, and reflection capabilities:
- `StackOps` (extends `BaseStackOps` with frame access and slot operations)
- `ResolutionOps` (extends `BaseResolutionOps` with `ResolutionContext`)
- `ReflectionOps` (extends `BaseReflectionOps` + `IntrinsicDispatchOps` + `ReflectionLookupOps`)
- `LoaderOps` (extends `BaseLoaderOps` with `ResolverService` and `SharedGlobalState`)
- `CallOps` (extends `BaseCallOps` with frame construction and method dispatch)
- `StaticsOps` (extends `BaseStaticsOps` with `StaticStorageManager` access)
- `VesOps`: The unified trait combining `ExceptionContext + PInvokeContext + StaticsOps + ThreadOps + CallOps`. Primary generic bound for instruction handlers.

### Usage Pattern
```rust
pub fn handle_instruction<'gc, T: VesOps<'gc> + ?Sized>(
    ctx: &mut T,
    instr: &Instruction,
) -> StepResult {
    let value = ctx.pop_i32();
    ctx.push_i32(value * 2);
    StepResult::Continue
}
```
