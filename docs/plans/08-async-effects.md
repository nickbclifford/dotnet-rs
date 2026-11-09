# Async/await VM effects

**Status:** Complete
**Outcome:** `String.Substring`, admitted multicast delegate invocation, reflection `Invoke`, static
field initialization, `GC.WaitForPendingFinalizers` and `Monitor.Enter` run as async native
bodies in both configurations for admitted shapes. Their existing fixtures pass. The bespoke
mechanisms they replace are deleted, except the scoped multicast continuation, reflection return
handling, legacy static-initializer triggers and timed monitor retry described below.

Builds on the explicit Substring continuation ([plan 04](04-native-continuations.md)) and keeps
its scheduling and rooting contract. Detailed notes for slices 2–5 are in
[`08-async-effects/`](08-async-effects/); they describe the completed implementations and their
verification.

## Problem

Native operations that cross GC safepoints, re-enter managed code or wait need a shared rooting
and scheduling contract. The consumers use native effects for admitted calls in both ST and MT:
Substring's bounded copy, multicast target calls, reflection invocation, static field
initialization, finalizer waiting and monitor acquisition. The synchronous and continuation
fallbacks below preserve excluded call shapes and initialization triggers.

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
  "Not yet planned") is fixed. Multicast invocations that require them stay on the existing
  continuation, selected before operands are committed.
- Transient and unmanaged pointers, and byref-containing aggregates. These stay on the legacy
  path; multicast retains its continuation for such shapes.
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
- **Finalizers.** Slice 2 added isolated managed segments, so finalizers can run while a native
  activation is parked without resuming or overwriting its caller, filter or handler context.
- **Reflection** uses transparent calls (slice 3).
- **Multicast fallback (2026-11-06).** Preserve the existing continuation for invocation shapes
  that effects cannot safely transport, including static-field byrefs. Route supported shapes
  through transparent calls. The fallback is an explicit exception to deleting the multicast
  continuation, not permission to transport excluded pointers or reject previously supported
  invocations. The fallback is `invoke_delegate` → `MulticastState` → `handle_multicast_step`;
  single-target dispatch does not use it. The slice note defines admission and verification.
- **Reflection fallback (2026-11-06).** Keep and correct the existing return continuation only
  for reflection calls excluded from effects, including non-inline and tail-prefixed dispatch.
  Admitted calls use transparent effects. The slice 3 note defines the fallback's lifetime,
  initializer retry and verification requirements. This is an exception to deleting
  `InvokeReturn`, not permission to transport excluded pointers or reject previously supported
  invocations. Its only producers are the fallback for `MethodInfo.Invoke` and both
  `ConstructorInfo.Invoke` overloads in `runtime_method_info_intrinsic_call`, and the fallback
  for `Activator.CreateInstance(Type, object[])` in `intrinsic_activator_create_instance_type_args`.
  The existing early rejection of unsupported declared byref returns remains.
- **Monitor timeout fallback (2026-11-07).** For timed monitor calls excluded from effects,
  use a frame-associated retry deadline rather than the shared host-thread-local deadline.
  Admitted calls retain activation-owned deadlines. The slice 5 note specifies the record's
  lifetime, cleanup and verification; pointer admission and monitor ownership are unchanged.
- **Static initialization** uses the capturing `try_call` for field instructions. The
  CLR-diffed `statics/cctor_filter_42.cs` shape observes a `TypeInitializationException` in its
  filter; see the [slice 4 note](08-async-effects/slice-4-static-initialization.md) for scope.
  The scoped legacy triggers—`newobj`, eligible method dispatch through
  `prepare_method_initialization`, and `RuntimeHelpers.RunClassConstructor`—retain retry and
  unwind-time wrapping. The [static initialization note](08-async-effects/slice-4-static-initialization.md)
  specifies their timing and operand contracts.

## Terms

| Term | Meaning |
|---|---|
| activation | One async body's VM-owned state: method and return identity, caller and stack ownership, roots, operands, outputs and future. One per entered intrinsic or instruction; nested async helpers share their caller's activation. |
| capability (`VmEffects`, `fx`) | The body's handle for requesting effects. Static, `!Send`, created only by the driver. |
| access (`VmAccess<'fx>`) | An exclusive borrow of `fx` granting synchronous VM operations. Holds no heap borrow, lock or lease. |
| region | The branded context lent to the synchronous `vm.with(\|region\| …)` callback for one call. |
| rooted value (`RootedValue`) | An owned handle to a value in the activation's root set. Scalars and null are stored inline. |
| effect | A suspension request: yield, call, initialize or wait. At most one is outstanding per activation. |
| turn | One executor loop iteration that polls an activation, possibly more than once. |

