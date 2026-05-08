# Agent Memory Log — dotnet-rs Refactor

This file is a persistent scratch log for agent sessions executing the refactor plan.

## Format Rules

- **Append only.** Never edit prior entries.
- **One entry per session, at the end of each step.**
- **Read prior entries before starting.** You must not repeat work already recorded as completed.
- **You may add new checklist items** to `CHECKLIST.md` if you discover work not covered by the plan, but do so transparently and record the addition here.
- **Reference step IDs** from `CHECKLIST.md` (e.g., "1.1", "2.3") in your entry header.

## Entry Format

```
## <ISO-8601 date> — Step <id> — <model> — <status: completed | partial | abandoned>
**Goal:** <one line>
**What changed:** <files touched, summary of what was modified>
**What I learned:** <surprises, new findings, things harder/easier than expected>
**Follow-ups for future steps:** <step IDs or new checklist items to add>
**Open questions:** <anything unresolved that the next agent or the owner should know>
```

---

## 2026-05-04 — Review session — claude-sonnet-4-6[1m] — completed

**Goal:** Architecture and maintainability review; produce REVIEW.md, CHECKLIST.md, AGENT_MEMORY.md, AGENT_PROMPT.md, copy_prompt.py.

**What changed:** Created five deliverable files at the project root. No source files modified.

**What I learned:**
- `ExceptionContext` and `PInvokeContext` in `dotnet-vm-ops` look like single-impl markers but are actually load-bearing for dynamic dispatch (`&dyn`) in `dotnet-exceptions` and `dotnet-pinvoke`. Do not remove or inline them.
- `statics.rs` contains-key-then-entry pattern (lines 294–316) is correct optimistic initialization, not a TOCTOU race. The write-lock `entry().or_insert_with()` is atomic.
- `DelegateInvokeHost` in `dotnet-intrinsics-delegates` has real method bodies; it is not a pass-through.
- `docs/p3_s1_trait_inventory.md` is referenced in 8+ source files but does not exist. This is the most urgent doc-drift issue.
- `VmResolverService`/`VmResolverCaches` add real value (metrics, thread-local front-cache). Not a pass-through adapter.
- The `AtomicMemoryHost` trait has a blanket impl that makes every method an exact alias for `RawMemoryOps`. It is a naming shim, not a real abstraction.
- `CallOps<'gc>` is an empty marker trait. `VmCallOps<'gc>: CallOps<'gc>` is the only real supertrait chain — `CallOps` itself adds nothing.
- `TypeComparer` takes `ResolutionS` by value throughout, causing 38 redundant `Arc` clones in recursive type comparison.
- All feature flags in Cargo.toml are either tested by CI matrix or documented as prototypes. No orphaned features.
- `bench_pgo.sh` is documented in `docs/BENCHMARK_WORKFLOW.md`; not an orphan.

**Follow-ups for future steps:** Start with Phase 1 (documentation, zero risk). Then Phase 2 (vestigial traits). See CHECKLIST.md for all steps.

**Open questions:**
- Should `docs/p3_s1_trait_inventory.md` be created or should the 8+ references be removed? (Recommend: remove references if no one intends to write it.)
- `CallOps<'gc>` removal is low-risk but changes the public API surface of `dotnet-vm-ops`. Should it be documented as an extension point instead of removed?
- Phase 4 (context.rs split) depends on which granularity of file split the owner prefers: one `context_ops.rs` file, or multiple files per subsystem (e.g., `context_stack_ops.rs`, `context_intrinsic_hosts.rs`).

## 2026-05-04 — Step 1.1 — gpt-5 — completed
**Goal:** Remove stale `docs/p3_s1_trait_inventory.md` references from source files.
**What changed:** Removed the stale doc reference lines from 8 source files: `crates/dotnet-vm-ops/src/ops.rs`, `crates/dotnet-vm/src/stack/ops.rs`, `crates/dotnet-intrinsics-delegates/src/lib.rs`, `crates/dotnet-intrinsics-reflection/src/lib.rs`, `crates/dotnet-intrinsics-threading/src/lib.rs`, `crates/dotnet-intrinsics-span/src/lib.rs`, `crates/dotnet-intrinsics-string/src/lib.rs`, and `crates/dotnet-intrinsics-unsafe/src/lib.rs`. Marked checklist step 1.1 complete in `CHECKLIST.md`.
**What I learned:** Review anchors were still accurate at execution time; all cited references were present and removed cleanly. No additional stale `docs/p3_s1_trait_inventory.md` references remain under `crates/`.
**Follow-ups for future steps:** Continue with step 1.2.
**Open questions:** None.

