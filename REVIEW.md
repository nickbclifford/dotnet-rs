# Architecture & Maintainability Review — dotnet-rs

## Executive Summary

The codebase is in overall good shape: the trait infrastructure is largely sound after the recent `5cdf11b` simplification pass, the build-time codegen is well-justified and well-tested, and the error type hierarchy is clean after `adc74d1`. The concerns below are real but not catastrophic. The top findings are:

1. **`docs/p3_s1_trait_inventory.md` referenced in 9 source files but does not exist** — the most urgent doc-drift issue.
2. **`CallOps<'gc>` is an empty marker trait with no methods** — exists only as a supertrait of `VmCallOps`; should be removed or documented as a deliberate extension point.
3. **`AtomicMemoryHost<'gc>` is a naming shim via blanket impl** — delegates every method verbatim to `RawMemoryOps`; adds `threading_` prefixes only, no new invariants.
4. **`context.rs` (1401 lines) mixes the core `VesContext` struct with 12+ trait implementation blocks** — makes navigation and incremental compilation harder than necessary.
5. **`TypeComparer` takes `ResolutionS` by value** — causes 38 redundant `.clone()` calls across recursive type comparison methods; changing to `&ResolutionS` eliminates them.
6. **Experimental feature smoke tests in `check.sh` are compile-only** — they run `cargo test --features X` but verify only that the feature doesn't break compilation, not that it changes behavior.
7. **`return_frame()` uses `unwrap()` on a hot path** — `context.rs:229` is called on every `ret` instruction; stack underflow panics instead of returning a `VmError`.
8. **3 open TODOs that represent real gaps**: argv not wired up (`executor.rs:165`), `RuntimeType` doesn't encode element types for arrays/vectors (`intrinsics/mod.rs:514`), span non-ordinal comparison not implemented (`span/equality.rs:320`).

**Headline recommendation:** Focus first on Phase 1 (doc drift fixes, ~1 hour), then Phase 2 (remove vestigial trait plumbing, M effort, zero behavioral risk), then Phase 3 (comparer ergonomics and hot-path error handling, M effort).

---

## Methodology

**Read:** All 11 `docs/*.md` files end-to-end. Root `Cargo.toml` and all 27 crate `Cargo.toml` files. Key files: `ops.rs` (532 lines), `context.rs` (1401 lines), `statics.rs`, `comparer.rs` (1087 lines), `coordinator.rs` (1300 lines), `access.rs` (1642 lines), `check.sh`, `build.rs` (vm and cli), `macros-core/src/lib.rs` (825 lines), `dotnet-intrinsics-threading/src/lib.rs`.

**Probed with grep/bash:** trait `impl` counts, `unwrap`/`expect` locations, all `TODO`/`FIXME`/`HACK`/`XXX` comments, all `#[allow(dead_code)]` annotations, all `pub use` re-export chains, feature flag cross-referencing, `AtomicMemoryHost` / `DelegateInvokeHost` / `ExceptionContext` / `PInvokeContext` callers, `p3_s1_trait_inventory.md` references, `bench_pgo.sh` references, `DOTNET_VM_EXTRA_*` env var callers, `CallOps` usage (excluding `VmCallOps`), `ResolutionS` clone sites.

**Spawned 5 parallel Explore agents:** dead code audit, abstraction complexity, build/codegen complexity, large file / error handling, Rust idiom quality. All agent conclusions were cross-checked against actual source before inclusion.

**Scope limits:** Did not run the build or tests (read-only session). Did not audit `crates/dotnet-vm/src/fuzzing.rs` (984 lines) or benchmark crates in depth.

---

## Findings

### Dead / Vestigial Code

---

<a id="F-DEAD-001"></a>
#### F-DEAD-001 — `CallOps<'gc>` is an empty marker trait

**File:** `crates/dotnet-vm-ops/src/ops.rs:295`

```rust
pub trait CallOps<'gc> {}
```

**Current state:** The trait has no methods. It has exactly two implementations in the entire workspace: an empty `impl<'a, 'gc> CallOps<'gc> for VesContext<'a, 'gc> {}` at `crates/dotnet-vm/src/stack/context.rs:444`, and an implicit supertrait satisfaction via `VmCallOps<'gc>: CallOps<'gc>` at `crates/dotnet-vm/src/stack/ops.rs:145`. No code path uses `T: CallOps<'gc>` as a bound independently of `T: VmCallOps<'gc>`.

**Why it's a problem:** The trait adds no capabilities, distinguishes no states, and enforces no invariants. Its only purpose is as a supertrait anchor — but `VmCallOps` already is the real anchor. Removing `CallOps` and making `VmCallOps` self-contained eliminates one trait from the public API surface and reduces the mental load of the trait tower. If it's intended as a forward-compatibility extension point, that intent should be documented.

