# Async intrinsic API: the body is the implementation

**Status:** Not started
**Outcome:** An async intrinsic can be registered with only its `async fn` body. Each remaining
`select` only captures values before commitment or excludes a named operand shape. Each
remaining `fallback` serves only such an excluded shape, a measured fast path, or one of the
residual non-inline callers listed below.

## Problem

Plan 08 requires every async intrinsic to have a synchronous `fallback` and lets it have a
`select`. There are 12 bodies with 19 registrations today. For most of them, the fallback is a
second full implementation:

| Intrinsic | Why the fallback exists today |
|---|---|
| `Thread.Yield`, `Thread.SpinWait` | Non-inline dispatch only. It is the same operation with the yield removed. |
| `Thread.Sleep` | Throws for `ms < -1`. Non-inline dispatch, using a legacy `Yield` retry against `monitor_retry_deadline`. |
| `Monitor.Enter(object)`, `TryEnter(object, int)` (+ `_FastPath`) | Argument throws. Non-inline dispatch, using `monitor_enter_safe` and a legacy retry. |
| `Monitor.ReliableEnter`, `Enter(object, bool&)`, `TryEnter(object, int, bool&)` (+ 2 helpers) | As above, plus `Static`, `Transient` and `Unmanaged` `bool&` origins that the activation cannot transport. Slice 3 admits `Static`. |
| `Monitor.Wait` | Throws `SynchronizationLockException` when the lock isn't owned. Non-inline dispatch, using `monitor_wait_legacy` and `LegacyWait`. |
| `GC.WaitForPendingFinalizers` | Returns immediately inside a finalizer. Non-inline dispatch, using a legacy `Yield` loop. |
| `String.Substring` | Fast path for strings of at most one chunk, plus argument throws. |
| Reflection `Invoke` (3), `Activator.CreateInstance` | Argument and transport shapes the activation does not admit (see the reflection entries in the [plans README](README.md#not-yet-planned)). |

Two gaps cause most of this duplication:

1. **Bodies can't easily throw.** The driver already turns `Err(NativeFailure::Managed(e))`
   into a throw at the call site (`advance_native_effect`, `native_effects.rs`). Plan 08 defines
   this, and reflection uses it. What's missing is a way for a body to create a named CoreLib
   exception. So selectors return `Fallback` for invalid arguments, and the legacy handler
   throws through `throw_by_name_with_message`.
2. **Only `InlineCall` can suspend, and several IL call sites don't use it.**
   `NativeIntrinsicHost::start_native` (`native_effects.rs`) sends every
   `NativeDispatchMode::Other` call straight to the fallback. `Other` comes from
   `ResolvedMethodDispatchMode::VmContext` (`dispatch_method`), `dispatch_method_tail` and
   `intrinsic_call`. The callers of `dispatch_method` include these ordinary IL call sites:
   - `constrained. call` (`instructions/calls.rs`, `call_constrained`)
   - `constrained. callvirt` (`instructions/calls.rs`, `callvirt_constrained`)
   - Single-cast delegate `Invoke`: `callvirt Invoke` → no body → `try_delegate_dispatch` →
     `delegate_dispatch_method` (`context_ops.rs`). This means `Action<int> s = Thread.Sleep;
     s(10)` uses the legacy sleep today.
   - The no-body abstract virtual redirect (`try_dispatch_no_body_abstract_virtual`)
   - `tail.`-prefixed intrinsic calls (`dispatch_method_tail`)

   Each of these runs at an IL call instruction whose completion is `Jump(call_ip + 1)`. That
   is the completion an inline activation already uses.

Blocking and bulk intrinsics are next in line for migration: the string, `Buffer`, `Memmove`
and span loops that return `StepResult::Yield` at GC safe points. As things stand, each would
need three functions. This plan settles the API first.

## Scope

**In scope:**
- A throw helper for bodies, with validation moved into the bodies of the threading and GC
  intrinsics.
- Inline dispatch at every IL call site listed above.
- An optional `fallback`, with the legacy fallbacks and machinery that become unused deleted.
- One measured decision on whether a small-input fast path needs a sync handler.

**Out of scope:**
- The residual non-inline callers:
  - Targets dispatched by the legacy `MulticastState` continuation (`dispatch/mod.rs`) and the
    legacy reflection `InvokeReturn` continuation (`reflection_ops_impl.rs`)
  - `newobj` of an intrinsic constructor (`intrinsic_call` from `instructions/objects/mod.rs`),
    whose completion must finish the constructor receiver
  - `.cctor` dispatch, the span equality slow path and the `Expression.Compile` redirect, all
    of which dispatch managed bodies, never intrinsics

  Retiring those continuations is the separate "Fallback continuations for excluded shapes"
  item.
- Transport for transient and unmanaged `bool&` origins. A `Transient` pointer is a value
  type on the evaluation stack and would need tracing; an `Unmanaged` one is a raw address
  whose stability across a suspension the VM can't check. The monitor byref variants keep
  their fallback and the frame retry deadline for those two shapes only. `Static` origins are
  in scope (see below).
- Reflection argument validation and transport shapes. Reflection keeps its fallback.
- Migrating any further intrinsics. They wait for this plan.

## Design

### Target API

```rust
// Common case: body only.
#[dotnet_intrinsic("static bool System.Threading.Thread::Yield()")]
pub async fn thread_yield(mut fx: VmEffects) -> NativeResult { … }

// Capture before commitment (a deadline that must start before the first VM access can
// wait for a stop). The selector does no validation: invalid values reach the body, which throws.
#[dotnet_intrinsic("static void System.Threading.Thread::Sleep(int)", select = sleep_select)]

// An operand shape the activation cannot transport: the selector returns `Fallback`.
#[dotnet_intrinsic("…Monitor::ReliableEnter(object, bool&)", select = reliable_enter_select,
                   fallback = intrinsic_monitor_reliable_enter)]
```

Rules:
- `select` is optional. It returns `Async` (with captured values), `Fallback` (only for a named
  excluded shape, or a fast path that slice 4 keeps) or `Reject`. It doesn't throw guest
  exceptions or validate arguments.
- `fallback` is optional. A selector that returns `Fallback` for an intrinsic with no fallback
  is already an internal error (`start_native_inner`).
- If an `Other` dispatch reaches an intrinsic with no fallback, the result is
  `NotImplemented("async intrinsic {method} reached through non-inline dispatch")`. Only the
  residual callers above can produce `Other`.
- This supersedes plan 08's "Entry and operand ownership" step 1: every non-inline mode no
  longer goes to the fallback, and a fallback is no longer mandatory. Update that section to
  point here.

### Throwing from a body

Add a `NativeBackend` method and a `VmAccess` wrapper that build a CoreLib exception the way
`throw_by_name_with_message` and `throw_argument_out_of_range` do. These allocate, set
`_message` (and `_paramName`) and run no constructor. The wrapper roots the result in the
activation and returns it as `NativeFailure::Managed`. A body can then write
`return Err(vm.exception("System.ArgumentNullException", msg)?)`.

The operands have already been committed by then. The driver discards them on a throw, which
matches the CLR: intermediate results on the evaluation stack are lost when an exception is
thrown (§I.12.4.2.5). Bodies
validate before their first suspension, so a throw comes before any wait or acquire. This
preserves the plan 10 rule that `Monitor` validates before acquiring.

`Monitor` and `GC` also need some read-only queries:
- reading the `bool&` flag (unsafe, cited `F10.RawMemoryAccessValid`, like
  `write_monitor_flag`);
- monitor ownership for `Wait`;
- whether the thread is running a finalizer. `advance_finalizers` already treats
  `processing_finalizer` as "not ready", so the in-finalizer early return must happen before
  the wait is registered. Otherwise a finalizer would wait for itself.

### Static `bool&` origins

`RootedValue::stash` (`effects.rs`) admits only `Heap`, `FrameHome` and (MT)
`CrossArenaObjectRef` pointers under `F6.NativePointerDescriptor`, so `Monitor.Enter(o, ref
s_taken)` on a static field takes the fallback. A `PointerOrigin::Static` holds no branded
pointer: it is an `Arc<StaticMetadata>` (type and generics) plus an offset. Every raw access
re-resolves the storage through `statics().get(type, generics)` and bounds-checks the offset
(`raw_memory_ops_impl/mod.rs`). Static storage lives as long as the VM, so the descriptor
stays valid across polls and needs no tracing. Admit `Static` in `stash` and in
`select_monitor`, and extend `F6.NativePointerDescriptor` in
`docs/INVARIANT_REGISTRY.md` and plan 08's managed-pointers section to "or a static
origin, re-resolved by type and generics on every access". This adds no new shared-memory
hazard, because the legacy fallback already writes the same static byte through the same
`write_bytes`. That hazard stays with the README's shared raw-memory access entry.

### Inline dispatch at IL call sites

Pass `ResolvedMethodDispatchMode::InlineCall` from the five call sites above. The
pending-target and initializer handling in `dispatch_method_with_mode` doesn't depend on the
mode, so only the intrinsic arm changes. Check each site against the activation's admission
and completion checks: the caller depth, the IP at the call instruction, and the operand base
after the call site has re-pushed the arguments. The `constrained.` sites and delegate
dispatch re-push their arguments before dispatching. Delegate dispatch replaces the delegate
receiver with the target (or nothing), and the target's signature determines the window.

## Slices

1. **Throws from bodies.** Add the exception helper and the queries. Move all argument
   validation for `Sleep`, every `Monitor` enter and `Wait`, and the in-finalizer return of
   `WaitForPendingFinalizers` into the bodies and the driver. Selectors keep only capture and
   shape exclusion. The fallbacks are unchanged and still validate for non-inline calls.
   - Tests: the existing argument-error fixtures and unit tests pass through the inline path.
     Add a unit test per moved check that fails if the check is removed from the body.
     `WaitForPendingFinalizers` inside a finalizer returns through the effect path.
2. **Inline IL call sites.** Switch the five call sites to inline dispatch.
   - Tests:
     - New diff fixtures for a single-cast delegate to `Thread.Sleep`.
     - A delegate to a contended `Monitor.Enter` (`multithreading`).
     - A closed delegate to a multi-chunk `"…".Substring`.
     - Real-dispatch unit tests with a test intrinsic for `constrained.` and `tail.`, which C#
       can't easily emit for the current intrinsics.
3. **Optional fallback.** Make `fallback` optional in `dotnet-macros-core` (the "async
   intrinsic requires a fallback" check), in the `dotnet-macros` entry generation and in
   `build_support/codegen.rs`. Make `NativeSelectorProvider::fallback()` return an `Option`.
   Change the `start_native` signature to match and return the `NotImplemented` error above.
   Delete the fallback and selector-only validation from `Yield`, `SpinWait`, `Sleep`,
   `Monitor.Enter(object)`, the `TryEnter(object, int)` pair, `Monitor.Wait` and
   `WaitForPendingFinalizers`. Delete the legacy machinery that is now unused:
   `monitor_wait_legacy`, `LegacyWait`, `monitor_wait_retry` and Sleep's use of the retry
   deadline. Before deleting each item, run a grep to confirm it has no remaining user.
   Admit `Static` `bool&` origins as described above, so the monitor byref fallback is left
   with only `Transient` and `Unmanaged` flags.
   - Tests:
     - A diff fixture for `Monitor.Enter(o, ref s_taken)` and `Monitor.TryEnter(o, 10, ref
       s_taken)` on a static field. In `multithreading`, make it contended so the activation
       suspends, writes the flag through after the wait, and the timed variant's deadline
       survives a GC stop.
     - A unit test where a stashed static-origin pointer is fetched and written after a
       collection.
     - Replace the `NativeDispatchMode::Other` legacy tests in `condition_wait_tests.rs`
     with tests that the remaining excluded shapes still fall back, and that `Other` without a
     fallback returns the `NotImplemented` error.
4. **Fast-path decision.** Measure `Substring` with its selector's fast-path branch removed,
   so small strings go through the body, against the current code. Use
   `crates/dotnet-benchmarks/benches/substring.rs` and the plan 08 throughput method: four
   alternating pairs, ST and MT, `bench-fat`, pinned to CPU 0.
   - If the mean small-`Substring` slowdown is no more than 1% in both configurations, remove
     the fast path. `Substring` keeps a fallback only if validation needs it, and slice 1
     should already have removed that need.
   - Otherwise, keep the `Fallback` fast path and record it in the macro docs as the sanctioned
     pattern for bulk intrinsics: a small input is handled synchronously, a large one goes
     through the body.
   - Record the numbers in this plan.

Each slice updates the docs it invalidates:
- `docs/BUILD_TIME_CODE_GENERATION.md` (async intrinsics, the generated provider example)
- the `#[dotnet_intrinsic]` doc comment in `dotnet-macros/src/lib.rs`
- the `NativeDispatchMode` and `NativeSelection` doc comments
- plan 08's entry section and the slice-5 legacy-timeout decision
- the README's "Fallback continuations for excluded shapes" entry

## Done when

- An async intrinsic with no `fallback` compiles and dispatches, and the scanner accepts it.
- No selector returns `Fallback` for an argument error. Every argument check on the
  threading and GC intrinsics is in a body and has a test that fails without it.
- The five IL call-site paths dispatch intrinsics inline. `NativeDispatchMode::Other` comes
  only from the residual callers listed in Scope, and its doc comment names them.
- Fallbacks remain only on the monitor byref variants (and their selectors return `Fallback`
  only for `Transient` and `Unmanaged` flags), on reflection `Invoke`/`Activator` and,
  depending on slice 4, on `Substring`. `monitor_wait_legacy`, `LegacyWait` and
  `monitor_wait_retry` are gone.
- A static `bool&` flag reaches the async body, and `F6.NativePointerDescriptor` names the
  static origin.
- `cargo test -p dotnet-vm --lib` and the full integration binary pass in both configurations.
  Clippy is clean in both. `scripts/check_doc_drift.sh` passes.

## Ask the maintainer only if

- An IL call site in the list can't satisfy the activation's IP, depth or operand-base
  checks without a new completion action.
- A fixture or a real program reaches a residual `Other` caller for an intrinsic whose
  fallback slice 3 deletes.
- Admitting `Static` needs anything beyond the descriptor, such as tracing, a type-lifetime
  rule or a change to how static storage is resolved.
- Supporting `Transient` or `Unmanaged` flags turns out to be small, which would remove
  `Monitor`'s last fallback.
- The slice 4 measurement is above 1% in one configuration and not in the other, or doesn't
  repeat.
