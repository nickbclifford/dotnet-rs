# Async effects hardening

**Status:** Complete
**Outcome:** The async native-effects layer rejects unsupported shapes before committing operands. The
effect-control and static-initialization protocols are explicit state machines whose illegal states
cannot be represented, and its syntax and host contracts are enforced and tested. Its documented
safety claims match the code. The static-field instruction path has a recorded cost, with a fast
path only if that cost is real.

Follow-up to [plan 08](08-async-effects.md) and [plan 09](09-retired-arena-lifetime.md) after a
post-merge review. Plan 08 stays the contract; this plan closes the gaps that review found.

## Problem

The review (read-only audits plus two reproduced defects) found no guard held across an `.await`
and no leaked registration, but it found these gaps. Evidence and anchors are in the kickoff
`REVIEW.md` while the run is active.

- An async intrinsic can only return `bool`, `string`, `object` or void. Any other return type
  fails at completion, after `pop_multiple` consumed its operands (`native_effects.rs`, output
  classification in `start_native_inner`).
- The default `NativeIntrinsicHost::start_native` rejects every call instead of running the
  fallback for non-inline modes (`dotnet-vm-ops/src/effects.rs`).
- `yield_now` and `vm()` skip the one-outstanding-effect checks the other effects perform; some
  error paths drop a future before retiring its child frames.
- The `#[dotnet_intrinsic]` async signature, public selector/fallback, scanner diagnostics and
  nested-`mod` registration are not enforced or tested.
- The compile-fail script skips annotated `compile_fail,EXXXX` blocks and nothing asserts
  `!Send`.
- Every `ldsfld`/`stsfld`/`ldsflda` builds a full activation even for an initialized type, with no
  recorded cost.
- Documentation and registry text overclaim or name things that do not exist; two fixtures lack a
  CLR comparison and no test covers `lockTaken` aliasing.

An independent run then executed what the review had only read. Miri, Loom, the compile-fail script and
stress loops were clean. It reproduced these against the CLR:

- `GC.WaitForPendingFinalizers` called from a finalizer, or from a `.cctor` a finalizer triggers, hangs.
  CoreCLR returns immediately (`FinalizerThread::FinalizerThreadWait` checks `IsCurrentThreadFinalizer`).
- `Monitor` accepts arguments the CLR rejects:
  - `lockTaken == true` acquires the lock instead of throwing `ArgumentException`;
  - a timeout below -1 waits forever instead of throwing `ArgumentOutOfRangeException`;
  - a null object throws `NullReferenceException` instead of `ArgumentNullException`.
- A legacy-triggered `.cctor` (`newobj`, method dispatch, `RunClassConstructor`) left INITIALIZING by a
  worker's host error makes every later initializer wait forever. The `ldsfld` effect path handles the same
  case through `InitializerClaim`.
- 20 of 47 single-guard mutants survive both configurations. They include the late-worker join that
  `2366d44f` fixed, two of the four `cd161956` sites, four one-outstanding-effect checks and the
  teardown cleanup paths.

## Scope

In: the items above, plus the review's smaller verified findings (reflection admission race,
slice 3/4 evidence, `INVENTORY.md`, README queue).

Out: new async consumers, thread-pool scheduling, `Thread.IsBackground`, P/Invoke marshalling,
static-field addresses (p12). Do not widen into a redesign of the effect driver.

