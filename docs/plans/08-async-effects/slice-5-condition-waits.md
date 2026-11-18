# Plan 08 slice 5: finalizer and monitor waits

**Status:** Complete
**Outcome:** `GC.WaitForPendingFinalizers` and the `Monitor.Enter`, `ReliableEnter` and `TryEnter`
overloads use VM-owned condition waits in ST and MT without retrying their intrinsic calls.
Finalizer scheduling and monitor ownership keep their existing semantics.

## Current design

**Finalizer waits.** Inline `GC.WaitForPendingFinalizers` uses `wait_finalizers`, which checks the
global pending and in-progress finalizer count, including the current arena's queue and
processing flag. A call made while `processing_finalizer` is set returns immediately from the
body without registering a wait or clearing the flag. Non-inline dispatch returns
`NotImplemented`.

**Monitor waits.** Every `Monitor` enter overload (`Enter`, `ReliableEnter`/`Enter(object, bool&)`,
the timed `TryEnter` overloads and the CoreLib `ReliableEnterTimeout` and
`TryEnter_FastPath_WithTimeout` aliases) and `Monitor.Wait` are async bodies with no synchronous
fallback (`crates/dotnet-intrinsics-threading/src/monitor.rs`); non-inline dispatch returns
`NotImplemented`. `TryEnter_FastPath`, `Exit`, `Pulse` and `PulseAll` stay synchronous. The
selectors reject a `Transient` `lockTaken` byref with `InvalidProgramException` before committing
operands; heap, frame-home, static, unmanaged and cross-arena origins are admitted. Bodies validate
`lockTaken`, a null object and the timeout, in that order, before acquisition (`Wait` validates
null, timeout and ownership before release). Each activation owns an absolute `Instant` deadline.

**`lockTaken` write.** `RootedValue::stash` stores the byref as a traced descriptor (origin, type,
offset, pin state); after the wait the body fetches it again in a fresh VM region and performs a
one-byte origin-aware `RawMemoryOps::write_bytes`, which checks the live owner and full range. A
retired frame home is rejected (`rooted_frame_home_descriptor_preserves_aliases_and_rejects_retired_home`).

**Condition-wait protocol.** Probe, register, recheck: registrations belong to the driver and are
removed on every terminal path, not by the future's destructor. Replies are addressed by
activation and effect generation, so stale notifications cannot resume another wait. A
notification only makes a registration eligible; the owning executor rechecks inside a fresh
mutation, and monitor acquisition happens synchronously on that probe, so there is no
acquired-but-undelivered state. Recursion increments once, cancellation never performs an
implicit `Monitor.Exit`, and once acquisition succeeds the guest owns the monitor even if later
native work fails. Waits run outside the mutation in bounded intervals (see
[Safe Points](../../THREADING_AND_SYNCHRONIZATION.md#safe-points)), participate in MT GC stops and
recheck afterwards. ST runs runnable managed or finalizer work before waiting and never
manufactures readiness.

The VM retains the `System.Native` library for the process lifetime: unloading it during teardown
unmapped its signal-thread entry and caused an intermittent SIGSEGV.

## Verification

- Deterministic executor tests cover notification before registration, between registration and
  recheck and after parking; spurious, duplicate and stale wakeups; cancellation; timeouts; and
  stops while waiting.
- Monitor fixtures with CLR comparisons cover contended `lock`, argument validation (pre-set
  `lockTaken`, invalid timeouts, null objects) and timeouts.
- Pinned Tree Borrows Miri controls for frame-home and heap `lockTaken` byrefs pass in ST and MT,
  including contended collection in MT ([CI](../../CI.md#miri)).
