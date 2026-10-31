# CI repair checklist

Execute one flat-ID step at a time. Each step ends with verification, a memory entry, and changing only its own `- [ ]` to `- [x]`. Budgets marked provisional in REVIEW.md are hypotheses to validate, not permission to declare CI repaired. Preserve coverage, denial policy, and blocking/advisory status.

## Phase 0: Baseline and exact gates
- [x] 0.1 Refresh hosted baseline evidence and record the current merged-tree SHA, failure classes, annotations, and timing limitations in memory. [effort: default] — refs REVIEW.md#F-EVID-001
- [x] 0.2 Run the exact CI rustdoc gate locally and record the committed link correction and any nonfatal diagnostics. [effort: default] — refs REVIEW.md#F-DOC-001
- [x] 0.3 Add workflow_dispatch to CI, Miri, and Fuzz without changing push/PR/schedule behavior; validate YAML and document branch-dispatch and same-SHA job-rerun commands. [effort: default] — refs REVIEW.md#F-EVID-001

## Phase 1: Compatible and accountable corpus replay
- [x] 1.1 Install cargo-fuzz 0.13.1 with Rust 1.95.0 and --locked, retain nightly-2026-05-27 for replay, and version the binary cache by all build inputs; verify installation and one local corpus replay. [effort: high] — refs REVIEW.md#F-FUZZ-001
- [x] 1.2 Add explicit corpus paths and a fail-closed libFuzzer execution-count guard for all four corpora; test valid, empty, skipped, truncated, and mismatched logs locally. [effort: high] — refs REVIEW.md#F-FUZZ-002
- [x] 1.3 Align fuzz.yml installation/cache/runtime invocation with the proven pair without changing exploratory durations or advisory status; validate YAML and command expansion. [effort: default] — refs REVIEW.md#F-FUZZ-003
- [x] 1.4 Push the scoped changes and dispatch or observe CI on that SHA; require all four fuzz-corpus-replay jobs to pass with file counts and executed counts, and record run URLs and SHA. [effort: high] — refs REVIEW.md#F-FUZZ-002

## Phase 2: Native-free finalizer lifecycle fixture
- [x] 2.1 Replace console-dependent finalizer observation with managed counters/phase state and a post-executor test assertion, restore the generated fuzzing test, and preserve the tail-collection and once-only lifecycle oracle; cover the executor's pending-finalizer return deferral with a dotnet-vm unit test that fails when the branch is bypassed; verify normal and fuzzing exact tests. [effort: high] — refs REVIEW.md#F-FIXT-001
- [x] 2.2 Verify fixture cache invalidation from changed source, reject stale prebuilt artifacts, and observe CI Test Suite (fuzzing) plus a normal leg passing on the fixture commit; record SHA and run evidence. [effort: default] — refs REVIEW.md#F-FIXT-001
- [ ] 2.3 Restore the blocking std::sync ceiling (71 > 70) broken by the finalizer observer's fully qualified `std::sync::atomic::Ordering::Acquire` in commit 1d53382d, without raising CEILING; verify `bash scripts/check_std_sync_ceiling.sh` and hosted Documentation and Source Policy Checks. [effort: default] — refs REVIEW.md#F-FIXT-001

## Phase 3: Durable diagnostics before long validation
- [x] 3.1 Design and test a small phase-aware logging/classification wrapper with synthetic setup, test, UB, count-guard, timeout, and cancellation cases while preserving nonzero exits. [effort: high] — refs REVIEW.md#F-DIAG-001
- [x] 3.2 Wire persistent logs, explicit setup/compile/execute phases, inner time budgets, summaries, and always-upload artifacts into affected replay and Miri jobs; verify YAML, shell tests, and a hosted artifact/summary on the exact SHA. [effort: default] — refs REVIEW.md#F-DIAG-001

## Phase 4: Coverage-preserving Miri shards and budgets
- [x] 4.1 Split blocking pointer controls into the 33 exact singleton shards in REVIEW.md, update both script catalogs/counts/tests and cache layout identity, and verify both 33-control discoveries and partition equality locally. [effort: high] — refs REVIEW.md#F-MIRI-001
- [x] 4.2 Split advisory VM and assembly selections into the exact singleton manifests in REVIEW.md, retain value/utils legs and all feature/Miri flags, and add Miri-aware union/duplicate/count guards and provisional cold budgets. [effort: high] — refs REVIEW.md#F-MIRI-002
- [ ] 4.3 Validate YAML, shell guard self-tests, both pointer discoveries, and Miri --list manifests for every configuration without executing full local Miri suites; record counts and partition proof. [effort: default] — refs REVIEW.md#F-MIRI-001, REVIEW.md#F-MIRI-002
- [ ] 4.4 Dispatch and monitor CI pointer/library gates and Miri advisory shards on the implementation SHA, collect cold and warm setup/compile/execute timings, and refine budgets only from evidence until all blocking controls pass and advisory suites terminate within budget. [effort: extra-high] — refs REVIEW.md#F-MIRI-001, REVIEW.md#F-MIRI-002, REVIEW.md#F-DIAG-001

## Phase 5: Hosted acceptance and documentation
- [ ] 5.1 Dispatch or observe CI, Miri, and aligned Fuzz on the final candidate SHA; record run URLs, artifact links, exact counts, timings, conclusions and deferred diagnostics, distinguishing advisory failures from unfinished runs. [effort: extra-high] — refs REVIEW.md#F-EVID-001, REVIEW.md#F-DIAGFIND-001
- [ ] 5.2 Update docs/CI.md and docs/FUZZING.md with the final pins/cache identities, exact shard manifests/counts, measured cold/warm budgets, run links and tested SHA, correcting contradictory corpus claims; run doc drift and exact rustdoc checks. [effort: default] — refs REVIEW.md#F-DOC-002
- [ ] 5.3 Audit completion criteria individually: all four corpora actually execute, native-free fixture passes normal/fuzzing, exact rustdoc passes, every blocking pointer/library control passes, advisory suites finish within configured budgets, and remaining findings and tested SHA are recorded. [effort: default] — refs REVIEW.md#F-EVID-001
