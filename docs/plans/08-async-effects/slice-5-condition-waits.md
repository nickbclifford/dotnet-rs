# Plan 08 slice 5: finalizer and monitor waits

**Status:** Complete
**Outcome:** `GC.WaitForPendingFinalizers` and the admitted `Monitor.Enter`, `ReliableEnter`
and `TryEnter` overloads use VM-owned condition waits in ST and MT without retrying their
intrinsic calls. Finalizer scheduling and monitor ownership retain their existing semantics.

## Current paths and scope

Ordinary inline `GC.WaitForPendingFinalizers` calls use `wait_finalizers`, which checks the
global pending and in-progress finalizer count, including the current arena's queue and processing flag. In `Executor::run`,
pending finalizers are scheduled before an outside-mutation wait; `process_pending_finalizers`
parks the caller in a separate segment while a finalizer runs, and `retire_finalizer_segment`
clears the flag. A call made while `processing_finalizer` is set returns immediately: the selector
uses the synchronous handler for inline dispatch, and the legacy fallback makes the same check for
non-inline calls. Neither path clears the flag. `intrinsic_gc_collect_2` yields after advancing
the IP, independently of the finalizer wait.

Admitted inline monitor calls use `monitor_enter` with an activation-owned deadline.
The synchronous monitor handlers remain fallbacks for excluded byrefs and non-inline dispatch.
Timed fallback calls keep a frame-associated retry deadline. `TryEnter_FastPath` and `Exit`
remain synchronous. In MT, monitor release notifies waiters; ST counts recursion and enters
immediately. Timeout results, recursion and ownership are preserved. `select_monitor` admits only
non-null objects, false `lockTaken` values and integer timeouts of at least -1; invalid arguments
select the synchronous handler before operand commitment. The handlers repeat those checks before
acquisition, then throw `ArgumentException`, `ArgumentOutOfRangeException` or
`ArgumentNullException` as appropriate; a null-object failure leaves `lockTaken` unchanged. The
core-library `ReliableEnterTimeout` and `TryEnter_FastPath_WithTimeout` paths share the timed
handler. The contended-lock fixture initializes its shared gate on the main arena.

The [main plan](../08-async-effects.md) excludes static, transient and unmanaged pointer
transport and tail-prefixed async dispatch. Select unsupported byref shapes before committing
operands and keep their existing legacy behavior; do not turn a managed pointer into an
unmanaged origin. A self-finalizer wait completes synchronously and does not widen the async
admission set. Delete the retry paths for admitted calls, not the behavior required by these
exclusions.

## Frame-home decision

No new frame-home **descriptor representation** is needed for `lockTaken`. `RootedValue::stash`
already converts `PointerOrigin::FrameHome` into a traced `RootCellValue::Pointer` containing
origin, type, offset and pin state. `RootedValue::fetch` resolves the serial and offset afresh
through `NativeBackend::frame_home_block`; the VM adapter searches both the active and parked
finalizer caller stacks. A retired or out-of-range home is rejected, and frame-home serials are
not reused. The same descriptor supports heap owners and, in MT, cross-arena owners. The existing
`rooted_frame_home_descriptor_preserves_aliases_and_rejects_retired_home` test checks descriptor
conversion, aliasing and retirement; it is not a monitor integration test.

The monitor consumer performs a bounded, origin-aware **one-byte write** after the wait: fetch
the descriptor within a fresh VM region and use the origin-aware `RawMemoryOps::write_bytes`
path, including the full access-range check, rather than retaining a branded pointer or observed
address across an await. The descriptor's fetch checks the starting offset, not the width of that write. Preserve
heap and frame-home write-through and the current fallback for excluded origins; enable real
monitor tests for the admitted origins and frame retirement. If a supported call requires a
pointer category outside the main plan's scope, apply its stop condition rather than widening
transport silently.

## Condition-wait design

Use one probe, register, recheck protocol for the two consumers. Condition-wait registrations
belong to the driver and are deregistered on every terminal path; a native future's destructor
does not own them. The legacy initializer path has a separate `InitializerClaim` on its `.cctor`
frame: dropping it only publishes failure and notifies waiters, and runs no managed code. Address
condition-wait replies by activation and effect generation; stale notifications and replies must
not resume another wait. Notifications only mark a registration eligible: the owning executor
rechecks the real condition inside a fresh mutation. Probe again after registration so an
intervening notification cannot be lost. A wake for a monitor means only retry the probe;
acquisition occurs synchronously on that probe, so there is no acquired-but-undelivered state.
Recursion increments exactly once, and cancellation never performs an implicit `Monitor.Exit`.
Once acquisition succeeds, the guest
owns the monitor even if later native work fails.

Wait outside the mutation, with bounded wake intervals and MT GC-stop participation. Use an
`Instant` deadline owned by each activation, not a poll count or thread-local deadline. ST runs
runnable managed or finalizer work before waiting and never manufactures readiness; preserve
existing blocking and deadlock behavior. The driver supports condition waits for monitor and
finalizer effects, and both intrinsic consumers use it for admitted calls. The static
initialization wait-graph handoff described in the [static initialization note](slice-4-static-initialization.md)
also uses driver-owned waiting and the [retained-arena prerequisite](../09-retired-arena-lifetime.md).

## Decision: legacy timeout retries (approved 2026-11-07)

