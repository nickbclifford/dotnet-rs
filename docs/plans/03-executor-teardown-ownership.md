# Executor teardown ownership and panic safety plan

**Status:** Not started
**Prepared:** 2026-11-01
**Baseline:** `18cf9f03`
**Scope:** Executor/TLS arena ownership, panic-safe registry drain, and the public `THREAD_ARENA` /
`Send` Executor surface. Deferred out of the cross-arena-provenance branch (steps 8.8–8.10)
by maintainer decision on 2026-11-01 after the step agents kept discovering deeper prerequisites.

## Evidence for the remaining ownership findings (post-repair)

1. **TLS collision (was 8.8).** `Executor::new` only checks that `THREAD_ARENA` is empty. After
   `extract_arena`, a second Executor can install its own Box; dropping the first Executor then takes
   the successor's Box with its own `None` registration and fallback first ID. Observed (non-UB
   witness): successor arena's `DropWitness` destroyed while another thread held a valid lease.
   Independent recreation (not recovered originals):
   `executor::executor_teardown_recreation_tests::recreation_extracted_executor_tls_ownership`
   (multithreading; currently `#[ignore]` pending repair; fails with `extracted Executor allowed a
   successor before its own drop`). No-features has no extraction/TLS backend (control:
   `recreation_executor_single_arena_cleanup_control`).
2. **Panic-unsafe drain (was 8.9, source-derived, unreproduced).** `teardown_arena` owns a plain `Box`
   across `ArenaRegistration::unregister_and_drain`. `drain_leases` calls `tracing::warn!` after 100ms
   and the self-held-lease branch calls `eprintln!` before `mem::forget`; a panicking subscriber or I/O
   error would unwind and free the arena before the drain completes. The 5.4 claim that teardown cannot
   panic is therefore not established.
3. **Public surface (was 8.10, source-derived, unexploited).** `Executor: Send` (compile-time checked),
   so it can move to a thread whose TLS slot differs; `gc::arena::THREAD_ARENA` is a public mutable
   `RefCell<Option<Box<GCArena>>>` re-exported from `gc`.

Separately, step 8.6 established retrospective native pre-fix baselines for the 5.3–5.5 tests
by overlaying their post-fix source on `cdb7b642`; those baselines are not evidence that the
remaining ownership findings above were repaired, and were not contemporaneous or Miri runs.

A lease prevents teardown but is not a GC root.

## Proposed direction (not reviewed)

- Fail closed: reject successor Executor creation on the same OS thread until an extracted Executor
  drops; an extracted guard may outlive it. A prototype passed the native test on the ordinary path but
  was discarded unreviewed.
- Make the TLS slot internal and Executor thread-affine (`!Send`), or document and test a reviewed
  alternative; audit public consumers.
- Make drain panic-safe (leak-on-unwind or non-panicking logging) without holding registry locks or TLS
  borrows during drain or leases across scheduler parking.
- Tests, regression-first: guard-first / Executor-first / repeated extraction / real successor teardown /
  panic-in-subscriber, release and join lease holders on every assertion path; native and Miri in both
  feature configurations.

## Completion gates

- Un-ignore the recreation and have it pass; panic-safety tests pass; no new pending-repair ignores.
- Both ceiling scripts and `./check.sh --fast` pass without exclusions.
- Reconcile docs that claim non-panicking teardown (5.4 combined entry, REVIEW F-LIFE-201).

## Known residual risk on the originating branch

Until this plan lands, R08 acceptance is not closed: the extracted-Executor collision and the
drain-panic path are open. The recreation remains in-tree as the single `#[ignore]`d pending-repair
test, and the multithreading cfg ceiling carries +1 for it (477 → 478, maintainer-approved 2026-11-01).