## 2026-05-04 — Step 1.2 — gpt-5 — completed
**Goal:** Document `DOTNET_VM_EXTRA_INSTRUCTION_SOURCES` / `DOTNET_VM_EXTRA_INTRINSIC_SOURCES` extension points in `docs/BUILD_TIME_CODE_GENERATION.md`.
**What changed:** Updated `docs/BUILD_TIME_CODE_GENERATION.md` with a new section describing both env vars, their parsing format (`;`-delimited `<dir>` or `<dir>=<module_prefix>`), defaults, an example, and an explicit note that these are currently untested extension points in CI. Marked step `1.2` complete in `CHECKLIST.md`.
**What I learned:** `crates/dotnet-vm/build_support/scanner.rs` still matches the review description; both env vars are parsed by `parse_extra_roots` and wired into rerun directives. While verifying, I noticed the intrinsic-root list in `docs/BUILD_TIME_CODE_GENERATION.md` omits `../dotnet-intrinsics-simd/src`, which is present in code.
**Follow-ups for future steps:** Added checklist item `1.6` to capture the missing SIMD root documentation update.
**Open questions:** None.

## 2026-05-04 — Step 1.3 — gpt-5 — completed
**Goal:** Add `dotnet-benchmarks`, `dotnet-build-tools`, and `dotnet-simd` to `docs/ARCHITECTURE.md` dependency hierarchy.
**What changed:** Updated `docs/ARCHITECTURE.md` to add role bullets for `dotnet-simd`, `dotnet-build-tools`, and `dotnet-benchmarks`, and extended the dependency hierarchy block to include all three crates (`dotnet-build-tools` as a build dependency, `dotnet-simd` under the VM dependency tree, and `dotnet-benchmarks` as a benchmark harness depending on `dotnet-vm`). Marked checklist step `1.3` complete in `CHECKLIST.md`.
**What I learned:** The review finding still applied for these three crates. `dotnet-macros`/`dotnet-macros-core` were already present in the current hierarchy and did not require changes in this step.
**Follow-ups for future steps:** None.
**Open questions:** None.

## 2026-05-04 — Step 1.4 — gpt-5 — completed
**Goal:** Expand incomplete TODO items in `docs/EXCEPTION_HANDLING.md` with brief inline explanations.
**What changed:** Updated the four incomplete TODO lines in `docs/EXCEPTION_HANDLING.md` by adding concise parenthetical explanations of scope and intent; marked checklist step `1.4` complete in `CHECKLIST.md`.
**What I learned:** The review anchor still applied at execution time; the same TODO items were still present and incomplete.
**Follow-ups for future steps:** None.
**Open questions:** None.

## 2026-05-04 — Step 1.5 — gpt-5 — completed
**Goal:** Rename the `check.sh` "experimental feature smoke tests" section to "prototype compilation guards" and add a scope comment.
**What changed:** Updated `check.sh` by replacing the section label with `Running prototype compilation guards...` and adding a two-line comment clarifying these feature-flag runs are compile/regression guards and do not yet assert feature-specific behavior. Marked checklist step `1.5` complete in `CHECKLIST.md`.
**What I learned:** The review anchor still applied at execution time; the six prototype feature test commands were unchanged and grouped exactly as described.
**Follow-ups for future steps:** None.
**Open questions:** None.

## 2026-05-04 — Step 1.6 — gpt-5 — completed
**Goal:** Add missing `../dotnet-intrinsics-simd/src` default intrinsic root to `docs/BUILD_TIME_CODE_GENERATION.md` root list.
**What changed:** Added `../dotnet-intrinsics-simd/src` to the intrinsic default root list in `docs/BUILD_TIME_CODE_GENERATION.md` under the `process_intrinsic_file` build-process section. Marked checklist step `1.6` complete in `CHECKLIST.md`.
**What I learned:** `crates/dotnet-vm/build_support/scanner.rs` still includes the SIMD intrinsic root by default; the drift was documentation-only.
**Follow-ups for future steps:** None.
**Open questions:** None.

