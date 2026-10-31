# Handoff: design phase for async/await-as-effects in dotnet-rs

> **Historical (design phase complete 2026-10-30).** This is the brief the design was produced
> from. The outcome is [DESIGN.md](DESIGN.md); the implementation plan is
> [docs/plans/02-native-continuations.md](../../docs/plans/02-native-continuations.md).

You are picking up a design investigation in **dotnet-rs**, a Rust implementation of the
ECMA-335 Virtual Execution System (a .NET CIL interpreter). It is built on the `gc-arena`
garbage collector. The project owner is a solo developer. A feasibility prototype already
shows that Rust `async`/`await` can express the VM's GC safe points and other suspending
operations as **effects**, soundly, on **stable** Rust. Your job is to **finish the design**
before anyone implements it in the real VM.

The owner was explicit about the failure mode to avoid: *implementing towards a goal that
ends up looking hideous in context*. So this phase is about **trying several concrete
syntaxes and APIs, writing real intrinsics in each, and choosing on evidence**. Abstract
API sketches alone are not enough. Assume taste matters as much as soundness.

---

## 0. Where things are

- **Worktree:** `/home/nick/Desktop/dotnet-rs-async`. The branch is `explore/async-effects`, cut from `main` at `c01ba637`. Work only here; the main checkout at `/home/nick/Desktop/dotnet-rs` is on another branch and must not be touched.
- **Prototype:** `experiments/async-effects/`. It is a standalone crate with its own `[workspace]`, not a member of the dotnet-rs workspace.
  - `src/lib.rs`: toy interpreter, `Cx`, effects, driver, tests, cost bench.
  - `README.md`: findings summary.
  - `findings.html`: the long-form report the owner read and liked. **Read this first.**
- **Toolchain:** stable `1.95.0`, pinned in `rust-toolchain.toml`. Do not depend on nightly features. Miri is the one nightly use, and it uses the repo's pin, `nightly-2026-05-27`, with `MIRIFLAGS=-Zmiri-tree-borrows`. Strict provenance is known to be infeasible for this codebase, so tree-borrows is the standard.
- **GC:** dotnet-rs pins `gc-arena = "0.6.0"`. 0.7.0 (August 2026) exists and the prototype passes on it unchanged. Design against 0.6, and only note where 0.7 would matter.
- **Status:** nothing is committed yet. All `experiments/async-effects/` files are untracked. **Do not commit, push, or open PRs.** Leave your work in the worktree and report paths; the owner decides what to commit.
- **Commands:**
  - `cd experiments/async-effects && cargo test`
  - `cargo clippy --all-targets`
  - `MIRIFLAGS=-Zmiri-tree-borrows cargo +nightly-2026-05-27 miri test --lib`
  - `cargo test --release --lib cost -- --ignored --nocapture`

---

## 1. Background you need to internalize

### 1.1 Why this was previously rejected (still true, and important)

A July 2026 investigation concluded async was infeasible. Two facts hold today:

1. `gc-arena`'s `Arena::mutate` / `mutate_root` take `for<'gc> FnOnce(&'gc Mutation<'gc>, &'gc Root<'gc>) -> T`, so `'gc`-branded data cannot escape the closure, and collection happens only *between* `mutate` calls.
2. The compiler-generated state machine of an `async fn` cannot `impl Collect`. So a `Gc` held across `.await` would be untraced, then freed, then dangling.

The mistake was concluding "no async at all". The prototype avoids both problems:

- **Futures are `'static`, live outside the GC graph, and never hold a `Gc`.**
- **GC access happens only inside scoped regions:** `cx.with(|mc, engine| …)`, a `for<'gc>` closure. Its return type cannot mention `'gc`, so nothing branded is live at an `.await`. The compiler enforces this; there is a `compile_fail` doctest whose error is "`Value<'gc>` is invariant… lifetime may not live long enough".
- **GC references that must survive an `.await`** are stashed as `gc_arena::DynamicRoot`s (`'static`, traced from the root) and re-fetched inside the next region.
- **Each `.await` performs one effect** (`SafePoint`, `AwaitManagedReturn`) by recording it in a side channel and returning `Pending`. The executor loop is the **effect handler**: it runs a GC, or resumes the interpreter until a managed callee returns into the task.