The excluded static, transient and unmanaged byrefs and non-inline dispatch cannot use an
activation under the current admission rules. A timed legacy call can yield for an MT GC stop
before consuming its operands, so recreating its deadline on retry extends the timeout. The
current host-thread-local deadline also lets a nested call reuse or clear another call's deadline.

Use a scoped retry record for each outstanding legacy timed monitor invocation, associated
with its owning managed frame and call site. Create its absolute `Instant` deadline once, retain
it across GC-stop retries and caller-segment parking, and keep nested invocations independent.
The record contains no managed pointers or object addresses; operands remain on the existing
stack and admission remains unchanged. Clear it on success, timeout, exception, cancellation,
frame retirement and frame reuse, including cleanup of suspended segments. A later invocation
at the same call site must get a fresh deadline. The implementation must identify the actual
owner for non-inline calls and ensure a retry cannot consume a different call's record; if the
existing frame/call identity cannot express this, revisit the design before implementation.

Admitted calls continue to use activation-owned deadlines and driver waits. This exception
allows legacy `Yield` retries only for excluded calls and replaces `CURRENT_DEADLINE`; it does
not expand pointer transport or alter monitor ownership. Add real-dispatch deterministic tests
for a static-byref timeout spanning a GC stop, non-inline timeout retry, independent nested
finalizer timeout, and cleanup on terminal paths and frame reuse. Verify the excluded pointer's
write-through and the existing exception and operand behavior. These tests supplement the
full fixture and runtime verification requirements; this decision is not an implementation or
verification claim.

## Done when

- In ST and MT, ordinary `WaitForPendingFinalizers` calls use `wait_finalizers`, and admitted
  `Enter(object)`, `ReliableEnter`/`Enter(object, bool&)` and both timeout `TryEnter` overloads
  use `monitor_enter`. Calls selected for the explicitly excluded legacy cases keep their
  operands and behavior. The migrated calls do not replay after a wait. Remove
  `CURRENT_DEADLINE` and the migrated monitor `Yield` retries. `yield_spin` has been removed;
  the collection and excluded finalizer paths return an equivalent bare yield.
- Probe/register/recheck cannot lose a notification at any interleaving; a notification never
  asserts readiness or grants ownership. Generation-addressed replies and terminal cleanup
  prevent stale wakeups and leaked registrations. Deterministic schedule tests cover notify
  before registration, between registration and recheck, and after parking, plus spurious,
  duplicate and stale wakeups, cancellation and timeout. Test the real executor path, not only
  a mock; use deterministic schedules rather than Loom.
- Waits leave the mutation before blocking, periodically participate in MT GC stops and recheck
  after resumption. Per-activation deadlines survive stops without reset, and independent
  activations do not share them. ST runs available managed/finalizer work before sleeping,
  does not spin or invent readiness, and preserves existing blocking/deadlock semantics.
- Pending and running finalizers finish before an ordinary wait returns, including work scheduled
  while a caller is parked. A wait from within a finalizer returns immediately through both the
  selector and legacy fallback and never clears `processing_finalizer`. Verify cleanup and
  caller-segment restoration on normal and exceptional terminal paths.
- Monitor tests cover uncontended, contended, recursive, successful and expired timeout cases,
  stop while waiting, release/notification races and ownership after success or cancellation.
  `lockTaken` is written once with the correct result through a freshly resolved frame-home or
  heap byref, including collection, aliasing and retired-home rejection. Add a contended
  `Enter(object, bool&)`/C# `lock` fixture with a real CLR comparison; retain the existing
  monitor timeout and cross-arena fixtures. Verify selector/fallback behavior for excluded shapes.
  A CLR-diffed argument-validation fixture covers pre-set `lockTaken`, invalid int and TimeSpan
  timeouts, and null objects across the public Enter/TryEnter overloads.
- Build fixtures and run the finalization and monitor selections in both configurations, then the
  full fixture set once in each configuration as final verification. Confirm discovery and
  expected skips. Run focused deterministic executor tests in both configurations, affected pinned
  Miri controls for pointer
  or unsafe changes, `./check.sh --fast`, `./check.sh --full` and doc drift; update CI selectors
  and their manifest checker when adding enumerated tests. Record native, Miri, doctest and
  performance evidence separately, and grep for the removed symbols and retry paths.

## Verification (2026-11-07)

- Native: the full feature matrix and optional guards passed; the final fast run passed
  775 ST, 923 MT and 938 validation-all tests. The strengthened nested timeout assertion
  passed separately after those runs. The previous full fixture selections passed 258 ST and
  276 MT, including the monitor CLR comparisons.
- Miri: pinned Tree Borrows controls for frame and heap byrefs passed in ST and MT, including
  contended collection in MT. Integer-to-pointer warnings limit provenance evidence. These
  controls do not exercise the native dynamic loader.
- Doctests: vm-ops passed 6/6 in each mode. Doc drift passed 52/52.
- Performance: 20 isolated debug-process runs of the MT contended-lock fixture had a median
  elapsed time of 0.214 s (range 0.214–0.264 s). This includes process startup and fixture
  setup; there is no baseline comparison or throughput claim.

An intermittent nullable-fixture SIGSEGV was traced to an unmapped `System.Native` signal-thread
entry during VM teardown. The loader now retains that library for process lifetime; a native
loader test checks retention and ordinary-library unloading. One hundred isolated MT fixture
runs passed after the change. Other reported SIGTERM failures followed test-run cancellation.
