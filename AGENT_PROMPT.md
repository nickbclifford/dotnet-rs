You are repairing CI validation for dotnet-rs, not changing runtime pointer semantics. Work on one assigned step, preserve the merged M1c/M1d controls, and use measured evidence rather than treating a green advisory workflow as proof that its tests ran.

## Project context

- Repository: `/home/nick/Desktop/dotnet-rs`; branch: `supervised/unblock-ci`. dotnet-rs is a Rust .NET/CLI VM: assembly metadata/loading, resolution, interpreter, GC, intrinsics, P/Invoke, tracing and benchmarks across crates plus xtask/scripts/tests.
- Goal: restore reliable validation before the next pointer-provenance package. Make all four blocking corpus replays execute their checked-in inputs; replace native console-dependent finalizer observation with a strong managed lifecycle oracle; confirm the committed BenchHarness::run_prepared link with the exact rustdoc gate; split/rebudget blocking pointer controls and advisory VM/assembly Miri using hosted timings, preserving coverage/status/count guards; retain classified failure/timeout evidence; document exact tested SHA, runs and budgets.
- Out of scope: runtime provenance changes in VM/memory/value, interpreter/test performance optimization, strict-provenance enablement, unrelated CI cleanup, new allow-list exceptions or weaker native denial. Record real UB/provenance findings, do not repair them here.
- Stable pin is **1.95.0** via rust-toolchain.toml. Miri/fuzz target compiler is **nightly-2026-05-27**. Chosen cargo-fuzz is **0.13.1 built with 1.95.0 using --locked**, not built with the nightly. This combination installed locally and replayed three value corpora successfully; the step plan may update pins only with evidence. Initial analyzed HEAD is c01ba637297881f8614fc73fc6de72ff84f2e478; historical hosted main evidence is older bec18bd45495134b4002f482cf85e06d52fac59a, not proof of the merged tree.
- Canonical CI commands (run from repo unless noted): `cargo fmt --all -- --check`; `DOTNET_SKIP_BUILD=1 cargo clippy --all-targets --no-default-features [--features <matrix>] -- -D warnings`; `cargo build --verbose --no-default-features [--features <matrix>]`; `RUST_TEST_THREADS=1 DOTNET_TEST_TIMEOUT_SECS=180 DOTNET_USE_PREBUILT_FIXTURES=1 cargo test --verbose --no-default-features [--features <matrix>] -- --nocapture`; **`DOTNET_SKIP_BUILD=1 RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps --no-default-features`**. Bracketed arguments in this paragraph are explanatory, not literal shell syntax. Rustdoc does not deny every warning.
- Use `cargo run --quiet -p xtask -- matrix test-features --format json` (or clippy-features) for matrices; `cargo run -p xtask -- fixtures build` for fixtures, `cargo run --quiet -p xtask -- fixtures cache-key --profile debug` for the content key. Fixture .cs changes already invalidate the cache through the shared hash. `./check.sh --fast` is the local broad clippy/nextest wrapper, not the narrow default for a workflow repair; --full resolves the full matrix. bench.fish is profiling, not this package's acceptance gate. CI tests use cargo test; Loom uses `RUSTFLAGS="--cfg loom" cargo nextest run -p dotnet-vm --test loom_stw --no-default-features`.
- Pinned Miri flags stay `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`. Pointer controls use `bash scripts/pointer_provenance.sh verify-controls --configuration <no-features|multithreading>` then `controls --configuration <cfg> --shard <id>`. Never run diagnostic ignored probes natively; only p19 has native diagnostic permission.
- You are authorized to use gh for Actions runs, jobs, annotations, logs/artifacts and to dispatch/monitor validation runs. Common commands: `gh run list --branch supervised/unblock-ci`; `gh run list --workflow ci.yml --limit 30`; `gh run view RUN --json headSha,jobs,url`; `gh run view RUN --log-failed`; `gh run view RUN --job JOB --log`; `gh api repos/nickbclifford/dotnet-rs/actions/runs/RUN/jobs --paginate`; `gh api repos/nickbclifford/dotnet-rs/check-runs/JOB/annotations`; `gh run download RUN --dir /tmp/ci-artifacts-RUN`; `gh workflow run ci.yml --ref supervised/unblock-ci` (also miri.yml/fuzz.yml, after dispatch triggers exist); `gh run watch RUN --exit-status`; `gh run rerun RUN --job JOB` only for the same commit. Dispatch operates on workflows, not individual jobs. Observe affected jobs or rerun their existing job IDs; never claim a job rerun tests a newer SHA. Existing push/PR triggers target main/master, so branch push alone is not necessarily enough.
- Commit only assigned changes with existing `type(scope): summary` convention. When the step explicitly requires hosted validation, commit the scoped work and push only supervised/unblock-ci, then dispatch or observe its PR run. Record `git rev-parse HEAD` and remote run headSha equality. Avoid pushing again while a useful evidence run is active: concurrency cancels older runs. Do not push unrelated working-tree changes or change remote branch protection.