The owner thinks of GC operations as monadic effects; the closest model is ST-monad regions plus algebraic effects with a single top-level handler. That framing is welcome and should shape naming and docs.

### 1.2 Prior art: piccolo already does this

`kyren/piccolo` is a Lua VM by the gc-arena author. Its master branch has `src/async_callback.rs`, added in 2024 and **not in any crates.io release** (0.3.3 lacks it). Fetch it with
`gh api repos/kyren/piccolo/contents/src/async_callback.rs --jq .content | base64 -d`;
a copy is saved at `experiments/async-effects/reference/piccolo_async_callback.rs`. Read all ~540 lines. Key points:

- `async_sequence(mc, |locals, seq| async move { … }) -> BoxSequence<'gc>`.
- `seq.enter(|ctx, locals, exec, stack| …)` is the region. `Locals` is a **per-sequence `DynamicRootSet`** owned by a `Collect` wrapper, so abandoned sequences and cycles get collected.
- The future is polled **inside** `mutate`. `enter` reaches the context through a lifetime-erased shared slot, using about two `unsafe` transmutes, each with careful SAFETY comments.
- It documents the limitation that awaiting any non-`AsyncSequence` future panics, and it uses a noop waker. `std::task::Waker::noop()` is stable since 1.85, so you don't need its raw-waker vtable.

The prototype polls **outside** `mutate` instead, with the arena in a `RefCell` and **zero `unsafe`**. The tentative recommendation was piccolo-style for dotnet-rs, because a task can then live in the root as a `VmContinuation` variant, an intrinsic can start and poll its task in the same `mutate`, and the executor doesn't have to be restructured. **This is not settled; it is your first major decision** (see §3, D1).

### 1.3 How the real VM works today (verify every claim; line numbers drift)

- **Root and executor.**
  - `GCArena = Arena<GCArenaRoot>` with root `ExecutionEngine<'gc>` (`crates/dotnet-vm/src/dispatch/mod.rs:77`).
  - `Executor::run` (`crates/dotnet-vm/src/executor.rs:~414`) loops. Each iteration does GC debt and collection checks, a multithreading safe-point handshake (`thread_manager.safe_point(..)`, `GCCoordinator`), finalizer processing, then one `arena.mutate_root(|gc, c| c.run(GCHandle::new(..)))`.
  - It accesses the arena via `self.with_arena(..)`, which may involve locking under `feature = "multithreading"`.
  - `GCHandle::new` takes extra arguments under the `multithreading` and `memory-validation` features. Note the `unsafe { c.stack.arena_inner_gc() }` with `F1.GcHandleRooted` SAFETY citations.
- **Dispatch.**
  - `dispatch/mod.rs` runs a batched loop of about 128 instructions, polling `check_gc_safe_point()` every `poll_interval`. **This hot path must not regress.**
  - Before each instruction it records `original_ip` and `original_stack_height` so the instruction can be retried.
- **`StepResult`** (`crates/dotnet-vm-data/src/lib.rs:~123`): `Continue | Jump | FramePushed | Return | MethodThrew | Exception | Yield | Error`. `Yield` is today's `Poll::Pending`.
- **Continuations (Tier 1, commit `399849a1`).**
  - `VmContinuation<'gc>` (`dotnet-vm-data/src/lib.rs:~42`) has the variants `None | RetryInstruction | HandlerUnwinds(Vec<UnwindState<'gc>>)`.
  - `FrameReturnAction::InvokeReturn(RuntimeType)` is on `StackFrame.frame_continuation` (`dotnet-vm-data/src/stack.rs:~49`).
  - `MulticastState<'gc>` is on `StackFrame` (`stack.rs:~32`).
