# CI Architecture and Local Validation

This document describes the CI workflows for `dotnet-rs`, why they are split, and how to run equivalent checks locally.

## Test hierarchy and migration

**Agreed policy, 2026-11-02; steps 1-2 of the workflow implementation are in YAML (hosted effect unverified), steps 3-5 pending.** Shorten the time to a useful
answer while preserving checks for meaningful runtime regressions. The workflow inventory
below describes current YAML. Step 2 changed only when work runs; test selection, path
filtering, caches, timeouts and matrices are unchanged.

| Layer | When | Signal and completion |
|---|---|---|
| Fast core | Every PR; give these jobs runner capacity first | Format/source policy, core build/lint/native behavior, cheap library safety checks and the small Loom STW model |
| Targeted deep checks | Before accepting changes to the relevant unsafe, ownership, GC, threading, interpreter or interop paths | Affected library/VM Miri, confinement tests, corpus replay, ordering/lifecycle controls and feature-specific behavior |
| Full sweeps | Scheduled and manually available; before accepting broad storage/access/dispatch changes | Full feature matrix and passing pointer/assembly/VM diagnostic selections catch interactions outside a narrow selection |
| Investigation | A named failure or uncertainty | Guarded failing reproducer, extended fuzzing, Valgrind, profiling or supplemental model/toolchain experiments |

The target for ordinary fast feedback is **10–15 minutes**; relevant deep checks target
**20–35 minutes**. These are goals, not measured achievements or permission to cut a necessary
check off at that time. Full diagnostics may take an hour or longer outside ordinary PR
feedback. Report fast-check completion separately from deep and advisory completion.

### Selection and merge policy

Start with the existing fast configurations: no-default-features, multithreading, and
multithreading,validation-all. Keep the full ten-way xtask matrix for sweeps; select isolated
SIMD, fuzzing, metadata or generic-validation configurations when affected. Combined validation
does not replace tests of feature-disabled or isolated branches. Build fixtures once and share
them; preserve their source-hash checks. Compilation, native execution, Miri, Loom, native ABI
fixtures and leak diagnostics establish different things.

| Changed behavior | Additional pre-acceptance selection |
|---|---|
| Pointer arithmetic, codecs, raw blocks, stack/frame homes | Affected value/data/owner Miri, native semantic regressions and pointer controls; relevant pointer/raw-memory corpora |
| GC roots, arena registry/ownership or executor lifecycle | Relevant cross-arena/teardown/root controls in both applicable modes, VM Miri, STW model and executor replay where affected |
| Native continuations or intrinsic registration | Ops confinement/slot/root Miri, real ST bridge/progress/cancellation and ST/MT legacy table tests; fuzz-budget controls for terminal cleanup |
| Shared access, atomics, broad storage or dispatch | Full affected pointer/value/VM suites and feature configurations, ordering/overlap models and threaded tests; native FFI evidence if aliases can escape |
| Native ABI, marshalling or pinning | Native fixture calls for the affected ABIs and lifetimes; Miri of Rust-only primitives does not replace libffi evidence |
| Assembly/type loading, metadata, generics or SIMD | Relevant fixtures, isolated feature legs and the unsafe dependency paths actually exercised |
| Documentation/source-policy only | Drift/queue/link checks and native policy regressions; no VM execution campaign |

Implement selection as reviewed path/dependency families with explicit selected checks and
reasons. A change to a shared helper selects its consumers; a change to selection/workflows
selects tests of the selection and guard logic. Unknown or ambiguous runtime impact selects
broader coverage. An unrecognized path must not silently skip safety checks. Zero matching or
unexpectedly ignored tests fail selected lanes. Clearly distinguish a deliberately not-required
lane from an attempted check that failed to execute. Full sweeps supplement targeted acceptance;
unsafe changes must not rely solely on a later scheduled run.

Use a fast-result aggregate and a separate selected-safety result. Each must fail on missing,
failed or cancelled required work. Review their coverage before configuring branch protection.
Read-only inspection on 2026-11-02 returned “Branch not protected” for main and no applicable
branch rules. “Blocking” in the current inventory means pipeline-failing, not proof of an
administratively required merge check. No branch setting has been changed.

### Implementing the hierarchy

Make the next CI change concrete and bounded:

1. **Implemented; hosted effect unverified.** Keep `stack_value_cross_arena_retired` as a native source-policy regression and exclude its
   workspace text scan from Miri. Preserve value runtime and layout coverage. Remove the
   duplicate default value-Miri execution from the advisory workflow.
2. **Implemented; hosted effect unverified.** Let fast jobs complete before starting large fleets. Moving workflow names alone is not
   capacity isolation: remove ordinary push/PR launches of the full advisory Miri/Fuzz/Valgrind
   fleets, retain their scheduled/manual paths, and gate selected deep work after fast feedback.
   Native tests must not sit behind 66 pointer singleton jobs in the runner queue.
   Implementation: `miri.yml`, `fuzz.yml` and `valgrind.yml` now trigger only on `schedule` and
   `workflow_dispatch` (Valgrind gained dispatch). In `ci.yml`, the `fast-core` aggregate needs
   the twelve fast-layer jobs; `miri-cross-arena`, `miri-pointer-controls` and
   `fuzz-corpus-replay` need `fast-core`, so they start only after fast feedback succeeds and
   are skipped if it fails; the `deep-safety` aggregate reports on the deep layer. Neither
   aggregate is configured as a required check; that remains a later reviewed decision.
   Consequences: PRs no longer receive the advisory Miri, fuzz-campaign or Valgrind runs (they
   run nightly and on manual dispatch), and the full green wall time can be longer than before
   because deep work now starts after `fast-core`. That is the intended tradeoff: the fast
   answer comes first. The 30-60 minute feedback improvement remains an estimate, not a measurement.
3. Keep cheap raw-block/frame/owner tests in the ordinary core. Select expensive VM controls
   and corpus replay for affected code; retain the complete passing catalogs in full sweeps.
   The retired native refactor's slot/bridge tests are unmerged evidence. If a redesigned path
   adopts them, give them a coherent selection rather than appending them to pointer shard 1.
4. Amortize discovery/build/setup by crate, toolchain and feature configuration. Discover the
   complete catalog once per configuration, preserving independent manifest/ignore guards.
   Batch cheap checks and share compatible caches; retain individual execution diagnostics
   for expensive or failure-sensitive tests. Grouping jobs does not remove per-test VM work.
5. Verify representative selection cases, shared-dependency fallback, missing/ignored tests,
   failures and cancellation with the existing guard harnesses and bounded local checks.
   Measure the next available hosted candidate's fast, selected-deep and all-advisory elapsed
   times separately. If hosted evidence is unavailable, leave latency targets unverified.

