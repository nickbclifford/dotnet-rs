# Async/await VM effects

**Status:** Complete
**Outcome:** `String.Substring`, multicast delegate invocation, reflection `Invoke`, static field
initialization, `GC.WaitForPendingFinalizers` and `Monitor.Enter` run as async native bodies in
both configurations, and the bespoke mechanisms they replaced are deleted.

This plan generalized the Substring continuation of [plan 04](04-native-continuations.md). Later
plans finished the migration: [plan 16](16-retire-legacy-continuations.md) retired the scoped
reflection-return, legacy static-initializer and timed-monitor-retry fallbacks this plan had
kept, and [plan 26](26-thread-lifecycle-and-multicast-fallback.md) removed the remaining multicast
continuation. [Plan 13](13-async-intrinsic-api.md) defines the registration API, and
[plan 19](19-reflection-invoke-clr-parity.md) replaced this plan's transparent reflection calls
with CLR exception wrapping and copy-back. The current architecture summary is
[Native effects](../ARCHITECTURE.md#native-effects).

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
`select = path` and an optional synchronous `fallback = path`; see
[plan 13's target API](13-async-intrinsic-api.md#target-api). The selector runs synchronously
before anything is consumed and returns `Async(invocation)`, `Fallback` or `Reject(VmError)`.

The body pushes its result (nothing for void). The driver checks stack shape and advances the
caller's IP exactly once. Invocation metadata is owned and readable without heap access.

`fx.vm().await?` returns `VmAccess`. Reference-bearing values removed from VM storage become
rooted values; scalars stay inline. Cloning a rooted value shares its root, and dropping the
last clone releases it. Views borrow the access and run bounded synchronous operations.
`vm.with(|region| …)` is a synchronous higher-ranked callback for shared helpers. No branded
value, heap slice, lock or lease can leave the callback. Neither facade exposes recursive
dispatch, parking, retry or IP mutation. `substring_async`
(`crates/dotnet-intrinsics-string/src/search.rs`) is the reference bounded-copy body.

Instruction bodies (static field access) do not use the macro; the instruction handler calls the
same entry helper with an instruction completion action.

## Effects

| Effect | Behavior |
|---|---|
| `vm()` | If a collection or stop is pending, yields to the executor before granting access. After 32 grants in one turn (`VM_ACCESS_QUANTUM`), yields before the 33rd. |
| `yield_now()` | Always returns to the executor once. |
| `call(NativeCall)` | Transparent: returns a `NativeValue`. A managed exception escaping the child propagates without resuming Rust. |
| `try_call(NativeCall)` | Capturing: the child's handlers and cleanup run, then Rust receives `Err(RootedException)`. Outer filters run after the child unwinds. |
| `ensure_initialized(type, generics)` | Runs or waits for a `.cctor`, preserving publication, failure wrapping, recursion and wait-graph cycle rules. |
| `wait_finalizers()`, `monitor_enter`, `monitor_wait`, `sleep_until`, `event_wait`, `join_thread` | VM-owned condition waits (slice 5 and later plans). |

Only VM effects may suspend. Pure Rust loops need explicit `yield_now()`. A body may hold at
most one outstanding effect. Pending without a registered effect is an internal error.

## Design

### Storage and crates

| Crate | Owns |
|---|---|
| vm-data | `NativeId`; `StepResult::NativeSuspended(NativeId)`; the native return destination on a frame |
| vm-ops | `VmEffects`, `VmAccess`, views, `RootedValue`, `NativeValue`, `NativeCall`, effect request types, pinned future wrapper, the object-safe backend trait |
| vm | activation storage, bridge adapter, driver, effect handling, executor integration (`stack/native_effects.rs`) |
| macros, build_support | `async fn` recognition, generated entry symbol and selector wiring |

vm-data does not depend on vm-ops, and `VesContext`, `SharedGlobalState`, sync blocks and heap
managers stay out of vm-ops. The facade calls a narrow object-safe backend trait implemented by
the vm adapter; generic callback methods live on the concrete facade.

`CallState` owns the active `Vec<NativeActivation>` and the suspended native segments. A
`NativeActivation` (`crates/dotnet-vm/src/stack/context.rs`) stores its pending wait and call,
`NativeFutureOwner` (control and future), ambient exception, `DynamicRootSet`, operands and
outputs, optional instruction completion, initializer resume mode, native identity and method,
caller depth/IP and stack height, output shape and constructor-result state. The future and
control are `'static` and cannot hold branded values.

### Entry and operand ownership

The generated synchronous entry calls `NativeIntrinsicHost::start_native`, which:

1. Receives the dispatch mode. Inline `call`/`callvirt` (including constrained and tail calls,
   single-cast delegate forwarding and abstract redirects) admit suspension; other dispatch uses
   the registered fallback, or returns `NotImplemented`, before selecting or consuming operands.
   For an admitted mode it runs the optional selector with a read-only operand view.
2. For `Async`, classifies the declared return before commitment and rejects unsupported shapes
   without consuming operands.
3. Moves the whole receiver/argument window, in stack order, into the activation's operands. The
   caller stays at the call instruction.
4. Installs the activation and polls it once within the same mutation.

Failures before operand transfer leave the operands untouched. After transfer the activation is
committed: every outcome is completion, a managed throw or a host error; replay is impossible.
`vm.pop*()` roots an operand before removing it from traced storage; `vm.push()` appends to
traced outputs. A body must consume all operands before its first `call` effect.

### Managed calls from native bodies

`call` and `try_call` push the child as an ordinary frame whose native return destination
identifies the waiting activation and effect generation. The destination lives on the managed
child `StackFrame`, so it travels with suspended filter segments and is reset on frame reuse.
Reply generations are assigned on an effect's first poll, so a stale reply cannot resume another
effect. Exception search respects native boundaries (see
[Exception Handling — Native boundaries](../EXCEPTION_HANDLING.md#native-boundaries)), native calls
may start inside active filters and handlers, and tail calls and `jmp` work inside children.
Finalizers run in isolated managed segments, so a parked activation's caller, filter and handler
context are never resumed or overwritten by finalizer work.

### Completion

On `Ready(Ok)`, the driver requires no remaining operands and outputs matching the signature
(zero or one, correctly typed). It checks caller depth, IP and operand base, moves the outputs,
applies the completion action once (inline call: `Jump(call_ip + 1)`; synthetic-frame return;
or instruction completion) and destroys the activation.

On a managed throw, the driver discards operands and outputs, moves the exception into traced
exception state while the activation's roots are still live, then starts SEH at the original
call site. A host failure releases the activation and returns the existing `VmError`.

All terminal paths use `CallState::cleanup_native`: stop scheduling, deregister waits, retire
children, disable the bridge, drop the future and control, then release roots and storage.
Rust `Drop` never runs managed code.

### Poll bridge

`NativeControl` holds a nullable erased adapter pointer, an access counter, an entered flag,
a grant counter for the 32-grant budget and a `ControlState` protocol value. `NativeControl::transition`
handles issuance, idle access, effect registration and extraction, generation-matched replies,
poll validation and permanent revocation. Requests and replies are owned by mutually exclusive
states; rejected events preserve the state. It is `!Send`/`!Sync`, and only the driver creates it.

For one poll, the driver:

1. takes the future out of the activation;
2. builds the adapter by split-borrowing the activation's roots, operands and outputs and the
   rest of the engine state (never two `&mut` to the whole engine);
3. installs the adapter pointer with a reset guard that clears it on return and on unwind;
4. polls with a no-op waker;
5. restores the future and validates the result.

Every `VmAccess` operation reaches the adapter through one synchronous higher-ranked function
that reborrows it for the duration of the operation. Nested entry is rejected. The erasure is
confined to this function (`poll_native_impl`, `NativeControl::enter`). Its `SAFETY:` comment states
four premises: the erased and reborrowed types are identical, access is unique (entered flag),
the pointer is valid (install/reset guard), and panic ordering is safe (`F6.NativePollAdapter`).
The maintainer approved this lifetime erasure on the condition that it runs under pinned Tree
Borrows Miri.

When the poll returns, the driver accepts:
- `Pending` only with exactly one registered effect, no live `VmAccess`, a zero access counter
  and no outstanding storage scope;
- `Ready` only in the idle protocol state, with the same exclusion checks.

Anything else, including an access leaked with `mem::forget`, is a terminal internal error that
goes through cleanup. A poll panic unwinds through the reset guard. The activation, now
without its future, is destroyed by the existing host panic policy. There is no `catch_unwind`.

The stop and fairness checks are the same for the first poll and every resumed poll. A
requested yield is never re-polled within the same turn. The grant counter resets at the start
of each turn. After any yield from `vm()`, the stop and collection conditions are checked again
before granting access; a still-pending condition requests another yield to the executor.

### Roots

`RootedValue` wraps a private `DynamicRoot` to a locally allocated, immutable `Collect` cell
holding a reference value, managed-pointer descriptor, or typed reference with its pointer
descriptor and original type descriptor. The cell traces the reference or pointer origin,
including cross-arena recording (`dotnet-value/src/object/mod.rs`); a bare foreign handle stashed
directly would bypass that branch. Each newly rooted reference value or descriptor costs one GC
allocation plus a root slot.

A root belongs to its activation's set. Fetching uses gc-arena's set-checked `try_fetch`, and
a mismatch is an internal error. Intrinsic authors get no raw fetch.

Values move between activations (arguments, returns, exceptions) only by fetching from the
source set, copying into the destination set and then dropping the source. A child's
`DynamicRoot` is never forwarded to a parent (`F6.NativeRootTransfer`).

### Managed pointers

A retained managed pointer is a descriptor, resolved again inside each access: a heap owner
plus offset, a frame-home serial plus offset (frame-home serials are never reused), a static
origin resolved by type and generics on every raw access, an unmanaged address trusted under
`F10.RawMemoryAccessValid`, or (multithreading) a cross-arena origin. Committed static operands
and rooted descriptors carry no cached address. Reconstruction checks the live owner (and, cross-arena,
its lease) before use; a retired frame home or an out-of-range offset is rejected
(`F6.NativePointerDescriptor`). `Transient` byrefs are not transported; selectors reject them
before commitment. `localloc` addresses are unmanaged pointers (ECMA-335 §III.3.47) and carry no
pointer permission.

## Out of scope

External async runtimes; `select`/`join` over concurrent effects on one capability; typed
argument binding for intrinsic bodies. Guest `Task` scheduling and thread-pool work were later
built on CoreLib's own scheduler in [plan 15](15-corelib-task-and-thread-pool.md).

## Verification and accepted costs

- Pinned `nightly-2026-05-27` Tree Borrows Miri runs the bridge controls (bridge entry and panic
  reset, real-context collection, forgotten access, unregistered pending, changed caller, poll
  panic, and the multithreaded foreign-source copy) as exact singletons in the `miri-cross-arena`
  job ([CI](../CI.md#miri)); no bridge finding was reported. Dependency integer-to-pointer casts
  limit provenance detection; this is not strict-provenance evidence.
- Compile-fail tests reject a branded value, region, access or view escaping into the future or
  across `.await`.
- Throughput: slice 1 against main measured ST multi-chunk Substring −4.33%, ST small +0.97% and
  MT `string` +2.38%; the `string` change was binary layout (instructions −0.05 to −0.06%;
  `-C llvm-args=-align-all-functions=6` removed it). The final audit measured ST `dispatch`
  +2.92%; a later layout change and plan 16's removal of the per-instruction reflection check
  changed the dispatch loop since (see "String benchmark is code-placement sensitive" in the
  plans README). No unaffected control showed a repeatable slowdown above 1% at slice 2.

Do not restore the archived prototype (`archive/async-effects-2026-10`) wholesale.