- **The retry protocol.**
  - `yield_spin` / `back_up_ip` are in `crates/dotnet-vm/src/stack/context_ops.rs:~29-47, ~851`. A yielding intrinsic is **re-executed from scratch**.
  - That is why intrinsics **peek** their arguments instead of popping them; see `intrinsic_string_substring` in `crates/dotnet-intrinsics-string/src/search.rs:~60`.
  - It is also why chunked loops (`search.rs:~135, ~259`) lose all progress: they `return StepResult::Yield` mid-loop and restart at offset 0.
- **Intrinsic shape.**
  - `#[dotnet_intrinsic("static void System.GC::WaitForPendingFinalizers()")]` is a proc macro in `crates/dotnet-macros` (+ `dotnet-macros-core`); there is also build-time codegen, see `docs/BUILD_TIME_CODE_GENERATION.md`.
  - Signatures are generic over **trait bundles**, e.g. `fn f<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>>(ctx: &mut T, method: MethodDescription, generics: &GenericLookup) -> StepResult`.
  - The traits live in `crates/dotnet-vm-ops/src/ops.rs` (`TypedStackOps`, `ExceptionOps`, `MemoryOps`, `VesInternals`, …), `crates/dotnet-runtime-memory/src/ops.rs`, and `crates/dotnet-vm/src/stack/ops.rs` (`VesOps`).
  - The concrete context is `VesContext<'_, 'gc>`, built by `ExecutionEngine::ves_context(gc)` / `CallStack::ves_context`.
  - **Intrinsic crates do not depend on `dotnet-vm`.** That crate layering is deliberate, and it is the single biggest constraint on your API (see D3).
- **Representative suspending sites.** About 50 `Yield` / `should_yield` sites in total; grep for `StepResult::Yield`, `yield_spin`, `should_yield`.

  | Site | What it does today |
  |---|---|
  | `dotnet-intrinsics-string/src/{search,operations,constructors}.rs` | chunked string ops |
  | `dotnet-intrinsics-span/src/equality.rs`, `dotnet-intrinsics-unsafe/src/{buffer,mem_helpers}.rs` | chunked memory ops |
  | `dotnet-vm/src/intrinsics/gc.rs:~199` | `GC.WaitForPendingFinalizers`; its own comment says it "effectively spins" |
  | `dotnet-vm/src/statics.rs:~476` | `wait_for_init`, the static-constructor wait; returns a bare `should_yield` bool |
  | `dotnet-intrinsics-threading/src/monitor.rs:~112,149,240` | `MonitorLockResult::Yield` |
  | `dotnet-intrinsics-reflection/src/methods.rs:~375-436`, `types/type_construction.rs:~240` | reflection `Invoke`, via `set_frame_return_action(FrameReturnAction::InvokeReturn(..))` and completion in `dotnet-vm/src/stack/context.rs:~382` |
  | `dotnet-intrinsics-delegates/src/invoke.rs` | multicast delegate invocation via `MulticastState` |
  | `dotnet-intrinsics-reflection/src/types/{type_construction,type_queries}.rs` | other yields |
- **Heap objects.**
  - `ObjectRef<'gc>(pub Option<ObjectHandle<'gc>>)` with `ObjectHandle<'gc> = Gc<'gc, ThreadSafeLock<ObjectInner<'gc>>>` (`crates/dotnet-value/src/object/mod.rs:~255`).
  - Stack values are `StackValue` and include managed pointers (byrefs, interior pointers) with provenance tracking. Read `docs/pointer-provenance/` (untracked, three reviews from 2026-10-27). **Stashing a byref across an await is a hard design question.**
- **Borrow and GC scoping.** `GcScopeGuard`, `active_borrows` (a safe point is refused while borrows are active; `raw_memory_ops_impl/mod.rs:~976`), and `ThreadSafeLock` borrows. Write barriers are in `crates/dotnet-runtime-memory/src/write_barrier.rs`.
- **Feature matrix the design must survive:** `multithreading`, `memory-validation`, `fuzzing` (instruction budget), `bench-instrumentation`. In single-threaded builds, `Condvar::wait` is `unreachable!()` (`dotnet-utils/src/sync.rs`), and `statics.rs` has an explicit unreachable arm.