"Guard" refers only to existing storage and scope guards. Do not use it for `VmAccess`.

## Authoring contract

`#[dotnet_intrinsic]` accepts `async fn(mut fx: VmEffects) -> NativeResult`, an optional
`select = path`, and a required synchronous `fallback = path`. If a body has several annotations,
all selectors, when present, and all fallbacks must agree. The fallback is independent of the
optional selector and uses the existing intrinsic handler convention for non-inline dispatch and
call shapes not admitted by the selector. `NativeResult = Result<(), NativeFailure>`.

The selector runs synchronously before anything is consumed. It receives a shared host reference,
the method, generic metadata and a read-only operand view, and returns one of:

- `Async(entry)`: owned, already-validated scalars for the body (via `fx.invocation()`);
- `Synchronous(handler)`: run an existing handler by the existing convention (direct fast
  paths and legacy fallback);
- `Reject(VmError)`.

**Validation that can throw belongs in the selector or the synchronous handler**, so an async
body starts with valid arguments. It throws only for failures that arise during its own work.

```rust,ignore
/// Copy a selector-validated substring without retaining a heap borrow across a yield.
#[dotnet_intrinsic("string System.String::Substring(int)", select = substring_select, fallback = intrinsic_string_substring)]
#[dotnet_intrinsic("string System.String::Substring(int, int)", select = substring_select, fallback = intrinsic_string_substring)]
pub async fn substring_async(mut fx: VmEffects) -> NativeResult {
    let invocation = fx.invocation();
    let source = {
        let mut vm = fx.vm().await?;
        vm.discard_scalars(invocation.parameter_count)?;
        vm.pop_string()?
    };
    let mut buffer = substring_buffer(invocation.length);
    while buffer.len() < invocation.length {
        {
            let mut vm = fx.vm().await?;
            vm.with(|region| {
                let StackValue::ObjectRef(ObjectRef(Some(handle))) = region.fetch(&source)? else {
                    return Err(dotnet_types::error::ExecutionError::InternalError(
                        "native substring source invalid".into(),
                    )
                    .into());
                };
                let inner = handle.borrow();
                let HeapStorage::Str(chars) = &inner.storage else {
                    return Err(dotnet_types::error::ExecutionError::InternalError(
                        "native substring source invalid".into(),
                    )
                    .into());
                };
                if invocation
                    .start
                    .checked_add(invocation.length)
                    .is_none_or(|end| end > chars.len())
                {
                    return Err(dotnet_types::error::ExecutionError::InternalError(
                        "native substring range invalid".into(),
                    )
                    .into());
                }
                copy_substring_chunk(
                    chars,
                    invocation.start,
                    invocation.length,
                    buffer.len(),
                    &mut buffer,
                );
                Ok::<(), dotnet_types::error::VmError>(())
            })??;
        }
        if buffer.len() < invocation.length {
            fx.yield_now().await?;
        }
    }
    fx.vm().await?.push_new_string(buffer)
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
| `try_call(NativeCall)` | slice 4 (only `ensure_initialized` needs it) | Capturing: the child's handlers and cleanup run, then Rust receives `Err(RootedException)`. Outer filters run after the child unwinds. |
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

`CallState` in `ThreadContext` owns the active `Vec<NativeActivation>` and the suspended
native segments. A `NativeActivation` stores its pending wait and call, `NativeFutureOwner`
(control and future), ambient exception, dynamic roots, operands and outputs, optional
instruction completion, native identity and method, caller depth/IP and stack height, output
shape, synthetic-return state and reflection return actions. These are the shipped fields in
`stack/context.rs`. The future and control are
`'static` and cannot hold branded values. The activation's `DynamicRootSet` keeps root cells
reachable.

### Entry and operand ownership

The generated synchronous entry calls one shared runtime helper (a vm-ops host trait method,
e.g. `start_native`, that the `VesContext` implements). The helper:

1. Receives the dispatch mode as a parameter. For any mode other than an ordinary non-tail inline
   `call` or `callvirt`, it invokes the registered fallback before selecting or consuming operands.
   For an admitted mode it runs the optional selector with a read-only operand view; `Synchronous`
   and `Reject` return here. With no selector the driver selects the async body with `start` and
   `length` set to zero and `parameter_count` taken from the method signature (excluding the
   receiver). The generated registration must provide the fallback even when there is no
   selector; `Async` does not have to carry it. The macro and scanner reject a missing fallback
   or inconsistent fallback annotations; code generation forwards it with the selector and mode.
