# CI restoration review

## 1. Executive Summary

Analyzed and locally checked HEAD **`c01ba637297881f8614fc73fc6de72ff84f2e478`**, branch `supervised/unblock-ci`, initially clean. This includes `6ad043ca` (M1d owners/guards), `74125cb2` (controls), and `c01ba637` (outcomes); M1c is already in its ancestry. **There is no hosted run on this branch in the queried history; this HEAD has not been proved green in hosted CI.** The latest hosted CI baseline is the older main commit `bec18bd45495134b4002f482cf85e06d52fac59a` (2026-10-29), not the merged M1c/M1d tree. All four replay jobs fail installing cargo-fuzz before replay; a pointer shard exhausts its job budget; advisory assembly/VM suites repeatedly time out. The rustdoc correction is committed and the exact local gate now passes. The current tree hides the finalizer failure with a generated-test ignore; that is not a repair.

Ranked findings:

1. **Blocker** — [F-FUZZ-001](#F-FUZZ-001): cargo-fuzz 0.13.1's locked rustix dependency fails on the replay nightly. Building the same version with stable 1.95.0 succeeds locally; preserve the replay nightly.
2. **Blocker** — [F-FUZZ-002](#F-FUZZ-002): nonempty directories are not proof of replay. Explicit paths and execution counts must gate all four corpora.
3. **High** — [F-FIXT-001](#F-FIXT-001): console-dependent finalizer fixture fails denial mode in hosted baseline and is now ignored. Restore a strong lifecycle oracle without native I/O.
4. **High** — [F-MIRI-001](#F-MIRI-001), [F-MIRI-002](#F-MIRI-002): hosted budgets do not fit execution; split exact selectors, not coverage. Current new controls have no hosted timings.
5. **High** — [F-FUZZ-003](#F-FUZZ-003): exploratory Fuzz is green despite three stable-toolchain setup errors and executor timeout; align installation/invocation consistently.
6. **High** — [F-DIAG-001](#F-DIAG-001), [F-EVID-001](#F-EVID-001): no replay/pointer/Miri failure artifacts in baseline and no merged-tree hosted acceptance evidence.
7. **Info** — [F-DOC-001](#F-DOC-001): committed `run_prepared` link passes the exact local gate, not yet hosted on this HEAD.
8. **Medium** — [F-DOC-002](#F-DOC-002): docs contain stale corpus and timing claims; acceptance needs exact commit/run evidence.
9. **Info** — [F-DIAGFIND-001](#F-DIAGFIND-001): retain diagnostic findings without attempting runtime fixes.

## 2. Methodology and evidence limits

Research date: **2026-10-30 UTC**. Product code, tests and workflows were not edited. Read/captured README.md, CONTRIBUTING.md, docs/CI.md (including its local-equivalent, all Miri, strict-provenance deferral and Valgrind sections), docs/FUZZING.md, docs/VALIDATION_FEATURES.md, docs/GC_AND_MEMORY_SAFETY.md, docs/ASSURANCE_BACKGROUND.md, docs/plans/README.md, docs/plans/01-pointer-provenance.md, docs/pointer-provenance/README.md and INVENTORY.md. Inspected all four requested workflow files; check.sh, bench.fish, script catalog/self-tests, xtask fixture/matrix code, both real fuzz workspaces/targets/tools, fixture generation/harness and executor finalization path. There is **no root `fuzz/` or `corpus/`**: the prompt's paths are workspace-relative. No repository-local AGENTS.md/CLAUDE.md was found. `check.sh` uses xtask for matrices and fixtures but direct clippy/nextest for checks; CI uses direct cargo test, not nextest, except Loom and pointer discovery. bench.fish is profiling automation, not a CI gate.

`gh auth status` succeeded (nickbclifford); repository is `nickbclifford/dotnet-rs`, default branch main. Ran `gh run list --branch supervised/unblock-ci` (empty), `gh run list --workflow <ci.yml|miri.yml|fuzz.yml> --limit 30`, and explicit main lists. Latest 30 CI runs: 27 failure/3 success (last success 28730850060, 2026-07-05, SHA 399849a1, too old for this package). Latest 30 Miri runs: all cancelled, 2026-10-04 through 2026-10-30. Latest 30 Fuzz runs: 29 success/1 cancelled; success is not per-step success. Inspected detailed jobs and logs for:

| Run (URL suffix under `https://github.com/nickbclifford/dotnet-rs/actions/runs/`) | UTC date | Commit | Purpose |
|---|---|---|---|
| [40895583750](https://github.com/nickbclifford/dotnet-rs/actions/runs/40895583750) | Oct 29 | bec18bd45495134b4002f482cf85e06d52fac59a | All job conclusions, four installs, doc failure, fuzzing fixture, all 24 old pointer legs |
| [40576367888](https://github.com/nickbclifford/dotnet-rs/actions/runs/40576367888) | Oct 27 | 8fe4f76d851850558a15adcebb10d96851652b31 | Earlier topology, only 3 pointer shards/configuration; not directly comparable to current 17 |
| [41023482154](https://github.com/nickbclifford/dotnet-rs/actions/runs/41023482154) | Oct 30 | bec18bd45495134b4002f482cf85e06d52fac59a | All advisory jobs and VM/assembly logs |
| [40895583501](https://github.com/nickbclifford/dotnet-rs/actions/runs/40895583501) | Oct 29 | bec18bd45495134b4002f482cf85e06d52fac59a | Second advisory timing sample, VM/assembly logs |
| [41005354582](https://github.com/nickbclifford/dotnet-rs/actions/runs/41005354582) | Oct 30 | bec18bd45495134b4002f482cf85e06d52fac59a | Exploratory logs and downloaded timeout artifact |

Used `gh run view ID --json jobs,headSha`, `--log-failed`, `--job JOB --log`; paginated `gh api repos/nickbclifford/dotnet-rs/actions/runs/ID/jobs`; check-run annotations for pointer job 123523655481 and assembly job 123956041988. Baseline CI artifacts API returns **zero artifacts** (short-lived fixtures already absent); Fuzz download succeeds and contains `timeout-9ef65340e0aa7b9c713001ca0799997155c93c07`. Branch protection API says `Branch not protected` (404): “blocking” below means workflow gates with no continue-on-error, not a claim of remotely configured required checks. No run was dispatched, rerun, pushed or otherwise modified remotely during research.

Timing method: step seconds = completed_at − started_at; pre-run = run-step start − job start; compile = compiler-reported `Finished ... target(s) in ...` durations; execution = libtest `finished in` and per-probe wall reports. Per-probe stdout is buffered by pointer_provenance.sh, so timestamped output does **not** locate compile/sysroot boundaries inside a probe. Tables distinguish observed totals from compile reports, residual overhead and censored lower bounds. No uncensored full VM/assembly completion sample exists. No measured genuinely cold sysroot/build on current HEAD exists. Do not manufacture cold medians or infer execution from a cancelled job's total. Advisory sample size is two for step aggregates, pointer one per configuration/shard (old 12-shard layout). Per-shard medians equal maxima when n=1; new shards 13–17 have no hosted baseline. Candidate budgets below are explicitly provisional allocations with headroom, to be recalibrated in step 4.4.

Cheap local evidence on analyzed HEAD:

- `cargo +nightly-2026-05-27 fuzz --version`: installed 0.13.1 (initial binary's build provenance unknown).
- Independently `cargo +1.95.0 install cargo-fuzz --version 0.13.1 --locked --root /tmp/dotnet-ci-cargo-fuzz-0131`: succeeds, release compilation 5.53 s on this warm local host, then binary reports 0.13.1. This is not a hosted cold install benchmark.
- With that **new binary first in PATH**, `(cd crates/dotnet-value && ASAN_OPTIONS=detect_leaks=0 cargo +nightly-2026-05-27 fuzz run fuzz_raw_memory_access fuzz/corpus/fuzz_raw_memory_access -- -runs=0)` succeeds: 18 files, `#19 INITED`, `#19 DONE`, `Done 19 runs in 0 second(s)`. Build cached, 0.03 s. Further cheap replays with this same isolated binary passed: roundtrip129 files/182 callbacks and offset171 files/172 callbacks (both subsecond, cached builds). See F-FUZZ-002 for why callbacks need not equal N+1. The seed corpus shrinks to 17 coverage representatives, demonstrating why `corp:` is the wrong execution oracle.
- Exact `DOTNET_SKIP_BUILD=1 RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps --no-default-features`: succeeds on 1.95.0, 3.24 s warm. One nonfatal private-link warning remains (below).
- `bash scripts/pointer_provenance.sh verify-controls --configuration no-features` and `--configuration multithreading`: each discovers **33 exact non-ignored controls**.
- Native `cargo test ... -- --list`: assemblies 51, no-feature VM 92. **Not** Miri manifests. Pinned `cargo miri test -p dotnet-assemblies -- --list`: 38 library tests, zero doctests. Pinned VM listings with original three filters: 8 in **both** configurations, empty loom integration binary and doctests. These listing-only commands execute no full Miri suite.
- Tracked corpus census via `git ls-files` and stat, not untracked fuzz-generated seeds; four counts below. No full local Miri suites or slow executor corpus replay were run. No fixture redesign was attempted in research.

Toolchain audit: rust-toolchain.toml:4 pins 1.95.0; stable setup actions generally say `@stable`, while cargo invoked in the checkout follows the repository override. Explicit nightly pins occur in ci.yml (all blocking Miri and replay), miri.yml:51, scripts/pointer_provenance.sh (discovery/execution), its self-tests, CONTRIBUTING.md, docs/CI.md, FUZZING.md, VALIDATION_FEATURES.md, plans/01-pointer-provenance.md and inventory. fuzz.yml alone uses floating `@nightly` and unqualified cargo fuzz (three targets); executor explicitly says `+nightly`. Keep the dated pin and fix invocation rather than broadly retargeting Miri.

## 3. Current CI Topology

Triggers: **C** = ci.yml push/PR main/master, **A** = other workflows push/PR main and daily schedule (Fuzz 00:00, Miri 02:00, Valgrind 04:00 UTC). There is currently no workflow_dispatch. **B** = blocking by workflow, **A-job** = job-level continue-on-error, **A-step** = selected execution steps continue-on-error. Unspecified timeout means GitHub's default 360 minutes, not an explicit budget. `RC` = Swatinem/rust-cache@v2.8.2 default identity, `F` = `${runner.os}-dotnet-fixtures-${xtask-content-key}` with broad restore prefix. All jobs use ubuntu-latest. Ordinary features (ten): empty, multithreading, simd, multithreading,simd, generic-constraint-validation, memory-validation, metadata-validation, multithreading,memory-validation, multithreading,validation-all, fuzzing (confirmed with both xtask matrix commands).

| Workflow/job | Trigger/status | Toolchain; cache | Timeout min | Selection/guard/artifacts | Last observed result; timing |
|---|---|---|---|---|---|
| ci/doc-lint | C/B | repository 1.95.0; none | default | policy/slot/script/drift + exact doc command; no upload | failed old `run_dll` link; doc step 31 s; local current gate pass |
| ci/format | C/B | stable action/repo pin; none | default | cargo fmt --all -- --check; none | success |
| ci/loom | C/B | stable; none | 15 | RUSTFLAGS=--cfg loom, nextest dotnet-vm --test loom_stw --no-default-features; none | success; exhaustive bound unchanged |
| ci/matrix-definitions | C/B | stable; none | default | xtask clippy/test-features JSON; none | success |
| ci/clippy × ten | C/B | stable; RC | 15 | all-targets/no-default + matrix; -D warnings, DOTNET_SKIP_BUILD=1; none | all success |
| ci/build-script-regression | C/B | stable; RC | 20 | scripts/check_build_script_regressions.sh; none | success |
| ci/build-fixtures | C/B | stable + .NET10; F | default | xtask output-dir/cache-key/build; upload fixtures on success, retention 1 day | success; artifact now absent |
| ci/test × ten | C/B | stable + .NET10; RC, downloaded fixtures | 30 (test step 20; MT probes 5) | cargo build/test --verbose --no-default-features + matrix, threads1/180s; MT four hang filters/60s; none | nine pass, fuzzing fails finalizer at 254; tests 150s in fuzzing leg |
| ci/miri-value | C/B | dated nightly; RC | 15 | full default suite, m1a_3/m1b_8/m1d_11 no-features guards; tee stdout only, no upload | old full + first two selectors success, 28s pre-run/86s first run; new m1d unhosted |
| ci/miri-frame-blocks | C/B | dated nightly; RC | 15 | raw_block::7 and frame_alloc::8 each no-features/MT; stdout tee, no upload | absent from old baseline; unhosted |
| ci/miri-owner-retention | C/B | dated nightly; RC | 15 | stack::tests::m1d_8 each no-features/MT; stdout tee, no upload | absent from old baseline; unhosted |
| ci/miri-pointer-controls × 2 × 17 | C/B | stable + dated Miri; `miri-pointer-nightly-2026-05-27-<cfg>-<shard>` inside RC | 45 | exact discovery/native nextest metadata, one exact nonignored test per process, hard count/partition guards; /tmp logs not uploaded | old 2×12: 23 pass/MT9 cancelled; detailed timings below |
| ci/fuzz-corpus-replay × four | C/B | dated nightly; RC root + fuzz workspace; binary `${runner.os}-cargo-fuzz-0.13.1` | 15 | test nonempty default corpus then -runs=0, no execution guard/upload | all install failed: pre-install 30–78s, install 4–10s; compile/replay never reached |
| miri/value default | A/A-job | dated nightly; RC | 30 | full crate, threads1; no upload | pass, pre-run median/max34.5/36s, run64/65s |
| miri/utils no-features | A/A-job | dated nightly; RC | 30 | full crate no-default, threads1; no upload | pass, pre-run31/32s, run23.5/24s |
| miri/assemblies default | A/A-job | dated nightly; RC | 30 | full crate, threads1; cfg(not(miri)) exclusions, no count guard/upload | twice cancelled; pre-run33/35s, run1779.5/1782s censored |
| miri/VM no-features | A/A-job | dated nightly; RC | 30 | jmp_tests OR tail_calls OR fault_tests, threads1, no count guard/upload | twice cancelled; pre-run31.5/33s, run1783.5/1787s censored |
| miri/VM MT | A/A-job | dated nightly; RC | 30 | same filters + multithreading; no upload | twice cancelled; pre-run32.5/34s, run1781/1782s censored |
| fuzz/fuzz | A/A-step (setup blocking) | floating nightly action/repo default; RC root+VM/fuzz; binary `${runner.os}-cargo-fuzz` | default | three ×60s; executor structured generator +300s, tmin30s; executor-only artifacts always | green workflow despite three setup failures and executor libFuzzer timeout1752s |
| valgrind/valgrind | A/A-step (build blocking) | stable + .NET10; RC + F | 30 | integration binary no-features then MT; hello_world/nullable_boxing then two monitor timeouts; exact, threads1, timeout600s each; no uploads | no job logs sampled; outside repair |

Every Miri job keeps `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`. No strict-provenance claim is implied. docs/CI.md:411–435 explicitly defers strict lanes. Sources for topology: ci.yml:1–484, fuzz.yml:1–85, miri.yml:1–68, valgrind.yml:1–135 (identify by job names if line totals drift).

## 4. Findings

<a id="F-FUZZ-001"></a>
#### F-FUZZ-001 — Build the driver on stable, run fuzz targets on the pinned nightly

**Severity:** Blocker.

**Evidence:** ci.yml:428 (`CARGO_FUZZ_VERSION: "0.13.1"`), :471–479 (`Cache cargo-fuzz`, `Install cargo-fuzz`). Run 40895583750 jobs 123523654309/123523655050/123523654643/123523654670 all fail **Install cargo-fuzz**, not Replay. Binary cache miss is logged for `Linux-cargo-fuzz-0.13.1`. Exact common errors in `rustix-0.36.5/src/backend/linux_raw/io/errno.rs:28–29`: `attributes starting with rustc are reserved for use by the rustc compiler`, `cannot find attribute rustc_layout_scalar_valid_range_start in this scope` (also `_end`), and `failed to compile cargo-fuzz v0.13.1`. This is locked dependency/nightly compatibility, **not evidence of edition/MSRV failure or stale binary**. Upstream [0.13.1 Cargo.toml](https://raw.githubusercontent.com/rust-fuzz/cargo-fuzz/0.13.1/Cargo.toml) declares edition2021, no rust-version. Local isolated stable install and three nightly value-corpus replays all succeed.

**Current state:** Driver installation uses the replay nightly; cache identity omits its compiler. No replay job reaches target compilation. Existing local binary version alone does not establish how it was built.

**Proposed change:** Keep cargo-fuzz **0.13.1**, install using **`cargo +1.95.0 install cargo-fuzz --version "$CARGO_FUZZ_VERSION" --locked`**, explicitly provision 1.95.0 as well as nightly-2026-05-27; use nightly only for fuzz build/run. Print rustc/cargo versions for both and `cargo +nightly-2026-05-27 fuzz --version` on cache hits and misses. Use exact-only binary key **`${{ runner.os }}-${{ runner.arch }}-cargo-fuzz-v2-${{ env.CARGO_FUZZ_VERSION }}-build-1.95.0-locked-default`**, no old restore prefix. If installation flags/target/features change, encode them too and bump schema. A version mismatch fails rather than trusting cached state. This is a concrete locally validated compatible pair, not a proposal to upgrade everything to floating latest.

**Risk:** Linux hosted cold install remains to prove; driver target compilation depends on independent fuzz lockfiles and dated nightly. Keep any new setup error visible. **Effort:** high. **Verification:** isolated stable install, exact version assertion, raw-memory local replay, then all four hosted replay jobs on recorded SHA; check cold miss and warm hit identities.

<a id="F-FUZZ-002"></a>
#### F-FUZZ-002 — Prove seed execution, not just directory existence

**Severity:** Blocker.

**Evidence:** ci.yml:435–451 matrix and :479–484 (`test -n ... find`, then no explicit corpus argument). Actual committed corpus census:

| Working directory | Explicit corpus argument | Files | Total bytes | Min/max bytes | Minimum libFuzzer runs (initial empty input plus seeds) |
|---|---|---:|---:|---:|---:|
| crates/dotnet-value | fuzz/corpus/fuzz_managed_ptr_roundtrip | 129 | 1643 | 1/211 | >=130 (observed182) |
| crates/dotnet-value | fuzz/corpus/fuzz_managed_ptr_offset | 171 | 36467 | 1/2299 | >=172 (observed172) |
| crates/dotnet-value | fuzz/corpus/fuzz_raw_memory_access | 18 | 216 | 12/12 | 19 (locally proved) |
| crates/dotnet-vm/fuzz | corpus/fuzz_executor | 252 | 98874 | 2/4096 | >=253 (not locally replayed) |

The paths are correct, nonempty, and contain no zero-byte tracked files. Defaults currently point to these directories, so wrong-path/empty replay is **not the established install root cause**. However, no hosted execution proof exists. `fuzz/Cargo.toml` is actually under each crate; both edition2024 workspaces use libfuzzer-sys0.4. `corpus-tools` is `crates/dotnet-vm/fuzz/tools`, not a root crate. Its generators are appropriate for exploration, not replacing the checked-in replay corpus. `fuzz_executor.rs:7–11` catches and discards inner panics: successful seed execution is not proof of no VM panic.

**Current state:** Only precondition is nonempty `find`; libFuzzer can skip/reject seeds, and `corp:` measures retained coverage representatives, not number executed.

**Proposed change:** Add a small replay wrapper: resolve the matrix working directory and explicit corpus path; inventory tracked regular files with `git ls-files -z` (and confirm on-disk census matches; no generator), require N>0 and all size>0, write manifest/sizes/hash. Invoke `cargo +nightly-2026-05-27 fuzz run "$target" "$corpus" -- -runs=0` with ASAN_OPTIONS unchanged, `2>&1 | tee`. Preserve PIPESTATUS. Parse ANSI-stripped final `Done X runs in ...` and `#X DONE` plus reported `N files found`/`seed corpus: files: N`; require a unique successful replay, positive X, and **X>=N+1** for this pinned libFuzzer (initial empty input plus every nonempty seed); require INITED and DONE/final-summary run counts agree, proving the seed initialization completed without further mutation runs. Reject missing final summary, zero, truncated output, inconsistent counts, skipped corpus, stale path or nonzero command status. Do not require `corp:N`—raw-memory locally retained17 from18 seeds while executing19 inputs. Local roundtrip replay reports182 runs for129 files, offset172 for171, raw19 for18: **do not require equality to N+1**. Vendored libfuzzer-sys0.4.12 FuzzerLoop.cpp:812–840 invokes the empty callback, then RunOne on every nonempty seed; :679–700 may re-execute inputs for leak checks even with process-exit ASAN leak detection disabled. This explains why total callbacks can exceed N+1. Require loaded seed/file census=N, completed INITED and X>=N+1 together, not an arbitrary `>0` or a false exact-count oracle. Counts are derived from checked-in files with these census values as reviewed expectations, not hardcoded forever without manifest validation. Preserve logs even when guard fails.

**Risk:** Parser needs fixture logs for normal output variation; all runs completing does not guarantee all seeds deserialize meaningfully under arbitrary. That is outside this gate's execution claim. **Effort:** high. **Verification:** synthetic logs including `18 files/19 runs/corp17` passing, missing/0/18 runs failing, extra182 roundtrip callbacks passing with seed129/INITED182/DONE182, empty dirs/symlinks/missing files failing; hosted all four expected counts. Do not remove difficult seeds or reduce replay inputs to make jobs pass.

<a id="F-FUZZ-003"></a>
#### F-FUZZ-003 — Align the exploratory driver cache and explicit runtime toolchain

**Severity:** High.

**Evidence:** fuzz.yml:20 (`@nightly`), :39–48 (unversioned cache and install), three unqualified `cargo fuzz` steps, executor `cargo +nightly`. Run 41005354582/job123898905697 restores `Linux-cargo-fuzz`; all three value runs say **`error: the option Z is only accepted on the nightly compiler`**. Repository override selects stable despite installing a floating nightly. They are tolerated by step-level continue-on-error. Executor later reports `ERROR: libFuzzer: timeout after 1752 seconds` and uploads an artifact; workflow still succeeds.

**Current state:** Shared binary path but divergent cache identities/install policies, no logged version provenance. A green exploration workflow is misleading as replay evidence.

**Proposed change:** Reuse F-FUZZ-001's explicit stable install/pins/key, provision dated replay nightly, invoke all fuzz commands and tmin with **`cargo +nightly-2026-05-27`**; preserve exploratory durations (60/300/tmin30), corpus generation, and advisory execution status. Extend useful logs/artifact collection, not runtime semantics or fuzzer iteration limits. This is consistency needed to avoid the same installer problem and demonstrated stable invocation errors, not general workflow modernization.

**Risk:** Pinning exploration changes its compiler baseline; record it in FUZZING.md. Advisory executor timeout remains a finding, not authorization to optimize it. **Effort:** default after version selection. **Verification:** expansion/YAML checks and hosted Fuzz logs prove three value targets actually start libFuzzer; record executor outcome separately.

<a id="F-FIXT-001"></a>
#### F-FIXT-001 — Restore the finalizer regression instead of ignoring native I/O

**Severity:** High.

**Evidence:** `crates/dotnet-cli/tests/fixtures/gc/finalizer_after_entrypoint_0.cs:5–8` destructor prints `finalizer ran`; Main :13–17 allocates then **ends with GC.Collect()**. build.rs:183–207 derives test identity/exit code from source; :219–228 now adds `#[ignore]` for this exact generated fixture in fuzzing. `integration_tests_impl/macros.rs:2–31` generated macro asserts only exit code. Separate `fixtures.rs:119–129` (`#[cfg(not(feature="fuzzing"))] finalizer_runs_after_entrypoint_completes`) asserts exit0 **and exactly one stdout line**, stronger than generated exit-only check. Hosted 40895583750 Test Suite (fuzzing)/123523962686 fails at generated tests.rs:87: `expected exit code 0, got 254`, `System.EntryPointNotFoundException ... SystemNative_SetErrNo ... Console.WriteLine ... FinalizerAfterEntrypoint.Finalize ... Program.Main`.

Policy is instantiated at `crates/dotnet-vm/src/state.rs:443–447` (`p.with_sandbox(...DenySandbox)` under fuzzing); `crates/dotnet-pinvoke/src/sandbox.rs:17–28` denies both library and function; loader.rs:93 and :182 enforce those decisions. Do **not** edit any of these policies. Executor.rs:494–510 processes pending finalizers and :539–574 keeps running when collection queues finalizers at entrypoint completion. A managed check inserted after GC can accidentally mask this exact regression by giving the VM more instructions before returning.

**Current state:** Failure was suppressed in current tree; normal custom stdout test remains native-dependent. The old stdout oracle proves one finalizer invocation by process completion with a tail collection, not a precise externally observed instruction boundary. Do not claim more temporal precision than it measured.

**Proposed change:** Keep allocation and **tail GC.Collect()/return shape**. Replace console output with **managed integer static state** on Program: constructed count, finalizer count, a phase marker set immediately before the final tail collection, and phase observed by the destructor. Initialize/check the counter before the tail collection so early or duplicate execution is detectable. The finalizer increments count and records phase, with no console/Environment.Exit/PInvoke. Observe state **after `Executor::run()` returns and before executor/arena teardown**, asserting construction1, finalization1, expected phase, clean exit0/no managed exception. A zero Main exit alone cannot observe a finalizer that runs after Main's return value is computed.

Implement the observer only in CLI test harness/macros and build.rs test registration: reuse MethodDescription entrypoint construction in harness.rs:232–274 and retain the executor until the integer static reads/assertions finish. Existing public `shared.statics.get(type, GenericLookup::default())` (vm/statics.rs:282) and `storage.get_field_atomic(owner,name,Ordering::Acquire)` (value/storage.rs:329) allow scalar reads without exposing arena roots or changing runtime. Interpret exactly four-byte Int32 fields; don't keep a borrowed GC reference outside an arena. Keep the generated test name `integration_tests_impl::fixtures::gc_finalizer_after_entrypoint_0` enabled in fuzzing and use the special observer instead of the exit-only macro for this one source. Remove only this fixture from the fuzzing ignore condition. Replace the custom stdout test's observation with the same strong managed-state observer and make it available in fuzzing if it remains a distinct named test; do not leave a second broken console assertion. No change to unrelated P/Invoke fixture ignores.

The temporal guarantee to retain is collection at the end of Main and finalizer completion before reporting executor exit, exactly once; a second GC/WaitForPendingFinalizers inside Main alone is **not** an adequate replacement. Add test-only negative controls or documented temporary mutation checks showing missing/duplicate finalization and wrong phase fail, and that bypassing the pending-finalizer return branch makes this oracle fail. If the existing compiler shape does not exercise that branch, document the limitation and use an equivalent test IL fixture with the same end-of-entrypoint collection placement; do not change executor behavior in this package.

Fixture caching: ci.yml:176–203 calls xtask `fixtures cache-key`; xtask/main.rs:398–406 uses `fixture_cache_hash` over fixture .cs + csproj/shared inputs, matching build.rs:71. **Changing this .cs automatically invalidates F.** Verify before/after keys and `.fixtures_hash`; broad fallback restore must still rebuild on non-exact hit. Do not add a gratuitous hardcoded cache bump instead of proving hash invalidation.

**Risk:** Highest design risk: do not turn an exit-drain regression into only an in-Main finalization test, or drop the MT arena before observation. Reuse scalar storage APIs in tests, not runtime/provenance changes. **Effort:** high. **Verification:** rebuild `cargo run -p xtask -- fixtures build`; then exact generated selector with `cargo test -p dotnet-cli --test integration_tests --no-default-features [--features fuzzing] ... -- --exact --test-threads=1 --nocapture`, require **1 passed/0 ignored**, plus custom lifecycle selector in both modes and MT regression coverage. Stale prebuilt hash must fail. Hosted fuzzing and normal Test Suite pass on fixture SHA; record cache key and result.

<a id="F-DOC-001"></a>
#### F-DOC-001 — The rustdoc correction is committed and passes locally

**Severity:** Info (hosted acceptance remains required).

**Evidence:** crates/dotnet-benchmarks/src/lib.rs:448 links `BenchHarness::run_prepared`, method exists at :385 in the same impl. `git log -p -S run_prepared -- crates/dotnet-benchmarks` attributes correction from run_dll to **7501d631e42116aa2a6b9f702ea03587b9030c90**, present in HEAD, not an uncommitted patch. Old hosted doc job123523654341 says `unresolved link to BenchHarness::run_dll`. ci.yml:35–39 uses **`DOTNET_SKIP_BUILD=1 RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps --no-default-features`**. It does **not** use `-D warnings`, `--all-features`, workspace flags, or nightly.

**Current state:** Exact gate passed locally on HEAD, repository 1.95.0. A nonfatal warning remains at vm/src/stack/managed_ptr_decode_cache.rs:70 (`public documentation for clear links to private item Self::invalidate_after_collection`); that is not the broken-link deny lint.

**Proposed change:** No product edit. Run exact gate again in step0.2; hosted doc-lint must pass on final candidate SHA. Preserve flags; record warning as diagnostic, don't expand to doc cleanup. **Risk:** Incorrectly strengthening flags would create an unrelated failure. **Effort:** default. **Verification:** exact local command + hosted doc-lint result, with SHA.

<a id="F-MIRI-001"></a>
#### F-MIRI-001 — Split serial pointer pairs before raising budgets

**Severity:** High.

**Evidence:** ci.yml:384–423 runs two configurations ×17 shards with 45min whole-job timeout; scripts/pointer_provenance.sh:5–79 catalogs 33 controls, retains exact discovery and per-test pass recognition later in script. Old run40895583750 has only12 shards/configuration. MT9/job123523655481 starts its run at14:54:56, finishes p21 at15:17:54 (`finished in 1297.99s`), then starts p22, cancelled at15:38:38. Annotation: **`The job has exceeded the maximum execution time of 45m0s`**. The script's 2700s **per-test** timeout cannot finish before the overall 45min limit when two tests plus setup are serialized. This is budget exhaustion, not evidence p22 failed or reported UB.

**Current state:** Current 17-shard layout adds M1c/M1d coverage absent from hosted baseline. Exact native and nextest guards pass locally for33 in each configuration. Local inventory timings are warm/contended, explicitly not cold hosted evidence (docs/CI.md:110–125).

**Proposed change:** Use 33 singleton shards in catalog order, same matrix configurations and exact selectors below. One process/test removes cumulative-pair budget failures and gives each new test independent evidence. Update expected_controls **and** catalog shard fields, shard_counts, CLI validation/help and scripts/tests/test_pointer_provenance.py together. Keep all33 enabled, same no-default/features/MIRIFLAGS, no --ignored. Preserve both current cross-check catalogs and exact single-pass recognition. Strengthen partition guard: unique identities; union exactly discovered enabled pointer controls (exclude only explicit diagnostic catalog entries); each identity belongs to exactly one shard/configuration; number selected equals reviewed count; fail on newly enabled unmatched controls. Cache layout revision **`miri-pointer-v2-nightly-2026-05-27-<configuration>-<shard>`** prevents old paired identities from masquerading as new singleton caches.

Set explicit miri setup, discovery/compilation and execution steps; prepare manifests through --list so sysroot/compile isn't silently charged to test execution. Initial singleton job45min and process30min (5compile+25execute), see arithmetic below; retain logs and mark budget exhaustion if inner timeout fires. Do not reduce test bodies/iterations. New selectors are not assumed to share old p21 timing.

**Risk:** 66 jobs instead of34; higher cold compile/runner cost and concurrency contention. Prefer max-parallel cap if needed, record queue time separately from execution. Consolidation can follow only after complete hosted timings; reliable singleton evidence is safer than invented pair weights for unmeasured controls. **Effort:** high. **Verification:** script self-tests, both33 discoveries, exact --list/partition equality, all66 hosted pass; also all value/frame/owner blocking library lanes pass. Cold and warm data required before final budgets accepted.

<a id="F-MIRI-002"></a>
#### F-MIRI-002 — Advisory VM/assembly suites need independently measurable shards

**Severity:** High.

**Evidence:** miri.yml:19–44 gives30min per full crate/VM filtered suite, job-level continue-on-error. Two sampled runs both exhaust budgets in all3 affected entries. Oct30 assembly starts tests08:01:04; first drop test completes08:14:38 (814.2s), second08:28:58 (860.2s); third not completed before08:29:56 cancellation. Oct29 first and second drop test completions are15:01:47 and15:11:42, then cancellation. VM MT Oct30 starts tests08:01:13; fault-executed completes08:12:05 (~652s), skipped-normal08:24:00 (~714s); next not finished by30min. VM no-features corresponding execution intervals ~561/704s. Entire suites cannot fit30min even before later tests. Annotation for assembly123956041988: **`The job has exceeded the maximum execution time of 30m0s`**.

**Current state:** No count guard. Native assemblies51 differs from Miri38 due existing cfg(not(miri)) guards; **do not promote native count51 into a Miri expectation or add new exclusions**. VM filter union is8, in both configurations. --list confirms empty integration/doctest selections today.

**Proposed change:** Exact singleton manifest of38 assembly selectors and8 VM selectors per configuration below; retain value/utils entries unchanged/advisory. Build the **original** selection's Miri --list, compare the union to a committed reviewed manifest and enforce exactly-once membership. Separate source manifest used by sharding from independently discovered baseline selection, so a missing manifest test cannot validate itself. Each shard runs exact named selector, --test-threads=1, count1/ignored0. Guard original empty ancillary binaries/doctests too; if these gain tests, fail and add coverage deliberately rather than silently converting all full-crate selection to --lib forever. Do not execute original full suites again in addition to shards. Use per-suite/config/shard schema-versioned cache keys `miri-advisory-v2-nightly-2026-05-27-<crate>-<configuration>-<shard>`. Set job70min, inner process50min initially; explicit setup5 + compile5 + execution45 +25% headroom, rounded up. Singleton execution allowance is deliberately provisional because only the first two cases have uncensored timings; it does not claim unseen tests finish in45min. Preserve advisory status for all new entries.

**Risk:** 54 affected advisory jobs rather than3 (38+8+8), plus retained value/utils. Repeated loader initialization loses possible thread-local reuse; use exact singleton coverage as initial diagnostic layout, monitor cost and rebalance only with complete data. A single test may still exceed45min; classify and record before revising budgets, never shorten its workload. A real UB report is deferred, not fixed here. **Effort:** high. **Verification:** pinned Miri --list guards for assemblies38/VM8+8; hosted all shards terminate within budgets (pass, or explicit advisory diagnostic failure) and no missing/duplicate selectors. Existing value/utils must still run. Any remaining UB/test failure has a durable diagnostic record.

<a id="F-DIAG-001"></a>
#### F-DIAG-001 — Retain phase logs before cancellation erases evidence

**Severity:** High.

**Evidence:** ci.yml:321–382 pipes stdout only to RUNNER_TEMP, pointer script uses mktemp then prints path/output; no affected CI/Miri upload step. Advisory miri.yml ends at cargo invocation. CI baseline artifacts API total_count0; Fuzz uploads only executor artifacts, not three target/setup logs. Pointer script already classifies `miri-ub`, `assertion-or-panic`, `timeout-or-killed`, and `compile-setup-or-process-failure`; reuse that logic instead of losing its useful exact-count recognition.

**Current state:** Cancellation can interrupt redirected output before the runner prints it; per-test logs live only on ephemeral runner. Whole-job timeout cannot reliably run cleanup/upload, even with always().

**Proposed change:** A small `scripts/ci_diagnostics.sh` wrapper (or equivalently testable Python helper) takes phase, case ID, log dir and inner timeout; writes start/status metadata with UTC timestamps, SHA, command, features/MIRIFLAGS/toolchain/cache identity. `mkdir -p "$RUNNER_TEMP/ci-diagnostics/<job-key>"` before risky setup; use `2>&1 | tee` and preserve child exit via PIPESTATUS, no blanket `|| true`. Record compiler and sysroot setup in distinct steps/logs (`cargo +nightly-2026-05-27 miri setup`, original-selection --list for compilation). Pointer script accepts a caller-provided durable log directory, keeping mktemp fallback for local use. Classification: timeout124/137 **with deadline evidence** => budget-exhaustion; actual Miri `error: Undefined Behavior` => UB; compile/sysroot/tool install failures => setup-error (include subphase); libtest panic/FAILED => test-failure; count/manifest mismatch => validation-guard-failure. Unknown nonzero stays unclassified process error rather than guessed UB. Cancellation without deadline evidence is cancelled/unknown, not automatically timeout. Trap TERM/INT and write partial status; separate finalizer step fills missing results from step outcome/start markers. Summary must distinguish advisory diagnostic failure from successful completion.

Inner timeouts leave at least2min for summary/upload before job timeout; set per-step setup/build bounds as well as command execution bounds so a hung setup is cancellable early. Artifact step `if: always()` uploads logs, manifests, metadata, Miri stderr and fuzz artifacts with unique names containing run-attempt/job/shard/configuration (retain e.g.14days), include partial logs, don't fail solely because no log was created before checkout. Add cancelled handling in summary. Whole-job cancellation still cannot guarantee artifact upload: hosted monitor must fetch Actions logs/annotations via gh when artifacts missing; document that fallback. Use workflow default token permissions unless additional read permission is actually required. No changes to unrelated Valgrind design.

**Risk:** Wrapper can mask exit status or conflate timeout/UB if poorly tested; cancellation is best-effort. **Effort:** high design, default wiring. **Verification:** synthetic success/setup compiler error/assertion/UB/timeout/TERM/missing-summary tests, stdout+stderr and child exit preserved; YAML; hosted artifact and summary inspectable. Do this before long shard acceptance.

<a id="F-EVID-001"></a>
#### F-EVID-001 — Acceptance must name the actual merged-tree commit

**Severity:** High.

**Evidence:** local HEAD c01ba637 vs latest main hosted bec18bd4; branch run list empty. All three workflows lack workflow_dispatch (`ci.yml:7–11`, miri.yml:7–13, fuzz.yml:7–13). Main branch unprotected per API, so report workflow blocking semantics accurately.

**Current state:** Prior M1c/M1d local results do not prove this merged tree in hosted environment. Workflow conclusions alone hide tolerated exploration failures and timeout-cancelled Miri.

**Proposed change:** Add minimal workflow_dispatch triggers to CI/Miri/Fuzz, preserving existing triggers/concurrency. Step agents commit scoped work using existing `type(scope): summary` style and push `supervised/unblock-ci` only when ready for hosted checks. Use `gh workflow run ci.yml --ref supervised/unblock-ci`, similarly miri.yml/fuzz.yml; or observe the PR-triggered run on the exact commit to avoid redundant cost. GitHub dispatches **workflows**, not individual jobs: observe affected job subsets, and for targeted repeats use `gh run rerun RUN --job JOB` only for the same SHA. Don't claim job reruns test a later commit. No need to add broad job-selection machinery merely for this package. `gh run watch ID --exit-status`, `gh run view ID --json headSha,jobs,url`, job logs, annotation/artifact APIs provide acceptance evidence. Do not cancel running evidence inadvertently with another push (concurrency cancel-in-progress). Final tested commit must include merged M1c/M1d plus CI/fixture changes; record exact full SHA, parent baseline, run IDs/URLs, attempts, every shard count and cache class. Documentation-only follow-up commits must explicitly name the tested ancestor; rerun exact docs gate on final documentation SHA rather than calling an untested hash tested.

**Risk:** Provisional budgets and unseen tests may require multiple monitoring rounds; branch actions need pushed code and permissions. **Effort:** extra-high hosted monitoring, default bookkeeping. **Verification:** completion checklist enumerates four successful replays with counts, unignored fixture pass normal/fuzzing, exact doc pass, all blocking pointer/library controls pass, advisory VM/assembly selectors all complete within configured budgets (diagnostic errors explicitly recorded, no pending/timeout), useful artifacts, final docs. Advisory workflow green with cancelled jobs does not pass acceptance.

<a id="F-DOC-002"></a>
#### F-DOC-002 — Replace stale claims with measured, scoped evidence

**Severity:** Medium.

**Evidence:** docs/CI.md:80–125 gives17-shard layout and warns cold hosted timings unmeasured; FUZZING.md:112–130 documents 0.13.1 nightly installation context and default-corpus replay but no proof; :260–269 later contradicts committed corpus policy (`raw_memory_access ... narrow exception`, executor regenerated on demand). rust-toolchain.toml comment calls nightly Miri-only despite replay. Feature docs correctly distinguish blocking controls/advisory suites.

**Current state:** Documentation is partly accurate but cannot describe a reliable replay or measured hosted budgets yet.

**Proposed change:** Update only affected CI/FUZZING documentation: stable driver compiler vs nightly target compiler, cache schemas, explicit paths/count semantics, final shard manifest and budget components/headroom/inner+job timeouts, measured cold/warm distributions/sample sizes and run links/full tested SHA. Correct contradictory corpus text; explain fourth rebase target exists locally in value workspace but is **not** among the four assigned replay gates (don't add it opportunistically). Record any remaining advisory UB/assertion/budget findings distinctly, not as proof of runtime soundness. Don't reopen strict-provenance or edit pointer milestone status just to mark CI green.

**Risk:** Evidence ages; retain date/SHA/run attempts and sample size. **Effort:** default after hosted validation. **Verification:** `bash scripts/check_doc_drift.sh`, `bash scripts/check_plan_queue.sh`, exact rustdoc gate, links/anchors, compare documented manifest with actual runner.

<a id="F-DIAGFIND-001"></a>
#### F-DIAGFIND-001 — Record pre-existing diagnostics without runtime repair

**Severity:** Info (not new provenance-fix authorization).

**Evidence:** Current pointer catalog/inventory retains6 diagnostic-only probes including reported-UB p09; docs/pointer-provenance/README.md and INVENTORY.md explicitly limit local permissive TB evidence. Sampled advisory logs **do not show a Miri UB report**; they are truncated by timeouts. Fuzz41005354582 generated executor timeout artifact SHA-named above; no reason established for its1752s case. Local rustdoc emits private-link warning described above. FUZZING.md claims panic catching reports crashes but fuzz_executor.rs catches/discards panics; do not infer panic-free execution from replay success.

**Current state:** These findings predate this package or remain diagnostic uncertainty. Do not rerun known-UB probes natively or use --ignored in blocking shards.

**Proposed change:** Preserve artifact link/name and quoted diagnostics, open/deferred status and owner/follow-up outside this package. If hosted reshards expose real UB, record selector/configuration/SHA/full stack/log, allow advisory failure to remain advisory, but require job termination and all other assigned cases scheduled. Never fix VM/memory/value semantics, suppress UB, optimize interpreter/tests, lower iteration counts or drop slow tests in this package. A blocking control UB would prevent completion and require escalation, not continue-on-error.

**Risk:** Confusing no UB observed with no UB exists. **Effort:** default recording. **Verification:** diagnostic table in final docs/memory distinguishes setup, assertions, UB, cancellation and budget exhaustion and explicitly identifies unobserved cases.

## 5. Shard and Budget Proposal

### Timing baseline and decomposition

The following old pointer table is generated from **40895583750**, sample1 per cell; setup is job-start→Run-control-start (includes discovery/native compilation); run is Run-control wall; `C` is compiler-reported Miri compile totals (last two `Finished` reports), `E` sum of libtest execution reports. Neither C nor residual is a pure cold benchmark; discovery runs twice today (explicit step and controls entrypoint), and residual includes second discovery/sysroot/process overhead. No cold/warm median is claimed from this sample. For each singleton the parent pair's successful execution report is the useful weight, not its former local inventory wall.
| Configuration / old shard | Pre-run incl native discovery (s) | Run wall (s) | Miri compile C (s) | Execution E (s) | Result |
|---|---:|---:|---:|---:|---|
| multithreading / 1 | 56 | 958 | 7.41 | 914.18 | success |
| multithreading / 2 | 63 | 1385 | 12.83 | 1324.37 | success |
| multithreading / 3 | 68 | 1600 | 13.18 | 1537.55 | success |
| multithreading / 4 | 74 | 1467 | 29.11 | 1396.66 | success |
| multithreading / 5 | 89 | 2060 | 35.68 | 1975.38 | success |
| multithreading / 6 | 84 | 1699 | 35.96 | 1615.9 | success |
| multithreading / 7 | 89 | 1784 | 35.97 | 1699.65 | success |
| multithreading / 8 | 76 | 1781 | 29.39 | 1711.37 | success |
| multithreading / 9 | 91 | 2622 | partial | 1297.99 + unfinished p22 | cancelled |
| multithreading / 10 | 85 | 1879 | 36.07 | 1795.2 | success |
| multithreading / 11 | 91 | 1389 | 36.0 | 1304.65 | success |
| multithreading / 12 | 92 | 1639 | 35.78 | 1555.77 | success |
| no-features / 1 | 67 | 1415 | 12.68 | 1358.09 | success |
| no-features / 2 | 70 | 2166 | 13.19 | 2104.53 | success |
| no-features / 3 | 66 | 1544 | 12.94 | 1485.31 | success |
| no-features / 4 | 92 | 1207 | 33.83 | 1128.19 | success |
| no-features / 5 | 65 | 1262 | 25.2 | 1203.17 | success |
| no-features / 6 | 89 | 2259 | 38.9 | 2172.41 | success |
| no-features / 7 | 66 | 1095 | 22.17 | 1043.58 | success |
| no-features / 8 | 68 | 1199 | 26.37 | 1136.29 | success |
| no-features / 9 | 81 | 1438 | 36.12 | 1356.23 | success |
| no-features / 10 | 67 | 1091 | 25.49 | 1030.47 | success |
| no-features / 11 | 81 | 1551 | 35.81 | 1469.23 | success |
| no-features / 12 | 67 | 1428 | 25.33 | 1368.58 | success |

Advisory Oct30 decomposition (seconds; two-run step medians/maxima are in topology):

| Suite | Toolchain/cache/dependency pre-run | Implicit Miri sysroot portion inside run | Reported test compile | Execution observed | Sample limitation |
|---|---:|---:|---:|---|---|
| assemblies | 35 | ~22 | 22.36 | first2: ~814+860, later incomplete; run total1777 | warm Rust cache, no complete suite |
| VM no-features | 30 | ~18 | 27.90 | first2: ~561+704, later incomplete; run total1787 | same |
| VM multithreading | 31 | ~20 | 34.84 | first2: ~652+714, later incomplete; run total1782 | same |

Sysroot portion is start-of-run→sysroot-done log, approximate; compile and libtest overhead are separate. The older sample also cancels after30min. A complete warm/cold distribution is **unavailable**, not zero. Setup actions plus warm cache restore are typically30–35s here, but first-time nightly installation in the replay sample was47s rather than10–11s. Explicitly allocate cold setup/compile before execution when validating the new layout.

### Blocking pointer singleton manifest

Each row below is **one exact test** in **each** configuration (`--no-default-features`, optionally `--features multithreading`), `-p dotnet-vm --lib`, `-- --exact <selector> --test-threads=1`, never --ignored. New IDs are sequential catalog order, not old shard numbers. All rows have expected count1, same initial job timeout45min and per-process30min. Singleton p21 already took21.63min hosted; pairing it exhausted45min. Candidate arithmetic per row: **5min cold setup/sysroot +5min compilation/native discovery +25min execution =35min; 25% headroom=8.75min; ceil(43.75)=44min, configure45min** (additional1min rounding/cleanup). Inner compile+execution process30min leaves room for5min setup and diagnostics; setup/build wrappers also bounded, so uploads retain at least2min. Cold allocations exceed warm measurements intentionally; execution allowance for new/unobserved tests is provisional, not a measurement. If any single case exceeds this, collect evidence and revise the budget, not the test.

| New shard | Exact selector | Expected tests per configuration | Initial job / inner process min |
|---|---|---:|---|
| 1 | `pointer_provenance_tests::p00_smoke` | 1 | 45 / 30 |
| 2 | `pointer_provenance_tests::p02_ldloca_stind_i4_control` | 1 | 45 / 30 |
| 3 | `pointer_provenance_tests::p03_ldloca_convu_ldind_i4` | 1 | 45 / 30 |
| 4 | `pointer_provenance_tests::p05_single_localloc_roundtrip` | 1 | 45 / 30 |
| 5 | `pointer_provenance_tests::p07_localloc_one_past_end` | 1 | 45 / 30 |
| 6 | `pointer_provenance_tests::p11_ldelema_stind_heap_array_control` | 1 | 45 / 30 |
| 7 | `pointer_provenance_tests::p10_ldelema_convu_stind_heap_array` | 1 | 45 / 30 |
| 8 | `pointer_provenance_tests::p20_heap_convu_then_typed_write_then_raw_store` | 1 | 45 / 30 |
| 9 | `pointer_provenance_tests::p13_ldelema_managed_add_far_out_of_bounds_then_conv_u` | 1 | 45 / 30 |
| 10 | `pointer_provenance_tests::p14_initblk_heap_array_via_managed_ptr` | 1 | 45 / 30 |
| 11 | `pointer_provenance_tests::p16_cpblk_heap_arrays_via_managed_ptr` | 1 | 45 / 30 |
| 12 | `pointer_provenance_tests::p_null_through_pointer_local_address` | 1 | 45 / 30 |
| 13 | `pointer_provenance_tests::p06_localloc_minus_one_then_plus_one` | 1 | 45 / 30 |
| 14 | `pointer_provenance_tests::p08_localloc_overshoot` | 1 | 45 / 30 |
| 15 | `pointer_provenance_tests::p17_localloc_ptr_ceq_self` | 1 | 45 / 30 |
| 16 | `pointer_provenance_tests::p18_localloc_ptr_clt` | 1 | 45 / 30 |
| 17 | `pointer_provenance_tests::p21_localloc_signed_extreme_offsets_roundtrip` | 1 | 45 / 30 |
| 18 | `pointer_provenance_tests::p22_localloc_wrap_to_zero_roundtrip` | 1 | 45 / 30 |
| 19 | `pointer_provenance_tests::p23_localloc_pointer_loop_unsigned_branch_compare` | 1 | 45 / 30 |
| 20 | `pointer_provenance_tests::p24_unsafe_byte_offset_direction_and_deref` | 1 | 45 / 30 |
| 21 | `pointer_provenance_tests::p25_convi_and_stind_i_pointer_roundtrip_deref` | 1 | 45 / 30 |
| 22 | `pointer_provenance_tests::p26_native_int_boundary_store_load_roundtrip` | 1 | 45 / 30 |
| 23 | `pointer_provenance_tests::p27_stind_i_fresh_pointer_through_local_slot_dereferences` | 1 | 45 / 30 |
| 24 | `pointer_provenance_tests::p28_stind_i_fresh_pointer_through_heap_memory_dereferences` | 1 | 45 / 30 |
| 25 | `pointer_provenance_tests::p04_double_localloc_write_first` | 1 | 45 / 30 |
| 26 | `pointer_provenance_tests::p29_localloc_blocks_survive_growth_and_are_16_byte_aligned` | 1 | 45 / 30 |
| 27 | `pointer_provenance_tests::p30_localloc_block_survives_nested_callee_localloc` | 1 | 45 / 30 |
| 28 | `pointer_provenance_tests::p31_localloc_blocks_released_on_return_and_pooling` | 1 | 45 / 30 |
| 29 | `pointer_provenance_tests::p32_localloc_blocks_released_on_tail_call_replacement` | 1 | 45 / 30 |
| 30 | `pointer_provenance_tests::p33_localloc_blocks_released_on_exception_unwind` | 1 | 45 / 30 |
| 31 | `pointer_provenance_tests::p34_localloc_blocks_released_on_executor_teardown` | 1 | 45 / 30 |
| 32 | `pointer_provenance_tests::p35_ldflda_on_stack_value_type_then_ldind` | 1 | 45 / 30 |
| 33 | `pointer_provenance_tests::p36_static_address_survives_forced_collection` | 1 | 45 / 30 |

Partition proof to implement: the listed33 unique names equal current expected_controls exactly (verified while generating this review); old sixteen pairs plus singleton have33 total; new33 singleton sets are disjoint. Both configurations receive that identical union. Discovery verifies enabled/nonignored membership, and future unknown enabled probes must fail rather than fall out of coverage. Existing six diagnostic-only tests stay excluded intentionally, exactly as before. The topology table includes the other blocking library gates: preserve full default value suite plus no-feature3/8/11, raw7×2/frame8×2, stack8×2. Initial15min library budget can be represented as3setup+3compile+5execution=11, 25% headroom2.75 =>13.75, configure15; **provisional for new M1d jobs**, validate all in hosted CI. No library test is relocated into an advisory suite.

### Advisory exact singleton manifests

Keep `-p dotnet-assemblies` default features and the original VM no-default/MT feature configurations. Run each selector with `-- --exact <selector> --test-threads=1`; each row expected1, ignored0. Exact original Miri --list census is38 assemblies and8 VM for each configuration. The lists below came from actual pinned Miri listing, **not** source grep or native51. Each row initially has **job70min / inner process50min**:5cold setup/sysroot +5compile +45execution=55;25% headroom13.75 =>68.75; round to70. This is a diagnostic budget proposal, not a claim every unseen case takes45min; the largest measured assembly interval is14.34min, VM11.91min, but later cases have no uncensored timing. The45min allowance grants >3× those observed per-case times for uncertainty while still bounding execution. Step4.4 must replace provisional allocations with complete hosted cold/warm measurements before accepting the final budgets. If a single test cannot finish, report partial and gather data; don't silently raise timeouts or claim completion.

| Assembly shard | Exact selector | Count | Job / process min |
|---|---|---:|---|
| A01 | `drop_tests::test_assembly_loader_drop_keeps_descriptors_valid` | 1 | 70 / 50 |
| A02 | `drop_tests::test_corlib_wkt_resolution_failures_are_retryable` | 1 | 70 / 50 |
| A03 | `drop_tests::test_exception_dispatch_state_prefers_slash_then_falls_back_to_plus` | 1 | 70 / 50 |
| A04 | `drop_tests::test_memory_extensions_slow_path_signature_uses_read_only_span` | 1 | 70 / 50 |
| A05 | `drop_tests::test_type_description_keeps_arena_alive` | 1 | 70 / 50 |
| A06 | `host::tests::classifies_bundle_signature_as_single_file` | 1 | 70 / 50 |
| A07 | `host::tests::classifies_elf_as_nativeaot` | 1 | 70 / 50 |
| A08 | `host::tests::classifies_managed_pe_with_cli_header` | 1 | 70 / 50 |
| A09 | `host::tests::classifies_pe_without_cli_header_as_nativeaot` | 1 | 70 / 50 |
| A10 | `host::tests::disable_policy_requires_exact_match` | 1 | 70 / 50 |
| A11 | `host::tests::select_framework_version_applies_all_roll_forward_policies_for_10_0_0_request` | 1 | 70 / 50 |
| A12 | `host::tests::short_or_unknown_input_defaults_to_managed` | 1 | 70 / 50 |
| A13 | `support_contract_tests::every_generated_id_and_typed_accessor_resolves_its_prescribed_owner` | 1 | 70 / 50 |
| A14 | `support_contract_tests::generated_runtime_slot_enum_is_in_embedded_support_assembly` | 1 | 70 / 50 |
| A15 | `support_contract_tests::generated_slot_descriptors_preserve_stable_ids_and_exact_shapes` | 1 | 70 / 50 |
| A16 | `support_contract_tests::handle_override_is_limited_to_the_support_descriptor` | 1 | 70 / 50 |
| A17 | `support_contract_tests::loaded_support_metadata_has_an_independent_literal_slot_census` | 1 | 70 / 50 |
| A18 | `support_contract_tests::runtime_type_handle_object_ref_and_usize_views_round_trip` | 1 | 70 / 50 |
| A19 | `support_contract_tests::separate_span_layout_accessors_preserve_their_offsets` | 1 | 70 / 50 |
| A20 | `support_contract_tests::test_registry_slot_count` | 1 | 70 / 50 |
| A21 | `support_contract_validation_tests::duplicate_annotations_report_the_id_and_both_locations` | 1 | 70 / 50 |
| A22 | `support_contract_validation_tests::exact_signature_mismatch_reports_expected_and_found_shape` | 1 | 70 / 50 |
| A23 | `support_contract_validation_tests::generated_runtime_slot_ids_are_required` | 1 | 70 / 50 |
| A24 | `support_contract_validation_tests::misplaced_id_reports_actual_and_expected_locations` | 1 | 70 / 50 |
| A25 | `support_contract_validation_tests::missing_id_reports_its_exact_expected_slot` | 1 | 70 / 50 |
| A26 | `support_contract_validation_tests::runtime_slot_field_without_used_implicitly_is_a_contract_violation` | 1 | 70 / 50 |
| A27 | `support_contract_validation_tests::stub_schema_rejects_duplicate_targets_without_mutating_registered_maps` | 1 | 70 / 50 |
| A28 | `support_contract_validation_tests::stub_schema_requires_exact_instance_string_in_place_of_field` | 1 | 70 / 50 |
| A29 | `support_contract_validation_tests::stub_schema_requires_exactly_one_non_null_in_place_of_named_argument` | 1 | 70 / 50 |
| A30 | `support_contract_validation_tests::unknown_id_is_a_contextual_contract_violation` | 1 | 70 / 50 |
| A31 | `support_contract_validation_tests::wrong_staticness_reports_actual_and_expected_staticness` | 1 | 70 / 50 |
| A32 | `validation_tests::tests::test_empty_method_name` | 1 | 70 / 50 |
| A33 | `validation_tests::tests::test_empty_typedef_table` | 1 | 70 / 50 |
| A34 | `validation_tests::tests::test_invalid_module_name` | 1 | 70 / 50 |
| A35 | `validation_tests::tests::test_self_inheritance` | 1 | 70 / 50 |
| A36 | `validation_tests::tests::test_valid_metadata` | 1 | 70 / 50 |
| A37 | `version_tests::tests::test_version_equality` | 1 | 70 / 50 |
| A38 | `version_tests::tests::test_version_parsing` | 1 | 70 / 50 |

| VM shard (both configurations) | Exact selector | Count per configuration | Job / process min |
|---|---|---:|---|
| V01 | `fault_tests::tests::test_fault_handler_executed_on_exception` | 1 | 70 / 50 |
| V02 | `fault_tests::tests::test_fault_handler_skipped_on_normal_exit` | 1 | 70 / 50 |
| V03 | `jmp_tests::tests::test_jmp_inside_try` | 1 | 70 / 50 |
| V04 | `jmp_tests::tests::test_jmp_instruction` | 1 | 70 / 50 |
| V05 | `jmp_tests::tests::test_jmp_invalid_stack` | 1 | 70 / 50 |
| V06 | `jmp_tests::tests::test_jmp_signature_mismatch` | 1 | 70 / 50 |
| V07 | `tail_calls_tests::non_tail_call_chain_grows_frame_depth` | 1 | 70 / 50 |
| V08 | `tail_calls_tests::tail_call_chain_keeps_frame_depth_bounded` | 1 | 70 / 50 |

Disjointness is by exact string identity, not substring filter:38 unique A rows =38 listed assembly tests;8 unique V rows =the OR-union of jmp_tests/tail_calls/fault_tests. Both VM configurations listed the same8. Existing assembly cfg(not(miri)) exclusions remain unchanged. Ancillary Miri listings currently yield0 doctests/VM loom tests: guard these empties explicitly and fail on new coverage rather than discard it. Value/default and utils/no-features advisory full-crate jobs remain with30min timeout; collect their setup/compile/execution logs too. Intentional cross-workflow duplication (blocking value full suite and advisory value suite) remains; no new duplication within a suite/configuration is intended.

Budget acceptance: observe one cold run with the new schema keys and one warm repeat per affected shard on the **same SHA**, identify cache outcomes from logs; record setup, compile, execution, upload times separately and p50/max (n at least2, with cold/warm separated rather than averaging them). Keep25% execution/cold uncertainty headroom and at least2min artifact margin; final configured timeout must exceed the sum. If only one cache class is obtainable, retain that limitation and don't present the candidate budgets as measured. Avoid caching away the very cold path being budgeted. Queue delay and workflow makespan are not test execution.

## 6. Out-of-Scope / Deferred

No VM/memory/value provenance semantics changes, strict-provenance enablement, interpreter/test optimization, workload/iteration reduction, new allow-list or relaxed native policy, test ignores, or blocking-to-advisory conversion. Do not add the value `fuzz_managed_ptr_rebase` target to this package's matrix merely because its local files exist (none of its local corpus files are tracked). No broad dependency/toolchain upgrades, action deprecation modernization, Valgrind redesign, branch-protection administration, or unrelated doc warning cleanup. Six ignored diagnostic probes remain separate; known-UB diagnostics must never run natively. New real UB findings are evidence/deferred work, not implementation tasks for this CI package.

## 7. Phase Plan

These IDs, descriptions, effort levels and finding refs match CHECKLIST.md. Each step includes its verification in the description or referenced finding; hosted monitoring must record full SHA and run URLs in memory. Phases1/2 provide focused replay/fixture evidence; phase3 installs durable diagnostics before phase4's long validation. Final phase validates the complete merged candidate rather than treating earlier partial runs as acceptance.


## Phase 0: Baseline and exact gates

- 0.1 Refresh hosted baseline evidence and record the current merged-tree SHA, failure classes, annotations, and timing limitations in memory. [effort: default] — refs REVIEW.md#F-EVID-001
- 0.2 Run the exact CI rustdoc gate locally and record the committed link correction and any nonfatal diagnostics. [effort: default] — refs REVIEW.md#F-DOC-001
- 0.3 Add workflow_dispatch to CI, Miri, and Fuzz without changing push/PR/schedule behavior; validate YAML and document branch-dispatch and same-SHA job-rerun commands. [effort: default] — refs REVIEW.md#F-EVID-001

## Phase 1: Compatible and accountable corpus replay

- 1.1 Install cargo-fuzz 0.13.1 with Rust 1.95.0 and --locked, retain nightly-2026-05-27 for replay, and version the binary cache by all build inputs; verify installation and one local corpus replay. [effort: high] — refs REVIEW.md#F-FUZZ-001
- 1.2 Add explicit corpus paths and a fail-closed libFuzzer execution-count guard for all four corpora; test valid, empty, skipped, truncated, and mismatched logs locally. [effort: high] — refs REVIEW.md#F-FUZZ-002
- 1.3 Align fuzz.yml installation/cache/runtime invocation with the proven pair without changing exploratory durations or advisory status; validate YAML and command expansion. [effort: default] — refs REVIEW.md#F-FUZZ-003
- 1.4 Push the scoped changes and dispatch or observe CI on that SHA; require all four fuzz-corpus-replay jobs to pass with file counts and executed counts, and record run URLs and SHA. [effort: high] — refs REVIEW.md#F-FUZZ-002

## Phase 2: Native-free finalizer lifecycle fixture

- 2.1 Replace console-dependent finalizer observation with managed counters/phase state and a post-executor test assertion, restore the generated fuzzing test, and preserve the tail-collection regression and once-only oracle; verify normal and fuzzing exact tests. [effort: high] — refs REVIEW.md#F-FIXT-001
- 2.2 Verify fixture cache invalidation from changed source, reject stale prebuilt artifacts, and observe CI Test Suite (fuzzing) plus a normal leg passing on the fixture commit; record SHA and run evidence. [effort: default] — refs REVIEW.md#F-FIXT-001

## Phase 3: Durable diagnostics before long validation

- 3.1 Design and test a small phase-aware logging/classification wrapper with synthetic setup, test, UB, count-guard, timeout, and cancellation cases while preserving nonzero exits. [effort: high] — refs REVIEW.md#F-DIAG-001
- 3.2 Wire persistent logs, explicit setup/compile/execute phases, inner time budgets, summaries, and always-upload artifacts into affected replay and Miri jobs; verify YAML, shell tests, and a hosted artifact/summary on the exact SHA. [effort: default] — refs REVIEW.md#F-DIAG-001

## Phase 4: Coverage-preserving Miri shards and budgets

- 4.1 Split blocking pointer controls into the 33 exact singleton shards in REVIEW.md, update both script catalogs/counts/tests and cache layout identity, and verify both 33-control discoveries and partition equality locally. [effort: high] — refs REVIEW.md#F-MIRI-001
- 4.2 Split advisory VM and assembly selections into the exact singleton manifests in REVIEW.md, retain value/utils legs and all feature/Miri flags, and add Miri-aware union/duplicate/count guards and provisional cold budgets. [effort: high] — refs REVIEW.md#F-MIRI-002
- 4.3 Validate YAML, shell guard self-tests, both pointer discoveries, and Miri --list manifests for every configuration without executing full local Miri suites; record counts and partition proof. [effort: default] — refs REVIEW.md#F-MIRI-001, REVIEW.md#F-MIRI-002
- 4.4 Dispatch and monitor CI pointer/library gates and Miri advisory shards on the implementation SHA, collect cold and warm setup/compile/execute timings, and refine budgets only from evidence until all blocking controls pass and advisory suites terminate within budget. [effort: extra-high] — refs REVIEW.md#F-MIRI-001, REVIEW.md#F-MIRI-002, REVIEW.md#F-DIAG-001

## Phase 5: Hosted acceptance and documentation

- 5.1 Dispatch or observe CI, Miri, and aligned Fuzz on the final candidate SHA; record run URLs, artifact links, exact counts, timings, conclusions and deferred diagnostics, distinguishing advisory failures from unfinished runs. [effort: extra-high] — refs REVIEW.md#F-EVID-001, REVIEW.md#F-DIAGFIND-001
- 5.2 Update docs/CI.md and docs/FUZZING.md with the final pins/cache identities, exact shard manifests/counts, measured cold/warm budgets, run links and tested SHA, correcting contradictory corpus claims; run doc drift and exact rustdoc checks. [effort: default] — refs REVIEW.md#F-DOC-002
- 5.3 Audit completion criteria individually: all four corpora actually execute, native-free fixture passes normal/fuzzing, exact rustdoc passes, every blocking pointer/library control passes, advisory suites finish within configured budgets, and remaining findings and tested SHA are recorded. [effort: default] — refs REVIEW.md#F-EVID-001
