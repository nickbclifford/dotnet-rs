# Async intrinsic API: the body is the implementation

**Status:** Complete
**Outcome:** An async intrinsic can be registered with only its `async fn` body. Each remaining
`select` only captures values before commitment, excludes a named operand shape, or selects
a measured fast path. Each remaining `fallback` serves only such an excluded shape, a measured
fast path, or one of the residual non-inline callers listed below.

## Problem

This inventory and diagnosis describe the pre-plan baseline. Admitted threading and Substring
validation and the finalizer self-wait return now belong to their bodies; inline IL dispatch and
optional fallbacks are complete. The measured Substring fast-path decision is recorded below.

At plan start, plan 08 required every async intrinsic to have a synchronous `fallback` and
allowed a `select`. There were 12 bodies with 19 registrations. For most of them, the
fallback was a second full implementation:

| Intrinsic | Why the fallback existed at plan start |
|---|---|
| `Thread.Yield`, `Thread.SpinWait` | Non-inline dispatch only. It is the same operation with the yield removed. |
| `Thread.Sleep` | Throws for `ms < -1`. Non-inline dispatch, using a legacy `Yield` retry against `monitor_retry_deadline`. |
| `Monitor.Enter(object)`, `TryEnter(object, int)` (+ `_FastPath`) | Argument throws. Non-inline dispatch, using `monitor_enter_safe` and a legacy retry. |
| `Monitor.ReliableEnter`, `Enter(object, bool&)`, `TryEnter(object, int, bool&)` (+ 2 helpers) | As above, plus `Static`, `Transient` and `Unmanaged` `bool&` origins that the activation cannot transport. Slice 3 admits `Static`. |
| `Monitor.Wait` | Throws `SynchronizationLockException` when the lock isn't owned. Non-inline dispatch, using `monitor_wait_legacy` and `LegacyWait`. |
| `GC.WaitForPendingFinalizers` | Returns immediately inside a finalizer. Non-inline dispatch, using a legacy `Yield` loop. |
| `String.Substring` | Fast path for strings of at most one chunk, plus argument throws. |
| Reflection `Invoke` (3), `Activator.CreateInstance` | Argument and transport shapes the activation did not admit; plan 16 removed this fallback and [plan 19](19-reflection-invoke-clr-parity.md) updated reflection semantics. |

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
  - Targets dispatched by the legacy `MulticastState` continuation (`dispatch/mod.rs`); the
    reflection `InvokeReturn` continuation was removed by plan 16
  - Plan 16 later removed `newobj` of an intrinsic constructor (it now dispatches inline with a
    completion action) and the public `ExecutionEngine::unified_dispatch`/`dispatch_method`
    APIs; neither is a residual caller.
  - `.cctor` dispatch, the span equality slow path and the `Expression.Compile` redirect, all
    of which dispatch managed bodies, never intrinsics

  Plan 16 retired the others; the multicast continuation remains for excluded shapes.
- Transport for transient and unmanaged `bool&` origins. A `Transient` pointer is a value
  type on the evaluation stack and would need tracing; an `Unmanaged` one is a raw address
  whose stability across a suspension the VM can't check. (Plan 16 slice 5b later admitted
  `Unmanaged` and rejected `Transient`.) The monitor byref variants kept
  their fallback and the frame retry deadline for those two shapes only at plan time. `Static` origins are
  in scope (see below).
- Reflection argument validation and transport shapes. Reflection kept its fallback under this
  plan; plan 16 removed it and [plan 19](19-reflection-invoke-clr-parity.md) updated its behavior.
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
  excluded shape, or a fast path that slice 4 keeps) or `Reject`. The migrated threading
  selectors don't throw guest exceptions or validate arguments. Reflection's pre-existing
  validation exclusions remain out of scope; Substring checks validity only to choose its
  measured fast path and sends invalid operands to the body.
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
validate before registering an operation wait or acquiring a monitor, although the initial VM
access itself may wait for a GC stop. This preserves plan 10's validation-before-acquire rule.

`Monitor` and `GC` also need some read-only queries:
- reading the `bool&` flag (unsafe, cited `F10.RawMemoryAccessValid`, like
  `write_monitor_flag`);
- monitor ownership for `Wait`;
- whether the thread is running a finalizer. `advance_finalizers` already treats
  `processing_finalizer` as "not ready", so the in-finalizer early return must happen before
  the wait is registered. Otherwise a finalizer would wait for itself.

### Static `bool&` origins

`RootedValue::stash` (`effects.rs`) admits `Heap`, `FrameHome`, `Static` and (MT)
`CrossArenaObjectRef` pointers under `F6.NativePointerDescriptor`; static monitor flags
therefore reach the async body. A `PointerOrigin::Static` holds no branded
pointer: it is an `Arc<StaticMetadata>` (type and generics) plus an offset. Every raw access
re-resolves the storage through `statics().get(type, generics)` and bounds-checks the offset
(`raw_memory_ops_impl/mod.rs`). Static storage lives as long as the VM, so the descriptor
stays valid across polls and needs no tracing. `stash`, `fetch` and `select_monitor` admit `Static`,
and `F6.NativePointerDescriptor` names the static origin. Committed operands and reconstructed
descriptors are addressless. This adds no new shared-memory hazard, because the legacy fallback already writes the same static byte through the same
`write_bytes`. That hazard stays with the README's shared raw-memory access entry.

### Inline dispatch at IL call sites