{{TOOL_CAPABILITIES}}

## Assignment

You are executing step {{STEP_ID}} — {{STEP_TITLE}} in phase {{PHASE_NUMBER}} ({{PHASE_TITLE}}).

{{STEP_DESCRIPTION}}

## This phase's checklist items

{{CHECKLIST_ITEMS}}

## Relevant review section

{{REVIEW_SECTION}}

## Prior memory

{{PRIOR_MEMORY}}

## Full checklist snapshot

{{CHECKLIST_SNAPSHOT}}

## Mandatory procedure

1. Read `AGENT_MEMORY.md` in full before doing anything.
2. Verify the code at the cited location still matches what REVIEW.md describes before touching it.
   - Inspect HEAD/status and repository instructions. Line numbers drift; use the quoted job/function identity. Historical main results are not current-HEAD validation. Check the reviewed corpus and selector manifests against actual current files.
3. Make only the scoped change. Do not fix adjacent issues, even if tempting.
   - Never add continue-on-error to blocking work, add ignores, drop tests/seeds, lower workload/iteration counts, loosen denial policy or add native exceptions. Tests-only scalar observation is allowed; runtime semantic changes are not.
4. Run the narrowest verification that proves the change is correct.
   - YAML/static checks are necessary but not hosted behavioral proof. Record command, exit status, counts, cache class and full tested SHA. Preserve logs even on failure.
   - Budget exhaustion is a distinct failure class. Do not silently increase timeouts without setup/compile/execution evidence; provisional budgets do not count as validated. Advisory UB/test failure may remain explicitly recorded, but unfinished/time-exhausted suites do not satisfy completion.
5. Tick the step box in `CHECKLIST.md` (change `- [ ]` to `- [x]`).
   - Tick only when its verification succeeds and acceptance is met; on blocked/partial work leave it unchecked and explain.
6. Append an entry to `AGENT_MEMORY.md` using the format defined there.
   - Keep previous entries unchanged; include full SHA and hosted run IDs/URLs/attempts, exact diagnostic text and measurements. Distinguish implemented from verified, and measured from provisional.
7. Do not expand scope. If you discover additional work, add new checklist items at the end of the relevant phase and note them in your memory entry.
   - Use the next unused flat ID, one valid effort hint and an existing review finding reference; don't add a runtime repair to this CI package.
8. Surface concerns. If something in the review plan is wrong or more complex than expected, document it in your memory entry and do not proceed. Do not guess; stop and report.
   - Preserve evidence if GitHub access is unavailable. Explain which verification is missing; don't invent timing or mark an untested package complete.
9. Call `step.complete` as your final action with all fields filled in: `step.complete({ status, summary, verificationCommand, verificationResult, concerns?, newChecklistItems? })`.
   - The API tool is named `step_complete`; invoke it directly with an object, not by printing a call. Populate status, summary, verificationCommand, verificationResult and any concerns/newChecklistItems. On a partial step use blocked completion status and explain what remains. Use recorded run_check evidence when available.

## Verification scope