**Proposed change:** Either (a) delete `CallOps<'gc>` and remove the `: CallOps<'gc>` supertrait from `VmCallOps`, updating the two blank impls; or (b) add a doc comment: `/// Extension point for future call-site operations that must live in dotnet-vm-ops rather than dotnet-vm. Currently has no methods.`

**Risk:** Low. `CallOps` is in the public API of `dotnet-vm-ops` but has no downstream consumers outside this workspace (it's a top-level binary). Removing it requires updating 3–4 files.

**Features touched:** None.

**Effort:** S.

---

<a id="F-DEAD-002"></a>
#### F-DEAD-002 — `AtomicMemoryHost<'gc>` is a naming shim via blanket impl

**File:** `crates/dotnet-intrinsics-threading/src/lib.rs:53–131`

**Current state:** The trait defines 5 methods (`threading_compare_exchange_atomic`, `threading_exchange_atomic`, `threading_exchange_add_atomic`, `threading_load_atomic`, `threading_store_atomic`). Every method has a default implementation that calls the identically-named method on `RawMemoryOps<'gc>` with no additional behavior. The trait is then given a blanket implementation at line 131:

```rust
impl<'gc, T: RawMemoryOps<'gc> + ?Sized> AtomicMemoryHost<'gc> for T {}
```

This means every type implementing `RawMemoryOps` automatically implements `AtomicMemoryHost` with exactly the same behavior. The `ThreadingIntrinsicHost` alias requires `+ AtomicMemoryHost<'gc>` (line 145), which is always satisfied by anything that satisfies `RawMemoryOps<'gc>`.

**Why it's a problem:** The trait is a namespace decoration. The `threading_` prefix is the only difference from calling `RawMemoryOps` directly. Any caller bound on `AtomicMemoryHost<'gc>` gets the same behavior as one bound on `RawMemoryOps<'gc>`, but with extra trait ceremony. The trait communicates semantic intent (these ops are used for Interlocked semantics) but does so by adding a redundant API layer rather than by documentation or type-level distinction.

**Proposed change:** Remove `AtomicMemoryHost<'gc>`. In `ThreadingIntrinsicHost`, replace `+ AtomicMemoryHost<'gc>` with `+ RawMemoryOps<'gc>`. Update all call sites in the threading intrinsic handlers to call `RawMemoryOps` methods directly (the rename is mechanical). Add a module-level doc comment explaining which `RawMemoryOps` methods are used for atomic/Interlocked semantics.

**Risk:** Low. All callers are within `dotnet-intrinsics-threading`. No cross-crate API impact.

**Features touched:** None.

**Effort:** S.

---

<a id="F-DEAD-003"></a>
#### F-DEAD-003 — `StaticIntrinsicEntry` debug fields suppressed with `#[allow(dead_code)]`

**File:** `crates/dotnet-vm/src/intrinsics/static_registry.rs:18–27`

**Current state:** `type_name`, `member_name`, and `arity` are suppressed with `#[allow(dead_code)]`. The comment (line 20) says they're "for metadata and debugging, not used in runtime dispatch."

**Assessment:** LEGITIMATE. These fields are set by generated code and could be exposed in debug output. No action required, but the comment is the right explanation and suffices. Included here to document that it was investigated.

**Effort:** None.

---

<a id="F-DEAD-004"></a>
#### F-DEAD-004 — `docs/p3_s1_trait_inventory.md` referenced in source but does not exist

**Files:** `crates/dotnet-vm-ops/src/ops.rs:11–12`, `crates/dotnet-vm/src/stack/ops.rs:24–25`, `crates/dotnet-intrinsics-delegates/src/lib.rs:4`, `crates/dotnet-intrinsics-reflection/src/lib.rs:5`, `crates/dotnet-intrinsics-threading/src/lib.rs:4`, `crates/dotnet-intrinsics-span/src/lib.rs:4`, `crates/dotnet-intrinsics-string/src/lib.rs:4`, `crates/dotnet-intrinsics-unsafe/src/lib.rs:4`.

**Current state:** 8+ files reference `docs/p3_s1_trait_inventory.md` as if it contains a "trait/call-site inventory and hot-path mapping." The file does not exist in the repository.

**Why it's a problem:** This is a broken documentation link across 8 source files. Any developer following the reference to understand the trait system will find nothing.

**Proposed change:** Either (a) create `docs/p3_s1_trait_inventory.md` documenting what each intrinsic host trait requires and why, or (b) remove the references and rely on the inline doc comments already present on the trait aliases.

**Risk:** Zero (documentation only).

**Effort:** S (removal) or M (create the document).

---

### Over-Complicated Abstractions

---

<a id="F-OVER-001"></a>
#### F-OVER-001 — `context.rs` mixes core struct with 12+ trait implementations (1401 lines)

**File:** `crates/dotnet-vm/src/stack/context.rs`

**Current state:** The file contains:
- Lines 1–423: The core `VesContext` struct, its `new`/`drop`, and the core methods.
- Lines 424–1401: 12+ trait implementation blocks: `LoaderOps`, `SimdCapabilityOps`, `CallOps`, `ResolutionOps`, `BaseMemoryOps`, `ReflectionOps`, `DelegateInvokeHost`, `IntrinsicStringHost`, `LayoutQueryHost`, `SpanPointerIntrospectionHost`, `SpanObjectFactoryHost`, `SpanRuntimeHost`, `MonitorHost`, `ExceptionContext`, `PInvokeContext`, `VmPInvokeContext`, `VmExceptionContext`, `VmCallOps`, and more.

The file grows as each new intrinsic subsystem is integrated by appending its `impl` block. This makes it a habitual destination for ad-hoc additions.

**Why it's a problem:** A 1401-line file combining a struct definition with 12+ trait implementations is hard to navigate, slows incremental compilation (every change to any trait impl recompiles the whole file), and hides the fact that each trait impl is a small, cohesive unit that could be understood in isolation.

**Proposed change:** Keep the core struct in `context.rs` (roughly lines 1–423). Create `context_ops.rs` as a sibling module with `use super::VesContext;` and move all `impl VesContext` blocks for external traits there. Alternatively, split into several files: `context_stack_ops.rs`, `context_resolution_ops.rs`, `context_intrinsic_hosts.rs`. Each split is independently mergeable.

**Risk:** Moderate. This is a mechanical move of `impl` blocks. Requires updating `mod.rs` to declare the new submodules. All types already imported. No behavior change.

**Features touched:** None (all impls are feature-unconditional or already gated).

**Effort:** M.

---

<a id="F-OVER-002"></a>
#### F-OVER-002 — Trait alias `DelegateInvokeHost` in `dotnet-intrinsics-delegates` vs. its use

**File:** `crates/dotnet-intrinsics-delegates/src/lib.rs:20–46`

**Current state:** `DelegateInvokeHost<'gc>` is a standalone trait (not a `trait_alias!`) with 5 methods: `dispatch_method`, `lookup_method_by_index`, `frame_stack_mut`, `return_frame`, and `frame_has_multicast_state`. It is implemented once: `impl<'a, 'gc> dotnet_intrinsics_delegates::DelegateInvokeHost<'gc> for VesContext<'a, 'gc>` at `context.rs:601`.

Unlike `AtomicMemoryHost`, this trait is not a blanket-impl shim — it has real method bodies in its `VesContext` implementation. The real question is whether a separate named trait adds clarity or just indirection.

**Assessment:** Looking at the handlers that use it (`try_delegate_dispatch`, `delegate_get_method`, `invoke_delegate`), the bound `T: DelegateIntrinsicHost<'gc> + DelegateInvokeHost<'gc>` is reasonable. The trait groups the specific VM-internal capabilities needed for delegate dispatch (frame stack access, method dispatch). This is a legitimate abstraction — it documents the delegate crate's dependency on VM internals without importing `VesContext` directly.

**Verdict:** NOT vestigial. The separation between `DelegateIntrinsicHost` (pure stack/memory ops) and `DelegateInvokeHost` (VM-internal frame ops) is a valid and intentional design seam. No action required.

---

<a id="F-OVER-003"></a>
#### F-OVER-003 — `ExceptionContext` and `PInvokeContext` appear single-impl but are load-bearing for dynamic dispatch

**File:** `crates/dotnet-vm-ops/src/ops.rs:345–370`

**Current state:** Each trait appears to have only one implementation (`VesContext`). However, both are used as `&dyn ExceptionContext<'gc>` in `crates/dotnet-exceptions/src/lib.rs:24,132,163` and as `&dyn PInvokeContext<'gc>` in `crates/dotnet-pinvoke/src/lib.rs:229,542,575,601`.

**Assessment:** These traits are the abstraction boundary that lets `dotnet-exceptions` and `dotnet-pinvoke` operate without depending on `dotnet-vm` internals. They ARE the reason the exception and P/Invoke crates can live in separate crates. Removing them would force those crates to depend on `dotnet-vm`, creating circular dependencies or a monolith. They are NOT overcomplicated — they are necessary seams.

**Verdict:** LOAD-BEARING. No action required. Included here to prevent future agents from re-litigating this.

---

### Build / Codegen Complexity

---

<a id="F-BUILD-001"></a>
#### F-BUILD-001 — Experimental feature smoke tests are compile-only, not behavioral

**File:** `check.sh:38–44`

```bash
echo "Running experimental feature smoke tests..."
cargo test --features bench-instrumentation -- --nocapture
cargo test --features heap-diagnostics -- --nocapture
cargo test -p dotnet-vm --features deadlock-diagnostics -- --nocapture
cargo test --features segmented-eval-stack-prototype -- --nocapture
cargo test -p dotnet-vm --features instruction-dispatch-jump-table -- --nocapture
cargo test -p dotnet-vm --features "instruction-dispatch-jump-table dispatch-super-instruction-prototype" -- --nocapture
```

**Current state:** These 6 commands run the test suite with experimental feature flags enabled. They verify that the features don't break compilation and don't cause existing tests to fail. They do **not** verify that the features actually change behavior — no test asserts that `bench-instrumentation` counters are incremented, or that `instruction-dispatch-jump-table` takes a different code path, or that `heap-diagnostics` emits diagnostic output.

**Why it's a problem:** The section heading is "Running experimental feature smoke tests" but the tests are really "compilation regression guards." This gap means a feature could silently regress (e.g., the jump-table dispatch never triggers, or the heap diagnostic path never executes) without CI catching it. The mismatch between the label and the actual test coverage can mislead developers into thinking feature behavior is validated.

**Proposed change:** Either (a) rename the section in `check.sh` to `Running prototype compilation guards` and add a comment explaining that behavioral validation is not yet implemented; or (b) for at least `instruction-dispatch-jump-table` and `dispatch-super-instruction-prototype`, add unit tests in gated `#[cfg(feature = "...")]` blocks that verify the alternate code path is exercised. The minimal viable test for the jump-table feature is a `#[test]` that runs a short instruction sequence and asserts it uses the jump-table dispatcher (e.g., via a counter or by structural difference in the generated code).

**Risk:** Minimal — this is a test-gap, not a bug.

**Features touched:** `bench-instrumentation`, `heap-diagnostics`, `deadlock-diagnostics`, `segmented-eval-stack-prototype`, `instruction-dispatch-jump-table`, `dispatch-super-instruction-prototype`.

**Effort:** S (comment change) or M (behavioral tests).

---

<a id="F-BUILD-002"></a>
#### F-BUILD-002 — `DOTNET_VM_EXTRA_INSTRUCTION_SOURCES` and `DOTNET_VM_EXTRA_INTRINSIC_SOURCES` are untested extension points

**File:** `crates/dotnet-vm/build_support/scanner.rs` (the `instruction_source_roots()` and `intrinsic_source_roots()` functions that read these env vars)

**Current state:** The build script supports injecting custom instruction and intrinsic source directories via env vars. No test validates that these env vars are parsed correctly or that code generated from external sources is valid. No CI job exercises them.

**Why it's a problem:** If these extension hooks bit-rot (e.g., the format of `DOTNET_VM_EXTRA_INSTRUCTION_SOURCES` changes or the scanner has a regression), there is no test to catch it. The documentation reference in `BUILD_TIME_CODE_GENERATION.md` mentions them but doesn't include a regression test citation.

**Proposed change:** Add a test to `scripts/check_build_script_regressions.sh` that sets `DOTNET_VM_EXTRA_INSTRUCTION_SOURCES=<test fixture path>` and verifies the build script parses it without error. Alternatively, add a clearly documented note in `BUILD_TIME_CODE_GENERATION.md` that these hooks are "untested extension points" so future maintainers know their status.

**Risk:** Minimal.

**Effort:** S.

---

<a id="F-BUILD-003"></a>
#### F-BUILD-003 — The attribute-driven codegen is well-justified (non-finding)

**Files:** `crates/dotnet-vm/build.rs`, `crates/dotnet-build-tools/`, `crates/dotnet-macros/`, `crates/dotnet-macros-core/`

**Assessment:** The build script's attribute scanning approach eliminates hand-maintained dispatch tables, provides compile-time deduplication detection, and allows instruction/intrinsic handlers to live close to their implementations. The `dotnet-build-tools` crate is fully utilized by multiple build scripts. The proc-macro split (macros-core as shared library) is sound and prevents key format drift. The `dotnet-cli/build.rs` fixture pre-compilation is necessary because .NET DLLs cannot be generated on-the-fly by Rust tests.

**Verdict:** The build system complexity is justified by the problem. No simplification recommended.

---

### Code Residency / Module Organization

---

<a id="F-RESID-001"></a>
#### F-RESID-001 — `access.rs` is 1642 lines but is a coherent single-purpose module

**File:** `crates/dotnet-runtime-memory/src/access.rs`

**Assessment:** All 1642 lines serve one purpose: low-level memory access with GC-awareness. The file is organized around `MemoryOwner` abstraction, `WriteBarrierFlushGuard`, and the `RawMemoryAccess` methods (reads, writes, atomic ops, bounds checking, cross-arena tracking). There are no natural split points that wouldn't increase cross-file coupling. The `#[allow(dead_code)]` suppressions are justified. The `unwrap`/`expect` calls are in truly infallible code paths.

**Verdict:** Fine as-is. Documented to prevent future re-investigation.

---

<a id="F-RESID-002"></a>
#### F-RESID-002 — `comparer.rs` is 1087 lines but is a coherent single-purpose module

**File:** `crates/dotnet-types/src/comparer.rs`

**Assessment:** All 1087 lines answer one question: "Is type A assignable/equal to type B?" The caching strategy (recursive assignability cache with in-progress tracking) requires all the logic to be in one place to share the cache fields. No natural split points without breaking the caching invariant.

**Verdict:** Fine as-is. See F-TYPES-001 for the `ResolutionS` clone issue that does affect this file.

---

<a id="F-RESID-003"></a>
#### F-RESID-003 — `coordinator.rs` is 1300 lines but is clean

**File:** `crates/dotnet-vm/src/gc/coordinator.rs`

**Assessment:** The single-threaded vs. multi-threaded split is clean via `#[cfg(feature = "multithreading")]`. The `CollectionSession` typestate pattern is well-designed. No accretion observed.

**Verdict:** Fine as-is.

---

<a id="F-RESID-004"></a>
#### F-RESID-004 — `cross_arena.rs` is correctly placed in `dotnet-utils`

**File:** `crates/dotnet-utils/src/gc/cross_arena.rs`

**Assessment:** The cross-arena reference tracking is used by both `dotnet-vm` and `dotnet-runtime-memory`. Placing it in `dotnet-utils` avoids a circular dependency. The 1047-line file contains tightly coupled lease-tracking code (bitset fast path, generational reference stamping, `ArenaLease` RAII guard) that benefits from being co-located.

**Verdict:** Fine as-is.

---

### Type System & Error Handling

---

<a id="F-TYPES-001"></a>
#### F-TYPES-001 — `TypeComparer` takes `ResolutionS` by value, causing 38 redundant clones

**File:** `crates/dotnet-types/src/comparer.rs`

**Current state:** All public and private methods on `TypeComparer` take `res1: ResolutionS` and `res2: ResolutionS` by value. Since these methods are recursive, every recursive call clones the `ResolutionS` values. There are 38 `.clone()` calls in this file alone for `res1` and `res2` — examples at lines 41, 66, 67, 83–87, 202, 240, 244, 260, 265, 276–279, 313, 380, 396.

`ResolutionS` wraps `Option<(Arc<MetadataArena>, NonNull<Resolution<'static>>)>`, so each clone increments an `Arc` refcount. This is cheap individually but happens on every recursive step of type comparison.

**Why it's a problem:** The cloning is unnecessary — the values are read-only during comparison. Changing to `res1: &ResolutionS` / `res2: &ResolutionS` eliminates all 38 clones. Type comparison happens during every `callvirt`, `isinst`, `castclass`, and virtual dispatch resolution.

**Proposed change:** Change all method signatures in `TypeComparer` from `res1: ResolutionS` to `res1: &ResolutionS` (and similarly `res2`). Update callers (most callers are within the same file; a few are in `dotnet-runtime-resolver`). The change is mechanical but touches 15–20 function signatures.

**Risk:** Low. Pure ergonomic change. All behavior preserved. The borrow lifetime is satisfied within each call.

**Features touched:** `generic-constraint-validation`.

**Effort:** M.

---

<a id="F-TYPES-002"></a>
#### F-TYPES-002 — 3 open TODOs representing real gaps

**Files and lines:**

1. `crates/dotnet-vm/src/executor.rs:165` — `// TODO: initialize argv (entry point args are either string[] or nothing, II.15.4.1.2)`. Entry point arguments (`string[] args`) are not passed to managed code. Programs expecting `args` get an empty array or null. This affects CLI usability for any .NET program that reads `Main(string[] args)`.

2. `crates/dotnet-vm/src/intrinsics/mod.rs:514` — `// TODO: encode exact element type and rank into RuntimeType (Vector/Array)`. The `RuntimeType` for arrays and vectors doesn't carry the element type, which affects reflection accuracy for generic collection types.

3. `crates/dotnet-intrinsics-span/src/equality.rs:320` — `// TODO: Support non-ordinal comparison types if needed`. Span equality uses only ordinal comparison; `StringComparison.CurrentCulture` and similar variants are not supported.

**Assessment:** These are all valid-concern TODOs, not stale. They represent implementation gaps, not cleanup. Prioritize: (1) argv initialization — affects all real programs; (2) RuntimeType encoding — affects reflection-heavy code; (3) non-ordinal span comparison — likely minor.

**Effort:** M (argv), L (RuntimeType encoding), S (span comparison note).

---

<a id="F-TYPES-003"></a>
#### F-TYPES-003 — Error type hierarchy is clean (non-finding)

**Assessment:** Post `adc74d1`, the typed error cleanup is complete. `VmError` absorbs specific sub-errors (`TypeResolutionError`, `ExecutionError`, `MemoryAccessError`, `PInvokeError`, `IntrinsicError`) via `From` implementations. Each sub-crate defines its own narrow error type. No giant enums absorbing everything. No error information lost.

**Verdict:** Good. No action required.

---

### Performance / Rust Idioms

---

<a id="F-IDIOM-001"></a>
#### F-IDIOM-001 — `return_frame()` uses `unwrap()` on a hot path

**File:** `crates/dotnet-vm/src/stack/context.rs:229`

```rust
pub fn return_frame(&mut self) -> StepResult {
    let frame = self.frame_stack.pop().unwrap();
    // ...
}
```

**Current state:** `return_frame()` is called from `handle_return()` on every `ret` instruction — one of the highest-frequency code paths in the VM. If the frame stack is empty at this point, the process panics rather than returning a `VmError`.

**Why it's a problem:** A well-formed .NET program should never call `ret` with an empty frame stack. But an ill-formed one can. The current behavior panics the entire process instead of throwing a managed exception or returning a `VmError::Execution(ExecutionError::InvalidOperation(...))`. This is inconsistent with the rest of the error handling (most `StepResult::Error` paths propagate gracefully).

**Proposed change:** Replace `unwrap()` with `ok_or_else` and return `StepResult::Error(VmError::Execution(ExecutionError::...))`:
```rust
let frame = match self.frame_stack.pop() {
    Some(f) => f,
    None => return StepResult::Error(VmError::Execution(ExecutionError::InvalidOperation(
        "ret instruction with empty frame stack".into()
    ))),
};
```

**Risk:** Low. This only fires on invalid programs. Adds one branch to an otherwise simple code path.

**Effort:** S.

---

<a id="F-IDIOM-002"></a>
#### F-IDIOM-002 — Executor arena `expect()` panics instead of returning error

**File:** `crates/dotnet-vm/src/executor.rs:62,77`

```rust
let arena = arena_opt.as_mut().expect("Thread arena not initialized");
let arena = arena_opt.as_ref().expect("Thread arena not initialized");
```

**Current state:** These panics fire if the thread-local arena is not initialized at the point of use. This can happen if executor lifecycle methods are called out of order.

**Why it's a problem:** This is an internal correctness check, not user-code protection. However, panicking from the entry-point `with_arena` method propagates out as an uncontrolled panic rather than a `VmError`, which breaks the assumption that `Executor::run` returns `Result<..., VmError>`.

**Proposed change:** Return `Err(VmError::Execution(ExecutionError::InternalError("Thread arena not initialized".into())))` from the affected code paths.

**Risk:** Low. The panic is currently a hard assertion; the proposed change preserves the error semantics while making it recoverable.

**Effort:** S.

---

<a id="F-IDIOM-003"></a>
#### F-IDIOM-003 — `DashMap` usage follows documented rules (non-finding)

**Assessment:** All `DashMap` accesses in the VM drop shard guards before performing any subsequent operations on the same map. The `cache_map_get_cloned` helper at `crates/dotnet-vm/src/state.rs:82–99` explicitly drops the guard before returning. No `len()` calls found while holding shard references.

**Verdict:** Good. Documented to prevent re-investigation.

---

<a id="F-IDIOM-004"></a>
#### F-IDIOM-004 — `statics.rs` read-then-write pattern is correct (non-finding)

**File:** `crates/dotnet-vm/src/statics.rs:294–316`

**Assessment:** The pattern of checking `contains_key` under a read lock, then using `entry().or_insert_with()` under a write lock (with expensive layout computation outside both locks) is correct. Two threads may both compute the layout (duplicate work), but only one will insert it due to `or_insert_with`'s atomic semantics. This is standard "optimistic lock-free initialization" and is not a TOCTOU race.

**Verdict:** Fine as-is.

---

### Documentation Drift

---

<a id="F-DOC-001"></a>
#### F-DOC-001 — `docs/p3_s1_trait_inventory.md` referenced everywhere but doesn't exist

This is the same finding as F-DEAD-004. It is the most urgent documentation issue in the codebase.

**Files:** `crates/dotnet-vm-ops/src/ops.rs:11–12`, `crates/dotnet-vm/src/stack/ops.rs:24–25`, and 6 intrinsics crates' `lib.rs` files.

---

<a id="F-DOC-002"></a>
#### F-DOC-002 — ARCHITECTURE.md does not list all crates

**File:** `docs/ARCHITECTURE.md`

**Current state:** The crate list in ARCHITECTURE.md does not mention `dotnet-benchmarks`, `dotnet-build-tools`, `dotnet-simd`, `dotnet-macros`, or `dotnet-macros-core` in the dependency hierarchy section (they are mentioned briefly or not at all).

**Proposed change:** Add `dotnet-build-tools`, `dotnet-simd`, and `dotnet-benchmarks` to the dependency hierarchy diagram with their roles.

**Risk:** None.

**Effort:** S.

---

<a id="F-DOC-003"></a>
#### F-DOC-003 — `docs/EXCEPTION_HANDLING.md` has incomplete TODO items

**File:** `docs/EXCEPTION_HANDLING.md` (last 6 lines)

```
- [ ] Add sequence diagrams for the two-pass exception model
- [ ] Document the relationship between `ExceptionState` transitions and `StepResult` variants
- [ ] Document edge cases: nested exceptions, exceptions in finally blocks, exceptions in filters
- [ ] Detail the `parse` function that converts dotnetdll metadata to `ProtectedSection`/`Handler`
```

These are legitimate documentation gaps that would help new contributors understand the most complex subsystem.

**Effort:** M.

---

## Non-Findings

These areas were investigated and found to be fine:

| Topic | Conclusion |
|-------|-----------|
| `ExceptionContext` / `PInvokeContext` — single-impl suspicion | Load-bearing for `dyn` dispatch in `dotnet-exceptions` and `dotnet-pinvoke`. Do not remove. |
| `DelegateInvokeHost` — suspected pass-through | Has real method bodies; groups VM-internal capabilities for delegate dispatch. Legitimate seam. |
| `VmResolverService` / `VmResolverCaches` — suspected pass-through | Does real work: per-call metrics, thread-local front-cache. Not a pass-through. |
| `bench_pgo.sh` orphan | Documented in `docs/BENCHMARK_WORKFLOW.md`. |
| `statics.rs` read-check + write-lock | Correct optimistic initialization; not a TOCTOU. |
| All `#[cfg(feature = "...")]` blocks | Every feature referenced in source is declared in its crate's `Cargo.toml`. |
| All feature flags in `Cargo.toml` | No orphaned features. All are exercised by CI matrix or documented prototypes. |
| `xtask` subcommands | All 5 subcommands referenced in CI workflows. |
| `cross_arena.rs` placement | Correctly placed in `dotnet-utils`; used by both `dotnet-vm` and `dotnet-runtime-memory`. |
| `dotnet-simd` vs `dotnet-intrinsics-simd` | Distinct concerns: architecture portability vs. .NET semantics. Not redundant. |
| `access.rs` (1642 lines) | Coherent single-purpose module; no split points. |
| `comparer.rs` (1087 lines) | Coherent; caching requires co-location. |
| `coordinator.rs` (1300 lines) | Clean single/multi-threaded split. |
| `AtomicMemoryHost` — is it used? | Used in `ThreadingIntrinsicHost` alias; but still a naming shim. See F-DEAD-002. |
| Error type cleanup completeness | `adc74d1` cleanup is complete. |
| `DashMap` lock ordering | Correct throughout. |
| `#[allow(dead_code)]` suppressions | All 3 types found are legitimate. |
| TODO/FIXME count | 3 total — all valid concerns listed in F-TYPES-002. |
| Dead enum variants | None found (all `StepResult`, `ExceptionState`, `HandlerKind` variants constructed). |
| Re-export chains | All `pub use` re-exports are consumed. |
| `dotnet-build-tools` crate | Fully utilized; all 10 functions have callers. |
| Proc-macro split (macros vs macros-core) | Justified for shared parsing between `build.rs` and proc-macros. |
| `dotnet-cli/build.rs` fixture generation | Necessary complexity; no simpler alternative given external .NET artifacts. |

---

## Refactor Plan

Phases are ordered by risk/effort/impact. Each phase leaves the codebase green after completion.

---

### Phase 1: Documentation Fix-up (Zero risk, 0–2 hours)

**Goal:** Eliminate all broken document references and document the intent of existing extension points.

**Findings addressed:** F-DEAD-004, F-BUILD-002, F-DOC-001, F-DOC-002, F-DOC-003 (partial).

**Steps:**
1.1 Create `docs/p3_s1_trait_inventory.md` OR remove the 8+ references to it in source files. (Removing is the simpler path if no one intends to write the document.)
1.2 Add a note to `docs/BUILD_TIME_CODE_GENERATION.md` documenting `DOTNET_VM_EXTRA_INSTRUCTION_SOURCES` and `DOTNET_VM_EXTRA_INTRINSIC_SOURCES` with examples and a note that they are untested extension points.
1.3 Add stub entries for `dotnet-benchmarks`, `dotnet-build-tools`, `dotnet-simd` to `docs/ARCHITECTURE.md` dependency hierarchy.
1.4 Update the TODO items in `docs/EXCEPTION_HANDLING.md` with brief inline explanations (even 2–3 sentences per item closes the gap without requiring full sequence diagrams).
1.5 Rename the "Running experimental feature smoke tests" section in `check.sh` to "Prototype compilation guards" and add a comment explaining their scope.

**Definition of done:** `check.sh` runs green; no broken doc references in source files; all referenced docs exist.

---

### Phase 2: Remove Vestigial Trait Plumbing (Low risk, 2–4 hours)

**Goal:** Remove the two identified dead/vestigial trait abstractions.

**Findings addressed:** F-DEAD-001, F-DEAD-002.

**Steps:**
2.1 Remove `CallOps<'gc>` from `dotnet-vm-ops/src/ops.rs`. Remove `: CallOps<'gc>` from `VmCallOps` in `dotnet-vm/src/stack/ops.rs`. Remove the empty `impl<'a, 'gc> CallOps<'gc> for VesContext<'a, 'gc> {}` in `context.rs`. Update all `pub use` re-exports of `CallOps`.
2.2 Remove `AtomicMemoryHost<'gc>` from `dotnet-intrinsics-threading/src/lib.rs`. In `ThreadingIntrinsicHost`, replace `+ AtomicMemoryHost<'gc>` with `+ RawMemoryOps<'gc>`. Update all call sites in threading intrinsic handlers to call `RawMemoryOps` methods directly (rename `threading_compare_exchange_atomic` → `compare_exchange_atomic`, etc., within the handler files).

**Definition of done:** `check.sh` green under all feature combinations. No references to `CallOps` or `AtomicMemoryHost` in source.

---

### Phase 3: Error Propagation in Hot Paths (Low risk, 1–2 hours)

**Goal:** Replace panics in execution-hot paths with proper `StepResult::Error` returns.

**Findings addressed:** F-IDIOM-001, F-IDIOM-002.

**Steps:**
3.1 In `context.rs:229`, replace `self.frame_stack.pop().unwrap()` with an explicit match that returns `StepResult::Error(...)` on empty stack. Update the signature of `return_frame` if needed.
3.2 In `executor.rs:62,77`, replace the two `expect("Thread arena not initialized")` panics with graceful error returns from `with_arena`.

**Definition of done:** `check.sh` green. No `unwrap()` or `expect()` in `return_frame()` or `with_arena()`.

---

### Phase 4: `context.rs` Trait-Impl Split (Moderate risk, 3–6 hours)

**Goal:** Break `context.rs` into a core struct file and one or more trait-implementation files to improve navigability and incremental compilation.

**Findings addressed:** F-OVER-001.

**Steps:**
4.1 Extract all `impl VesContext` blocks for external traits from `context.rs` into `context_ops.rs` (or multiple files if preferred). The core `VesContext` struct, its constructor, its internal helpers, and the `Drop` impl remain in `context.rs`.
4.2 Update `stack/mod.rs` to declare the new submodule(s).
4.3 Verify all trait bounds still resolve correctly by running `check.sh`.

**Definition of done:** `check.sh` green. `context.rs` is ≤ 500 lines; all `impl` blocks for external traits live in separate files.

---

### Phase 5: `TypeComparer` — `ResolutionS` by Reference (Low risk, 2–4 hours)

**Goal:** Eliminate 38 redundant `Arc` ref-count bumps in type comparison hot path.

**Findings addressed:** F-TYPES-001.

**Steps:**
5.1 Change all method signatures in `crates/dotnet-types/src/comparer.rs` from `res1: ResolutionS` to `res1: &ResolutionS`. Update all call sites within the file (remove `.clone()` calls).
5.2 Update callers in `crates/dotnet-runtime-resolver/` that call `TypeComparer` methods to pass references instead of clones.
5.3 Run `check.sh`.

**Definition of done:** `check.sh` green. No `.clone()` calls on `ResolutionS` arguments within `comparer.rs`.

---

### Phase 6: Address Open TODOs (Variable risk, ongoing)

**Goal:** Resolve or deliberately defer the three outstanding implementation TODOs.

**Findings addressed:** F-TYPES-002.

**Steps:**
6.1 Implement argv initialization for the entry point in `executor.rs`. Wire `std::env::args()` through to the managed `Main(string[] args)` entry point (ECMA-335 §II.15.4.1.2).
6.2 Encode element type and rank in `RuntimeType` for arrays and vectors (`intrinsics/mod.rs:514`). Define a `RuntimeType::Array { element: Box<RuntimeType>, rank: usize }` variant or equivalent.
6.3 Document span non-ordinal comparison limitation in `dotnet-intrinsics-span/src/equality.rs:320`; add a `TODO` comment with a concrete explanation of what would be needed to implement it.

**Definition of done:** `check.sh` green. At minimum, all three TODOs have either been addressed or replaced with a detailed "will not implement" comment explaining the tradeoff.

---
