# async/await as VM effects — design investigation (2026-10-30)

The current recommendation and its gates are in [DESIGN.md](DESIGN.md). Open
[gallery/index.html](gallery/index.html) for the side-by-side syntax comparison; see
[gallery/README.md](gallery/README.md) for the candid review and mock limitations.

**Final API: `let mut vm = fx.vm().await?`.** `VmEffects` is the effect capability;
`VmAccess` is an RAII loan lending rooted views. Acquisition honors a pending GC stop or
an exhausted 32-access native quantum;
actual heap references/locks remain in short internal regions. The [access comparison](gallery/region-api/README.md)
records the rejected callback alternatives and the chosen safety contract.

The standalone workspace contains the original safe outside-mutate experiment plus
`layered/{ops,intrinsics,vm}`. The chosen extension polls inside mutate, uses per-native
traced roots and exports `effect_intrinsics::chosen`. The default prototype VM uses the
chosen RAII bodies. Intrinsics depend only on ops. Production MT enablement, general byref
support and real SEH integration remain explicit gates.

From this directory, `cargo test`, `cargo clippy --all-targets` and the original Miri/cost
commands now include all four default workspace members. For the full recorded matrix:

```bash
python3 evidence/verify.py
python3 evidence/compare_compile.py
python3 gallery/build_gallery.py
```

[Evidence summary](evidence/README.md) describes coverage and limits. gc-arena 0.7 testing
below applies to the original toy only; the layered extension pins and tests 0.6.0.
No source under `crates/` was modified; the experiment uses the real dotnet-utils lock/scope
implementations through a path dependency.

---

# async/await as VM effects — feasibility prototype (2026-10-30)

Standalone crate, not a workspace member. Stable 1.95, gc-arena 0.6.0 (also passes
unchanged on 0.7.0). `cargo test`; Miri: `MIRIFLAGS=-Zmiri-tree-borrows cargo +nightly-2026-05-27 miri test --lib`.

## Verdict

**Feasible on stable.** The 2026-07-03 walls are real, but they only rule out one
design: a future that *holds `Gc<'gc>` across `.await`*. Under a different design,
`.await` points are effects and the executor is their handler:

| Piece | Role |
|---|---|
| `'static` future, outside the arena | the effectful computation; never holds a `Gc` |
| `cx.with(\|mc, engine\| …)` (`for<'gc>` closure → `mutate_root`) | a GC *region*. Nothing `'gc`-branded can be returned, so nothing `'gc` is live at an `.await`. The borrow checker enforces this (`compile_fail` doctest; error: "`Value<'gc>` is invariant… lifetime may not live long enough") |
| `DynamicRoot` (`Stashed`) | the only way a GC ref crosses an `.await`; it is traced from the root |
| `cx.safepoint().await`, `cx.call_managed(m, args).await` | effect operations: record the effect, return `Pending` |
| `Vm::run` | the handler: runs a full GC on `SafePoint`, and on `AwaitManagedReturn` resumes the interpreter until the callee frame (`ReturnTo::Task`) returns |

This is ST-monad/region typing: each `with` is a `runST`-style region, and stashing is
the explicit lift of a value across effect boundaries.

The hot dispatch loop stays synchronous. Tasks form a stack parallel to the frame stack,
so a task can call managed code that starts another task (tested: `CMP_NESTED`).

## Evidence

- `chunked_sum_keeps_progress_across_full_collections`: a full collection runs at every
  interpreter step and at every safe point. Progress (`off`, `acc`) survives, with exactly
  one safe point per chunk.
- `managed_callbacks_from_straight_line_rust`: an insertion sort written in Rust calls a
  managed comparer per comparison, including a comparer that itself suspends.
- `stash_is_what_keeps_objects_alive`: a `Drop` sentinel shows the stash is what keeps an
  object alive, and that dropping it lets the object be collected.