| Kind of change | Narrowest correct check |
|---|---|
| Workflow YAML/triggers/cache wiring | `actionlint .github/workflows/ci.yml .github/workflows/miri.yml .github/workflows/fuzz.yml` if installed (record unavailability); `git diff --check`; inspect expression/working-directory expansion. Dispatch affected workflow using `gh workflow run <file>.yml --ref supervised/unblock-ci`, monitor affected jobs and record headSha; targeted repeat on same SHA with `gh run rerun RUN --job JOB`. Static success alone is not acceptance. |
| cargo-fuzz install/pins/cache | `cargo +1.95.0 install cargo-fuzz --version 0.13.1 --locked --root /tmp/ci-fuzz-driver`; put `/tmp/ci-fuzz-driver/bin` first in PATH; `cargo +nightly-2026-05-27 fuzz --version`; `(cd crates/dotnet-value && ASAN_OPTIONS=detect_leaks=0 cargo +nightly-2026-05-27 fuzz run fuzz_raw_memory_access fuzz/corpus/fuzz_raw_memory_access -- -runs=0)`. Then hosted CI fuzz-corpus-replay four jobs: seed census129/171/18/252; execution count at least130/172/19/253 plus successful INITED/DONE and matching loaded file counts. Binary cache cold miss and warm hit must report intended version/build compiler. |
| Replay execution parser | Test synthetic good/missing/zero/truncated/mismatched output and nonzero child exits; N>0 regular nonempty tracked files, `files found` and `seed corpus` equalN, INITED/DONE/final X agree and X>=N+1. Do not require X=N+1: local roundtrip129 files yielded182 callbacks because libFuzzer can rerun inputs for leak checks. Do not confuse `corp:` retained representatives with executed inputs. |
| Finalizer fixture/harness/build registration | `cargo run -p xtask -- fixtures build`; `DOTNET_USE_PREBUILT_FIXTURES=1 cargo test -p dotnet-cli --test integration_tests --no-default-features integration_tests_impl::fixtures::gc_finalizer_after_entrypoint_0 -- --exact --test-threads=1 --nocapture`; repeat adding `--features fuzzing` before selector, require1 passed/0 ignored. Also run exact `integration_tests_impl::fixtures::finalizer_runs_after_entrypoint_completes` normal/fuzzing once adapted, and MT lifecycle coverage. Prove once/phase negative controls; compare xtask fixture cache-key before/after source and stale prebuilt rejection. Hosted CI Test Suite (fuzzing) and a normal leg must pass; tail GC placement and post-executor observation must survive. |
| Rustdoc/link | Exactly `DOTNET_SKIP_BUILD=1 RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps --no-default-features` on repository1.95.0; hosted CI Documentation and Source Policy Checks must pass same command. Preserve unrelated private-link warning as diagnostic, not scope expansion. |
| Pointer shard selectors/guards | `bash -n scripts/pointer_provenance.sh`; `python3 scripts/tests/test_pointer_provenance.py`; `bash scripts/pointer_provenance.sh verify-controls --configuration no-features` and multithreading; require33 enabled controls each, partition exactly once, count1 per proposed singleton. Use `controls --configuration <cfg> --shard <id> --dry-run` for expansion, then hosted corresponding CI shards and all library gates, not full local Miri. |
| Advisory Miri selectors | `cargo +nightly-2026-05-27 miri test -p dotnet-assemblies -- --list`; `cargo +nightly-2026-05-27 miri test -p dotnet-vm --no-default-features -- --list jmp_tests tail_calls fault_tests`; repeat VM with `--features multithreading`. Require38 and8+8 manifests, guard empty ancillary selections, exact-once union and ignored0. Then hosted miri.yml A/V shards plus retained value/utils; record cold/warm phases and all conclusions, not just workflow status. |
| Diagnostics wrapper | `bash -n scripts/ci_diagnostics.sh` if shell-based, focused new script tests invoking synthetic setup error/test panic/Miri Undefined Behavior/timeout124/TERM/missing count and successful commands. Assert child status and both streams retained, expected classification, partial metadata and artifact-safe paths; inspect hosted always-upload artifact and summary. |
| CI/FUZZING documentation only | `bash scripts/check_doc_drift.sh`; `bash scripts/check_plan_queue.sh`; exact rustdoc gate above where relevant. Check links/anchors and documented manifests against runner source; no full native/Miri suite is required solely for prose. Tested code SHA and later docs SHA must be distinguished. |

Your very last action is calling `step.complete` through `step_complete`. Emit nothing and call no tool after it.
