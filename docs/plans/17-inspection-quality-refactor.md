# Inspection-driven code quality refactor

**Status:** In progress
**Outcome:** Shared semantic contracts replace the identified duplication, and remaining inspection
entries have verified fixes or documented reasons to retain them without weakening runtime safety.

## Problem

The inspection ledger in [REVIEW.md](../../REVIEW.md) identifies repeated intrinsic decoding,
reflection slot selection, memory windows and runtime transitions, plus feature-sensitive Rust
readability and dependency findings. Textual duplicate removal alone can hide rather than remove
representation knowledge. Inspections cover only one feature configuration.

## Scope

The ordered slices below are the approved boundaries. Preserve guest behavior, public APIs,
feature gating and the predicates in [the invariant registry](../INVARIANT_REGISTRY.md). Plans
[15](15-corelib-task-and-thread-pool.md) and [16](16-retire-legacy-continuations.md) govern active
Task, pool, timer and effects paths; the restricted multicast fallback remains. Numeric domain
unification, dependency upgrades and new project-wide lints are outside this refactor.

## Design

Move genuinely shared contracts to their owning semantic boundary and migrate all users together.
Keep distinct representations and lifecycle phases distinct. Use generated support-slot accessors,
not a second registry; keep unsafe operations and their proofs local. Enabled VM paths and tests
must use each slice immediately. The ledger records the disposition of every exported occurrence;
it is supporting evidence, not another architecture plan.

## Slices

### 1.1 Unify intrinsic operand and reflection-slot selection

Complete the byte Min/Max operand decoder in dotnet-intrinsics-core and the validated reflection index-slot selector in dotnet-intrinsics-reflection; migrate both callers of each shared concept and remove the duplicated definitions (aggregate groups 1–2). Preserve integer truncation, pop order, canonical names and read-error versus write-contract behavior. Fold touched qualifications/lifetimes into this change. Add enabled edge/type-error and reflection-wrapper tests. Acceptance: both pairs use one semantic implementation, all four wrapper kinds are tested, crate-scoped clippy/nextest for supported no-feature and MT combinations, slot coverage, fmt and doc drift pass.

### 1.2 Make raw-memory windows and byref layout selection explicit

In dotnet-vm stack/raw_memory_ops_impl, replace groups 25–28 with one checked-range contract used by inline reads/writes, slice copies and pointer-sized accesses, plus one effective frame-home layout selection used by read and write. Migrate every affected caller, retaining separate unsafe operations and local proofs; fold qualifications here rather than churning this file later. Exercise zero length, end boundary, usize overflow, stale homes and managed-byref width through enabled VM paths. Acceptance: no unchecked offset-plus-length in the affected copy helpers, exact BoundsCheck payloads retained, byref provenance preserved; VM clippy/nextest across shared matrix, release clippy, focused pointer Miri, provenance lints and doc drift pass.

### 1.3 Share exception initialization and executor publication contracts

Unify exception message/parameter-name initialization used by ExceptionOps and NativeBackend (groups 22–23), preserving rooting and host-error versus managed-exception boundaries; route both paths through the same field-initialization contract. Consolidate repeated executor finalizer publication (group 12) without merging lifecycle phases. Unify the object-write and address-resolution error translations (groups 20–21) at their semantic boundary; preserve stale-home errors and guest exception messages. Migrate all callers and remove the old branches; retain distinct monitor-poll phases (group 24). Acceptance: enabled sync/effect exception, object init/store, ldarga/ldloca and teardown tests exercise new paths; VM and dependent CLI tests in no-feature, MT and MT+validation-all, clippy, focused Miri for changed GC boundaries, provenance/doc drift and release clippy pass.

### 1.4 Centralize event transitions and value-type allocation accounting

In dotnet-vm threading/events, give locked event state explicit expire-waiters and deliver-auto-reset-signal transitions, used by both signal and cancellation/cleanup paths (groups 30–31). In stack/stack_ops_impl, consolidate value-type write allocation accounting at an item-gated semantic helper used by every existing write (group 29), preserving exactly-once metrics and GC debt. Include callers, enabled timeout/cancel/handoff and value-type-write tests and affected synchronization docs together. Acceptance: timeout ordering, pending-signal retention and no double accounting demonstrated in native tests; VM no-feature/MT/MT+validation-all tests and clippy, bench-instrumentation guards, release clippy, Loom and doc drift pass.