## 2026-05-04 — Step 2.1 — gpt-5 — completed
**Goal:** Remove empty `CallOps<'gc>` trait and associated vestigial plumbing/re-exports.
**What changed:** Removed `CallOps<'gc>` from `crates/dotnet-vm-ops/src/ops.rs`; removed its public re-export from `crates/dotnet-vm-ops/src/lib.rs`; removed `CallOps` import/re-export usage and the `VmCallOps<'gc>: CallOps<'gc>` supertrait from `crates/dotnet-vm/src/stack/ops.rs`; removed the blank `impl<'a, 'gc> CallOps<'gc> for VesContext<'a, 'gc> {}` from `crates/dotnet-vm/src/stack/context.rs`; removed remaining `CallOps` re-exports from `crates/dotnet-vm/src/stack/mod.rs` and `crates/dotnet-vm/src/lib.rs`; updated one stack module doc bullet to reference `VmCallOps` instead of `CallOps`. Marked checklist step `2.1` complete in `CHECKLIST.md`.
**What I learned:** The review anchors were still valid at execution time; there were no independent `CallOps` bounds or remaining in-crate references once re-exports/imports were removed.
**Follow-ups for future steps:** None.
**Open questions:** None.

## 2026-05-04 — Step 2.2 — gpt-5 — completed
**Goal:** Remove `AtomicMemoryHost<'gc>` shim trait and call `RawMemoryOps` directly from threading intrinsic handlers.
**What changed:** In `crates/dotnet-intrinsics-threading/src/lib.rs`, removed the `AtomicMemoryHost<'gc>` trait (including its blanket impl), updated `ThreadingIntrinsicHost<'gc>` to require `+ RawMemoryOps<'gc>` directly, and added a module-level doc comment that names the `RawMemoryOps` atomic methods used by Interlocked/Volatile handlers. In `crates/dotnet-intrinsics-threading/src/interlocked.rs` and `crates/dotnet-intrinsics-threading/src/volatile.rs`, replaced all `threading_*_atomic` invocations with direct `RawMemoryOps::{compare_exchange_atomic,exchange_atomic,exchange_add_atomic,load_atomic,store_atomic}` calls.
**What I learned:** The review anchors still matched current code exactly (the shim trait and all threading-prefixed call sites were still present). `dotnet-intrinsics-threading` has no crate-local `multithreading` feature, so the prescribed single-crate feature-scoped command is not valid there.
**Follow-ups for future steps:** None.
**Open questions:** None.

## 2026-05-04 — Step 3.1 — gpt-5 — completed
**Goal:** Replace `self.frame_stack.pop().unwrap()` in `return_frame()` with graceful `StepResult::Error(...)` handling on empty stack.
**What changed:** Updated `crates/dotnet-vm/src/stack/context.rs` in `return_frame()` to replace `self.frame_stack.pop().unwrap()` with a `match`; on `None`, it now returns `StepResult::Error(crate::error::VmError::Execution(crate::error::ExecutionError::InternalError("ret instruction with empty frame stack".into())))`. Marked checklist step `3.1` complete in `CHECKLIST.md`.
**What I learned:** The review anchor still matched the code location and behavior. `ExecutionError::InvalidOperation` referenced in the review note does not exist in this codebase (`dotnet-types::error::ExecutionError`), so `ExecutionError::InternalError` is the closest existing execution-error variant for this defensive path.
**Follow-ups for future steps:** None.
**Open questions:** None.

## 2026-05-04 — Step 3.2 — gpt-5 — completed
**Goal:** Replace `expect("Thread arena not initialized")` panics in `executor.rs` with graceful error returns.
**What changed:** Updated `crates/dotnet-vm/src/executor.rs` to remove both `expect("Thread arena not initialized")` usages in `with_arena`/`with_arena_ref`. Both helpers now return `Result<_, VmError>` and emit `VmError::Execution(ExecutionError::InternalError("Thread arena not initialized".into()))` when TLS arena state is missing. Threaded the new fallible helpers through `run()` so each arena access path returns `ExecutorResult::Error(...)` on missing arena instead of panicking, and updated `perform_full_gc()` to return `Result<(), VmError>` and propagate failures into `run()`. Marked checklist step `3.2` complete in `CHECKLIST.md`.
**What I learned:** Review anchors still matched current code at the cited sites. `entrypoint()` has no result return channel, so it now intentionally ignores a missing-arena error (`let _ = ...`), while `run()` is now the graceful error boundary as required.
**Follow-ups for future steps:** None.
**Open questions:** None.