Also out, because they belong to other subsystems or need a new mechanism (logged under "Not yet
planned"):
- reflection argument validation and byref write-back after a throw (subsequently addressed by
  [plan 19](19-reflection-invoke-clr-parity.md));
- finalizers running with the interrupted mutator's thread identity;
- exceptions escaping a filter;
- `try/finally` nested in a `finally`;
- `[MethodImpl(Synchronized)]`, `Monitor.Wait`/`Pulse`/`IsEntered` and the other missing intrinsics the run found.

Two pre-existing soundness bugs are fixed separately before this plan runs:
- the single-threaded GC can sweep a queued finalizable object (`collect_debt`);
- a race in Monitor sync-block creation lets two threads hold one lock. Log any
pre-existing CLR gap that is not fixed here under "Not yet planned" in the plans README.

## Design

Prefer the smallest change that makes each contract enforced at the earliest point.

- **State machines over guarded flags.** The review and mutation findings share one shape: protocol
  state is spread across independent fields, and scattered guards reject illegal combinations of them.
  A guard can be deleted with nothing failing, because no test reaches that combination.
  - Make the state one value. Each operation is one transition: match on (state, event), and every
    other pair is an error in that one match.
  - Test the transition table, not individual guards.
  - `WorkerRegistry`'s `Lifecycle` enum is the in-tree model.
  - This applies to the two protocols below. The executor, frame stack and exception state are not
    redesigned here.
- **Effect control.** `NativeControl` (`dotnet-vm-ops/src/effects.rs`) keeps `effect`, `awaiting`,
  `generation`, `mailbox`, `issued` and `disabled` as separate cells. `Mailbox` already says what is
  outstanding, so `awaiting` duplicates it.
  - Replace them with one control-state enum: unissued; idle; effect registered; awaiting a reply of
    generation *g*; reply ready; disabled.
  - Each facade operation (`yield_now`, `vm()`, `call`, `try_call`, `ensure_initialized`,
    `wait_finalizers`, `monitor_enter`, take, reply, post-poll validation) becomes one transition.
  - The one-outstanding-effect rule then holds by construction rather than by per-effect checks.
  - The bridge fields (`adapter`, `entered`, `accesses`, `grants`) stay as they are. They guard
    re-entrancy and borrows within one poll, not the protocol.
- **Static initialization.** `StaticStorage` keeps `init_state: AtomicU8` and `initializing_thread:
  AtomicU64` as two atomics, written by separate stores.
  - A reader can see INITIALIZING with owner 0. `assert_ne!(target, 0, …)` in `statics.rs` assumes it
    cannot (READ, not reproduced).
  - Nothing ties leaving INITIALIZING to the life of the initializing frame. That is why a legacy
    `.cctor` dropped by a worker's host error hangs every later waiter.
  - Encode state and owner in one atomic word, with compare-and-swap transitions and a typed enum view.
    `Initializing` carries its owner, and lock-free fast-path reads stay possible.
  - Every trigger, legacy or effect, takes the same claim guard. Dropping the guard without an explicit
    publish or fail transition moves the type to `Failed`.
  - A synchronization-only claim slot on `StackFrame`, cleared by its existing return, unwind,
    disposal and pool lifecycle, is authorized for legacy ownership. This does not authorize a new
    scheduler, exception-search protocol or frame-indexed executor ledger.

- **Reject before commitment.** Classify every intrinsic return with `multicast_value_shape` and
  reject unsupported shapes before operands are consumed. Choose the output shape from the
  method's delegate-ness, not its name.
- **Host contract.** Make the default `start_native` honor the fallback for non-inline modes, or
  make the method required.
- **Effect discipline.** This is the effect-control state machine above. Separately, retire child
  frames before dropping a future on error paths.
- **Syntax.** Check the async signature (one `VmEffects` parameter, `NativeResult` return) in
  `expand_intrinsic` and diagnose non-public selectors/fallbacks and unregistered nested-`mod`
  annotations in the scanner, with file and function paths in every scanner error.
- **Static fields.** Measure first with the plan 08 paired method. Add a synchronous fast path
  (complete inline when the type is `Initialized` or `Recursive`) only for a repeatable regression
  above 1%; otherwise record the cost in the slice 4 note.
- **CLR compatibility.** Verify the reflection-race behavior against the CLR before changing it.
- **Approved guest-visible changes (maintainer, 2026-11-08):**
  - In-finalizer `WaitForPendingFinalizers` returns immediately, matching the CLR.
  - `Monitor` validates its arguments before acquiring, matching the CLR. The admitted bodies
    now own those checks; retained fallbacks still validate their excluded and non-inline calls.
  - A legacy-triggered `.cctor` dropped without a managed exception marks its type failed, so later
    accesses throw `TypeInitializationException` instead of hanging. The `ldsfld` effect path already
    does this for the same case.
- **Mutation evidence.** Each guard the review or the mutation run names has a test that fails when the
  guard is disabled.

## Slices

Ordered by crate dependency so each step's dependencies are already green: baseline
measurement; `dotnet-vm-ops`; macros and build scripts; reflection; `dotnet-vm`; tests and CI
scripts; documentation and housekeeping. Steps are in `CHECKLIST.md` for the run.

## Done when

- Each item has a test or a recorded measurement; new tests fail when their guard is disabled.
- None of the 20 surviving mutants survives, except a mutant shown to be equivalent, with the reason
  recorded. A guard that the state machines make unrepresentable counts as resolved.
- Effect control and static initialization each have one state value with one transition function,
  and a test enumerates every (state, event) pair.
- The finalizer self-wait, `Monitor` argument and legacy `.cctor` cases have CLR-diffed fixtures or lib
  tests and match the CLR.
- An async intrinsic with an unsupported return type is rejected before operands are consumed.
- The syntax rules above are enforced with tests, and the compile-fail script covers annotated
  blocks and `!Send`.
- `./check.sh --fast` passes in all three feature configs and the pinned Tree Borrows Miri
  selectors in `docs/CI.md` pass for any touched unsafe path.
- Plan 08/09 documents, the invariant registry and `INVENTORY.md` describe the shipped code; the
  plans README queue is current and this plan is marked Complete.

## Ask the maintainer only if

- A fix would change observable guest behavior beyond the approved changes listed under Design.
- A fix needs a new frame-stack or scheduler mechanism.
- A state-machine conversion would need to change the executor, frame stack or exception state, or
  could not keep a lock-free initialized fast path.
- A performance regression above 1% stays unexplained.
- The CLR comparison contradicts a documented decision (for example reflection exception wrapping).