2. For `Async`, classifies the declared return before commitment. Unsupported return shapes are
   rejected without entering the body or consuming operands. The multicast selector routes its
   unsupported shapes to the existing fallback before reaching this check.
3. Moves the whole receiver/argument window, in stack order, into the activation's operands,
   using the normal pop accounting. The caller stays at the call instruction, and the helper
   records the base after consumption.
4. Installs the activation and polls it once within the same mutation.

Failures before operand transfer leave the operands untouched. After transfer the activation is committed:
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

All terminal paths use `CallState::cleanup_native`: stop scheduling, deregister waits, retire
children, disable the bridge, drop the future and control, then release roots and storage.
Executor and dispatch error/exit paths call this shared cleanup. Rust `Drop` never runs managed
code.

### Poll bridge

`NativeControl` holds a nullable erased adapter pointer, an access counter, an entered flag,
a grant counter for the 32-grant budget and a `ControlState` protocol value. `NativeControl::transition`
handles issuance, idle access, effect registration and extraction, generation-matched replies,
poll validation and permanent revocation. Requests and replies are owned by mutually exclusive
states; rejected events preserve the state. It is
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
- `Ready` only in the idle protocol state, with the same exclusion checks.

Anything else, including an access leaked with `mem::forget`, is a terminal internal error that
goes through cleanup. A poll panic unwinds through the reset guard. The activation, now
without its future, is destroyed by the existing host panic policy. This design adds no
`catch_unwind`.

The stop and fairness checks are the same for the first poll and every resumed poll. A
requested yield is never re-polled within the same turn. The grant counter resets at the start
of each turn. After any yield from `vm()`, check the stop and collection conditions again before
granting access; a still-pending condition requests another yield to the executor.

### Roots

`RootedValue` wraps a private `DynamicRoot` to a locally allocated, immutable `Collect` cell
holding a reference value or managed-pointer descriptor. The cell traces the reference or
pointer origin, including cross-arena recording (`dotnet-value/src/object/mod.rs`). A bare foreign
handle stashed directly would bypass that branch. Each newly rooted reference value or descriptor
costs one GC allocation plus a root slot.

A root belongs to its activation's set. Fetching uses gc-arena's set-checked `try_fetch`, and
a mismatch is an internal error. Intrinsic authors get no raw fetch.

Values move between activations (slice 2: arguments, returns, exceptions) only by fetching
from the source set, copying into the destination set and then dropping the source. Never
forward a child's `DynamicRoot` to a parent.

MT tests must root a foreign reference obtained through the plan-06 helper, then collect.

### Managed pointers

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
   - Slice 2 adds finalizer segment isolation while native work is parked.
   - Delete the `NativeSubstringActivation` machinery.
   - No reply state, frame IDs or segment isolation.
   - `substring_selector_preserves_operands_and_selects_only_large_valid_ranges` and
     `substring_dispatch_admits_only_inline_calls` cover selection, operand preservation and
     unsupported dispatch modes. Stop tests cover repeated polls with a stop still pending and a
     real executor collection before access resumes.
2. **Multicast delegates through transparent `call`.** Adds:
   - the activation stack, `NativeId`s and a native return destination on frames;
   - the single-slot request/reply state and reply generations;
   - exception search that respects native boundaries;
   - finalizer segment isolation (parked activations may now wait on managed work);
   - native calls from inside filters and handlers, on plan 07's nested segments;
   - tail calls and `jmp` in children;
   - heap and frame-home byref descriptors for admitted multicast arguments.
3. **Reflection `Invoke` through transparent `call`.** Reuses the byref descriptors and adds
   boxing and write-through. Retains corrected return handling only for the scoped fallback.
4. **`ldsfld` through `ensure_initialized`.** Adds the instruction completion action and the
   MT wait-graph handoff.
5. **`WaitForPendingFinalizers`, then `Monitor.Enter`,** through one condition-wait protocol.

Each slice ends with its consumer on the new path for admitted shapes and its tests enabled.
The four scoped legacy paths are the multicast continuation, reflection return handling, legacy
static-initializer triggers and timed monitor retries. Add facade operations only when a consumer
needs them.

## Done when

**Slice 1**
- Every test in `native_substring_tests.rs` passes against the async path, in both
  configurations. `substring_selector_preserves_operands_and_selects_only_large_valid_ranges`
  and `substring_dispatch_admits_only_inline_calls` cover selection and dispatch.
  `pending_finalizer_runs_in_separate_segment_during_copy` confirms finalizer work runs in an
  isolated segment while the native copy remains parked.