### 1.4 Docs to read in the repo

Read `docs/ARCHITECTURE.md`, `docs/GC_AND_MEMORY_SAFETY.md`, `docs/EXCEPTION_HANDLING.md`,
`docs/THREADING_AND_SYNCHRONIZATION.md`, `docs/DELEGATES_AND_DISPATCH.md`,
`docs/INVARIANT_REGISTRY.md` and `docs/SAFETY_CITATION_AUDIT.md` (the `F1.*` / `F5.*` SAFETY
citation scheme any new `unsafe` must follow), `docs/BENCHMARK_WORKFLOW.md`, `docs/CI.md`,
`docs/plans/README.md` (the active plan queue; position this work relative to it), and
`CONTRIBUTING.md`.

---

## 2. Your goal and deliverables

Produce a design that a later implementation effort can execute without re-deciding
anything important, and that the owner has *seen* applied to real code.

1. **`experiments/async-effects/DESIGN.md`**, the design document:
   - the chosen API with rationale;
   - every rejected alternative, each with a short code sample and the specific reason it lost;
   - the answers to every decision in §3;
   - the effect catalogue with handler semantics in single-threaded *and* multithreaded builds;
   - the exception model;
   - the stash model, including byrefs;
   - the observability story;
   - performance guardrails;
   - unsafe inventory with SAFETY-citation sketches;
   - risks and open questions you could not close, called out honestly.
2. **A syntax gallery: `experiments/async-effects/gallery/`.**
   - For each of 2–4 candidate API shapes, port the same 6 representative real intrinsics (list in §4), written against that candidate.
   - Put today's code next to each port so they can be compared directly.
   - This is the main deliverable for the "does it look hideous" question.
   - Make the ports as close to compiling as possible. Ideally they compile against a mock of the real layering (next item); where that isn't practical, mark clearly which parts are pseudo-code.
3. **An extended prototype that mirrors the real crate layering.**
   - Restructure or extend `experiments/async-effects` (a small inner workspace is fine) into at least an `ops`-like trait crate, an `intrinsics`-like crate that depends **only** on the ops crate, and a `vm`-like crate that implements the traits and drives tasks.
   - Implement the **chosen** design for real: it compiles on stable 1.95, has tests, is clippy-clean and Miri tree-borrows clean. Keep the `compile_fail` soundness doctests, and add new ones for every escape you claim to prevent.
   - If you pick piccolo-style in-`mutate` polling, the prototype must include that `unsafe` with proper SAFETY reasoning and Miri coverage.
4. **A staged implementation plan**, at the end of `DESIGN.md`.
   - Slice phases **by crate dependency order** (vm-data → vm-ops → intrinsic crates → vm). A previous refactor in this repo slicing by semantic axis caused mass blocked steps, so don't.
   - Make the first slice a minimal vertical spike: port one chunked string intrinsic and add a test that progress survives forced GC-stop requests.
   - Number steps as `N.M` so they can feed the owner's orchestrated refactor workflow later.
5. **A short report back** (your final message): the chosen design in a paragraph, the gallery verdict, the top 3 risks, and file paths.

**Scope boundary:** do not modify anything under `crates/`. Read it freely, and you may build
or run the real workspace to check claims. If a real-VM spike would genuinely settle a question
(e.g. how `GCHandle::new` interacts with in-`mutate` polling under `multithreading`), do it in a
throwaway copy or under `experiments/`, and say so in the report.

---

## 3. Decisions you must make (with evidence)

For each one, write down the options, try the leading ones in code, and record the verdict.

**D1. Poll site.**
- **Inside `mutate`, piccolo-style:** task stored in the GC root, lifetime-erased context slot, `unsafe`.
- **Outside `mutate`, prototype-style:** safe code, but it fights `with_arena` / multithreading locking and costs a `mutate_root` per region.
- Evaluate against the real `Executor::run` structure and the `multithreading` safe-point handshake.
- Also evaluate a hybrid: run the first poll inline at spawn so tasks that complete synchronously never leave the dispatch path.