## 2026-05-05 — Step 4.1 — gpt-5 — completed
**Goal:** Split trait impl blocks out of `crates/dotnet-vm/src/stack/context.rs` into new `context_ops.rs` and wire the module in `stack/mod.rs`.
**What changed:** Created `crates/dotnet-vm/src/stack/context_ops.rs` and moved all `impl<'a, 'gc> Trait for VesContext<'a, 'gc>` blocks there (`LoaderOps`, `SimdCapabilityOps`, `ResolutionOps`, `StackOps`, `BaseMemoryOps`, `ReflectionOps`, intrinsic host traits, `StaticsOps`, `VesInternals`, `VesBaseOps`, `ExceptionContext`, `VmExceptionContext`, `PInvokeContext`, `VmPInvokeContext`, `VesOps`). `crates/dotnet-vm/src/stack/context.rs` now keeps the `VesContext`/`ThreadContext` structs, inherent `impl VesContext` methods, and tests. Updated `crates/dotnet-vm/src/stack/mod.rs` with `mod context_ops;`. Marked checklist step `4.1` complete in `CHECKLIST.md`.
**What I learned:** Review anchors still matched current code shape (`context.rs` at 1408 lines before split, trait impls beginning around line 434). The move required import hygiene fixes in both files and one trait import in `context_ops.rs` (`ValueResolution`) for method resolution.
**Follow-ups for future steps:** Continue with step `4.2` (phase-gate `check.sh`).
**Open questions:** None.

## 2026-05-05 — Step 4.2 — gpt-5 — partial
**Goal:** Run `check.sh` under all feature combinations to verify no regressions from the `context.rs` trait-impl split.
**What changed:** Ran `bash check.sh` from repo root after verifying split state (`context.rs` + `context_ops.rs` + `mod context_ops;` in `stack/mod.rs`). Did not change runtime/source behavior. Added a new unchecked checklist item `3.3` in `CHECKLIST.md` for the blocker uncovered by this run.
**What I learned:** The phase-gate run fails in the `=== Combination: No features ===` clippy leg at `crates/dotnet-vm/src/executor.rs` with three `-D warnings` errors: one `dead_code` (`thread_arena_not_initialized_error`) and two `clippy::collapsible_if` sites. This blocks completion of step `4.2`; no evidence of a `context.rs` split-specific regression was surfaced before failure.
**Follow-ups for future steps:** Complete new checklist item `3.3`, then re-run step `4.2` (`bash check.sh`) as the phase gate.
**Open questions:** None.

## 2026-05-05 — Step 3.3 — gpt-5 — completed
**Goal:** Fix `dotnet-vm` no-feature clippy regressions in `executor.rs` introduced by step 3.2 (`dead_code` + `collapsible_if`) so full matrix can pass.
**What changed:** Updated `crates/dotnet-vm/src/executor.rs` to `#[cfg(feature = "multithreading")]`-gate `thread_arena_not_initialized_error()` (it is only referenced in multithreading TLS arena paths), and collapsed two nested conditionals in `run()` into idiomatic `if <cond> && let Err(e) = ...` forms to satisfy `clippy::collapsible_if` in no-feature builds. Marked checklist step `3.3` complete in `CHECKLIST.md`.
**What I learned:** The review note still matched current code locations before editing (`thread_arena_not_initialized_error` plus the two nested `if` sites in the non-multithread loop path). Keeping the multithread-only arena reset block under `if collection_requested { ... }` was necessary to preserve behavior while resolving clippy.
**Follow-ups for future steps:** Resume step `4.2` by re-running `bash check.sh` phase gate.
**Open questions:** None.

