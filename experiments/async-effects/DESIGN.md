# Native continuations: async/await as VM effects

Design investigation, 2026-10-30. Target: stable Rust 1.95.0, gc-arena **0.6.0**.
No real VM source was changed. This document supersedes the tentative recommendation in
`findings.html`; that report and the original outside-mutate prototype are retained.

**Final choice: `let mut vm = fx.vm().await?`, where `fx: VmEffects` and
`vm: VmAccess<'_>` is an RAII access loan lending rooted views.** Acquisition honors a
pending GC stop before returning the guard. The guard and its views contain no bare Gc,
heap reference or lock; actual GC-branded access remains inside short internal regions.
Poll native futures inside the executor's existing mutation, first inline at spawn, with
a traced per-thread native stack and a separate DynamicRootSet for each activation.
Async expresses VM effects, not an OS async runtime or a .NET Task implementation.

This was the selected API for the original experiment. The
[comparison](gallery/region-api/README.md) retains four compiled variants and the reasons
for choosing RAII. The parked redesign may revisit the mechanism without an approval ceremony;
none of the alternatives is automatically a production entry point.

The API and single-arena mechanism are executable. Foreign-arena liveness and real two-pass
SEH integration remain explicit implementation gates. MT keeps legacy intrinsic paths until
its root-publication/owner-lifetime gate passes. Selecting this syntax does not claim those
semantic integrations are complete, or waive the existing byref eligibility limits.

## Current project scope: parked for redesign

The investigation and measurements below are historical experiment evidence. The original
implementation effort reached an unmerged branch; main does not contain its native runtime
path or benchmark changes. The maintainer is retiring that refactor. [Plan 02](../../docs/plans/02-native-continuations.md)
owns the redesign boundary, while [the queue](../../docs/plans/README.md) prioritizes CI feedback
and Executor ownership. There is no requirement to finish the old checklist before proceeding.

The selected RAII syntax, private poll slot and staged effect catalogue are design candidates
with tested model evidence, not a frozen API or automatic migration roadmap. Preserve the
underlying ownership/rooting/confinement requirements; assess which mechanism earns its cost
for one useful runtime behavior. Later findings about registration, actual ST collection,
acknowledgment, finalizer timing and indirect-call eligibility are summarized in plan 02.

