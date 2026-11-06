# Async/await VM effects

**Status:** Not started (blocked on plans [06](06-cross-arena-collection-fixture.md) and
[07](07-nested-filter-suspension.md))
**Outcome:** `String.Substring`, multicast delegate invocation, reflection `Invoke`, `ldsfld`
static initialization, `GC.WaitForPendingFinalizers` and `Monitor.Enter` run as async native
bodies in both configurations. Their existing fixtures pass. The bespoke mechanisms they
replace are deleted (listed per slice under "Done when").

Builds on the explicit Substring continuation ([plan 04](04-native-continuations.md)) and keeps
its scheduling and rooting contract. Detailed notes for slices 2–5 are in
[`08-async-effects/`](08-async-effects/). Fill each one in before starting its slice.

## Problem

Native code that must survive a GC safepoint, re-enter managed code or wait currently uses one
of several unrelated mechanisms, each with its own state and bookkeeping:

- **Substring.**
  - ST: a dedicated traced activation (`NativeSubstringActivation`,
    `dotnet-vm/src/stack/context.rs`), an offer hook on `IntrinsicStringHost`, a
    `substring_eligible` flag set in `unified_dispatch_direct` and cleared in intrinsic dispatch
    (`stack/call_ops_impl.rs`), a codegen special case that generates `is_substring`
    (`dotnet-vm/build_support/codegen.rs`), and checks in `step`/`run`
    (`dispatch/mod.rs`) and in finalizer processing (`executor.rs`, `context_ops.rs`).
  - MT: the offer hook declines (`context_ops.rs`, `offer_substring_continuation`), so a
    safepoint yield throws away the copied prefix and the retry starts over.
  - Tests: `dotnet-vm/src/native_substring_tests.rs`; fixtures
    `strings/substring_chunks_42.cs` and `strings/substring_fallback_42.cs`.
- **Multicast delegates** keep target array, index and arguments in `MulticastState` on a
  frame (`dotnet-intrinsics-delegates/src/invoke.rs`).
- **Reflection `Invoke`** post-processes returns through
  `FrameReturnAction::InvokeReturn` (`dotnet-intrinsics-reflection/src/methods.rs`,
  `stack/context.rs`).
- **`ldsfld`** returns early with the result of `initialize_static_storage` and is retried
  (`instructions/objects/fields.rs`, `statics.rs`).
- **`GC.WaitForPendingFinalizers`** spins via `yield_spin` (`dotnet-vm/src/intrinsics/gc.rs`).
- **`Monitor.Enter`** returns `Yield` and retries (`dotnet-intrinsics-threading/src/monitor.rs`).

One effects API replaces these with ordinary sequential Rust and one rooting and scheduling
contract.

## Scope

In:
- The six consumers above, one slice each (slices 4 and 5 share one wait protocol).
- ST and MT for every slice. MT requires plan 06, because MT activations root references
  owned by other arenas.
- Native calls from inside an active filter or handler, once plan 07 makes suspended segments
  nest (slice 2).

Out:
- **Localloc addresses.** ECMA-335 §III.3.47: `localloc` returns an unmanaged pointer (native
  int). They are scalars here and carry no pointer permission.
- **Static field addresses as transport** until `p12_ldsflda_convu_stind_static` (README,
  "Not yet planned") is fixed.
- Transient and unmanaged pointers, and byref-containing aggregates. These stay on the legacy
  path.
- External async runtimes, guest `Task` scheduling, thread-pool work, `Thread.Sleep`/`Join`.
- `select`/`join` over concurrent effects on one capability.
- Typed argument binding for intrinsic bodies.
- Tail-prefixed calls into async intrinsics (legacy path). Tail calls and `jmp` inside a
  managed child of a native body are handled in slice 2.
- New reflection exception semantics. Calls stay transparent and no
  `TargetInvocationException` wrapping is added (maintainer decision; revisit only for an
  observed compatibility failure).

## Decisions already made

- **Lifetime erasure.** The poll bridge uses new unsafe lifetime erasure, and the maintainer
  has approved it. Run it under pinned Tree Borrows Miri. Record any Miri finding against the
  bridge and raise it with the maintainer; a finding does not by itself block a slice.
- **MT waits on plan 06**, not on a foreign-source fallback.
- **Finalizers.** Slice 1 keeps the current deferral: finalizers wait until a parked native
  operation completes. Segment isolation arrives with the first slice where a parked
  activation waits on managed work (slice 2).
- **Reflection** uses transparent calls (slice 3).

## Terms