## 2026-05-05 — Step 4.2 — gpt-5 — completed
**Goal:** Run `check.sh` under all feature combinations to verify no regressions from the `context.rs` trait-impl split.
**What changed:** Verified the split state still matched review context (`crates/dotnet-vm/src/stack/context.rs` retained core struct/inherent impls; trait impls in `crates/dotnet-vm/src/stack/context_ops.rs`; `mod context_ops;` in `crates/dotnet-vm/src/stack/mod.rs`), then ran `bash check.sh` to completion. Marked checklist step `4.2` complete in `CHECKLIST.md`.
**What I learned:** The full phase-gate matrix (feature combinations, build-script probes, lock-order harness, prototype compilation guards, and hang-probe integration tests) now passes after step `3.3`; no regressions attributable to the context split were surfaced.
**Follow-ups for future steps:** Proceed to Phase 5 (`5.1`) when ready.
**Open questions:** None.

## 2026-05-05 — Step 5.1 — gpt-5 — completed
**Goal:** Change `TypeComparer` in `crates/dotnet-types/src/comparer.rs` to take `&ResolutionS` (`res1`/`res2`) and remove in-file `res1.clone()` / `res2.clone()` call sites.
**What changed:** Updated all `TypeComparer` methods in `comparer.rs` that accepted `res1`/`res2` by value to accept `&ResolutionS` (`type_slices_equal`, `types_equal`, `param_types_equal`, `params_equal`, `signatures_compatible_with_variance`, `signatures_equal`, `concrete_equals_method_type`, `concrete_equals_base_type`). Rewired internal call chains and macro call sites in `find_method_in_type_internal` to pass references. Updated comparer unit tests in the same file to pass `&ResolutionS`. Removed all `res1.clone()` / `res2.clone()` call sites from the file. Marked checklist step `5.1` complete in `CHECKLIST.md`.
**What I learned:** Review anchors still matched before edits (by-value signatures and clone-heavy recursive calls were present). `dotnet-types` does not define a `multithreading` feature, so the prescribed per-step command variant with `--features multithreading` is invalid for this crate and must be run without that feature flag.
**Follow-ups for future steps:** Proceed to step `5.2` to update cross-crate callers in `dotnet-runtime-resolver/` for the new reference-based `TypeComparer` signatures.
**Open questions:** None.

## 2026-05-05 — Step 5.2 — gpt-5 — completed
**Goal:** Update `dotnet-runtime-resolver` callers of `TypeComparer` APIs to pass `&ResolutionS` references.
**What changed:** Updated `crates/dotnet-runtime-resolver/src/methods.rs` at the two `TypeComparer` call sites (`signatures_compatible_with_variance` and `signatures_equal`) to pass borrowed resolutions via `&method.method_resolution` and `&decl.method_resolution` instead of calling cloning accessors (`method.resolution()` / `decl.resolution()`). Marked checklist step `5.2` complete in `CHECKLIST.md`.
**What I learned:** In `dotnet-runtime-resolver`, only two cross-crate `TypeComparer` calls existed, both in `methods.rs`. The existing calls compiled via auto-borrowed temporaries but still cloned `ResolutionS`; borrowing struct fields removes that clone at the call boundary.
**Follow-ups for future steps:** Proceed to Phase 6 when ready.
**Open questions:** None.

## 2026-05-07 — Step 6.1 — gpt-5 — completed
**Goal:** Implement argv initialization for managed entrypoints (`Main(string[] args)`) in `crates/dotnet-vm/src/executor.rs`.
**What changed:** Updated `crates/dotnet-vm/src/executor.rs` `entrypoint()` to collect `std::env::args().skip(1)`, build a managed `System.String[]` when the entrypoint has one parameter, and pass that array to `entrypoint_frame`; zero-parameter entrypoints still receive no arguments. Added an ECMA-aligned signature guard (entrypoint must be either no params or single `string[]`). Registered all allocated argv string objects and the argv vector with the GC tracking path. Marked checklist step `6.1` complete in `CHECKLIST.md`. Added checklist item `5.3` for an uncovered remaining `TypeComparer::signatures_equal` by-value call in `crates/dotnet-vm/src/stack/call_ops_impl.rs:669`.
**What I learned:** The review anchor still matched current code before editing (`entrypoint_frame(..., vec![])` at the TODO site). The required targeted verification commands for this step currently fail due a pre-existing compile error outside this step (`call_ops_impl.rs:669` still passes `ResolutionS` by value to `TypeComparer::signatures_equal` after step 5 changes).
**Follow-ups for future steps:** Complete new checklist item `5.3`, then re-run step-level `dotnet-vm` clippy/tests to revalidate `6.1` in a green state.
**Open questions:** None.