**D2. Task storage and ownership.**
- Options: a `VmContinuation::Task(..)` variant, a per-thread task stack on `ThreadContext`, or `Collect` wrapper `+` per-task `DynamicRootSet` (piccolo `Locals`) versus one global set.
- Specify what happens to suspended tasks on thread exit, `Aborted`, the fuzz budget running out, and uncaught exceptions.
- Specify how tasks nest with managed frames: a task calls managed code, which calls an intrinsic that starts a task. The prototype handles this with a task stack and `ReturnTo::Task` frames.

**D3. The region API across crate boundaries. This is the hardest problem.**
- Intrinsic crates are generic over `T: SomeOps<'gc>` and can't name `VesContext`. A `'static` future can't hold `&mut T`, and the region must construct a context for a fresh `'gc` each time.
- Candidates to try:
  - a host trait with a GAT, e.g. `trait TaskHost: 'static { type Ctx<'gc>: TypedStackOps<'gc> + ExceptionOps<'gc> + …; fn with<R>(&mut self, f: impl for<'gc> FnOnce(&mut Self::Ctx<'gc>) -> R) -> R; }`, with async intrinsics generic over `H: TaskHost`;
  - a `dyn`-compatible variant (generic methods aren't object-safe; check whether a `&mut dyn FnMut` trampoline works);
  - a concrete type defined in `dotnet-vm-ops`.
- Check monomorphization and compile-time cost.
- Check that `impl for<'gc> FnOnce(&mut Self::Ctx<'gc>)` actually infers well at call sites. HRTB plus GAT closures are notorious for "implementation of FnOnce is not general enough". **If inference forces type annotations at every region, that is a "hideous" verdict and must be recorded.**

**D4. Argument and result convention.**
- Today intrinsics peek args because of retry-from-scratch. With tasks, decide: pop and stash at spawn? Who pushes the return value? Where does the IP advance?
- What does an async intrinsic's signature look like, and its `#[dotnet_intrinsic]` registration (does the macro accept `async fn`; does it generate the spawn shim)?
- Avoid macro magic that hides the soundness rules; the region boundaries should stay visible.

**D5. The exception model.**
- How a task throws: today `ctx.throw_by_name_with_message(..)` returns `StepResult`, so a task probably returns `Result<_, Thrown>` and uses `?`.
- How a managed exception thrown *inside* a `call_managed` callee reaches the task: the unwinder must stop at the task boundary and resume the task with `Err`, and the task chooses whether to catch or propagate.
- How stack traces render a task frame.
- How this interacts with `ExceptionState`, filters, `HandlerUnwinds`, and `original_ip`.
- Read `docs/EXCEPTION_HANDLING.md` closely. This is the biggest *semantic* risk.

**D6. Stashing.**
- Typed handles for `ObjectRef` (including null), strings, arrays, and whole `StackValue`s.
- **Managed pointers / byrefs:** can they be stashed (owner object plus offset?), or are they forbidden across awaits? Use the pointer-provenance reviews.
- The cost model and the API ergonomics of `fetch`. Prefer something that reads like `let s = h.get(cx)` over `roots.fetch(&h)` everywhere.
- Cross-arena references under `multithreading`: can a per-thread `DynamicRootSet` hold them? **If not, say so loudly; it may be a wall.**

**D7. Borrow guards.** Confirm that `ThreadSafeLock` borrows, `GcScopeGuard`, and `active_borrows` cannot be held across an await. They should be `'gc`-tied and therefore confined to a region; prove it with `compile_fail` tests. Also decide whether `active_borrows`' runtime check becomes redundant for task code.

**D8. The effect catalogue.**
- At minimum: `SafePoint`, `CallManaged`, `WaitStaticInit`, `WaitFinalizers`, `MonitorAcquire` (blocking), and possibly `Sleep` / `Join`.
- For each, give the handler semantics in single-threaded and multithreaded builds, deadlock detection (statics have a `WaitGraphEdgeGuard`), and abort or timeout behavior.
- Decide whether effects are a closed enum (prototype) or something more open.
- **Explicit non-goal:** awaiting real external futures (async I/O, .NET `Task` interop). Keep it a non-goal unless you find a clean extension; note it as future work.

**D9. Naming.** "Task" collides with .NET's `System.Threading.Tasks.Task`. Pick vocabulary that won't confuse a .NET-semantics reader: e.g. "native continuation", "effectful intrinsic", "sequence", "suspension". Name the effect-handler concept clearly. Lean into the effects framing in naming and docs.

**D10. Performance guardrails.**
- No change to the batched dispatch loop.
- Measure spawn and region overhead in the extended prototype.
- Describe how the real implementation would be benchmarked (`docs/BENCHMARK_WORKFLOW.md`, `crates/dotnet-benchmarks`).
- State a budget, e.g. "an intrinsic that doesn't suspend must cost ≤ X ns more than today".

**D11. Observability.** Tracer (`crates/dotnet-tracer`), the instruction ring buffer, and debugging a suspended task: what does a dump show?

**D12. Migration path.** Old-style (`StepResult::Yield` + retry) and new-style intrinsics must coexist indefinitely. Define the rule for which intrinsics should migrate and which should stay synchronous.

---

## 4. The syntax gallery: which intrinsics to port

Port these **real** intrinsics into every candidate API, next to their current code:

1. **`String.Substring`** or another chunked loop in `dotnet-intrinsics-string/src/search.rs`: safe points mid-loop, keeping progress.
2. **`GC.WaitForPendingFinalizers`**, `dotnet-vm/src/intrinsics/gc.rs`: waiting on a condition.
3. **`Monitor.Enter`**, `dotnet-intrinsics-threading/src/monitor.rs`: blocking plus multithreading.
4. **Reflection `MethodBase.Invoke`**, `dotnet-intrinsics-reflection/src/methods.rs` plus completion in `dotnet-vm/src/stack/context.rs`: managed re-entry, return-value boxing, by-ref argument marshaling (EF Core work added by-ref Invoke marshaling), exception propagation. **This is the stress test.**
5. **Multicast delegate invoke**, `dotnet-intrinsics-delegates/src/invoke.rs`: a loop of managed calls carrying state.
6. **Static-constructor wait / trigger**, `dotnet-vm/src/statics.rs`: deadlock-aware waiting.

Add a 7th of your choosing if it exposes something the others don't, for example an
intrinsic that holds a byref across a suspension.

For each candidate, write a frank aesthetic review:

- How much noise do regions, stashes and fetches add?
- Do HRTB or GAT annotations leak into intrinsic bodies?
- Does it read like the surrounding code? Match the repo's idioms, comment density and naming.
- Would a reader new to the codebase see where the GC can run?

The owner wants to *look* at this and judge, so make the gallery easy to diff by eye.

---

## 5. Working style and expectations

- **Evidence over assertion.** Every soundness claim needs either a `compile_fail` doctest or a Miri-run test. Every performance claim needs a measurement. Every "the VM does X" claim needs a `file:line` you actually read.
- **Report walls honestly.** If something is truly infeasible (cross-arena stashing, the exception unwinder, HRTB inference), say so plainly and propose the least-bad alternative. Do not paper over it. A design that's 80% async with an explicit, well-justified carve-out beats a forced 100%.
- **Avoid sunk-cost bias toward the prototype.** It was written to prove feasibility, not to be the API, so feel free to discard its shape.
- **Prefer fewer, sharper options.** Two well-built candidates beat four sketches.
- **Code style:** Rust 2024 edition, rustfmt defaults, clippy-clean. SAFETY comments use the repo's citation scheme.
- **Ask before anything outward-facing or irreversible.** No commits, pushes, deletions outside `experiments/async-effects/`, or dependency changes to the real workspace.
- **Final message:** a concise summary per §2.5. The owner reads the report and the files, not your tool output.
