# Async intrinsic API: the body is the implementation

**Status:** Complete
**Outcome:** An async intrinsic can be registered with only its `async fn` body. Each remaining
`select` only captures values before commitment, excludes a named operand shape, or selects a
measured fast path. Each remaining `fallback` serves only such an excluded shape, a measured fast
path, or non-inline dispatch.

## Background

[Plan 08](08-async-effects.md) originally required every async intrinsic to have a synchronous
`fallback`, which for most bodies was a second full implementation, because bodies could not
easily throw and only some IL call sites used inline dispatch. This plan added a throw helper,
made every IL call site dispatch inline, and made `fallback` optional. Later plans removed the
remaining legacy machinery: [plan 16](16-retire-legacy-continuations.md) deleted the reflection
`InvokeReturn` continuation and the monitor retry paths (`monitor_wait_legacy`, `LegacyWait`,
`monitor_retry_deadline`, `monitor_enter_safe`), and
[plan 26](26-thread-lifecycle-and-multicast-fallback.md) deleted the `MulticastState` continuation.

## Design

### Target API

```rust
// Common case: body only.
#[dotnet_intrinsic("static bool System.Threading.Thread::Yield()")]
pub async fn thread_yield(mut fx: VmEffects) -> NativeResult { … }

// Capture before commitment (a deadline that must start before the first VM access can
// wait for a stop). The selector does no validation: invalid values reach the body, which throws.
#[dotnet_intrinsic("static void System.Threading.Thread::Sleep(int)", select = sleep_select)]

// A measured fast path or non-inline caller: the selector may return `Fallback`.
#[dotnet_intrinsic("string System.String::Substring(int, int)", select = substring_select,
                   fallback = intrinsic_string_substring)]
```

Rules:
- `select` is optional. It returns `Async` (with captured values), `Fallback` (only for a named
  excluded shape or a measured fast path) or `Reject`. Selectors do not throw guest exceptions or
  validate arguments, except that a selector may choose a fast path for valid inputs; invalid
  operands go to the body. A selector may return `InvalidProgram` for an operand the activation
  must never transport (a `Transient` `Monitor` `lockTaken` byref).
- `fallback` is optional. A selector that returns `Fallback` for an intrinsic with no fallback is
  an internal error.
- A non-inline dispatch (`NativeDispatchMode::Other`) of an intrinsic with no fallback returns
  `NotImplemented("async intrinsic {method} reached through non-inline dispatch")` without
  consuming operands.

Current registrations: `Thread.Yield`, `Monitor.Enter(object)` and `GC.WaitForPendingFinalizers`
are body-only. `Thread.SpinWait`, `Thread.Sleep`, `Thread.Join(int)`, `Monitor.Wait` and the
other `Monitor` enter overloads (including the byref and timed aliases) have capture selectors
and no fallback. `String.Substring`, `String.IsNullOrWhiteSpace`, the `SpanHelpers` bulk
operations and several reflection intrinsics keep a fallback for their fast paths or non-inline
callers; the source annotations are authoritative.

### Throwing from a body

A `VmAccess` wrapper (`exception`) builds a CoreLib exception the way `throw_by_name_with_message`
does (allocate, set `_message` and `_paramName`, run no constructor), roots it in the activation
and returns `NativeFailure::Managed`. The operands are already committed; the driver discards them
on a throw, matching ECMA-335 §I.12.4.2.5. Bodies validate before registering a wait or acquiring
a monitor, preserving plan 10's validation-before-acquire rule. Read-only queries let bodies read
the `bool&` flag, check monitor ownership and see whether the thread is running a finalizer (so
an in-finalizer `WaitForPendingFinalizers` returns before registering a wait).

### Static `bool&` origins

`RootedValue::stash` admits `Heap`, `FrameHome`, `Static`, `Unmanaged` and (MT)
`CrossArenaObjectRef` pointers under `F6.NativePointerDescriptor`. A `PointerOrigin::Static` holds
no branded pointer, only `Arc<StaticMetadata>` and an offset; every raw access re-resolves the
storage and bounds-checks the offset, so the descriptor stays valid across polls without tracing.
Shared-memory ordering for that byte remains the plans README's "Shared raw-memory access" entry.

### Inline dispatch at IL call sites

Five IL call sites pass `ResolvedMethodDispatchMode::InlineCall`, so their intrinsic targets may
suspend: `constrained. call`, `constrained. callvirt`, single-cast delegate `Invoke`, the no-body
abstract virtual redirect, and `tail.`-prefixed intrinsic calls. Each completes with
`Jump(call_ip + 1)`, as an inline activation does, and was checked against the activation's caller
depth, IP and operand-base checks.

## Substring fast-path measurement

Removing the selector branch that returns `Fallback` for a valid range of at most one chunk
(`valid_length <= SUBSTRING_CHUNK_SIZE` in `substring_select`,
`crates/dotnet-intrinsics-string/src/search.rs`) slowed the empty/small workload in every pair:

| Configuration / workload | Mean change over four pairs |
|---|---:|
| ST empty/small | +31.59% |
| ST large control | +0.78% |
| MT empty/small | +28.41% (above 21% in every pair) |
| MT large control | +0.04% |

The valid-small synchronous branch is therefore kept, and this pattern (a measured synchronous
path for valid small inputs, an async body for larger ones) is sanctioned for bulk intrinsics.
Invalid Substring arguments still reach the body, and the fallback also serves non-inline calls.

Measured with Rust 1.95.0 and .NET SDK 10.0.114, CPU-pinned, alternating launch order, with
byte-identical fixtures on both sides:

```sh
taskset -c 0 cargo bench -p dotnet-benchmarks --bench substring --profile bench-fat --no-default-features -- --sample-size 10 --warm-up-time 1 --measurement-time 4 --noplot
taskset -c 0 cargo bench -p dotnet-benchmarks --bench substring --profile bench-fat --no-default-features --features multithreading -- --sample-size 10 --warm-up-time 1 --measurement-time 4 --noplot
```