Use [CONTRIBUTING](../../CONTRIBUTING.md#engineering-decisions-and-plan-authority) for routine
adaptation and [CI policy](../../docs/CI.md#test-hierarchy-and-migration) for verification. Supporting
edits and corrections to a historical sketch need no separate approval. Changes to supported
behavior, public compatibility, scope or unresolved safety tradeoffs still require a decision.
Do not run the full experiment or broad advisory fleets after each implementation step.

## Reading and evidence map

- [Offline visual gallery](gallery/index.html), [frank review and model limits](gallery/README.md).
- [Chosen RAII bodies](gallery/region-api/raii.rs), [callback baseline](gallery/concrete/ports.rs),
  [GAT alternative](gallery/gat/ports.rs): all compile with seven matching operations each. [Current source excerpts](gallery/current/).
- [Ops crate](layered/ops/src/lib.rs), [intrinsics crate](layered/intrinsics/src/lib.rs),
  [VM driver and tests](layered/vm/src/lib.rs). Intrinsics depend **only** on ops.
- [Verification runner](evidence/verify.py); command, exit status and elapsed time in
  `evidence/*.log`. [Negative-test diagnostics](evidence/compile-fail-diagnostics.log)
  verify that compile failures are lifetime/borrow errors, not missing APIs.
- [Piccolo reference](reference/piccolo_async_callback.rs), all 540 lines read. Its local
  roots, static futures, scoped context slot and handler interpretation are the precedent;
  its external-future and escaped-capability limitations also apply here.

Source coordinates below are in this worktree, inspected during this investigation; they
are not promises that line numbers stay fixed. The requested three historical provenance
reviews are not present as three files here. The current durable record is
`docs/pointer-provenance/{README,INVENTORY}.md`; it explicitly says several original
review/test artifacts are unavailable. This design does not pretend to have read missing files.

| Fact checked | Evidence |
| --- | --- |
| Root is `ExecutionEngine`; collection traces its stack | `crates/dotnet-vm/src/dispatch/mod.rs:77-90` |
| `with_arena` uses a TLS `RefCell` in MT, a field in ST | `crates/dotnet-vm/src/executor.rs:104-132` (no mutex in this method itself) |
| Abort and fuzz budget precede collection/dispatch | `executor.rs:414-435` |
| STW handshake is outside arena mutation | `executor.rs:483-489` |
| Finalizers precede `mutate_root(... c.run(...))`; GCHandle feature arguments | `executor.rs:493-539` |
| Batch is 128; poll interval choices 32/64/128 | `dispatch/mod.rs:33-62,231-270` |
| Instruction retry snapshots, old continuation clearing | `dispatch/mod.rs:136-138,200-210` |
| Old continuation can hold handler unwinds | `crates/dotnet-vm-data/src/lib.rs:42-54`; `stack/context_ops.rs:29-78` |
| `ThreadContext` is VM-owned, so it can contain ops-owned frames without a data→ops cycle | `crates/dotnet-vm/src/stack/context.rs:533-550` |
| Substring peeks, copies UTF-16 chunks, discards progress on Yield | `crates/dotnet-intrinsics-string/src/search.rs:60-158` |
| Finalizer wait retries; monitor infinite wait result | `crates/dotnet-vm/src/intrinsics/gc.rs:199-211`; `crates/dotnet-intrinsics-threading/src/monitor.rs:112-139` |
| Reflection setup and remote return boxing | `crates/dotnet-intrinsics-reflection/src/methods.rs:375-445`; `stack/context.rs:377-409` |
| Reflection byref ownership already distinguishes box payload and array slot | `methods.rs:826-833,939-1015,1019-1078` |
| Multicast keeps target array/index/args in a dummy frame | `crates/dotnet-intrinsics-delegates/src/invoke.rs:24-146` |
| Static wait uses a scoped wait edge, acquire loads and bounded MT waits | `crates/dotnet-vm/src/statics.rs:476-541` |
| ObjectRef has a special foreign-arena tracing branch | `crates/dotnet-value/src/object/mod.rs:279-324` |
| Scope guard borrows its context, not literally always `'gc` | `crates/dotnet-utils/src/lib.rs:124-160` |
| ThreadSafeLock guards borrow the lock; MT tracks write-guard depth | `crates/dotnet-utils/src/gc/thread_safe_lock.rs:21-54,83-116` |
| Borrow-aware safe-point refusal | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:976` |
| Search currently scans down to frame zero, then clears stacks on unhandled exception | `crates/dotnet-exceptions/src/lib.rs:324-355,458-484` |
| Unwind target currently only Handler/Instruction | `crates/dotnet-vm-data/src/exceptions.rs:61-67`; `dotnet-exceptions/src/lib.rs:545-565` |
| Macro preserves the function and adds a filter; does not generate a spawn shim | `crates/dotnet-macros/src/lib.rs:116-135` |

The architectural constraints were also checked against `docs/ARCHITECTURE.md`,
`GC_AND_MEMORY_SAFETY.md`, `EXCEPTION_HANDLING.md`, `THREADING_AND_SYNCHRONIZATION.md`,
`DELEGATES_AND_DISPATCH.md`, `INVARIANT_REGISTRY.md`, `SAFETY_CITATION_AUDIT.md`,
`BUILD_TIME_CODE_GENERATION.md`, `BENCHMARK_WORKFLOW.md`, `CI.md`, `plans/README.md`, and
`CONTRIBUTING.md`. The active queue contains plan 01, pointer/provenance, still in progress.
This design is queued as [plan 02](../../docs/plans/02-native-continuations.md); it is not a
completion of plan 01 M1e/M1g/M3. The real workspace was not built or benchmarked in this investigation.

## API and dependency boundary (D3, D9)

```rust
// In dotnet-vm-ops; public production surface (abbreviated declarations).
pub struct VmEffects { /* private, thread-affine; not Clone/Send */ }
pub struct VmAccess<'fx> { /* exclusive loan of &'fx mut VmEffects */ }
pub struct VmString<'view, 'fx> { /* rooted view borrowing VmAccess; no Gc or slice */ }
impl VmEffects {
    pub async fn vm(&mut self) -> Result<VmAccess<'_>, NativeFailure>;
    pub async fn safe_point(&mut self) -> Result<(), NativeFailure>;
    pub async fn call_managed(&mut self, call: ManagedCall) -> NativeOutcome;
    pub async fn wait_finalizers(&mut self) -> Result<(), NativeFailure>;
    pub async fn monitor_acquire(&mut self, object: StashedObject,
        deadline: Option<Instant>) -> Result<bool, NativeFailure>;
    pub async fn wait_static_init(&mut self, key: StaticInitKey)
        -> Result<(), NativeFailure>;
}
async fn substring(mut fx: VmEffects, source: StashedString,
                   start: i32, length: Option<i32>) -> NativeOutcome {
    let count = {
        let mut vm = fx.vm().await?;
        vm.string(&source).substring_len(start, length)?
    };
    let mut result = Vec::with_capacity(count);
    let mut offset = 0;
    while offset < count {
        let end = (offset + CHUNK_SIZE).min(count);
        {
            let mut vm = fx.vm().await?;
            vm.string(&source).copy_range(
                start as usize + offset, start as usize + end, &mut result)?;
        }
        offset = end;
    }
    let mut vm = fx.vm().await?;
    Ok(vm.new_string(result))
}
```

The exact executable body is [raii.rs](gallery/region-api/raii.rs), selected as
`effect_intrinsics::chosen`; the prototype's default VM dispatch uses it. There are no
lifetime/type annotations inside its scopes and no per-intrinsic GC-stop polling dance.
`VmAccess::with` is the explicit helper-level escape hatch:

```rust
impl VmAccess<'_> {
    pub fn with<R>(&mut self,
        f: impl for<'gc> FnOnce(&mut Region<'gc, 'gc>) -> R) -> R;
}
```

Only this internal region callback lends branded ObjectRef/Gc values. High-level views
return scalars, owned buffers, stashes, or other proxies tied to the access loan; they have
no Deref to a heap object/slice. `Region` remains a lower-level ops facade for adapters and
shared helpers, not the normal intrinsic-body argument. Reflection's guarded helper
adapters use `VmAccess::with` without changing its marshaling/boxing policies.

The production signatures above give unit/bool outputs to condition effects; the executable
mock uses `Outcome` uniformly to keep its interpreter small. `NativeFailure` is a sum of
managed `Thrown` (rooted exception + preserved-trace policy) and host `VmError`. Abort and
budget are terminal executor outcomes, not catchable managed exceptions. Mock string-valued
`Failure::Managed` is **not** the production exception representation.

`VmEffects` is the intrinsic's capability for scoped heap access, safe points, managed
calls and condition waits; use `fx` for its local binding. The stored computation is a
**native continuation**, its suspension request is an **effect**, and the executor-side
dispatcher is the **native effect handler**. Use those terms in dumps and docs; reserve
Task for .NET Task.
`VmAccess` is the lexical access loan. Its views borrow the guard, and it exclusively
borrows `fx`; drop the guard, or leave its scope, before another VM effect. Internal
regions are the ST-style brand boundary: stashing explicitly lifts a reference out of that
brand into a traced handle. Awaiting acquisition may yield **before** access is granted;
view operations never collect, park, recursively dispatch or await managed code.

### Acquisition, scope and destruction semantics

`vm()` checks the existing borrow-aware stop predicate and the native access quantum.
A pending stop or exhausted quantum performs the existing SafePoint effect before returning
`VmAccess`. With no request and remaining quantum, acquisition is Ready in the current poll. This is a typed
composition of existing effects, not a new handler enum case. A new request may arrive
while a scope runs; bounded operations and later acquisitions/explicit safe points bound
latency. Each native poll starts with 32 access acquisitions. Exhaustion returns control
to the outer executor even if GC is not requested, so its abort and fuzz-budget checks
cannot be starved by a loop of immediately-ready access requests. SafePoint handling must
not optimize this fairness yield into an inline repoll. The acquisition counter lives in
the static control block; it adds nothing to ordinary CIL dispatch. A no-stop test proves
only 32 scopes run before yielding and that abort/budget cancellation prevents the 33rd.
A native pure-Rust loop with no VM access still needs explicit safe points.

Acquisition occurs before validation, each copy chunk, and result allocation in the chosen
string port. Under the mock's permanently asserted stop flag, its three-chunk string test
therefore takes five collections, rather than the callback baseline's two inter-chunk
collections. Each character is still copied exactly once. No extra polling enters the
ordinary 128-instruction dispatch batch.

Guard scope is **not** a single long heap borrow. Each view operation re-fetches the root
and constructs a short region/context, drops actual storage guards/leases there, then
returns. Holding multiple scalar operations in one lexical scope is allowed; it does not
imply fewer context fetches. Keep each operation bounded, and do not expose a persistent
lock, bare reference or raw pointer permission from a proxy. General direct heap lending
across awaits is not part of this design.

`VmAccess::drop` has no VM work: it ends the Rust loan without consulting a poll-local
context. It is safe on return, error, unwind, cancellation and after arena teardown. No
async Drop or deferred cleanup is needed. A ready Rust helper may be awaited while a rooted
view is live; actual external Pending is still rejected by the native handler contract.
Another `fx` effect while the loan/view remains live is a compile error.

Forgetting this access loan leaks no heap borrow or exclusion token: no Gc or lock is
stored in it. This is distinct from forgetting a real GcScopeGuard, whose leaked exclusion
is still detected. An escaped/leaked capability or rooted proxy can only reach the heap
through the installed poll slot; after native completion/teardown it fails the null/mismatch
check before dereferencing heap storage. Arena ownership is not inferred from the Rust
lifetime of `fx`. Actual root publication and arena-generation obligations remain below.

The experiment keeps `borrow_vm`, `enter`, `with_vm`, `vm_ready`, `checkpoint_vm` and
`vm_async` hidden in documentation for comparison/safety tests. The production public entry
is `vm()`; do not ship all those alternatives or restore `vm_guard` as a second name.

### Concrete backend contract

The facade is defined below the VM; it does not name `VesContext`. The VM constructs a
short-lived `NativeRegionAdapter<'a, 'gc>` containing/reborrowing a `VesContext` for each poll.
The backend is a **new, narrow, object-safe trait** in vm-ops, not `dyn VesOps` and not
`dyn ThreadingIntrinsicHost` with its VM-owned SyncBlock associated type.

Use these groups of operations; expose no retry/IP mutation through the region:

| Region capability | Backend contract / existing implementation to forward |
| --- | --- |
| GC allocation and root cells | validated `GCHandle`; Base MemoryOps (`ops.rs:405-470`), string/array allocation and object registration |
| Metadata/boxing | LoaderOps, ResolutionOps, ReflectionOps (`ops.rs:397-404,500-525`); `RuntimeType`, MethodDescription and GenericLookup are owned metadata |
| Reflection lookup/marshaling | move the required `ResolutionContextHost` / registry **declarations** from the reflection crate into vm-ops, retain reexports at old paths; narrow marshaling helpers to these capabilities |
| Static readiness/claim | enum `Ready / Run(method, lookup) / Wait(key) / Failed(exception)`; adapter wraps StaticStorageManager, owns .cctor publication |
| Stop request and borrow tracking | delegate to the existing borrow-aware check and BorrowScopeOps |
| Managed invocation, monitor/finalizer waits | effect requests, not direct backend calls which park or recursively dispatch |

All these data types are already below vm or can be declared in vm-data. In particular, do
not move `SharedGlobalState`, `HeapManager`, or a VM sync-block type into vm-ops. Finalizer
readiness is probed by the handler, so a vm-ops→runtime-memory cycle is unnecessary. Monitor
effects identify the rooted object; sync-block lookup stays in vm. Delegate target lookup
can use lower-level registry operations and crate-local helpers without exposing frame-stack
mutation. `VesInternals` is deliberately absent from the facade.

Where existing helper bounds are broader than their operations (reflection's current
`ReflectionIntrinsicHost` includes `VesInternals`), narrow them rather than implementing
`back_up_ip` or old frame-return actions on `Region`/`VmAccess`. Generic storage closure methods belong
on the internal facade and adapt to object-safe callbacks only if needed; the core MemoryOps and
RawMemoryOps methods actually inspected are already object-safe. Do not claim that every
existing trait bundle is non-object-safe.

Allocation helpers must issue/check tokens against the same owner: a production adapter
obtains its own no-active-borrows token immediately before forwarding an allocation.
Never pass a token issued for `Region` to a `VesContext` that checks token identity. The
prototype uses the real scope guard with region-owned identity and shared backend counters;
it does not model GCHandle's feature arguments. The real executor already builds that handle
with its correct `multithreading` and `memory-validation` arguments; reuse it for polling.

This is one facade implementation per operation, one monomorphic async body per intrinsic,
and a vtable call for backend operations. Most string chunk work uses the fetched object
directly within the region, not one vtable call per character. No HRTB/GAT annotations leak
into either compiled gallery's intrinsic bodies. The mock only implements the listed
operations it exercises; it does not establish every forwarding impl against real VM traits.

## Polling and ownership (D1, D2)

Choose **inside-mutate polling with inline first poll**. The prototype now implements the
slot, per-continuation roots, static pinned future and in-mutate polling. An explicit test
creates and completes a native frame in one `mutate` without inserting it into a root stack.
The miniature VM's convenient `spawn` method queues before `step`; it is not a claim that a
real instruction shim should add that extra mutation.

The production sequence is:

1. A selected intrinsic shim validates eligibility, consumes arguments, stashes them in a
   new native activation and polls immediately in the current mutation.
2. Ready: convert the result under its own roots, push exactly once, drop the activation;
   return the normal successful step result. No executor round trip.
3. Pending with SafePoint/condition: store activation + effect in traced state, return
   native-suspension disposition. This leaves `mutate_root` and `with_arena` before parking.
4. Pending with CallManaged: materialize arguments/recreated marshaled byrefs under the
   current roots, push a managed frame with `ReturnToNative(id)`, retain the waiting parent
   native activation, and dispatch managed instructions normally.
5. Managed return/exception delivery marks the addressed activation runnable. Poll it at the
   next engine-entry/return slow path, outside the batched ordinary-instruction loop.

`SafePoint` hands control to the existing executor collection check/STW handshake. It must
not call `safe_point()` or collect while a `with_arena` TLS borrow, mutation, VesContext,
heap guard or arena lease remains live. To borrow the real root safely, take the ready native activation out of its vector for
one poll, then build the VesContext adapter from the remaining engine fields. Restore the
activation/effect before leaving mutation. Collection cannot occur while that local
wrapper is temporarily outside the root. Alternatively split-borrow the frame vector and
context fields as the prototype does; never manufacture two mutable references to the
whole engine. Native region operations cannot recursively start dispatch or spawn another
native activation; nesting occurs only after the CallManaged handler has regained control.
A poll panic is a terminal VM invariant failure, with ordinary RAII slot cleanup.

The reference implementation installs the slot only
around one `Future::poll`; it is null during all handler waits and collections.

### Storage layout and nesting

Use `ThreadContext.native_frames: Vec<NativeActivation<'gc>>`, with monotonic native IDs
and stable managed frame IDs for linkage. Do **not** add `VmContinuation::Task` as an
exclusive alternative to `HandlerUnwinds`. Old handler unwind state and a native activation
can be live together. `yield_spin` currently changes that enum; putting both mechanisms
in one exclusive variant invites lost state.

Production crate placement avoids a data→ops cycle:

- vm-data: `NativeId`, `NativeFrameHeader<'gc>` (method/IL origin, parent frame identity,
  stack base, roots), managed-frame return destination and native unwind target data.
- vm-ops: `VmEffects`, private shared slot, `NativeFrame<'gc>` wrapper/header + static
  pinned future, effect/request types and the region facade. This is the prototype's
  `NativeFrame` with the header factored into vm-data, not an extra boxed Any object.
- vm: `ThreadContext` and scheduling/handler adapter. It already depends on vm-ops, so
  `Vec<NativeFrame<'gc>>` introduces no reverse dependency. Keep the header/roots traced.

A synthetic native boundary is also present in logical frame order for exception search and
stack dumps; the separate vector stores its Rust state, not a separate exception topology.
Nested order is `managed A → native N1 → managed B → native N2`; N1 cannot run until its
addressed managed child is retired. A native async helper called directly by another native
body shares its capability/root set and is not a new logical boundary.

`DynamicRoot`s are set-specific. Never deliver a child handle directly into a parent's
future: fetch under the source set, stash into the destination set while both sets are live,
then drop the source. In the mock, managed replies live as branded values on managed frames
and `deliver` performs that transfer before a parent poll. Tests collect after transfer and
after source teardown. Reusing the same handle within one native async helper is fine.

### Cancellation and terminal paths

On normal completion or propagation: retire exactly the completed activation after fetching
its result. On host VmError, uncaught managed exception, executor exit, requested abort or
fuzz budget exhaustion: retire all applicable native activations, unregister waits, release
roots, clear mailboxes, then finish existing executor teardown. The fuzz path currently
returns early (`executor.rs:427-433`); route it through that common cleanup as part of the
integration. Do not rely only on `Executor::drop` after `run` returns.

Dropping a future is synchronous Rust destruction, not async cleanup: no region capability
may be used in Drop. Handler-owned registrations and initialized-type publication need
ordinary RAII/explicit terminal cleanup. An abandoned `.cctor` claim is published failed and
waiters notified on every terminal path. No managed finally is executed on host abort unless
the VM's existing abort contract is separately changed; this design does not change it.
The cancellation test proves future destruction and return to baseline GC allocation for
this single-arena model, for both abort and budget.

## Arguments, results, IP and registration (D4, D12)

Keep **explicit synchronous annotated shims**. The current proc macro parses an `ItemFn`
and re-emits it; accepting the tokens `async fn` is not runnable registration support.
Build-time ID dispatch still expects a synchronous StepResult handler. No async macro
extension is needed in the first implementation.

```rust
// Production sketch: not compiled as a real-VM shim in this experiment.
#[dotnet_intrinsic("string System.String::Substring(int, int)")]
fn substring_entry<'gc, T: TypedStackOps<'gc> + NativeSpawnOps<'gc>>(
    ctx: &mut T, method: MethodDescription, generics: &GenericLookup,
) -> StepResult {
    // Eligibility/arity/overload checks happen before commitment.
    ctx.start_native(method, generics, |spawn| {
        let length = spawn.pop_i32();
        let start = spawn.pop_i32();
        let source = spawn.pop_string()?; // null/type error becomes rooted NativeFailure
        spawn.run(move |fx| Box::pin(substring(fx, source, start, Some(length))))
    })
}
```

`NativeSpawnOps` is the generic boundary only the shim needs. `start_native` owns a
transactional argument area and the new root set; `spawn` cannot borrow the ambient `T`
into the returned future. Its builder's result must contain a `NativeFuture + 'static`.
Arguments are popped in stack order via existing pop hooks (trace/frame height accounting),
not retried. Selection and metadata fallibility happen before consuming operands when
legacy fallback is possible. After commitment, errors terminate/throw; never replay a
partially performed native computation.

Record caller frame ID, call-site `original_ip`, stack base, intrinsic identity and success
resume IP (`call_ip + 1`) in the header. While suspended, keep the caller at the call-site;
it is not dispatched while a native child owns that call. On successful completion, push
the result (nothing for void), set the saved caller's resume IP, and return a disposition
that prevents the dispatch loop also incrementing it (`Jump(resume_ip)` for inline ordinary
calls, frame-return machinery for intrinsic methods with synthetic frames). This is a
single **completion helper shared by both paths**, not individual intrinsic bookkeeping.
On throw, leave the caller at `call_ip` so its protected region is searched correctly.
Managed calls issued by a native body return to the native ID, never increment the
suspended caller's IP. Tail-prefixed intrinsic calls stay legacy in the initial rollout, selected before argument
commitment. A managed child's tail-call/`jmp` frame replacement must preserve its
return-to-native destination and logical boundary; add dedicated tests before enabling
native calls in those positions.

Keep `original_stack_height` only for the old retry protocol; native
activation stack bases govern its argument/result cleanup. Do not call `back_up_ip` for a
native suspension, and do not represent it by `RetryInstruction`.

Add a distinct `StepResult::NativeSuspended(NativeId)`/engine disposition in vm-data, so
`Yield` cannot accidentally select old retry logic. `FramePushed` remains usable for managed
re-entry; the native header records the boundary. Old intrinsics and old continuations
coexist indefinitely. Move an intrinsic when it has useful progress to preserve or a
managed-return state machine to simplify. Keep small synchronous intrinsics, atomic fast
paths, single-target delegate dispatch, arbitrary-byref paths and raw FFI synchronous or
on their existing continuation mechanism. No blanket async conversion.

## Exceptions: capturing and transparent boundaries (D5)

A future returns `Result<NativeReturn, NativeFailure>`. `Thrown` contains an owned root
for the actual managed exception and trace-preservation intent, not `StepResult::MethodThrew`
(which is already an extracted display exception). `VmAccess::with` adapters stash any branded managed exception before returning a rooted
`Result`. Typed facade methods follow the same rule. Do not add a second `try_enter` API;
RAII scopes and the helper-level region boundary remain visible.

The crucial semantic choice is that `Propagation::Capture` on a managed call creates an implicit native **catch-all
boundary**, analogous to a native runtime helper catching a managed exception. This is not
a transparent pause of one global exception search:

1. Search managed frames and their catches/filters above the nearest native boundary.
2. If no managed handler matches, select `UnwindTarget::Native(id)` as a synthetic catch.
   Do not search the native caller's parent filters yet, and do not extract/clear stacks.
3. Run finally/fault clauses above the boundary using the existing second pass. A new
   exception thrown by a finally replaces the active exception and searches anew, still
   respecting the nearest boundary. `.cctor` frame retirement still wraps/marks failure
   exactly where `unwind_frame` does today.
4. After all child frames retire, root the exception in the addressed activation, clear
   the child exception state, and reply `Err(Thrown)` to that future. It may handle it,
   transform it, or propagate with `?`.
5. Propagation drops that native activation and begins a new parent search at its saved
   call-site IP, preserving the original exception object's stack trace. A fresh exception
   constructed by native code starts a new trace; explicit propagation does not erase it.

This choice changes filter timing versus a completely transparent call chain: an outer
filter runs after cleanup inside the native catch boundary. That is deliberate and must be
observable in conformance fixtures. There is no workable promise of transparent CLR
first-pass filter timing if arbitrary Rust code can catch/transform the child exception
after unwinding. For an intrinsic which must be transparent, use
`Propagation::Transparent` on its managed call. Search skips this native boundary without resuming Rust. If the selected managed
handler lies outside it, unwind runs child finally/fault clauses, drops the native future
at logical frame retirement, then continues to the outer handler. If a handler lies inside
the child, the native future remains live and later gets the normal return. A transparent
call cannot catch a managed failure in Rust; there is never an `Err(Thrown)` reply for that
call. Host failure still follows the terminal host-error path. This preserves first-pass
outer-filter timing. No user Rust code or region access runs from the dropped future's
cleanup. The compiled reflection, multicast and static-trigger ports select Transparent
to preserve the existing managed propagation paths. Capture is separately tested as an explicit native
catching capability, not imposed on these migrations. The mock proves that transparent
failure does not resume Rust catch code,
but does not prove actual filter ordering. Keep the old managed Invoke path until that
integration test passes.
Reflection's exception-wrapping compatibility also needs fixture evidence; the displayed
port preserves the current VM's propagation policy, rather than introducing a new
TargetInvocationException policy in this refactor. That policy may require a deliberate
reflection-specific Capture/wrap boundary in a separate semantics change.

In `dotnet-exceptions`, insert boundary checks in search *before* the current frame-zero
fallback. Add native targets to unwind-section exit tests and completion, not only the
return helper. Current unhandled handling (`lib.rs:478-484`) clears all frames; letting it
run first would destroy the native caller. Index-based HandlerAddress values remain valid
only for the active logical stack; use stable IDs in native linkage and remap/validate when
frames are suspended/restored. Avoid pretending a side vector alone participates in search.

Filters and handler-unwind continuations require an additional boundary context record:
when a native intrinsic starts during `Filtering` or `ExecutingHandler`, retain the ambient
mode, `original_ip`, stack height and suspended-frame segment on its logical boundary.
Child managed execution gets its own exception context. On success or a caught child
failure restore the ambient context before resuming native/parent code; propagation must
use the existing nested-filter exception policy, which treats an escaped filter exception
as filter failure after its own finally cleanup. `HandlerUnwinds` stays independent. Native
activations and their roots must move with a suspended logical frame segment, and must not
be polled while that segment is suspended.

**Gate:** native managed re-entry from inside an active filter/handler remains on the legacy
path until suspended-segment tests cover it. The provenance record also reports the existing
nested-filter suspended-frame overwrite (F-LIFE-004); this experiment neither repairs nor
assumes it away. The mock's `Failure::Managed` tests prove reply routing/catching and owner
retention, not SEH, filter ordering, trace formatting or real unwind integration.

Render a synthetic native frame using the managed intrinsic name, native ID and originating
managed IL offset. Do not show compiler-generated Rust poll frames in a managed stack trace.
Preserve the throwing callee's trace across native propagation; no repeated native frame
per poll. A caught exception is not an uncaught executor result.

## Stashes, pointers and borrow discipline (D6, D7)

### Values and typed handles

Use private typed wrappers `StashedObject`, `StashedString`, `StashedArray` and `StashedValue`.
Intrinsic bodies use `vm.string(&h)` and array/field view methods; proxies borrow the access
loan and never return a bare Gc. Lower-level `h.get(cx)` rebrands only inside the owning
`VmAccess::with` region, for adapters and shared helpers. Object null is represented inline, not
an allocated fake object. String/array wrappers are validated on creation. View methods validate layout/ranges on each access. Plain scalars and
void are inline; whole values containing references use a local `Gc<RootCell<'gc>>` whose
payload implements Collect. Cloning a handle shares its root-cell/slot; dropping the last
handle frees the slot, and collection reclaims its cell later. Per-continuation sets prevent
global-registry cycles and make teardown explicit.

The mock implements scalar/null/void/object plus string/array wrappers. Production
`StashedValue::try_stash` must exhaustively inspect every StackValue variant, including
reference-bearing value types. Never implement "stash arbitrary StackValue" by serializing
it or accepting integerized managed addresses. Ordinary native integers are scalar values,
not rooted pointer permissions. A later dereference still needs the existing pointer model.
TypedReference, general ManagedPtr, unowned addresses and byref-containing value types are
ineligible for a native suspension unless translated into the specific supported descriptor
below **before commitment**. Return values and outbound call arguments obey the same rule.

General byref-returning delegates remain legacy. In particular, do not discover after a
managed side effect that its return cannot be stashed and then replay the call. Eligibility
includes the declared result layout/signature; dynamically reference-bearing values need
validated layout-driven tracing, not a late surprise fallback.

### Why a local root cell matters for foreign objects

The gc-arena 0.6 source actually inspected is
`~/.cargo/registry/src/*/gc-arena-0.6.0/src/dynamic_roots.rs:53-75`: `stash` invokes a
backward barrier on the adopted Gc directly; root-set slots are Gc pointers. A bare foreign
ObjectHandle bypasses dotnet-rs's ObjectRef foreign-tracing protocol.

Therefore always stash a **locally allocated cell containing ObjectRef/StackValue**, whose
Collect reaches `ObjectRef::trace`. This costs **one GC allocation per newly stashed
reference-bearing value**, in addition to a root slot. The feasibility report's "no
per-root allocation" describes DynamicRootSet slots, not this necessary VM adaptation.
No ad-hoc `Gc::from_ptr` rebranding or cross-arena lifetime extension is introduced.

This repairs the tracing *route*, not foreign-owner liveness. The existing ObjectRef trace
reads the owner ID under an explicitly accepted STW risk boundary; M1e/M1g root publication,
arena exit and generation reuse remain unresolved in the provenance inventory. A local
DynamicRootSet **alone cannot establish safe cross-arena stashing**. An ArenaLease prevents
teardown only during an access, is not an object root, and must end before parking; keeping
one in a future is not a fix.

**Rollout decision:** the new native path is selected only in single-threaded builds at
first. `multithreading` retains the existing intrinsic implementations (including real
monitor behavior). Enable the native path there only after an integrated root-cell test
covers foreign object retention, owner exit, generations/ID reuse and incremental barrier
publication under the accepted GC protocol. This is a real production gate, not claimed
coverage from compiling threaded locks in the experiment. If that protocol cannot retain
a foreign object across a parked waiter, keep foreign-reference-bearing native paths
legacy; do not weaken their liveness contract. gc-arena 0.7 does not solve that wall.

### Supported interior locations

Allow a **symbolic marshaled heap location**, never a saved pointer: rooted owning object,
validated logical field/array element/boxed-payload offset, declared layout, access mode.
Re-fetch and range-check in every region, acquire any storage guard/lease for that access,
and derive the address there. Root owner and metadata outlive the managed callee. This is
not a universal managed-pointer stash facility.

Reflection has exactly this backing already (`InvokeByRefSlot`): value-type refs address a
box (retain existing box identity; allocate zero box for null); reference-type refs address
the original object[] slot (write-through assignment). The native future holds the rooted
parameters array and a call plan. The handler materializes byrefs immediately before
pushing the managed frame, **inside the same mutation**, and the managed stack's existing
Collect/provenance logic keeps them alive while the callee runs. No Branded ManagedPtr is
in the future or the static effect mailbox. Write barriers and layout-sized operations
remain the existing memory layer's responsibility. No deferred copy-out that changes aliasing
or hides writes on exception. Constructor result rooting follows the same rule.

General stack/local/argument homes, localloc, static storage, Transient pointers and native
FFI pointers are **not** supported stash targets. M1c stable localloc blocks do not prove
stack-home relocation safe, and M1d owner fixes do not complete M3/GC transport. In particular
`Monitor.ReliableEnter(object, ref bool)` and arbitrary chunked memory intrinsics stay on
legacy paths; the gallery ports `Monitor.Enter(object)` only. A dedicated future stable
stack-home identity can expand eligibility after the provenance plan, not during this refactor.

### Guards are confined, but runtime checks remain

`ThreadSafeLock` guards borrow their lock; `GcScopeGuard<'ctx>` borrows its issuing context.
They are not all literally parameterized by `'gc`, but when obtained through a region
their owners cannot outlive that region. The original eight negative doctests cover escaping Value,
read guard, scope guard, heap byref, Mutation, write guard, context, and branded input captured into a static future. The lock and scope
types are the real dotnet-utils types, not mock guards. The chosen access API and retained alternatives add seven
negative tests covering awaitable requests and the RAII rooted-view loan. Diagnostic logs show lifetime
errors; read/write guards also report temporary-reference E0515, not just that error.

Keep `active_borrows` and MT write-guard counters. `mem::forget(c.scope())` is safe Rust and
leaks the logical exclusion even though no borrowed value escapes. An executable test
requires the poll to reject it. Poll validates entry/exit borrow depth; effects may not
park while any existing guard/lease remains. The production facade must not export an
unbranded long-lived exclusion token. Raw synchronous code still needs the current checks.
The prototype's public mc field is for soundness experiments; production keeps GCHandle
private and routes allocation through checked facade operations.

## Effect handler contract (D8)

Use a **closed enum** in vm-ops (or request payload data in vm-data). Adding an effect requires
one exhaustive handler arm and feature-specific tests. No type-erased user handlers/plugins.
The only future executor is the VM. `Waker::noop()` is sufficient; external async I/O and
.NET Task/await interop are explicit non-goals. A future returning Pending without one VM
effect is an invariant panic. Awaiting an immediately-ready Rust helper is harmless;
unsupported external suspension is what the runtime check rejects.

Effects carry owned metadata/scalars and stashed operands only. The activation owns the
set that makes those operands meaningful. Mailboxes are single-use and native-ID addressed;
no stale condition notification may complete a later wait. Probe/register/recheck avoids
lost wakeups, and predicates are always checked again after a notification. No managed
exception is synthesized merely because a wait is slow.

| Effect | Single-threaded handler | Multithreaded handler, once enabled | Completion / deadlock / abort |
| --- | --- | --- | --- |
| SafePoint | Exit mutation, run normal debt/full-request checks, finalizer phase, resume | Exit arena access; existing `thread_manager.safe_point` / coordinator protocol; resume after stop lifted | Unit reply; not unconditional full GC. Abort/budget checked before next poll. |
| CallManaged, Capture | Push child with addressed native catch target; dispatch normally | Same, on this executor thread/arena | Root/rehome value or exception; full child unwind before Err. Host errors terminal. |
| CallManaged, Transparent | Push child whose native boundary is skipped in first-pass search | Same | Normal reply only. Escaping exception retires future during managed unwind; no Rust catch. |
| WaitFinalizers | If queue empty and not processing, ready inline; otherwise run finalizer managed frames as children while waiter stays rooted, then recheck | Coordinate with existing finalizer ownership/queue; bounded wait/notification plus GC-stop checks outside arena access | Predicate is today's queue-empty AND !processing. Calling from the active finalizer itself remains legacy until self-wait semantics have a conformance fixture; do not clear processing flag to fake readiness. No public timeout; host abort works. |
| MonitorAcquire(object, absolute deadline) | Current ST SyncBlock enters/reenters immediately, matching `sync/single_threaded.rs:23-69`; never call Condvar::wait | Try acquire under sync-block lock; on contention register waiter, release lock, recheck/park in bounded intervals, service stop requests; reacquire/recheck before owning monitor | Bool for timed enter; Enter requires acquired. Deadline survives all stops; recursion increments once on acquisition. No new monitor-cycle exception; dump owners/waiters for deadlock diagnosis. Cancel unregisters before retiring roots; a completed acquire is committed, never replayed. |
| WaitStaticInit(type+generic key) | Same-thread recursion is Ready; Run triggers managed .cctor; Waiting is invariant-unreachable, as today | Preserve state acquire/release and atomic init-lock→wait-graph cycle check; wait outside mutation, refreshing owner/edge; on cycle use existing Recursive policy | Failed becomes existing TypeInitializationException policy; .cctor success/failure publishes/notifies before waking callers. Owner abort publishes failure. No public timeout; abort removes edge. |

An always-pending safe point in the mock deliberately forces collection for test coverage.
Its condition handlers deterministically complete waits; **they are not evidence** for OS
notification races, self-finalizer waits, monitor fairness, wait-graph cycles or real MT GC.
A production ready predicate can be satisfied in the current mutation and repoll inline,
without parking. SafePoint always returns to the outer executor, including when caused
by the 32-access quantum rather than a GC request; it need not collect. Separately bound
other inline progress to 32 effect completions per engine entry; if exhausted,
preserve the runnable activation and return to the outer executor. Chunked CPU work checks
at the existing chunk boundaries; this is also where abort/fuzz fairness is charged.

A blocked handler owns an explicit `WaitRegistration` with native ID, wait generation,
reason, deadline and object/type identity, not a borrow guard into `VesContext`. It keeps
only owned Arc/state references and the native activation's traced roots. No mutex/lease
is held between bounded 10ms waits. Reuse the existing graph's cycle policy, not a new
unrelated lock graph. Transfer `WaitGraphEdgeGuard` ownership explicitly into an owned
registration (Arc manager + waiter ID); RAII removes the edge on completion, GC-stop handoff,
abort, error and panic. If removed for a GC stop, re-run the atomic cycle-check/insert before
waiting again. Merely refreshing an edge after a stop is insufficient. The current manager
inserts before returning Waiting (`statics.rs:438-445`), so the new handoff must be atomic
with respect to that ownership transfer.

Sleep/Join are **deferred**, not unhandled enum variants. Keep existing implementations.
A future extension would use absolute monotonic deadlines and the same bounded wake/stop
protocol; Join must retain the managed thread identity/lifecycle record, detect self-join
according to the existing runtime policy and stay unsupported where thread creation is
unsupported. No external future is accepted as a shortcut.

## Alternatives tried and rejected

These are API alternatives within this investigation, not all possible coroutine designs.

### Region syntax and access loans (D3, D7, D9)

The final RAII choice is applied to all seven bodies in `gallery/region-api/raii.rs`.
The matching archived variants remain compiled and visible in the four-way gallery.

```rust
let n = fx.with_vm(|cx| read(cx));              // synchronous callback
let n = fx.vm_ready(|cx| read(cx)).await;       // originally trialed as vm(callback)
let n = fx.checkpoint_vm(|cx| Ok(read(cx))).await?;
let n = fx.vm_async(async |cx| read(cx)).await;
```

The first retains callback noise and manual stop checks. The second adds await syntax
without a checkpoint. The third is sound and useful but leaves closure/Ok wrappers in
each intrinsic. The async-closure form must reject actual Pending while its untraced future
holds GC data; that runtime restriction makes it a poor default for VM effects. The chosen
RAII access loan retains the confined proof inside view methods, while giving bodies
ordinary method calls and lexical scope exit. `vm_guard` was its trial name and is removed.
`gc` was rejected as a name because VM access is not a collection request.

```rust
let mut vm = fx.vm().await?;
let mut source = vm.string(&source_root);
// fx.safe_point().await?; // rejected if this loan/view is still live
let count = source.substring_len(0, None)?;
```

A hypothetical `let heap = fx.raw_heap().await?` returning arbitrary Gc/slices was not
established: a borrow of `fx` does not by itself establish the owning arena's lifetime,
particularly for leaked/forgotten guards and native completion/teardown. The chosen rooted
views hold no such borrowed heap state. They are not a claim that all raw-borrowing RAII
architectures are impossible; that different ownership/exclusion proof is outside this design.

### Outside mutation, all safe (D1)

```rust
fn enter<R>(&self, f: impl for<'gc> FnOnce(&mut Engine<'gc>) -> R) -> R {
    self.arena.borrow_mut().mutate_root(|_, e| f(e))
}
```

This is the retained original `src/lib.rs`, with tests and benchmarks. It is a valid,
safe mechanism. It loses on integration: intrinsic crates still need an ops abstraction,
inline dispatch already holds arena access, and each region must reacquire `with_arena`
and construct a fresh GCHandle/context. Re-entering TLS while a mutation is live panics;
moving all polls outward requires restructuring return/finalizer/retry scheduling. This is
an architectural judgment, not a measured claim that mutate_root is slow. Its toy benchmark
is actually faster than the new wrapper's spawn benchmark; the two include different work.

### In-mutate only after returning to executor

```rust
ctx.queue_native(future);
return StepResult::NativeSuspended(id); // even an immediately-ready operation leaves dispatch
```

Mechanically sound, but loses the synchronous completion path. Choose inline first poll,
then the same root-owned frame after Pending. `inline_first_poll_and_dyn_trampoline` verifies
completion in one mutation. Do not poll a future on every ordinary instruction.

### GAT host (D3)

```rust
trait NativeHost: 'static {
    type Ctx<'gc>: RegionAccess<'gc>;
    fn enter<R>(&mut self, f: impl for<'gc> FnOnce(&mut Self::Ctx<'gc>) -> R) -> R;
    fn perform(&mut self, effect: Effect) -> impl Future<Output = Outcome>;
}
async fn substring<H: NativeHost>(mut fx: H, source: StringRoot, ...) -> Outcome { /* gallery */ }
```

The working candidate compiles/runs all gallery operations. It loses because there is one
real VM context, while every host instantiation generates another async state machine and
the host abstraction leaks into every signature. This is a code-generation consequence,
not a measured repository compile-time slowdown. It could be chosen if multiple genuine
hosts emerge; it is not "hideous" in its bodies.

The first two-lifetime GAT had `Ctx<'gc, 'a>: RegionAccess<'gc> where 'gc: 'a` and
`for<'gc, 'a> FnOnce(&mut Self::Ctx<'gc, 'a>)`. Its impl hit rustc's higher-ranked bound
limitation (#100013). Saved source: `gallery/gat/rejected/two_lifetimes.rs.txt`. The one-brand
poll facade repairs it centrally. A separate function-item generality failure occurs at
`Arena::new(DynamicRootSet::new)`; explicit backend closure solves it. Neither candidate
requires annotating every intrinsic's closure parameter.

### Fully dynamic host trampoline (D3)

```rust
trait DynRegionHost {
    fn enter_dyn(&mut self, f: &mut dyn for<'gc> FnMut(&mut Region<'gc, 'gc>));
}
fn enter_erased<R>(host: &mut dyn DynRegionHost,
                  f: impl for<'gc> FnOnce(&mut Region<'gc, 'gc>) -> R) -> R {
    let mut f = Some(f);
    let mut out = None;
    host.enter_dyn(&mut |c| out = Some(f.take().unwrap()(c)));
    out.unwrap()
}
```

This exact shape is implemented and tested. FnMut trampoline + Option-held FnOnce works.
A fully erased `perform` would additionally return `Pin<Box<dyn Future<Output=Outcome> + '_>>`
or expose a poll primitive; native async trait methods themselves are not dyn-compatible.
It loses because it adds another callback/return slot layer (and potentially per-effect
boxing) without removing the need for a concrete facade/backend somewhere. Keep vtable
erasure at backend operations, not at every region invocation. Generic methods on
`dyn NativeHost` directly are not an option; its GAT/generic enter are not dyn-compatible.

### Exclusive continuation enum and global roots (D2)

```rust
enum VmContinuation<'gc> { None, RetryInstruction, HandlerUnwinds(...), Native(...) }
// OR: all futures stash into engine.global_roots
```

The exclusive variant cannot naturally hold an outer native activation and a nested handler
unwind simultaneously. A global registry prevents tying root lifetime to an abandoned
native computation and risks self-rooting cycles. The per-thread stack plus per-native roots
keeps logical nesting explicit. A Collect wrapper alone without frame/return linkage is
also insufficient; it tells GC what to trace, not which future a managed return should resume.

### Branded async context / automatic wrapping macro (D3, D4)

```rust
async fn substring<'gc, T: StringIntrinsicHost<'gc>>(ctx: &mut T) { /* await */ }
#[dotnet_intrinsic(...)] async fn substring(fx: VmEffects) { /* implicit args/spawn */ }
```

The first is ruled out by static-future/brand confinement tests; it does not become sound
by boxing. The second could be implemented, but hides where arguments become roots and
where instruction commitment occurs. Keep the annotated explicit shim; no macro-generated
soundness boundary. A later boilerplate macro may emit only that reviewed shim and must
expand visibly, not make regions implicit.

### Whole StackValue or pointer addresses as stashes (D6)

```rust
let saved = fx.enter(|c| c.pop_managed_ptr().as_ptr() as usize);
fx.safe_point().await?;
// reconstructing a pointer here has no owner/root/range proof
```

An integer is not lifetime-branded, so the compiler cannot reject carrying it. It also does
not authorize a later unsafe dereference. No API accepts it as a rooted pointer. General
managed-pointer transport and raw serialization lose owners/provenance and are excluded.
A heap-owner/index descriptor is accepted only with the checked managed marshaling protocol.

## Performance and observability (D10, D11)

The dispatch batch constant, ordinary instruction handlers, safe-point poll interval and
ring-buffer push path stay unchanged. Native polls occur only at spawn, engine entry with
a runnable native activation, or managed return/unwind slow paths. Existing no-suspension
intrinsics stay synchronous and incur **zero added poll/region/allocation work**.

On this host (AMD Ryzen 9 9900X, x86_64, rustc 1.95.0), three serial release samples in
`evidence/cost-{1,2,3}.log` measured:

| Operation | Observed range |
| --- | --- |
| black_box baseline loop | 0.184–0.232 ns/op |
| Internal in-mutate region (trivial predicate) | 1.085–1.382 ns/op |
| Chosen vm() acquisition + scalar read | 6.331–8.192 ns/op |
| Chosen vm() acquisition + rooted string read | 7.780–8.784 ns/op |
| Spawn + synchronous completion (102,400 operations) | 51.385–66.014 ns/op |
| Spawn + one suspension/reply + second poll | 74.308–94.572 ns/op |
| Original outside-mutate toy region | 0.5–0.7 ns/op |
| Original toy spawn/suspend, different allocation/root setup | 11.0–14.0 ns/op |

These are elapsed-time microbenchmarks, not calibrated cycle counts or confidence intervals.
The cost tests run with `--test-threads=1`; the first sample also shows CPU warmup variance.
Chosen access loops include their 32-acquisition poll/reply fairness round trips; the
microdriver acknowledges SafePoint directly without collecting. Both acquisitions and view
operations are measured through the tiny mock host, not a real GCHandle/VesContext. There
is no implication that a whole RAII scope caches a branded context or retains a heap lock.
Scalar spawn includes Rc/future/root-set allocation and destruction, but GC is run **outside** the
measured batches. Stashing a reference adds a root-cell GC allocation not exercised by the
scalar spawn benchmark. The internal-region measurement is for a tiny mock adapter; GCHandle, real
VesContext construction, tracing and locking costs are absent. Neither implementation's
numbers establish the other's real-VM cost. Do not advertise a speedup from in-mutate polling.
The first report's near-free spawn number must not become a production budget by accident.

Acceptance budgets (targets, not measured achievements):

- No new allocation/poll on a legacy synchronous intrinsic or ordinary CIL dispatch.
- Selected native intrinsic completing synchronously: **≤100 ns added** over its old shim
  on this machine, excluding intrinsic work; otherwise keep/expand its direct fast path.
- Region transition through the real adapter: **≤10 ns added**, measured separately from
  requested heap work; no per-character dynamic dispatch.
- Existing Criterion dispatch throughput: no repeatable >1% regression across paired
  baseline/change runs; investigate confidence intervals before claiming a regression.
- Forced GC-stop string test must show each UTF-16 element copied once, progress survives
  every request, bounded stop latency, and no retry allocation explosion.
- Account future boxes, Rc, root cells, owned temporary buffers and peak retained roots
  in GC/external-allocation metrics; drop large owned buffers/obsolete handles promptly.

Use the real `docs/BENCHMARK_WORKFLOW.md` commands in the implementation worktree:
`cargo bench --profile bench-fat -p dotnet-benchmarks --bench end_to_end -- --sample-size 10`,
then focused dispatch, long substring, JSON and EF InMemory workloads, paired ST/MT builds.
Measure guard acquisition and each rooted-view operation separately: the lexical loan is
not a single cached GC region.
Use `scripts/profile_perf.sh bench --name json --sample-size 30` for a steady-state trace,
not a one-shot fixture. Keep the profiler manifest and compiler/CPU/build configuration.
Run before/after retained-memory tests and benchmark tracing disabled/enabled separately.
The experiment does not measure real-workspace compile-time changes. The separate
`evidence/compare_compile.py` warms the same ops dependencies, then builds an example
instantiating the same six futures on VmEffects for each candidate, three times with fresh
intrinsic artifacts. See `evidence/compile-cost.log` for exact times and executable sizes.
The historical callback/GAT measurement favored GAT (0.177–0.184s / 486,224 bytes versus concrete
0.226–0.227s / 490,592 bytes); there is no empirical compile-time reason here to reject it. This small sequential micro-build does not predict
workspace rebuilds, and includes linker/driver overhead. Generic multiple-host expansion
remains a scalability cost model. Those figures compare the earlier callback facade, not
the chosen RAII implementation; the choice of concrete is API simplicity for one VM host.

### Native state in diagnostics

`dotnet-tracer/src/lib.rs:35-73` has structured events and bounded backpressure;
`dispatch/ring_buffer.rs:4-14` stores cheap `(IP, static opcode-name)` entries. Add native
spawn/suspend/resume/return/throw/cancel events via guarded emitters, formatting only when
enabled. Include thread/native/parent IDs, managed method identity, call-site IL, effect,
wait generation and deadline/owner identity. Events may be dropped under existing
backpressure policy; state dumps must be sufficient without the event history.

Keep instruction history as CIL history. Add a separate bounded native-transition ring,
updated only at effect/activation boundaries, and dump it beside the instruction ring.
Never invent an executed IL instruction for each future poll. A dump should read like:

```text
thread 7  managed M::Caller IL_002a
  native #18 System.String::Substring  at Caller IL_002a
    suspended: SafePoint  poll=3  since=...  roots=1
    progress: copied=131072/200000
```

Use explicit optional progress counters/labels owned by the native header; Rust async state
layout is opaque and is never introspected. Strings/objects are described through their
normal checked region access in a dump, never by blindly dereferencing saved addresses.
Record poll/effect counts, duration and maximum live roots under bench-instrumentation.
Fuzzing charges each native poll/effect and existing executor quantum; native loops must
reach bounded chunk/effect boundaries so a budget/abort cannot be starved. Dump all stacked
and suspended native activations, including those hidden by filter-frame suspension.

## Unsafe inventory and safety claims

The original outside-mutate toy remains zero-unsafe. The new ops bridge contains **three
unsafe blocks**: pointer lifetime erasure on slot installation, inverse lifetime erasure on
entry, and the unique-pointer dereference. No pin projection unsafe is needed: future is
already `Pin<Box<dyn Future>>`. The RAII layer adds no unsafe blocks; it reuses the slot. The
static access-quantum counter contains no GC reference. No raw waker implementation or manual unsafe Collect is
added; Collect derives trace the per-frame root set and require static future/shared fields.

| Site | Required proof and registry citations | Executed falsifier |
| --- | --- | --- |
| Slot::install cast | `F6.NoEscapeAcrossArena`: same representation/size, original mutation and adapter live for one poll, pointer inaccessible after RAII reset even on panic | Miri inline/nested polls, panic reset test; co-located const size assertion |
| Slot::enter inverse cast | `F6.NoEscapeAcrossArena`, `F1.GcHandleRooted`: HRTB provides a fresh confined brand, original roots match, no fabricated static reference, pointer taken out of slot before user callback | lifetime negative tests, Miri re-entry rejection and transfers |
| Slot::enter dereference | unique live pointer, dynamic exclusion of nested entry, no GC while callback runs; `F8.NoSafepointWhileHeapBorrowed` plus `F1.GcHandleRooted` | Miri recursive-enter/panic tests, real lock guards and forgotten-scope rejection |
| Root wrapper tracing (derive) | `F5.TracesEveryGcRef`: future/shared are static; root set and any branded reply/header traced; no GC reference inside ignored static fields | forced collection, root transfer, cancellation allocation-baseline tests |
| Existing GCHandle construction (unchanged) | retain `F1.GcHandleRooted` citation at executor's arena-inner-GC access; no new lifetime extension there | future real ST/MT unsafe-gate suites, not claimed here |

`Slot::enter` is higher-ranked over a one-brand Region facade; the cast does not assert that
an actual Mutation lives forever. It chooses a brand confined to the callback. The raw
pointer represents a uniquely borrowed real Region whose two actual lifetimes may differ;
only lifetime parameters change, no field/layout. Slot removal prevents a second mutable
reference even if another capability were to reach the slot. Reset/restore guards run in
reverse order on unwind. Rc keeps slot allocation alive if the capability escapes; methods
then panic on null rather than dereference freed stack storage. Future polling is serialized
and thread-affine. Do not add unsafe Send/Sync, reentrant polling or concurrent slot access.

Production should add a precise registry predicate for poll-slot exclusivity if the existing
F6/F8 descriptions prove too broad; add the row with the implementation as CONTRIBUTING
requires. This design does not edit the authoritative registry for experimental code.
Layout erasures carry the existing citations and co-located size assertion already.

Miri is bounded dynamic evidence, not a soundness proof. Current evidence covers this bridge,
local GC and actual utility guard backends with/without their MT/validation features. It
covers neither real foreign-arena transport nor SEH nor coordinated OS-thread parking.
Strict provenance is deliberately not run; the repo's Miri pin and Tree Borrows policy are
used. gc-arena 0.7 remains orthogonal: the handoff says the original prototype passes it,
but **this extension was only tested against 0.6.0**. Do not change the real dependency.
The local-root-cell tracing requirement and foreign-liveness gate remain under 0.7.

## Remaining risks and hard gates

1. **Foreign arena lifetime / root publication.** Local cells preserve ObjectRef's tracing
   route, but do not settle M1e/M1g. MT uses legacy registrations until the ownership gate
   passes. No fabricated cross-arena brand or permanently held ArenaLease is acceptable.
2. **Real SEH, filters and suspended frames.** Both propagation modes have explicit semantics
   and the mock checks routing, including no Rust resumption on transparent throw. Real
   two-pass search/unwind, nested filters, ambient handler restoration, cctor wrapping and
   native frame recycling still require integration evidence. F-LIFE-004 is pre-existing.
3. **Cost and eligibility complexity.** Root-cell allocation, boxing, broad helper traits,
   byref-containing values and fast-path regression can erode the benefit. Keep the selected
   surface narrow, typed rooted views, explicit shims and legacy fallback before commitment. The two compiled
   candidates settle syntax/inference, not every real helper's forwarding implementation.

No unverified universal soundness or speed claim is part of this recommendation. The design
was the basis for the retired vertical spike; prototype evidence does not authorize a wholesale
migration or require restarting that implementation. The affected safety/behavior contracts require relevant tests under the current CI hierarchy;
future-stage gates apply when that stage is selected, not as Substring prerequisites. Choose any redesigned API for a demonstrated runtime need; do not expand scope merely to
complete the experimental catalogue.

## Staged implementation plan (D12)

The staged plan (stages 1–4, steps `N.M`) now lives in
[docs/plans/02-native-continuations.md](../../docs/plans/02-native-continuations.md), which owns
scope, dependencies and status. This document remains the design record it cites.
