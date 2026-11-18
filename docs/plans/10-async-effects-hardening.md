# Async effects hardening

**Status:** Complete
**Outcome:** The async native-effects layer rejects unsupported shapes before committing operands;
the effect-control and static-initialization protocols are explicit state machines whose illegal
states cannot be represented; and the authoring syntax and host contracts are enforced and tested.

Follow-up to [plan 08](08-async-effects.md) and [plan 09](09-retired-arena-lifetime.md) after a
post-merge review and an independent run that executed the review's findings against the CLR.

## What changed

- **Effect control is one state value.** `ControlState` (`crates/dotnet-vm-ops/src/effects.rs`)
  replaces separate effect/awaiting/generation/mailbox/issued/disabled cells; every facade
  operation is one transition through `NativeControl::transition`, so the one-outstanding-effect
  rule holds by construction. `control_transition_table` enumerates every (state, event) pair.
  The bridge fields (adapter, entered flag, access and grant counters) are unchanged.
- **Static initialization is one atomic word.** `InitState` encodes state and owner together with
  compare-and-swap transitions and a lock-free initialized read (`crates/dotnet-vm/src/statics.rs`).
  Every trigger takes an `InitializerClaim`; dropping a claim without an explicit publish moves the
  type to `Failed`.
- **Reject before commitment.** Async intrinsic returns are classified with
  `multicast_value_shape` before operands are consumed; the default
  `NativeIntrinsicHost::start_native` honors the fallback for non-inline modes.
- **Syntax.** `expand_intrinsic` (`crates/dotnet-macros/src/lib.rs`) checks the async signature;
  the scanner diagnoses non-public selectors/fallbacks and async handlers in inline modules.
  `scripts/check_vm_effects_compile_fail.py` covers annotated `compile_fail` blocks and `!Send`.
- **Static-field fast path.** The measured cost justified completing `ldsfld`/`stsfld`/`ldsflda`
  inline for initialized and recursive types (see
  [slice 4](08-async-effects/slice-4-static-initialization.md)).

## Approved guest-visible changes

The maintainer approved these to match the CLR:

- `GC.WaitForPendingFinalizers` called from a finalizer (or a `.cctor` it triggers) returns
  immediately instead of hanging.
- `Monitor` validates arguments before acquiring: `lockTaken == true` throws `ArgumentException`,
  a timeout below −1 throws `ArgumentOutOfRangeException`, and a null object throws
  `ArgumentNullException`.
- A `.cctor` abandoned without a managed exception (for example by a worker's host error) marks
  its type failed, so later accesses throw `TypeInitializationException` instead of waiting
  forever.

## Accepted evidence

Mutation testing found 20 of 47 single-guard mutants surviving both configurations before this
plan; each named guard then got a test that fails when it is disabled, or became unrepresentable
under the state machines.

Out of scope here and later handled elsewhere: reflection argument validation and copy-back
([plan 19](19-reflection-invoke-clr-parity.md)), exceptions escaping a filter and nested
`try/finally` ([plan 11](11-exception-control-flow.md)), and `Monitor.Wait`/`Pulse` and
`[MethodImpl(Synchronized)]` ([plans 12](12-managed-threading-intrinsics.md) and
[18](18-managed-api-clr-gaps.md)).