Stop when that behavior is reviewable, regression coverage is accounted for, and job selection
and result reporting agree. Workflow changes land in bounded steps (1-2 are implemented as noted above; steps 3-5 are
not). No runtime repair, new test platform or automatic timeout
increase is part of this CI slice. Do not trigger/retry hosted workflows without authorization.

### Costs, retained signal and risks

The October 31 accepted [CI run](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062)
took 68m03s and its [advisory Miri run](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847)
took 83m03s. Their 99 and 57 jobs are that SHA's counts, not the current branch's job count.
Native test jobs started after 56–64 minutes although fixture/matrix dependencies finished in
about a minute; individual pointer jobs took about 7–20 minutes. CI plus Miri peaked at 20
simultaneous running jobs. Runner contention is supported; account concurrency settings were
not independently established. The November 2 [push CI](https://github.com/nickbclifford/dotnet-rs/actions/runs/41367145909)
took 87m39s; value-Miri exhausted its budget in the source scan after its library tests passed.

The accepted CI/Miri jobs occupied about 1,411 runner-minutes, including 876 pointer and 473
advisory Miri runner-minutes. These sums are resource use, not workflow wall time or billing.
Queueing, setup, compilation and interpretation must be measured separately. Cache hits cannot
remove interpreter execution cost. The existing 45/70-minute limits are caps, not runtimes.
A 30–60-minute reduction in ordinary feedback delay is an estimate from removing queue pressure,
not a benchmark of an implemented hierarchy. See the [acceptance record](ci-evidence/2026-10-31-acceptance.md)
for phase measurements and their sampling limits.

Retain the small exhaustive three-thread Loom model for a distinct concurrency signal, guarded
corpus replay for deterministic regressions, and relevant Miri for unsafe boundaries. Keep
Valgrind for native leak/uninitialized-memory evidence that leak-ignored Miri cannot provide.
Pure source scans run natively. Consolidate cheap metadata/string selectors after reviewing
whether their unsafe dependencies add a distinct Miri signal. Longer fuzzing remains exploratory;
triage saved crashes and promote useful regressions instead of counting a green advisory job.

The principal risk is delayed discovery of interactions outside a selected family. Broad shared
changes therefore require full affected suites before acceptance, and full scheduled sweeps
retain coverage. Missing setup/count evidence, cancellations and advisory failures remain visible.

## Current workflow reference

The triggers and matrix sizes below describe current YAML. Commands are references
for selected work, not instructions to launch every lane after every step.

## Workflow Table

| Workflow file                    | Toolchain | Blocking?                | Trigger                        | Purpose                              |
|----------------------------------|-----------|--------------------------|--------------------------------|--------------------------------------|
| `.github/workflows/ci.yml`       | stable / pinned nightly | Yes | push/PR to `main`/`master` + manual dispatch | `fast-core` and `deep-safety` aggregates (deep jobs wait for `fast-core`); source policy, format, strict-provenance lints, clippy, test, exhaustive three-thread Loom STW model, `miri-value` (full default-feature value suite plus no-features M1a, M1b, M1d and atomic selectors), `miri-frame-blocks` (RawBlock and FrameAllocations library tests), `miri-owner-retention` (stack owner tests), `miri-pointer-controls` (33 filtered VM controls), and all four fuzz-corpus replay gates |
| `.github/workflows/fuzz.yml`     | stable-built driver / pinned nightly | No (`continue-on-error`) | daily cron + manual dispatch (no push/PR) | Fuzzing coverage                     |
| `.github/workflows/miri.yml`     | nightly-2026-05-27 | No (`continue-on-error`) | daily cron + manual dispatch (no push/PR) | UB and memory-safety checks          |
| `.github/workflows/valgrind.yml` | stable    | No (`continue-on-error`) | daily cron + manual dispatch (no push/PR) | Leak and uninitialized-memory checks |

## Branch dispatch and targeted repeats

When a hosted run is authorized, assign its actual branch to CI_REF before using these
commands. Current dispatch selects whole workflows, not individual jobs:

```bash
gh workflow run ci.yml --ref "$CI_REF"
gh workflow run miri.yml --ref "$CI_REF"
gh workflow run fuzz.yml --ref "$CI_REF"
gh workflow run valgrind.yml --ref "$CI_REF"
```

Alternatively observe the PR-triggered run for the exact commit. Use `gh run view RUN --json headSha,jobs,url` to verify the SHA and job results,
`gh run watch RUN --exit-status --interval 60` to monitor, and `gh run view RUN --job JOB --log` for
execution evidence. A targeted `gh run rerun RUN --job JOB` repeats a job on that
run's original SHA, not a newer commit. Dispatch selects a workflow, not individual jobs.
Avoid another push while useful evidence is running: concurrency can cancel older runs.
Advisory success alone does not prove that all tests completed; inspect job and step logs.

## `ci.yml` — Blocking Correctness Gate

`ci.yml` runs the main test suite across a matrix of feature combinations to ensure that validation logic does not regress. See [Validation Features](VALIDATION_FEATURES.md) for a detailed breakdown of what each feature validates.

Jobs are listed below. Two aggregate jobs follow the list: `fast-core` and `deep-safety`.
The fast layer is `doc-lint`, `format`, `loom`, `provenance-lints`, `matrix-definitions`,
`clippy`, `build-script-regression`, `build-fixtures`, `test`, `miri-value`,
`miri-frame-blocks` and `miri-owner-retention`. The deep layer (`miri-cross-arena`,
`miri-pointer-controls`, `fuzz-corpus-replay`) has `needs: [fast-core]`, so it starts only
after fast feedback succeeds and is skipped when it fails.

Jobs:

1. `doc-lint` (Documentation and Source Policy Checks): enforces the multithreading cfg-occurrence budget with `scripts/check_mt_cfg_ceiling.sh` and the direct-`std::sync` ceiling with `scripts/check_std_sync_ceiling.sh`; runs `cargo run --quiet -p xtask -- verify-slots` to audit real support-slot consumers; runs `scripts/check_doc_drift.sh` (doc-to-code and plan-queue-status drift detector); and runs a broken intra-doc-link check (`RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps --no-default-features`, with `DOTNET_SKIP_BUILD=1`)
2. `format`: `cargo fmt --all -- --check`
3. `provenance-lints`: pinned-nightly (`nightly-2026-05-27`), blocking `scripts/check_provenance_lints.sh`, which denies `fuzzy_provenance_casts` and `lossy_provenance_casts` in every first-party crate (workspace with no, default and all features, plus both fuzz crates) by injecting `-Zcrate-attr=feature(strict_provenance_lints)`. This is a source-level gate on `as` casts between pointers and integers, not strict-provenance Miri evidence; see [Strict-provenance deferral](#strict-provenance-deferral)
4. `loom`: stable, blocking exhaustive Loom model of the STW protocol. It runs exactly `RUSTFLAGS="--cfg loom" cargo nextest run -p dotnet-vm --test loom_stw --no-default-features`; `RUSTFLAGS` selects the rustc cfg (rather than a nonexistent nextest `--cfg` option). The shared model builder is fixed at three total threads (the collector/model thread and two spawned mutators) and leaves preemption, permutation, and duration exploration uncapped.
5. `matrix-definitions`: resolves the clippy/test feature matrices from `xtask`; the `clippy` and `test` jobs depend on it
6. `clippy`: feature matrix resolved from `xtask`
   - source of truth: `cargo run --quiet -p xtask -- matrix clippy-features --format json`
   - CI sets `DOTNET_SKIP_BUILD=1` for this job as an explicit analysis-only guardrail
7. `build-script-regression`: targeted probes for build-script skip/env/rerun invalidation behavior
8. `build-fixtures`: uses `xtask` to resolve fixture output path and compute the fixture cache key from the same input set used by `dotnet-cli/build.rs` (`.cs` fixtures, fixture `.csproj` files, and shared MSBuild/NuGet config candidates)
9. `test`: feature matrix resolved from `xtask` (`cargo run --quiet -p xtask -- matrix test-features --format json`); its `multithreading` leg also runs the hang-probe integration-test step
10. `miri-value`: pinned-nightly, blocking full `dotnet-value` Miri suite with default features (multithreading) (the source-policy scan test is cfg-excluded under Miri), plus exact no-features runs of the three `m1a_`, eight `m1b_` and eleven `m1d_` library regressions and the five `atomic_tests::` tests; the no-features steps are not a full-crate Miri suite
11. `miri-frame-blocks`: pinned-nightly, blocking Miri tests for `RawBlock` and `FrameAllocations` in no-features and multithreading configurations (details below)
12. `miri-owner-retention`: pinned-nightly, blocking `dotnet-vm-data` stack owner-retention tests in both configurations (details below)
13. `miri-pointer-controls`: pinned-nightly, blocking filtered VM Tree Borrows controls (details below); this is not a full or strict VM Miri suite
14. `fuzz-corpus-replay`: pinned-nightly, blocking replay of each committed fuzz corpus (`-runs=0`) with `cargo-fuzz` 0.13.1: `fuzz_managed_ptr_roundtrip`, `fuzz_managed_ptr_offset`, `fuzz_raw_memory_access`, and `fuzz_executor`
15. `fast-core`: aggregate over the twelve fast-layer jobs listed above; `if: always()`, and it fails unless every needed job's result is `success` (failed, cancelled and skipped all fail), printing the offending jobs. Not a required check.
16. `deep-safety`: aggregate over `fast-core`, `miri-cross-arena`, `miri-pointer-controls` and `fuzz-corpus-replay`; `if: always()`, same all-must-succeed rule. If `fast-core` did not succeed it fails with a "not executed" message, which is distinct from the failure of an attempted deep job. Not a required check.

The blocking `miri-value` job runs the full `dotnet-value` test suite with the crate's default
features, which include `multithreading`; the `m1a_`, `m1b_`, enabled `m1d_` and `atomic_tests::` library regressions are part of
that suite, including multithreading-only cross-arena tests. The one integration test, the
`stack_value_cross_arena_retired` source-policy scan of workspace `.rs` files, is `cfg(not(miri))`
and so not run (or listed as ignored) under Miri; it runs natively in the `test` job's no-features and
`multithreading` legs. Locally on 2026-11-02 the scan alone took 807s under Miri on main, while the whole remaining
`dotnet-value` Miri suite took about 30s including compilation; relief of the hosted 15-minute budget is unverified. Four further steps run the following selectors with the same job-level pinned `MIRIFLAGS`
and check the logs for exactly three, eight, eleven and five passing tests respectively, with zero ignored:

```bash
cargo +nightly-2026-05-27 miri test -p dotnet-value --lib --no-default-features \
  pointer::tests::m1a_ -- --test-threads=1
cargo +nightly-2026-05-27 miri test -p dotnet-value --lib --no-default-features \
  pointer::tests::m1b_ -- --test-threads=1
cargo +nightly-2026-05-27 miri test -p dotnet-value --lib --no-default-features \
  pointer::tests::m1d_ -- --test-threads=1
cargo +nightly-2026-05-27 miri test -p dotnet-value --lib --no-default-features \
  atomic_tests:: -- --test-threads=1
```

This adds no-features coverage for those regressions, not a second full-crate Miri configuration. The
job does not enable `memory-validation`, so its conditional `m1b_` ignore does not apply.

### Blocking `miri-frame-blocks` lane

This blocking library-test lane uses pinned `nightly-2026-05-27` with
`MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"` and a
15-minute job timeout. It runs the `raw_block::` tests in `dotnet-utils` and the `frame_alloc::`
tests in `dotnet-vm-data`, each with no features and with `multithreading`; every run is checked
for exactly seven raw-block tests or eight frame-allocation tests passing, with zero ignored.
The latter includes frame suspension/restoration at the data-structure level. The lane checks these
two components only: it is not VM Miri coverage, does not establish strict provenance, and is not a
soundness proof.

### Blocking `miri-owner-retention` lane

This library lane uses pinned `nightly-2026-05-27`,
`MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"` and a
15-minute timeout. It runs `dotnet-vm-data` `stack::tests::m1d_` with no features and
with `multithreading`, requiring eight passed and zero ignored in each run. It is not a VM
Miri suite, strict-provenance evidence, or a soundness proof.

### Blocking `miri-cross-arena` lane

The independent recreations (not recovered originals) run under pinned
`nightly-2026-05-27` and `-Zmiri-tree-borrows -Zmiri-disable-isolation
-Zmiri-ignore-leaks`, with separate no-features and multithreading jobs. Each
runtime singleton has a 900-second diagnostics deadline and requires exactly
one passed, zero failed and zero ignored. The workflow is the exact selector
manifest; diagnostics are summarized and uploaded even on failure.

Both configurations run the value publication model and origin ABA recreation,
a bounded Executor teardown, a collection and a reduced teardown stress. MT
uses the ID40000 and registered-boundary tests, actual active-lease teardown,
and the three-arena fixed-point payload/reclamation test. No-features instead
uses the local untagged codec and short-storage controls, single-arena teardown
and collection controls; compact owner packing, registry, cross-arena transport
and real multi-arena collection are absent there. The job checks utils compiles
and confirms the transport and registry selectors are absent in that configuration.
MT separately runs the structural and runtime transport tests and all six registry
recreations. An additional isolated value transport-core singleton uses strict
provenance; it is not a whole-VM strict-provenance run. The ordinary Miri runs
ignore leaks and do not prove soundness or leak-freedom. The native feature matrix
runs the real collection and stress tests without a Miri-only substitute. The
MT stress covers normal/extracted teardown with held/released leases (20 native
rounds, four under Miri); a timed-out worker can still detach. The foreign-branded
fixture concern and the deferred [Executor ownership issues](plans/03-executor-teardown-ownership.md) remain open.

The current manifest has **7 no-features and 15 multithreading permissive
singletons**, plus one MT-only strict-core singleton. Each row below is one exact
`--lib SELECTOR -- --exact --test-threads=1` invocation; `V`, `U`, and `VM`
mean `dotnet-value`, `dotnet-utils`, and `dotnet-vm` respectively.

| Configuration | Crate | Exact selector |
|---|---|---|
| no-features | V | `object_tests::tests::recreation_codec_local_invalid_control` |
| no-features | V | `object::tests::recreation_codec_short_storage` |
| no-features | VM | `executor::executor_teardown_recreation_tests::recreation_executor_single_arena_cleanup_control` |
| no-features | VM | `executor::executor_collection_recreation_tests::m1e_recreation_vm_collection` |
| both | V | `object_tests::tests::recreation_publication_pending_survival` |
| both | V | `pointer::tests::m1g_recreation_origin_aba` |
| both | VM | `executor::executor_teardown_recreation_tests::m1e_recreation_teardown_stress` |
| multithreading | V | `object_tests::tests::recreation_codec_id40000` |
| multithreading | V | `object_tests::tests::recreation_codec_registered_boundaries` |
| multithreading | VM | `executor::executor_teardown_recreation_tests::recreation_executor_normal_teardown_active_lease` |
| multithreading | VM | `executor::executor_collection_recreation_tests::m1e_recreation_vm_collection_fixed_point_payload_and_reclamation` |
| multithreading | U | `gc::cross_arena::tests::m1e_recreation_transport` |
| multithreading | U | `gc::cross_arena::tests::m1e_recreation_transport_permissive_control` |
| multithreading | U | `gc::cross_arena::tests::m1g_recreation_registry_duplicate` |
| multithreading | U | `gc::cross_arena::tests::m1g_recreation_registry_fast_bit_transaction` |
| multithreading | U | `gc::cross_arena::tests::m1g_recreation_registry_generation_exhaustion` |
| multithreading | U | `gc::cross_arena::tests::m1g_recreation_registry_identity_token_reuse` |
| multithreading | U | `gc::cross_arena::tests::m1g_recreation_registry_drain_reuse_control` |
| multithreading | U | `gc::cross_arena::tests::m1g_recreation_registry_checked_counter_control` |
| multithreading, strict-core | V | `pointer::tests::m1e_recreation_transport_strict_core` |

The job has a 45-minute outer budget, reserves three minutes for diagnostics, and
uses 600 seconds for sysroot setup and 900 seconds per compile, absence check,
and singleton. The strict-core invocation appends `-Zmiri-strict-provenance`;
all results are uploaded in the configuration-specific diagnostics artifact.
The 22 permissive and one strict invocations passed locally in step 7.1; hosted
execution and branch protection are not established by those local runs.

### Blocking `miri-pointer-controls` lane

This job gates 33 enabled VM controls, not the full VM suite. It uses
`nightly-2026-05-27`, `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`,
and two configurations (`no-features`, `multithreading`) with 33 singleton shards each.
The exact manifest from `scripts/pointer_provenance.sh` is:

| Shard | Exact test |
|---|---|
| 1 | `pointer_provenance_tests::p00_smoke` |
| 2 | `pointer_provenance_tests::p02_ldloca_stind_i4_control` |
| 3 | `pointer_provenance_tests::p03_ldloca_convu_ldind_i4` |
| 4 | `pointer_provenance_tests::p05_single_localloc_roundtrip` |
| 5 | `pointer_provenance_tests::p07_localloc_one_past_end` |
| 6 | `pointer_provenance_tests::p11_ldelema_stind_heap_array_control` |
| 7 | `pointer_provenance_tests::p10_ldelema_convu_stind_heap_array` |
| 8 | `pointer_provenance_tests::p20_heap_convu_then_typed_write_then_raw_store` |
| 9 | `pointer_provenance_tests::p13_ldelema_managed_add_far_out_of_bounds_then_conv_u` |
| 10 | `pointer_provenance_tests::p14_initblk_heap_array_via_managed_ptr` |
| 11 | `pointer_provenance_tests::p16_cpblk_heap_arrays_via_managed_ptr` |
| 12 | `pointer_provenance_tests::p_null_through_pointer_local_address` |
| 13 | `pointer_provenance_tests::p06_localloc_minus_one_then_plus_one` |
| 14 | `pointer_provenance_tests::p08_localloc_overshoot` |
| 15 | `pointer_provenance_tests::p17_localloc_ptr_ceq_self` |
| 16 | `pointer_provenance_tests::p18_localloc_ptr_clt` |
| 17 | `pointer_provenance_tests::p21_localloc_signed_extreme_offsets_roundtrip` |
| 18 | `pointer_provenance_tests::p22_localloc_wrap_to_zero_roundtrip` |
| 19 | `pointer_provenance_tests::p23_localloc_pointer_loop_unsigned_branch_compare` |
| 20 | `pointer_provenance_tests::p24_unsafe_byte_offset_direction_and_deref` |
| 21 | `pointer_provenance_tests::p25_convi_and_stind_i_pointer_roundtrip_deref` |
| 22 | `pointer_provenance_tests::p26_native_int_boundary_store_load_roundtrip` |
| 23 | `pointer_provenance_tests::p27_stind_i_fresh_pointer_through_local_slot_dereferences` |
| 24 | `pointer_provenance_tests::p28_stind_i_fresh_pointer_through_heap_memory_dereferences` |
| 25 | `pointer_provenance_tests::p04_double_localloc_write_first` |
| 26 | `pointer_provenance_tests::p29_localloc_blocks_survive_growth_and_are_16_byte_aligned` |
| 27 | `pointer_provenance_tests::p30_localloc_block_survives_nested_callee_localloc` |
| 28 | `pointer_provenance_tests::p31_localloc_blocks_released_on_return_and_pooling` |
| 29 | `pointer_provenance_tests::p32_localloc_blocks_released_on_tail_call_replacement` |
| 30 | `pointer_provenance_tests::p33_localloc_blocks_released_on_exception_unwind` |
| 31 | `pointer_provenance_tests::p34_localloc_blocks_released_on_executor_teardown` |
| 32 | `pointer_provenance_tests::p35_ldflda_on_stack_value_type_then_ldind` |
| 33 | `pointer_provenance_tests::p36_static_address_survives_forced_collection` |

Each leg runs `bash scripts/pointer_provenance.sh verify-controls --configuration <configuration>`
then `bash scripts/pointer_provenance.sh controls --configuration <configuration> --shard <1..33>`.
Discovery and partition guards require 33 non-ignored controls; each shard requires one pass and zero
ignored. The cache key is `miri-pointer-v2-nightly-2026-05-27-<configuration>-<shard>`.
The lane does not enable `memory-validation`: p13 and the separate
`conv_u8_after_far_oob_managed_add_does_not_panic` regression are conditionally ignored with
that feature because their deliberate 4096-byte managed offsets trigger its debug bounds trap.

The earlier M1c `./check.sh --fast` run passed in 88.442 s, with nextest reporting 554 passed / 9
skipped (no features), 625 / 7 (multithreading), and 639 / 9 (multithreading,validation-all).

This lane does not execute the remaining ignored probes, establish strict provenance, cover all unsafe
VM paths, or imply runtime soundness. Ignored-probe execution is restricted to the diagnostic
entrypoint documented below. The broader `miri.yml` VM jobs remain advisory.

Hang probes use tighter timeouts and run these filters individually:

- `integration_tests_impl::fixtures::test_allocation_pressure`
- `integration_tests_impl::fixtures::test_gc_coordinator`
- `integration_tests_impl::fixtures::test_multiple_arenas`
- `integration_tests_impl::fixtures::test_stw_stress`

### Feature Ownership (Post-Extraction)

Feature forwarding in `dotnet-vm` is intentionally limited to crates that actually gate source code on that feature:

| Feature | Crates Owning `#[cfg(feature = ...)]` Code |
|---|---|
| `multithreading` | `dotnet-vm`, `dotnet-runtime-resolver`, `dotnet-runtime-memory`, `dotnet-pinvoke`, `dotnet-tracer`, `dotnet-utils`, `dotnet-value` |
| `memory-validation` | `dotnet-vm`, `dotnet-runtime-memory`, `dotnet-utils`, `dotnet-value` |
| `fuzzing` | `dotnet-vm`, `dotnet-pinvoke`, `dotnet-types`, `dotnet-utils`, `dotnet-value` |

## Local Equivalent

During iteration run focused checks for the changed behavior. `./check.sh --fast` is the current
three-configuration integration pass; run it once for a coherent implementation outcome, not
for every checklist item. `--full` runs the full feature matrix and additional guards for broad
changes/sweeps. Neither script yet implements automatic change selection.

### Build/Fixture Environment Contract

These env vars define the build-script behavior for analysis-only and fixture workflows:

| Variable | Used by | Effect |
|---|---|---|
| `DOTNET_SKIP_BUILD=1` | `dotnet-assemblies/build.rs`, `dotnet-cli/build.rs` | Skip dotnet restore/build and use stub/metadata-only paths for analysis commands (`dotnet-cli` applies this in non-prebuilt mode). |
| `DOTNET_USE_PREBUILT_FIXTURES=1` | `dotnet-cli/build.rs` | Do not run dotnet restore/build for fixtures; require prebuilt fixture tree and validate `.fixtures_hash`. |
| `DOTNET_FIXTURES_BASE=/path` | `dotnet-cli/build.rs` | Read/write fixture artifacts at this path instead of the default Cargo-derived path. |
| `DOTNET_TEST_FILTER=<substring>` | `dotnet-cli/build.rs` | Restrict generated fixture tests to matching fixture paths. |

Precedence in `dotnet-cli/build.rs`:

1. `DOTNET_USE_PREBUILT_FIXTURES=1` takes priority; build.rs validates prebuilt artifacts/hash and never runs dotnet build/restore in that mode.
2. Otherwise build.rs compares fixture hash and only runs dotnet build/restore when artifacts are stale and skip mode is not active.
3. `cargo clippy` auto-enables skip behavior in both dotnet build scripts via `clippy-driver` wrapper detection (`RUSTC_WORKSPACE_WRAPPER`/`RUSTC_WRAPPER`/`CLIPPY_ARGS`).

### Building Fixtures Locally

If you are working on the VM and don't want to wait for .NET compilation during every `cargo test` run, you can build the fixtures once:

```bash
cargo run -p xtask -- fixtures build
```

The convenience script is a thin wrapper over the same xtask entrypoint:

```bash
./scripts/build_fixtures.sh
```

Then run tests using the prebuilt fixtures:

```bash
DOTNET_USE_PREBUILT_FIXTURES=1 cargo test
```

`xtask fixtures build` now writes `.fixtures_hash` next to the compiled fixture tree. When
`DOTNET_USE_PREBUILT_FIXTURES=1` is set, `dotnet-cli/build.rs` validates that hash against
current fixture inputs and fails fast on stale, missing, or incomplete artifacts (including
missing DLLs).

You can also specify a custom output directory:

```bash
cargo run -p xtask -- fixtures build --output-dir /tmp/my-fixtures
DOTNET_FIXTURES_BASE=/tmp/my-fixtures DOTNET_USE_PREBUILT_FIXTURES=1 cargo test
```

Or use Cargo-style output conventions (matching `dotnet-cli/build.rs` path expectations):

```bash
# Default convention:
cargo run -p xtask -- fixtures output-dir
# -> target/debug/dotnet-fixtures

# Non-default profile:
cargo run -p xtask -- fixtures output-dir --profile bench-fat
# -> target/bench-fat/dotnet-fixtures

# Explicit target triple:
cargo run -p xtask -- fixtures output-dir --target x86_64-unknown-linux-gnu
# -> target/x86_64-unknown-linux-gnu/debug/dotnet-fixtures
```

For cache debugging, you can print the same fixture cache key input digest used by CI:

```bash
cargo run -p xtask -- fixtures cache-key --profile debug
```

Historical `crates/dotnet-cli/tests/bin` and `crates/dotnet-cli/tests/obj` directories are local/legacy MSBuild artifacts and are intentionally ignored (via root `.gitignore` `bin/` and `obj/` patterns). They are not used by current Cargo build-script output paths, which are target-tree based (`target/<profile>/dotnet-fixtures` and `target/.../build/.../out`).

### Analysis-Only Commands (`cargo clippy`)

`crates/dotnet-assemblies/build.rs` and `crates/dotnet-cli/build.rs` skip dotnet restore/build work during `cargo clippy` by detecting `clippy-driver` through Cargo wrapper env (`RUSTC_WORKSPACE_WRAPPER`/`RUSTC_WRAPPER`).

If you need to force skip behavior outside clippy (for example a local `cargo check` run), set:

```bash
DOTNET_SKIP_BUILD=1 cargo check --workspace --all-targets
```

### Loom STW Model

Run the independent blocking Loom leg separately from the normal feature matrix:

```bash
DOTNET_SKIP_BUILD=1 RUSTFLAGS="--cfg loom" \
  cargo nextest run -p dotnet-vm --test loom_stw --no-default-features
```

Loom is a rustc `cfg(loom)`, not a Cargo feature: do not add `--features multithreading`
and do not pass a nonexistent `--cfg loom` argument to `cargo nextest`. The model builder has
exactly three total threads (one collector/model thread and two spawned mutators) and leaves
preemption, permutation, and duration exploration uncapped. It is therefore a blocking,
exhaustive check at that deliberately small bound, not a replacement for the normal matrix.

### Full Check Matrix

```bash
# Format
cargo fmt --all -- --check

# Clippy matrix (source of truth from xtask)
readarray -t CLIPPY_FEATURES < <(
  cargo run --quiet -p xtask -- matrix clippy-features --format lines
)
for features in "${CLIPPY_FEATURES[@]}"; do
  if [ -z "$features" ]; then
    cargo clippy --all-targets --no-default-features -- -D warnings
  else
    cargo clippy --all-targets --no-default-features --features "$features" -- -D warnings
  fi
done

# Test matrix (source of truth from xtask)
readarray -t TEST_FEATURES < <(
  cargo run --quiet -p xtask -- matrix test-features --format lines
)
for features in "${TEST_FEATURES[@]}"; do
  if [ -z "$features" ]; then
    DOTNET_TEST_TIMEOUT_SECS=180 cargo test --no-default-features -- --nocapture --test-threads=1
  elif [[ "$features" == *"multithreading"* ]]; then
    DOTNET_TEST_TIMEOUT_SECS=180 cargo test --no-default-features --features "$features" -- --nocapture
  else
    DOTNET_TEST_TIMEOUT_SECS=180 cargo test --no-default-features --features "$features" -- --nocapture --test-threads=1
  fi
done

# Hang probes (multithreading only)
for TEST in \
    "integration_tests_impl::fixtures::test_allocation_pressure" \
    "integration_tests_impl::fixtures::test_gc_coordinator" \
    "integration_tests_impl::fixtures::test_multiple_arenas" \
    "integration_tests_impl::fixtures::test_stw_stress"; do
  DOTNET_TEST_TIMEOUT_SECS=60 timeout 300 \
    cargo test --no-default-features --features multithreading \
      -p dotnet-cli --test integration_tests "$TEST" \
      -- --test-threads=1 --nocapture
done
```

Or run:

```bash
bash check.sh
```

### Build-Script Regression Probes

The repository includes a dedicated probe script that validates three contracts:

1. clean `cargo clippy --workspace --all-targets` does not invoke dotnet/MSBuild work.
2. toggling `DOTNET_SKIP_BUILD` invalidates `dotnet-assemblies` output (`support.dll` stub -> rebuilt non-empty DLL).
3. `dotnet-vm` directory-level `rerun-if-changed` retriggers on add/remove under `src/intrinsics`.

Run locally with:

```bash
bash scripts/check_build_script_regressions.sh
```

### Support-slot Consumer Audit

The generated support-assembly storage ABI has 21 primary accessors. `xtask verify-slots` parses
the committed `crates/dotnet-assemblies/support_slots.def` contract and scans non-test authored
Rust in the VM, reflection, delegate, and span intrinsic consumer roots. It fails closed if a
configured root is missing, empty, unreadable, or unparsable; if a call names a stale/generated
accessor outside the contract; or if any primary accessor has no real runtime consumer. The
intentional bounded Span-or-ReadOnlySpan convenience accessors count as evidence for both of their
separate primary accessors. Alternate generated views are permitted but do not replace required
primary-consumer coverage.

Run the same blocking source-policy check locally:

```bash
cargo run -p xtask -- verify-slots
```

### Documentation Drift Check

The `doc-lint` job runs the doc-to-code drift detector and the broken intra-doc-link check.
Run locally with:

```bash
bash scripts/check_doc_drift.sh
DOTNET_SKIP_BUILD=1 RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" \
  cargo doc --no-deps --no-default-features
```

### Multithreading cfg-Occurrence Budget

Current `doc-lint` and `check.sh` enforce the numeric ceiling in
`scripts/check_mt_cfg_ceiling.sh`. Prefer module boundaries and existing abstractions.
Necessary bounded growth is an ordinary engineering review decision: explain the semantic
boundary and measured delta in the same change, without separate approval for each location.
Do not obscure syntax or delete unrelated code to fit a count; lower the ceiling when the
canonical count falls. The retired native branch's allowances are not changes to main.

```bash
bash scripts/check_mt_cfg_ceiling.sh
```

### Direct `std::sync` Ceiling

Current `doc-lint` and `check.sh` also run `scripts/check_std_sync_ceiling.sh`. This is a lexical
regression check, not proof of synchronization safety. Prefer repository wrappers; justify and
measure necessary bounded changes in the same reviewed implementation. The semantic access,
ordering and unsafe contracts remain mandatory. No repeated maintainer approval is required
solely for a token count. The scripts remain unchanged by this pivot.

```bash
bash scripts/check_std_sync_ceiling.sh
```

## Fuzz corpus replay — Blocking Regression Checks

The four replay paths, stable-built driver pin, count guard and observed executions are documented
in [Blocking Corpus Regression](FUZZING.md#blocking-corpus-regression). They are distinct from the
advisory duration-based campaigns.

## `miri.yml` — Non-Blocking UB Checks

`miri.yml` remains an advisory matrix. It runs only on its daily 02:00 UTC schedule and by manual dispatch; ordinary push/PR no longer launches it. Its `miri` job no longer has a `dotnet-value` entry: that default-feature run duplicated the `miri-value` job in `ci.yml`, which is the only `dotnet-value` Miri gate.

The retained advisory `miri` job has a single `dotnet-utils` no-features leg, which runs the full suite.
The singleton matrix has 38 `dotnet-assemblies` default-feature selections (A01–A38) and the
same eight `dotnet-vm` selections (V01–V08) in each of no-features and multithreading: 54 jobs.
Its Miri discovery guards verify the original assembly suite and VM `jmp_tests`, `tail_calls`,
`fault_tests` union, no duplicates, no ignored selections and empty ancillary listings. Each shard
requires one pass, zero failures and zero ignored. The manifest in
`scripts/miri_advisory_manifest.json` is:

| Shard | Exact library test |
|---|---|
| A01 | `drop_tests::test_assembly_loader_drop_keeps_descriptors_valid` |
| A02 | `drop_tests::test_corlib_wkt_resolution_failures_are_retryable` |
| A03 | `drop_tests::test_exception_dispatch_state_prefers_slash_then_falls_back_to_plus` |
| A04 | `drop_tests::test_memory_extensions_slow_path_signature_uses_read_only_span` |
| A05 | `drop_tests::test_type_description_keeps_arena_alive` |
| A06 | `host::tests::classifies_bundle_signature_as_single_file` |
| A07 | `host::tests::classifies_elf_as_nativeaot` |
| A08 | `host::tests::classifies_managed_pe_with_cli_header` |
| A09 | `host::tests::classifies_pe_without_cli_header_as_nativeaot` |
| A10 | `host::tests::disable_policy_requires_exact_match` |
| A11 | `host::tests::select_framework_version_applies_all_roll_forward_policies_for_10_0_0_request` |
| A12 | `host::tests::short_or_unknown_input_defaults_to_managed` |
| A13 | `support_contract_tests::every_generated_id_and_typed_accessor_resolves_its_prescribed_owner` |
| A14 | `support_contract_tests::generated_runtime_slot_enum_is_in_embedded_support_assembly` |
| A15 | `support_contract_tests::generated_slot_descriptors_preserve_stable_ids_and_exact_shapes` |
| A16 | `support_contract_tests::handle_override_is_limited_to_the_support_descriptor` |
| A17 | `support_contract_tests::loaded_support_metadata_has_an_independent_literal_slot_census` |
| A18 | `support_contract_tests::runtime_type_handle_object_ref_and_usize_views_round_trip` |
| A19 | `support_contract_tests::separate_span_layout_accessors_preserve_their_offsets` |
| A20 | `support_contract_tests::test_registry_slot_count` |
| A21 | `support_contract_validation_tests::duplicate_annotations_report_the_id_and_both_locations` |
| A22 | `support_contract_validation_tests::exact_signature_mismatch_reports_expected_and_found_shape` |
| A23 | `support_contract_validation_tests::generated_runtime_slot_ids_are_required` |
| A24 | `support_contract_validation_tests::misplaced_id_reports_actual_and_expected_locations` |
| A25 | `support_contract_validation_tests::missing_id_reports_its_exact_expected_slot` |
| A26 | `support_contract_validation_tests::runtime_slot_field_without_used_implicitly_is_a_contract_violation` |
| A27 | `support_contract_validation_tests::stub_schema_rejects_duplicate_targets_without_mutating_registered_maps` |
| A28 | `support_contract_validation_tests::stub_schema_requires_exact_instance_string_in_place_of_field` |
| A29 | `support_contract_validation_tests::stub_schema_requires_exactly_one_non_null_in_place_of_named_argument` |
| A30 | `support_contract_validation_tests::unknown_id_is_a_contextual_contract_violation` |
| A31 | `support_contract_validation_tests::wrong_staticness_reports_actual_and_expected_staticness` |
| A32 | `validation_tests::tests::test_empty_method_name` |
| A33 | `validation_tests::tests::test_empty_typedef_table` |
| A34 | `validation_tests::tests::test_invalid_module_name` |
| A35 | `validation_tests::tests::test_self_inheritance` |
| A36 | `validation_tests::tests::test_valid_metadata` |
| A37 | `version_tests::tests::test_version_equality` |
| A38 | `version_tests::tests::test_version_parsing` |
| V01 | `fault_tests::tests::test_fault_handler_executed_on_exception` |
| V02 | `fault_tests::tests::test_fault_handler_skipped_on_normal_exit` |
| V03 | `jmp_tests::tests::test_jmp_inside_try` |
| V04 | `jmp_tests::tests::test_jmp_instruction` |
| V05 | `jmp_tests::tests::test_jmp_invalid_stack` |
| V06 | `jmp_tests::tests::test_jmp_signature_mismatch` |
| V07 | `tail_calls_tests::non_tail_call_chain_grows_frame_depth` |
| V08 | `tail_calls_tests::tail_call_chain_keeps_frame_depth_bounded` |

The cache shared key is `miri-advisory-v2-nightly-2026-05-27-<crate>-<configuration>-<shard>`.
The filters do not prove all cross-arena or stop-the-world paths are dynamically exercised.
The workflow uses `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`.

### Strict-provenance deferral

No current Miri job adds `-Zmiri-strict-provenance`. Plan 01's Phase 9 CI work
was explicitly closed by owner-directed deferral on 2026-08-05; this is an
accepted missing gate, not implied coverage. In particular:

- `dotnet-value` now preserves provenance through object-reference storage and
  pointer-valued atomics. Its ordinary pinned Miri suite passes locally; the
  stricter gate remains deferred because unmanaged address reconstruction and
  other exposed-provenance paths still require separate work.
- `dotnet-runtime-memory` passed its pinned-nightly strict-provenance baseline,
  but it has never had an entry in this advisory matrix and was not added solely
  to satisfy the plan's stale gate wording.
- both `dotnet-vm` entries retain their existing flags and filters unchanged.

The blocking `provenance-lints` job in `ci.yml` (2026-10-31) is a separate,
source-level gate: it denies implicit `as` casts between pointers and integers so
every remaining exposure or reconstruction is spelled `expose_provenance` /
`with_exposed_provenance*`. It does not run Miri and does not reopen this
deferral; exposed-provenance paths remain and are what keep strict Miri red.

Reopening strict-provenance CI requires explicit authorization and a green local
run for each intended leg. Do not enable a known-red matrix entry or infer strict
coverage from the ordinary `miri-value` job in `ci.yml`.

Local commands:

```bash
rustup toolchain install nightly-2026-05-27
rustup component add miri --toolchain nightly-2026-05-27

MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
cargo +nightly-2026-05-27 miri test -p dotnet-value -- --test-threads=1
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
cargo +nightly-2026-05-27 miri test -p dotnet-assemblies -- --test-threads=1
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
cargo +nightly-2026-05-27 miri test -p dotnet-vm --no-default-features -- --test-threads=1 jmp_tests tail_calls fault_tests
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
cargo +nightly-2026-05-27 miri test -p dotnet-vm --no-default-features --features multithreading -- --test-threads=1 jmp_tests tail_calls fault_tests
```

### Hosted evidence and budgets

On 2026-10-31 candidate `b8e98239cec56a4e62c38682c27d3a59886262a0`, attempt 1:
[CI](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062) passed 99/99 jobs,
including 66 pointer shards and blocking library gates; [Miri](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847)
passed 57/57 jobs, including all 54 guarded singleton executions. The later evidence and documentation
commits are not that tested SHA. See the [evidence ledger](ci-evidence/2026-10-31-acceptance.md)
for every job, artifact, counts and phase timings. [Fuzz](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237425991)
was green with an advisory executor panic, not four successful campaigns; the
[crash artifact](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237425991/artifacts/13055518930)
is retained subject to Actions retention. Diagnostic p09 has a prior reported UB; no new UB was reported
in these controls. Neither these results nor the budgets establish runtime soundness.

Measured seconds from diagnostic phases (primary cache hits called warm; miss does not prove an empty
runner). Compile ranges are individual phases; samples do not imply a universal bound:

| Group/cache | n | Setup | Compile | Guard | Execute |
|---|---:|---:|---:|---:|---:|
| pointer/miss | 54 | 12.567–22.198 | 19.192–59.618 | — | 310.545–1039.753 |
| pointer/hit | 12 | 13.095–20.504 | 4.156–34.188 | — | 345.014–820.099 |
| assembly/miss | 32 | 12.703–22.276 | 13.544–33.741 | 5.125–9.782 | 2.001–967.304 |
| assembly/hit | 6 | 15.569–20.285 | 6.803–14.713 | 7.067–9.417 | 2.901–985.964 |
| VM no-features/miss | 8 | 13.641–20.481 | 23.003–39.662 | 15.252–24.454 | 492.103–798.104 |
| VM multithreading/miss | 8 | 15.795–20.397 | 25.939–39.285 | 18.787–27.419 | 376.020–794.122 |

Hits cover only 12 pointer and 6 assembly shards; no VM warm sample. Two earlier cold attempts on
`0bedd2e08dca88078aca896d5afc772492999a91` reached 1658s pointer and 1396s assembly execution,
so the latest sample is not a worst-case bound. Pointer job timeout is 45min, with a 42min shared
inner deadline from diagnostics start, setup cap 600s, discovery and Miri build 1200s each, execute
2700s (also bounded by the remaining deadline and script timeout). Advisory singleton jobs have a
70min job timeout, 67min shared deadline, setup/compile/guard caps 300s each and execute cap 3000s.
Both subtract three minutes for evidence when setting the deadline in the first step; startup time
before that step can reduce the actual upload window, and hard cancellation can prevent upload.
Caps are not additive. The earlier execution maxima are
61% and 47% of their nominal execute caps, respectively, but actual headroom also depends on setup,
compile, guard and the shared deadline. All sampled phases finished within effective budgets; cache
residency and future timing are not guaranteed. The ledger records retained value/utils and focused
library counts, and the nonfatal rustdoc private-link warning.

### VM pointer-provenance harness and lane limits

`scripts/pointer_provenance.sh list` shows the explicit 39-entry catalog, classifications, native
diagnostic permissions and control shards. The 33 controls run as ordinary tests without
`memory-validation`; p13 is ignored with that feature. The M1c controls include p04 and p29–p34;
six probes remain ignored and diagnostic-only. The blocking `miri-pointer-controls` job and its
two-configuration, 33-singleton-shard mapping are documented above. Its permissive Tree Borrows results
are not a full VM Miri suite, strict-provenance evidence, or a runtime-soundness claim.

The only entrypoint for executing ignored pointer probes is the explicit diagnostic command:

```bash
bash scripts/pointer_provenance.sh diagnostic --configuration no-features --probe p19_ldelem_negative_index
# Preview a known-UB reproducer without running it:
bash scripts/pointer_provenance.sh diagnostic --configuration multithreading \
  --probe p09_ldloca_convu_survives_callee_stack_growth --dry-run
```

Miri diagnostics pin nightly-2026-05-27 and
`MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`.
All diagnostic lanes run one exact probe per process with a 45-minute process timeout, retain output
in a printed temporary directory, and report exit status and wall time. `--all` explicitly explores
the 6 diagnostic-only probes, not the 33 enabled controls, and is not a CI lane; inspect individual
logs on failure (a Cargo failure is not automatically Miri UB). Native diagnostics require
`--lane native` and are restricted to p19; never run other ignored probes natively.
Known-UB probes must not be run natively or outside the diagnostic entrypoint.

`verify-controls --configuration no-features` and its `multithreading` variant check exact native
discovery and nextest metadata confirming all 33 controls are enabled; neither command runs a probe.
`controls --configuration no-features --shard 1` runs the guarded p00 singleton under Miri without
ignored-test flags and checks each exact run count. The table above lists all 33 shards; either
configuration can be `multithreading`. Add `--dry-run` to preview discovery and shard commands without invoking Cargo. Safe tooling self-tests use a fake
Cargo executable: `python3 scripts/tests/test_pointer_provenance.py`; deliberate stale/misspelled-name
cases confirm the control guard rejects catalog mismatches.

## `valgrind.yml` — Non-Blocking Leak/Uninit Checks

This workflow runs only on its daily 04:00 UTC schedule and by manual dispatch; ordinary push/PR no longer launches it.

The workflow builds `dotnet-cli` integration tests across two legs — first `--no-default-features`,
then `--no-default-features --features multithreading` — and runs a curated subset of integration
tests under Valgrind on each leg.

Local commands:

```bash
sudo apt-get install -y valgrind libc6-dbg

# --- No-features leg ---
cargo test -p dotnet-cli --test integration_tests \
  --no-run --no-default-features

BINARY=$(ls -t target/debug/deps/integration_tests-* | grep -v '\.d$' | head -1)
echo "Binary: $BINARY"

for TEST in \
  integration_tests_impl::fixtures::hello_world \
  integration_tests_impl::fixtures::memory_nullable_boxing_42; do
  timeout 600s valgrind \
    --suppressions=valgrind.supp \
    --error-exitcode=1 \
    --leak-check=full \
    --show-leak-kinds=all \
    -s \
    "$BINARY" \
    --test-threads=1 \
    --nocapture \
    --exact \
    "$TEST"
done

# --- Multithreading leg ---
cargo test -p dotnet-cli --test integration_tests \
  --no-run --no-default-features --features multithreading

BINARY=$(ls -t target/debug/deps/integration_tests-* | grep -v '\.d$' | head -1)
echo "Binary: $BINARY"

for TEST in \
  integration_tests_impl::fixtures::threading_monitor_try_enter_timeout_42 \
  integration_tests_impl::fixtures::threading_monitor_try_enter_timeout_single_42; do
  timeout 600s valgrind \
    --suppressions=valgrind.supp \
    --error-exitcode=1 \
    --leak-check=full \
    --show-leak-kinds=all \
    -s \
    "$BINARY" \
    --test-threads=1 \
    --nocapture \
    --exact \
    "$TEST"
done
```

## `valgrind.supp` Suppression Policy

`valgrind.supp` should suppress only known false positives from the Rust runtime/test harness or external dependencies.

When adding a suppression:

1. Run Valgrind with `-s` and inspect the allocation stack.
2. Add suppression only if the allocation does not originate in `dotnet-rs` code.
3. Add a brief comment in `valgrind.supp` describing the source and date.
4. Keep the workflow subset table in this document up to date.

Never suppress leaks that originate in `dotnet-rs` crates.

## Valgrind Subset in CI

| Build leg                              | Test                                                                              |
|----------------------------------------|-----------------------------------------------------------------------------------|
| no features                            | `integration_tests_impl::fixtures::hello_world`                                   |
| no features                            | `integration_tests_impl::fixtures::memory_nullable_boxing_42`                     |
| `multithreading`                       | `integration_tests_impl::fixtures::threading_monitor_try_enter_timeout_42`        |
| `multithreading`                       | `integration_tests_impl::fixtures::threading_monitor_try_enter_timeout_single_42` |

## See Also

- [Validation Features](VALIDATION_FEATURES.md)
- [Fuzzing](FUZZING.md)
- [Benchmark Workflow](BENCHMARK_WORKFLOW.md)
- [Threading and Synchronization](THREADING_AND_SYNCHRONIZATION.md)
- [GC and Memory Safety](GC_AND_MEMORY_SAFETY.md)
