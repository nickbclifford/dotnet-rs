# Delegates and Method Dispatch

This document describes the delegate system, the multicast delegate stepping protocol, and how method dispatch flows from CIL instruction to executed code.

## Overview

Method dispatch in `dotnet-rs` has several interleaved paths that interact in non-obvious ways:

1. **Normal CIL dispatch**: Instruction → handler function → stack manipulation
2. **Intrinsic dispatch**: Method call intercepted → native Rust handler
3. **Virtual dispatch**: Runtime type → vtable lookup → resolved method
4. **Delegate dispatch**: Special interception before normal method resolution
5. **Multicast dispatch**: Sequential invocation of delegate targets with stepping protocol

## Dispatch Flow (`dispatch/mod.rs`)

### `ExecutionEngine::step`

The main dispatch loop in `step` / `step_normal`:

1. Check exception state (`ExceptionState::None`, `ExecutingHandler`, `Filtering`). If an exception is being handled, route to `ves_context.handle_exception()`.
2. Check for pending multicast delegate steps in the current frame's `multicast_state`. If present, route to `handle_multicast_step`.
3. Fetch the CIL instruction at the current Instruction Pointer (`IP`).
4. Record the original IP and evaluation stack height to allow for safe yield/suspension retries.
5. Emit an instruction trace when tracing is enabled and record it in the `InstructionRingBuffer` (in `dispatch/ring_buffer.rs`). This circular buffer maintains a history of the last N executed instructions to provide context during crashes or for performance analysis.
6. Look up the handler in `InstructionRegistry::dispatch` (generated at build time via `dotnet-macros`).
7. Execute the handler, which receives a `VesContext` and returns a `StepResult`.
8. Process the `StepResult`:
   - `Continue`: Increment IP and continue loop.
   - `Jump(target)`: Branch to the target IP and continue.
   - `Yield`: Return from `step` to yield control (e.g., GC safe point).
   - `Exception`: Restart the loop to trigger `handle_exception()`.
   - `Return`: Invoke `ctx.handle_return()`, pop the frame, and handle cross-frame exception/return state.

### `StepResult` Enum

The `StepResult` enum (defined in `crates/dotnet-vm-data`; canonical path: `dotnet_vm_data::StepResult`) dictates how the dispatch loop proceeds after executing an instruction or intrinsic:

- `Continue`: Instruction executed successfully; advance IP to the next instruction.
- `Jump(usize)`: Branch to the specified IP (used by `br`, `brtrue`, `brfalse`, etc.).
- `FramePushed`: A new method call frame has been pushed to the stack. Do not advance the IP (the newly pushed frame's IP starts at 0).
- `Return`: The current method has completed. Pop the frame and resume the caller.
- `MethodThrew(ManagedException)`: Used internally when an exception propagates unhandled out of a frame.
- `Exception`: An exception was thrown (either by a CIL instruction or explicitly). The dispatch loop must call `handle_exception`.
- `Yield`: A GC stop was requested or thread suspension is needed. The execution engine pauses and returns to the caller.
- `Error(VmError)`: A host-side VM error occurred (for example, metadata/type resolution failure, invalid CIL, invalid memory access, or P/Invoke load failure). This is distinct from managed .NET exceptions, which flow through `Exception`/`MethodThrew`.

## Delegate System (`dotnet-intrinsics-delegates`)

### How Delegates Work in .NET
A delegate is an object that wraps a method reference (and optionally a target object). `MulticastDelegate` chains multiple delegates together.

### Delegate Object Layout

Delegate objects in `dotnet-rs` mirror the standard .NET `System.Delegate` layout. The fields are accessed through the typed `DelegateView` / `DelegateViewMut` helpers, which read/write via `instance_storage.field::<T>(type, "name")` (see `crates/dotnet-intrinsics-delegates/src/helpers.rs`):
- `_target` (ObjectRef): The `this` reference for instance methods. For static methods, this is typically null.
- `_method` (usize): A method index. This index points to a `MethodDescription` cached in the runtime's global lookup.
- `targets` (ObjectRef, on `System.MulticastDelegate`): For multicast delegates, an array of individual delegate objects.

The runtime accesses these directly by name when performing delegate invocation or intrinsic operations like `Delegate.Target` and `Delegate.Method`.

### Special Dispatch Path: `try_delegate_dispatch`

`try_delegate_dispatch` runs *inside* the resolved-method dispatcher (`dispatch_resolved_method`), not as a pre-call intercept. It only engages for body-less methods on delegate types:
- It bails out immediately unless `method.body()` is `None` and `is_delegate_type` reports the parent inherits from `System.Delegate`.
- For `Invoke` it dispatches the delegate (single or multicast); `.ctor` returns `None` and is handled by the support-library stub; `BeginInvoke`/`EndInvoke` throw `System.NotSupportedException`.
- Returns `Some(StepResult)` if handled, `None` to fall through to normal dispatch.

### `invoke_delegate`

Single delegate invocation:
1. Pop the delegate object from the stack
2. Read target object and method index through `DelegateView` (centralizing the `_target`/`_method` layout invariant)
3. Resolve the method from the index
4. Build a `PreparedCall` so bound-`this` insertion and argument repush order are shared with multicast stepping, then dispatch the resolved method

### Multicast Delegate Stepping Protocol

Multicast delegates (created by `Delegate.Combine`) contain multiple targets that must be invoked sequentially. This is complex because each target is a full method call that goes through the normal dispatch loop.

**`MulticastState` struct** (in `crates/dotnet-vm-data/src/stack.rs`, re-exported by `dotnet-vm-ops` and `dotnet-vm`):
```rust
pub struct MulticastState<'gc> {
    pub targets: ObjectHandle<'gc>, // Array of delegate objects
    pub next_index: usize,          // Next target to invoke
    pub args: Vec<StackValue<'gc>>, // Preserved arguments for each invocation
}
```

**Stepping Protocol** (`ExecutionEngine::handle_multicast_step` in `dispatch/mod.rs`):
1. **Return Handling**: When `handle_multicast_step` is entered, it first checks if the previous target just returned. If the delegate signature has a return value, it pops and discards it (only the *last* target's return value is kept).
2. **Next Target**: It reads the next delegate object from the `targets` array using `next_index` (which is then incremented) via the branded object-reference vector iterator.
3. **Dispatch**: It builds a multicast `PreparedCall`, pushes the target object and preserved `args` back onto the evaluation stack in call order, then calls `dispatch_method` for the new target.
4. **Completion**: If `next_index` reaches the end of the array, `handle_multicast_step` delegates to `ctx.handle_return()` to finalize the multicast invocation and propagate the last return value to the caller.

### Key Delegate Intrinsics

| Intrinsic                 | Purpose                                   |
|---------------------------|-------------------------------------------|
| `delegate_combine`        | Creates multicast from two delegates      |
| `delegate_remove`         | Removes a delegate from a multicast chain |
| `delegate_equals`         | Compares two delegates for equality       |
| `delegate_get_target`     | Returns the target object                 |
| `delegate_get_method`     | Returns the method info                   |
| `delegate_get_hash_code`  | Hash for delegate identity                |
| `delegate_dynamic_invoke` | Late-bound invocation (NotSupported stub: throws `System.NotSupportedException`) |

## Virtual Dispatch

### Virtual Dispatch Algorithm (`callvirt` in `instructions/calls.rs`)

1. **Pop Arguments**: The `callvirt` handler determines the number of arguments from the method signature and pops them to inspect the `this` reference.
2. **Null Check**: If `this` is a null `ObjectRef`, it throws a `System.NullReferenceException`.
3. **Type Extraction**: The runtime type of `this` is extracted from its heap descriptor (or pointer origin for value types).
4. **Unified Dispatch**: The handler repushes the arguments and calls `ctx.unified_dispatch(source, Some(this_type), None)`.
5. **Resolution**: `unified_dispatch` invokes `ResolverService::resolve_virtual_method`:
   - It walks the inheritance chain and interface implementation tables to find the most-derived implementation of the base method for `this_type`.
   - The result is cached in `GlobalCaches` (the key includes the generic lookup).
6. **Execution**: The engine calls `dispatch_method` on the resolved concrete method.

*Note on Constrained Calls*: `callvirt` prefixed with `constrained.` uses `callvirt_constrained` to avoid boxing value types if they directly implement the method. It checks for a direct override before falling back to boxing and virtual dispatch.

### Interface Dispatch
- Interface methods are resolved to concrete implementations via interface maps
- The resolver walks the type's interface implementation table

## Intrinsic Dispatch

### Call Interception Path
When a resolved method call is about to be dispatched, both VM-context and `ExecutionEngine` API entrypoints route through the VM-local resolved-method dispatcher:
1. Classify the method against the generated/runtime intrinsic registry (using the shared intrinsic cache when available)
2. If intrinsic, dispatch the native handler directly
3. Otherwise handle P/Invoke, supported no-body runtime stubs, and delegate runtime methods
4. If none of those apply, push a managed call frame for normal IL execution

### Special Cases
- `Object.ToString` and `Object.GetType` are registered as intrinsics in `intrinsics/mod.rs` (via `#[dotnet_intrinsic]`)
- Some intrinsics have metadata that affects dispatch behavior (e.g., `filter_name`)

## Non-Obvious Connections

### Delegates ↔ Exception Handling
When a multicast delegate target throws, exception handling must interact with the multicast stepping state. The exception may need to propagate past the multicast dispatch frame.

### Delegates ↔ Reflection
`delegate_get_method` returns a `MethodInfo` reflection object, connecting the delegate system to the reflection registry (`ReflectionRegistry` in `state.rs`).

### Virtual Dispatch ↔ Generics
Virtual dispatch on generic types requires resolving the method with the correct generic instantiation. The virtual dispatch cache key includes the concrete type to handle this.

### Intrinsic Dispatch ↔ Build System
The intrinsic PHF table (generated at build time) and the runtime `IntrinsicRegistry` must agree on key format. Both use `dotnet-macros-core::ParsedSignature` for consistency (see [Build-Time Code Generation](BUILD_TIME_CODE_GENERATION.md)).

### Method Call ↔ Static Initialization
Any method call may trigger static field initialization (`.cctor`) for the declaring type. This is checked during method resolution and can cause recursive initialization or cross-thread waiting (see [Type Resolution and Caching](TYPE_RESOLUTION_AND_CACHING.md)).

## Method Resolution and Pointers

### `MethodSource` and `unified_dispatch`

The `MethodSource` enum (from the `dotnetdll` crate) represents an unresolved method reference, distinguishing between raw user definitions (`UserMethod`) and generic instantiations.

`ExecutionEngine::unified_dispatch` (in `dispatch/mod.rs`) is the central chokepoint for all method calls:
1. It accepts a `MethodSource` and an optional `this_type` (for virtual calls).
2. It resolves the `MethodSource` into a concrete `MethodDescription` using `find_generic_method`.
3. If `this_type` is provided, it performs virtual dispatch via `resolve_virtual_method`.
4. It forwards the final resolved method to `dispatch_method`, which handles intrinsic interception, P/Invoke, and normal frame pushing.

### Delegate Creation (`newobj`)

When `newobj` is called on a delegate type (in `instructions/objects/mod.rs`), the runtime applies a special transformation:
- It detects that the requested type inherits from `System.Delegate` or `System.MulticastDelegate`.
- Instead of executing the empty method body of the CIL delegate constructor, it seamlessly swaps the constructor target to the `.ctor` defined on the `System.Delegate` base class.
- The actual initialization logic resides in a C# support library stub bundled with the VM.

### Method Pointers (`ldftn` / `ldvirtftn`)

Delegates require method pointers. These are generated by CIL instructions (in `instructions/reflection.rs`):
- **`ldftn`**: Finds the exact concrete method via `find_generic_method` and looks up its global runtime index. It pushes this index onto the stack as a native integer (`isize`), which will become the `_method` field of a delegate.
- **`ldvirtftn`**: Pops the target `this` object from the stack, extracts its runtime type, and performs virtual dispatch. It pushes the runtime index of the *resolved most-derived method* onto the stack.

### Tail Calls (`tail.`)

The `tail.` prefix instruction is defined in the CIL standard to indicate that the current stack frame should be discarded before the next call.

`dotnet-rs` implements a guarded form of tail calls for `call` and `callvirt`:

- The `tail.` flag is carried through decoding and the `call` handler dispatches via `unified_dispatch_tail`.
- `callvirt` has a dedicated `CallVirtualTail` handler that performs the required null check / runtime type extraction and then requests tail dispatch.
- Tail-call frame replacement is only performed when it is safe (e.g., the call is immediately followed by `ret`, the evaluation stack is otherwise empty, and the call is not inside an exception region). If the guard fails, the VM falls back to a normal call.
