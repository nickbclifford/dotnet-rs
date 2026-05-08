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