| Term | Meaning |
|---|---|
| activation | One native body's VM-owned state: header, root set, operands, outputs and future. One per entered intrinsic or instruction. Nested async helpers share their caller's activation. |
| capability (`VmEffects`, `fx`) | The body's handle for requesting effects. Static, `!Send`, created only by the driver. |
| access (`VmAccess<'fx>`) | An exclusive borrow of `fx` granting synchronous VM operations. Holds no heap borrow, lock or lease. |
| region | The branded context lent to the synchronous `vm.with(\|region\| …)` callback for one call. |
| rooted value (`RootedValue`) | An owned handle to a value in the activation's root set. Scalars and null are stored inline. |
| effect | A suspension request: yield, call, initialize or wait. At most one is outstanding per activation. |
| turn | One executor loop iteration that polls an activation, possibly more than once. |

"Guard" refers only to existing storage and scope guards. Do not use it for `VmAccess`.

## Authoring contract

`#[dotnet_intrinsic]` accepts `async fn(mut fx: VmEffects) -> NativeResult` and an optional
`select = path` argument. If a body has several annotations, every one must name the same
selector. `NativeResult = Result<(), NativeFailure>`.

The selector runs synchronously before anything is consumed. It receives the method, generic
metadata and a read-only operand view, and returns one of:

- `Async(entry)`: owned, already-validated scalars for the body (via `fx.invocation()`);
- `Synchronous(handler)`: run an existing handler by the existing convention (direct fast
  paths and legacy fallback);
- `Reject(VmError)`.

**Validation that can throw belongs in the selector or the synchronous handler**, so an async
body starts with valid arguments. It throws only for failures that arise during its own work.

```rust,ignore
#[dotnet_intrinsic("string System.String::Substring(int)", select = substring_select)]
#[dotnet_intrinsic("string System.String::Substring(int, int)", select = substring_select)]
async fn substring(mut fx: VmEffects) -> NativeResult {
    // The selector validated the range and chose async only for multi-chunk results.
    let SubstringRange { start, length } = fx.invocation().selected();
    let source = {
        let mut vm = fx.vm().await?;
        vm.discard_scalars(fx.invocation().parameter_count())?;
        vm.pop_string()?
    };
    let mut buffer = Vec::with_capacity(length);
    while buffer.len() < length {
        {
            let mut vm = fx.vm().await?;
            vm.string(&source).copy_chunk(start, length, &mut buffer)?;
        }
        if buffer.len() < length {
            fx.yield_now().await?;
        }
    }
    let mut vm = fx.vm().await?;
    vm.push_new_string(buffer)?;
    Ok(())
}
```

The body pushes its result (nothing for void). The driver checks stack shape and advances the
caller's IP exactly once. Invocation metadata is owned and readable without heap access.

`fx.vm().await?` returns `VmAccess`. Reference-bearing values removed from VM storage become
rooted values; scalars stay inline. Cloning a rooted value shares its root, and dropping the
last clone releases it. Views borrow the access and run bounded synchronous operations.
`vm.with(|region| …)` is a synchronous higher-ranked callback for shared helpers. No branded
value, heap slice, lock or lease can leave the callback. Neither facade exposes recursive
dispatch, parking, retry or IP mutation.

Instruction bodies (slice 4) do not use the macro. The instruction handler calls the same
entry helper with an instruction completion action (see "Completion").

## Effects

| Effect | Available from | Behavior |
|---|---|---|
| `vm()` | slice 1 | If a collection or stop is pending, yields to the executor before granting access. After 32 grants in one turn, yields before the 33rd. |
| `yield_now()` | slice 1 | Always returns to the executor once. |
| `call(NativeCall)` | slice 2 | Transparent: returns a `NativeValue`. A managed exception escaping the child propagates without resuming Rust. |
| `try_call(NativeCall)` | slice 2 (first consumer decides) | Capturing: the child's handlers and cleanup run, then Rust receives `Err(RootedException)`. Outer filters run after the child unwinds. |
| `ensure_initialized(type, generics)` | slice 4 | Runs or waits for a .cctor and preserves publication, failure wrapping, recursion and wait-graph cycle rules. |
| `wait_finalizers()` | slice 5 | VM-owned condition wait. |
| `monitor_enter(object, deadline) -> bool` | slice 5 | VM-owned condition wait. On `true`, the guest owns the monitor even if later native code fails. |

Only VM effects may suspend. Pure Rust loops need explicit `yield_now()`. A body may hold at
most one outstanding effect. Pending without a registered effect is an internal error.

## Design

### Storage and crates