### 2.1 Give native-effect tests a coherent boundary-observation API

Replace scattered BOUNDARY_DROPS/RESUMES/NEXT reset, increment and observation plumbing in native_substring_tests with a small thread-local boundary-observation abstraction whose reset/snapshot/event methods express the test protocol; migrate all existing tests in this slice, preserving observable ordering and drop timing. Convert other exported trivial LocalKey<Cell> reads/writes to get/set/replace/take in their owning test/production modules, without forcing RefCell hooks or read-modify-write closures into fake helpers. Fold the touched test qualifications in. Acceptance: all 75 exported TLS sites resolved or individually justified, isolation and unwind/drop assertions retained, enabled native substring/return/teardown tests run in no-feature, MT and MT+validation-all, affected value/utils/vm-data crate tests, clippy and doc drift pass.

### 2.2 Finish the feature-aware Rust readability sweep

Resolve remaining exported qualifications and Rust idioms after structural slices: local imports of recurring domain types, assert_eq/assert_ne diagnostics, the two support.rs casts, both parenthesis pairs, simple lift expressions and genuinely uninformative lifetime names. Preserve named arena input/output contracts, expansion hygiene and sync-facade identity. Retain trait impl semantic grouping and macro-mutated bit_index with recorded reasons. Rename only the three standard-atomic fetch_update sites supported by the pin; retain the Loom-facade site. Acceptance: update every touched ledger row, no broad imports/allows or weakened invariants; fmt and full shared-feature clippy/nextest, optional diagnostic/instrumentation guards, release clippy, macro regression probes where token output changes, and doc drift pass. No new project-wide lint without proving a clean full matrix.

### 3.1 Remove unused direct dependency edges without upgrading APIs

Remove the four unused direct edges in dotnet-benchmarks (tracing, tracing-subscriber), dotnet-value (lru), and dotnet-cli (dotnet-value), including feature-forwarding references that exist only to propagate an otherwise unused edge; preserve real dependency feature propagation through actual consumers. Check all targets, build scripts, macro expansion and standalone fuzz manifests. Do not upgrade any dependency. Acceptance: cargo tree -e features confirms surviving feature routes, all affected targets build across shared and optional feature guards, CLI/benchmark smoke and crate tests pass, full matrix clippy/nextest and fmt pass; record all twelve major-version notices as scoped follow-ups in docs/plans/README.md Not yet planned with upstream evidence.

### 3.2 Close the inspection ledger and durable plan with reproducible checks

Complete the disposition ledger for every export entry and all 32 aggregate duplicate groups; retain the intentional __rerun_probe build fixture, standalone fuzz crates, generated output and every valid workflow input. Add durable IDE-scope guidance to the appropriate contributor/fuzz docs only where useful (attach fuzz manifests, exclude target/obj; no source suppressions). Record unrelated sync-facade and dependency follow-ups under Not yet planned, reconcile the registered plan exit conditions, and mark its Status complete only on evidence. Acceptance: no missing ledger rows/anchors, all planned fixes have enabled caller/test evidence, all retained entries have reasons; run ./check.sh --full, cargo fmt --check, CI release-clippy configurations, doc/provenance/build-script/slot checks and the relevant Miri/Loom lanes from changed paths. Recheck inspection-equivalent lints and manually audit dependency features if udeps/machete unavailable. Record unavailable lanes honestly; do not call skipped checks passes.

## Done when

Every slice's stated acceptance checks pass, every export and aggregate group has an evidenced
disposition, and the enabled application paths remain covered. Run the full validation and safety
lanes specified in the closure slice; report unavailable lanes rather than treating them as passes.

## Ask the maintainer only if

A change requires a guest-behavior or public-compatibility decision, a safety weakening, or a
change to the approved slice boundaries.