- MT: copy progress survives coordinated stops between chunks, with a foreign source string
  from the plan-06 helper rooted only through the activation.
- Compile-fail tests: a branded value, region, access or view escaping into the future or
  across `.await`. Runtime tests: a forgotten `VmAccess`, an unexpected `Pending`, a
  mismatched caller, a panic during a poll.
- The bridge has run under pinned Tree Borrows Miri, with results recorded.
- Deleted: `NativeSubstringActivation`, `offer_substring_continuation`, `substring_eligible`,
  `unified_dispatch_direct` and the `is_substring` codegen.
- Paired ST and MT release runs for small and multi-chunk Substring and ordinary dispatch.
  Investigate repeatable regressions above 1% on unaffected controls.

**Slices 2–5:** described in each completed note under `08-async-effects/`. The consumer's
fixtures pass, CLR comparisons cover applicable exception or filter ordering, and the listed
bespoke mechanisms are removed except for the four scoped legacy paths above.

## Slice 1 verification

Pinned `nightly-2026-05-27` Miri with Tree Borrows, disabled isolation and leak checking
ran each guarded exact singleton once: seven ST and eight MT, including bridge entry and panic
reset, real-context collection, forgotten access, unregistered pending, changed caller, poll
panic and the MT foreign-source copy. The p44 collection control also passed once in each
configuration. No bridge finding was reported. Dependency integer-to-pointer casts emitted
Miri warnings, limiting provenance detection; this is not strict-provenance evidence. The
commands and guarded selectors are in the `miri-cross-arena` job and `docs/CI.md`. The
final-branch audit reran `effects::tests::panic_resets_entry_and_installation` with the same
pinned Tree Borrows flags: 1/1 passed in each configuration. The known static-reference
alignment and atomic-provenance limitations are tracked in the [plans README](README.md#not-yet-planned);
this bridge control does not cover static reference stores.

### Throughput

Four pairs compared `48f3c454` with `55109bee` on an AMD Ryzen 9 9900X, using
`bench-fat`, separate target directories, copied and SHA-256-verified fixture DLLs, CPU 0,
separate Criterion homes, ten samples, one-second warmup and a four-second measurement target.
Launch order alternated. ST uses `--no-default-features`; MT uses the benchmark crate's default
`multithreading`. [08-evidence.json](08-evidence.json) retains samples, estimates, hashes and
diagnostic runs. Each percentage is the change between the two run mean point estimates, not
a paired confidence interval.

| Configuration / workload | Pair 1 | Pair 2 | Pair 3 | Pair 4 | Mean pair change |
|---|---:|---:|---:|---:|---:|
| ST empty/small Substring | +2.09% | +0.96% | -0.10% | +0.95% | +0.97% |
| ST multi-chunk Substring | -7.15% | -2.02% | -4.20% | -3.96% | -4.33% |
| ST dispatch | +0.12% | +0.04% | -1.24% | -0.66% | -0.43% |
| ST string | +1.23% | +0.29% | +0.98% | +0.12% | +0.65% |
| MT empty/small Substring | +0.77% | +0.06% | +0.38% | +1.81% | +0.75% |
| MT multi-chunk Substring | -1.25% | -3.56% | -1.83% | -1.93% | -2.14% |
| MT dispatch | -1.68% | -1.51% | -2.07% | -1.01% | -1.57% |
| MT string | +1.81% | +2.27% | +3.52% | +1.93% | +2.38% |

The MT string control's repeatable slowdown warranted investigation. Three additional alternating
pairs with `perf stat` reproduced +1.52%, +2.23% and +2.66% user cycles while instructions
fell by 0.05–0.06% and branches by 0.01–0.02%. Cycle samples were concentrated in
`intrinsic_string_is_null_or_white_space`; its hot Unicode loop's instructions were the same,
but the function and loop occupied different addresses modulo 64 in the two binaries. Rebuilding
both sides with `-C llvm-args=-align-all-functions=6` placed that function at the same modulo-64
alignment. Four new alternating MT string pairs then measured +0.72%, +0.39%, -0.07% and
+0.46%. This explains the control regression as sensitive to optimized binary layout rather
than increased executed work; it does not isolate a particular cache or front-end mechanism,
and the diagnostic flag changes alignment throughout the executable, not just that loop. The
primary table remains the unmodified `bench-fat` result. The other unaffected control, dispatch,
showed no repeatable slowdown above 1%. Small Substring uses the changed selector path; its
isolated >1% changes did not repeat across both orders.

## Slice 2 throughput

Four alternating baseline/candidate pairs compared `fb3484dc` with the multicast working tree
on the same AMD Ryzen 9 9900X, pinned to CPU 0. Each used `bench-fat`, separate targets and
Criterion homes, byte-identical copied fixture DLLs, ten samples, one-second warmup and a
four-second measurement target. ST disables default features; MT uses defaults. Raw estimates,
samples, fixture and production-source hashes are in [08-evidence.json](08-evidence.json).
Numbers are changes between mean point estimates, not paired confidence intervals.

| Configuration / workload | Pair 1 | Pair 2 | Pair 3 | Pair 4 | Mean pair change |
|---|---:|---:|---:|---:|---:|
| ST empty/small Substring | +0.17% | -0.33% | -0.50% | +0.57% | -0.02% |
| ST multi-chunk Substring | -1.61% | +3.16% | -3.47% | -0.12% | -0.51% |
| ST dispatch | -0.10% | -1.08% | -0.46% | +0.52% | -0.28% |
| ST string workload | -1.63% | -3.65% | -2.96% | -2.33% | -2.64% |
| MT empty/small Substring | -0.23% | -1.97% | -0.21% | -0.36% | -0.69% |
| MT multi-chunk Substring | +0.28% | +0.39% | +0.90% | -3.23% | -0.41% |
| MT dispatch | +0.76% | -0.74% | -0.27% | -0.29% | -0.14% |
| MT string workload | -7.42% | -7.27% | -7.65% | -6.63% | -7.24% |

No unaffected control shows a repeatable slowdown above 1%. These controls do not measure
multicast throughput; the observed string speedup is not attributed to a particular cause.

## Final-branch throughput audit

Four alternating pairs compared `48f3c454` (main) with `eb55b51d` plus documentation-only
finalizer edits, using `bench-fat`, CPU 0 on an AMD Ryzen 9 9900X, separate targets and Criterion
homes, identical SHA-256-verified fixture DLLs, ten samples, one-second warmup and a four-second
measurement target. The percentages compare run mean point estimates; they are not paired
confidence intervals. [08-final-evidence.json](08-final-evidence.json) retains the raw estimates,
samples, fixture hashes and diagnostics.

| Configuration / workload | Pair 1 | Pair 2 | Pair 3 | Pair 4 | Mean pair change |
|---|---:|---:|---:|---:|---:|
| ST empty/small Substring | +3.37% | +2.40% | +4.17% | -2.50% | +1.86% |
| ST multi-chunk Substring | -3.35% | -4.66% | -3.92% | -3.59% | -3.88% |
| ST dispatch | +3.18% | +1.79% | +2.93% | +3.79% | +2.92% |
| ST string | +1.70% | +2.81% | +1.04% | +1.06% | +1.66% |
| MT empty/small Substring | -0.73% | +1.74% | +0.36% | +0.35% | +0.43% |
| MT multi-chunk Substring | +0.41% | -1.76% | -1.00% | -1.23% | -0.90% |
| MT dispatch | -2.96% | +2.98% | -5.36% | +3.99% | -0.34% |
| MT string | -2.81% | +1.72% | +3.78% | +4.51% | +1.80% |

The ST dispatch control repeatedly slowed; investigation identifies additional work in the
interpreter rather than a Substring effect. Four whole-process `perf stat --test` pairs measured
1.93–1.96% more user instructions and 2.14–2.16% more branches; those counts include metadata
setup. An experimental build that disables the per-instruction pending-reflection check reduced
instructions by 1.53–1.61% and branches by 2.33–2.36% relative to the candidate; four timing
pairs changed dispatch by +0.54%, -2.72%, -3.08% and -2.17%. The ablation cannot be shipped:
it disables the required reflection fallback, and it also changes binary layout and optimization.
The measurements identify a concrete source of added work, not its exact contribution to elapsed
time. The checks remain in place to preserve invocation ordering.

String timing is sensitive to a change affecting optimized binary layout. A diagnostic that only marks that
reflection check unlikely changed ST string timing by -5.65%, -3.45%, -5.78% and -3.79%; the
unchanged whitespace function moved from offset 16 to 0 modulo 64. The hint was not applied.
With all functions aligned to 64 bytes, ST dispatch still slowed +2.04%, +7.56%, +3.67% and
+3.32%, while MT string changed +3.05%, -0.36%, -4.58% and +1.57%. Global alignment affects the
whole binary and does not isolate a cache or front-end mechanism. These diagnostics explain the
extra interpreter work and demonstrate layout sensitivity without claiming an exact decomposition
of the control timing changes. The primary results remain the unmodified builds.

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