| Crate | Owns |
|---|---|
| vm-data | `NativeId`; `StepResult::NativeSuspended(NativeId)`; the native return destination on a frame (slice 2) |
| vm-ops | `VmEffects`, `VmAccess`, views, `RootedValue`, `NativeValue`, `NativeCall` (slice 2), effect request types, pinned future wrapper, the object-safe backend trait |
| vm | activation storage on `ThreadContext`, bridge adapter, driver, effect handling, executor integration |
| macros, build_support | `async fn` recognition, generated entry symbol and selector wiring |

Do not move `VesContext`, `SharedGlobalState`, sync blocks or heap managers into vm-ops, and
do not make vm-data depend on vm-ops. The facade calls a narrow object-safe backend trait
implemented by the vm adapter. Generic callback methods live on the concrete facade, not the
trait object. Narrow existing helper bounds rather than implementing retry or dispatch traits
on the region.

Activation layout (slice 1 needs only one optional activation; slice 2 makes it a stack):

```text
NativeActivation<'gc>            #[derive(Collect)] #[collect(no_drop)]
  header: NativeHeader           caller depth, call-site IP, operand base, completion action
  roots: DynamicRootSet<'gc>
  operands: Vec<StackValue<'gc>>
  outputs: Vec<StackValue<'gc>>
  #[collect(require_static)] future: Pin<Box<dyn Future<Output = NativeResult>>>
  #[collect(require_static)] control: Rc<NativeControl>
```

The future and control are `'static` and cannot hold branded values. The activation's
`DynamicRootSet` is what keeps root cells reachable.

### Entry and operand ownership

The generated synchronous entry calls one shared runtime helper (a vm-ops host trait method,
e.g. `start_native`, that the `VesContext` implements). The helper:

1. Runs the selector with a read-only operand view, without consuming anything. `Synchronous`
   and `Reject` return here.
2. For `Async`, moves the whole receiver/argument window, in stack order, into the
   activation's operands, using the normal pop accounting. The caller stays at the call
   instruction, and the helper records the base after consumption.
3. Installs the activation and polls it once within the same mutation.

Failures before step 2 leave the operands untouched. After step 2 the activation is committed:
every outcome is completion, a managed throw or a host error. Legacy fallback and replay are
not possible after commitment.

`vm.pop*()` removes the last entry operand, rooting it before removing it from traced storage.
If rooting fails, the operand stays in place. `vm.push()` appends to traced outputs. Pop cannot
reach below the activation's operands. A body must consume all operands before its first
`call` effect.

### Completion

On `Ready(Ok)`, the driver requires no remaining operands and outputs matching the signature
(zero or one, correctly typed). It checks caller depth, IP and operand base, moves the outputs,
applies the completion action once and destroys the activation. Completion actions:

- **inline call**: `Jump(call_ip + 1)`;
- **synthetic-frame return** (slice 2/3 consumers dispatched through frames);
- **instruction**: slice 4; advance only after the instruction's own work completes.

On a managed throw, the driver discards operands and outputs, moves the exception into traced
exception state while the activation's roots are still live, then starts SEH at the original
call site. A host failure releases the activation and returns the existing `VmError`.

All terminal paths use one cleanup path: stop scheduling, deregister waits, retire children,
disable the bridge, drop the future and control, then release roots and storage. Every early
exit from `Executor::run` goes through it (today's substring clear in `executor.rs` is where it
hooks in). Rust `Drop` never runs managed code.

### Poll bridge

`NativeControl` holds a nullable erased adapter pointer, an access counter, an entered flag,
a grant counter for the 32-grant budget and, from slice 2, a single-slot mailbox. It is
`!Send`/`!Sync`, and only the driver creates it.

For one poll, the driver:

1. takes the future out of the activation;
2. builds the adapter by split-borrowing the activation's roots, operands and outputs and the
   rest of the engine state (never two `&mut` to the whole engine);
3. installs the adapter pointer with a reset guard that clears it on return and on unwind;
4. polls with a no-op waker;
5. restores the future and validates the result.

Every `VmAccess` operation reaches the adapter through one synchronous higher-ranked function
that reborrows it for the duration of the operation. Nested entry is rejected. The erasure is
confined to this function. Its `SAFETY:` comment must state four premises: the erased and
reborrowed types are identical, access is unique (entered flag), the pointer is valid
(install/reset guard), and panic ordering is safe.

When the poll returns, the driver accepts:
- `Pending` only with exactly one registered effect, no live `VmAccess`, a zero access counter
  and no outstanding storage scope;
- `Ready` only with the same exclusion checks.

Anything else, including an access leaked with `mem::forget`, is a terminal internal error that
goes through cleanup. A poll panic unwinds through the reset guard. The activation, now
without its future, is destroyed by the existing host panic policy. This design adds no
`catch_unwind`.