Pass `ResolvedMethodDispatchMode::InlineCall` from the five call sites above. The
pending-target and initializer handling in `dispatch_method_with_mode` doesn't depend on the
mode, so only the intrinsic arm changes. Check each site against the activation's admission
and completion checks: the caller depth, the IP at the call instruction, and the operand base
after the call site has prepared the arguments. The static `constrained. call` leaves its
operands in place; `constrained. callvirt` and delegate dispatch reconstruct their argument
windows. Delegate dispatch replaces the delegate receiver with the target (or nothing), and the
target's signature determines the window.

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

## Substring fast-path measurement

The completed slice-3 tree at `385edee6e1fcbfdb46d5a35c0538d2c7dc231891` was the
baseline. The candidate differed only by removing the selector branch that returns `Fallback`
for a valid range of at most one chunk; validation, selector capture, fallback and body were
otherwise unchanged. SHA-256 of `search.rs` was
`28e61ed73fc18d6470eac3d9339e604a9ec644b9b24c7e761f9d265d0ee76f9b` baseline and
`705b4cb3f47196c8587030f518c760a21d0ad2354f48fe6ff5ad45ebe3b3e614` candidate.
The current valid-small branch is retained: each of four pairs in each
configuration slowed down by well over 1% when removed. A measured synchronous path for valid
small inputs and an async body for larger inputs is sanctioned for bulk intrinsics; invalid
Substring arguments still reach the body, and the retained fallback also serves non-inline calls.

Measurements were on an AMD Ryzen 9 9900X (24 logical CPUs, CPU 0 pinned with `taskset`),
Linux 7.2.14-arch1-1 x86-64, `powersave` governor, Rust 1.95.0 (LLVM 22.1.2), .NET SDK
10.0.114 and Microsoft.NETCore.App 10.0.14. Separate baseline/candidate target directories and
ST/MT Criterion homes were used; no instrumentation features. Fixtures built once with the
benchmark single-file project were copied to both targets, verified identical before and after:

| Fixture | SHA-256 |
|---|---|
| `substring/Small_0/SingleFile.dll` | `ebd7b304bc6932a840feae071d42c2361602e08e1f1e702fc047664721ca9ba5` |
| `substring/Large_0/SingleFile.dll` | `1a7ac3f6e3d9523f924e60b360cc9aa34681f2f2d8fc3017c99e8705904160ee` |

Both sides used the following, with their own `CARGO_TARGET_DIR` and `CRITERION_HOME`;
Each configuration ran in order
baseline/candidate, candidate/baseline, baseline/candidate, candidate/baseline:

```sh
taskset -c 0 cargo bench -p dotnet-benchmarks --bench substring --profile bench-fat --no-default-features -- --sample-size 10 --warm-up-time 1 --measurement-time 4 --noplot
taskset -c 0 cargo bench -p dotnet-benchmarks --bench substring --profile bench-fat --no-default-features --features multithreading -- --sample-size 10 --warm-up-time 1 --measurement-time 4 --noplot
```

Each cell is baseline mean / candidate mean in milliseconds followed by the pair change
`100 * (candidate / baseline - 1)`, based on Criterion mean point estimates, not paired
confidence intervals. The last column is the arithmetic mean of the four percentage changes.
Criterion extended small-workload collection beyond the four-second target to collect 10 samples.

| Configuration / workload | Pair 1 | Pair 2 | Pair 3 | Pair 4 | Mean change |
|---|---:|---:|---:|---:|---:|
| ST empty/small | 107.4809 / 141.8008 (+31.93%) | 107.2046 / 143.0862 (+33.47%) | 108.2014 / 140.3793 (+29.74%) | 107.3406 / 140.8659 (+31.23%) | +31.59% |
| ST large control | 13.5304 / 13.4724 (-0.43%) | 13.3730 / 13.8346 (+3.45%) | 13.4149 / 13.3616 (-0.40%) | 13.4852 / 13.5507 (+0.49%) | +0.78% |
| MT empty/small | 113.5007 / 150.0324 (+32.19%) | 116.7881 / 142.1655 (+21.73%) | 115.0851 / 146.5101 (+27.31%) | 111.5406 / 147.6977 (+32.42%) | +28.41% |
| MT large control | 12.9507 / 13.9108 (+7.41%) | 13.3209 / 12.6638 (-4.93%) | 13.2620 / 13.3351 (+0.55%) | 12.9905 / 12.6156 (-2.89%) | +0.04% |

MT's control varied substantially between pairs, but the small-workload slowdown was above
21% in every pair and both launch orders, far beyond the 1% threshold. Cargo rebuilt workspace
crates under CPU 0 affinity before each measured invocation; both sides followed the same procedure.

## Done when

- An async intrinsic with no `fallback` compiles and dispatches, and the scanner accepts it.
- No migrated threading or Substring selector returns `Fallback` for an argument error;
  reflection's excluded validation shapes remained out of scope here; plans 16 and
  [19](19-reflection-invoke-clr-parity.md) subsequently addressed them. Every moved argument check on
  the threading and GC intrinsics is in a body and has a test that fails without it.
- The five IL call-site paths dispatch intrinsics inline. `NativeDispatchMode::Other` comes
  only from the residual callers listed in Scope, and its doc comment names them.
- Fallbacks remain only on the monitor byref variants (and their selectors return `Fallback`
  only for `Transient` and `Unmanaged` flags), on reflection `Invoke`/`Activator` and on
  `Substring` as measured above. Plan 16 subsequently removed the monitor and reflection
  fallbacks; [plan 19](19-reflection-invoke-clr-parity.md) updated reflection semantics.
  `monitor_wait_legacy`, `LegacyWait` and `monitor_wait_retry` are gone.
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