- Miri tree-borrows: clean.
- Cost, release build, toy arena: `with` round trip ≈ 0.6 ns; spawn + suspend + 2 polls
  ≈ 11–14 ns. This is optimistic. The real `with_arena` + `GCHandle::new` path costs more,
  but it is still fine for slow-path intrinsics. Keep it off the 128-instruction batch loop.

## Prior art: piccolo master already does this

`kyren/piccolo` `src/async_callback.rs` (2024; **unreleased**, not in crates.io 0.3.3),
by the gc-arena author. It provides `async_sequence(mc, |locals, seq| async move { … })`,
where `seq.enter(|ctx, locals, exec, stack| …)` is a `for<'gc>` region and `Locals` is a
**per-sequence `DynamicRootSet`**. The `Sequence` piccolo was cited for in July is now
*implemented by* an async block.

There are two differences from this prototype:

1. **Piccolo polls inside `mutate`.** The future is wrapped in a `Collect` `BoxSequence`
   owned by the GC graph, and `enter` reaches the context through a lifetime-erased shared
   slot. That design needs about two `unsafe` transmutes. This prototype polls *outside*
   `mutate` (arena in a `RefCell`) and needs **zero `unsafe`**, but each region costs a
   `mutate_root`.
2. **Piccolo uses per-task root sets** so that an abandoned sequence and its stashed locals
   get collected together, cycles included. A suspended .NET native frame is never garbage
   while its thread lives, so a single root set on the engine is acceptable for dotnet-rs.
   Per-task sets still give cleaner teardown on thread exit or abort.

**For dotnet-rs I'd lean piccolo-style.** The task can live in the root as something like
`VmContinuation::Task(..)`, which subsumes the Tier-1 type. An intrinsic can start and poll
its task inside the same `mutate`, so a task that never suspends costs almost nothing. The
`Executor::run` / `with_arena` structure doesn't have to change either. The cost is two
small `unsafe` blocks, which fit the existing SAFETY-citation discipline.

## What it would buy in dotnet-rs (concrete sites)

- **Chunked string/span intrinsics restart from zero.** For example,
  `dotnet-intrinsics-string/src/search.rs:135,259` does `return StepResult::Yield` mid-loop,
  then `back_up_ip` re-executes the whole instruction. With `safepoint().await`, progress is
  kept across the yield.
- **Tier-1 residue.** The `GC.WaitForPendingFinalizers` spin (`intrinsics/gc.rs:199`) and
  the static-init wait (`statics.rs:476`) both become `await`s on a condition.
- **Managed re-entry.** `FrameReturnAction::InvokeReturn` (reflection invoke) and
  `MulticastState` (delegate multicast) are hand-written "call managed, then continue"
  state machines, which is exactly what `call_managed(..).await` is.

## Open problems (not prototyped)

- **Exceptions through a task boundary.** The unwinder must stop at a `ReturnTo::Task`
  frame and deliver `Err` to the task. `call_managed` should return `Result`, and the task
  propagates the exception with `?`. Stack traces need a synthetic native frame.
- **Multithreading.** The `SafePoint` handler must call
  `thread_manager.safe_point(..)`/coordinator. Stashing a cross-arena reference into a
  per-thread `DynamicRootSet` is unverified.
- **Discipline the compiler doesn't check.** `.await` on anything other than a `Cx` effect
  panics at runtime ("Pending without an effect"), and a nested `with` panics. Piccolo has
  the same limitation.
- **Observability.** Task state is opaque to the tracer and ring buffer.
- **gc-arena 0.7.0** (Aug 2026) is orthogonal to this design: `Gc` points at `T` directly,
  and pacing is by Gc count, not bytes. It is relevant to the pointer-provenance work,
  since the 0.6.1 `Gc::cast`/`from_ptr` alignment caveat applies to 4 call sites in
  `runtime-memory/src/heap.rs` and `dotnet-value/src/object/mod.rs`, and it would affect
  the VM's GC pacing tuning.