The stop and fairness checks are the same for the first poll and every resumed poll. A
requested yield is never re-polled within the same turn. The grant counter resets at the start
of each turn.

### Roots

`RootedValue` wraps a private `DynamicRoot` to a locally allocated, immutable `Collect` cell
holding a `StackValue`. The cell's trace goes through `ObjectRef::trace`, including its
cross-arena branch (`dotnet-value/src/object/mod.rs`). A bare foreign handle stashed directly
would bypass that branch. Each newly rooted reference value costs one GC allocation plus a
root slot.

A root belongs to its activation's set. Fetching uses gc-arena's set-checked `try_fetch`, and
a mismatch is an internal error. Intrinsic authors get no raw fetch.

Values move between activations (slice 2: arguments, returns, exceptions) only by fetching
from the source set, copying into the destination set and then dropping the source. Never
forward a child's `DynamicRoot` to a parent.

MT tests must root a foreign reference obtained through the plan-06 helper, then collect.

### Managed pointers (slice 3)

A retained managed pointer is a descriptor, resolved again inside each access: a heap owner
plus offset, or a frame-home serial plus offset (frame-home serials are never reused). Static
descriptors are added only after p12. Each category is enabled together with its consumer and
its tests: conversion from the actual `PointerOrigin` variant, tracing, reconstruction, and
invalidation when the owning frame retires. The selector rejects unsupported categories before
commitment, and checks declared byref returns before any side effect.

## Slices

1. **Substring.**
   - Async registration with a selector: empty and single-chunk results are synchronous,
     invalid arguments go to the synchronous handler, unsupported call shapes go legacy.
   - Driver, bridge, `RootedValue`, a single optional activation, `vm()`, `yield_now()`,
     inline-call completion and cleanup, in ST and MT.
   - Keep the finalizer deferral.
   - Delete the `NativeSubstringActivation` machinery.
   - No mailbox, frame IDs or segment isolation.
2. **Multicast delegates through transparent `call`.** Adds:
   - the activation stack, `NativeId`s and a native return destination on frames;
   - the single-slot mailbox and reply generations;
   - exception search that respects native boundaries;
   - finalizer segment isolation (parked activations may now wait on managed work);
   - native calls from inside filters and handlers, on plan 07's nested segments;
   - tail calls and `jmp` in children.
3. **Reflection `Invoke` through transparent `call`.** Adds heap and frame-home byref
   descriptors, boxing and write-through. Deletes `FrameReturnAction::InvokeReturn`.
4. **`ldsfld` through `ensure_initialized`.** Adds the instruction completion action and the
   MT wait-graph handoff.
5. **`WaitForPendingFinalizers`, then `Monitor.Enter`,** through one condition-wait protocol.

Each slice ends with its consumer on the new path and its tests enabled. Add facade operations
only when a consumer needs them.

## Done when

**Slice 1**
- Every test in `native_substring_tests.rs` passes against the async path, in both
  configurations. `dispatch_entry_controls_substring_eligibility` becomes a selector test.
  `pending_finalizer_waits_until_copy_completes` keeps its assertion.
- MT: copy progress survives coordinated stops between chunks, with a foreign source string
  from the plan-06 helper rooted only through the activation.
- Compile-fail tests: a branded value, region, access or view escaping into the future or
  across `.await`. Runtime tests: a forgotten `VmAccess`, an unexpected `Pending`, a
  mismatched caller, a panic during a poll.
- The bridge has run under pinned Tree Borrows Miri, with results recorded.
- Deleted: `NativeSubstringActivation`, `offer_substring_continuation`, `substring_eligible`,
  `unified_dispatch_direct` (if nothing else uses it), and the `is_substring` codegen.
- Paired ST and MT release runs for small and multi-chunk Substring and ordinary dispatch.
  Investigate repeatable regressions above 1% on unaffected controls.

**Slices 2–5:** defined in each slice's note under `08-async-effects/` before the slice
starts. At minimum: the consumer's fixtures pass, CLR comparisons cover any exception or filter
ordering involved, and the listed bespoke mechanism is deleted.

## Ask the maintainer only if

- Any consumer's observable behavior would change: exception or filter order, finalizer
  timing beyond the stated slice-2 change, or monitor ownership.
- Miri reports a finding against the bridge (record it; see "Decisions already made").
- Supporting a consumer needs a pointer category beyond heap and frame-home, or a byref into
  a container that ECMA does not allow.
- A slice needs a new frame-stack or scheduler mechanism not named in its description.
- A repeatable performance regression remains unexplained.

Do not restore the archived prototype (`archive/async-effects-2026-10`) wholesale. Its
`DESIGN.md` solved several problems carried into the slice notes.
