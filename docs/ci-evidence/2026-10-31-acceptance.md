# Hosted candidate evidence — 2026-10-31

Tested candidate: `b8e98239cec56a4e62c38682c27d3a59886262a0`. All runs below are attempt **1**, workflow_dispatch. Later evidence/documentation commits are not this tested SHA.

## Conclusions

- [CI](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062): **99/99 jobs success**, including all 66 blocking pointer singleton shards, all library gates, all 10 native test matrix legs, four guarded replays, rustdoc and source policy.
- [Miri](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847): **57/57 jobs success**: matrix resolver, 54 advisory singletons (38 assembly/default + 8 VM/no-features + 8 VM/multithreading), retained value/default and utils/no-features. Each singleton actually reports **1 passed; 0 failed; 0 ignored**; all discovery/partition guards and all 518 diagnostic phases across CI/Miri classify success. No unfinished, cancelled, timed-out or UB-reporting phase.
- [Fuzz](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237425991): job success **with an advisory executor failure**, not four successful campaigns. Three value campaigns completed 61 seconds each; executor terminated with panic/deadly signal/exit 77, then minimization and upload finished. This is a terminated diagnostic failure, not an unfinished suite or a Miri UB finding.

## Guarded corpus replay

Counts are tracked files / loaded seed files / agreeing INITED, DONE and final callbacks. Guarded replay output and manifests are in the linked diagnostic artifacts. Rechecked downloaded logs with the current parser; these are not inferred from green status or `corp:` representatives.

| Target | Files/seeds | Executions | Setup / compile / execute (s) | Binary cache at execute | Artifact |
|---|---:|---:|---|---|---|
| fuzz_executor | 252/252 | 505 | — / 118.502 / 102.313 | `rust-cache-hit=false,cargo-fuzz-hit=true` | [artifact 13056389634](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056389634) |
| fuzz_managed_ptr_offset | 171/171 | 172 | 18.712 / 65.239 / 0.309 | `rust-cache-hit=false,cargo-fuzz-hit=` | [artifact 13056505066](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056505066) |
| fuzz_managed_ptr_roundtrip | 129/129 | 141 | 13.695 / 50.565 / 0.239 | `rust-cache-hit=false,cargo-fuzz-hit=` | [artifact 13056337557](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056337557) |
| fuzz_raw_memory_access | 18/18 | 19 | 18.178 / 64.851 / 0.261 | `rust-cache-hit=false,cargo-fuzz-hit=` | [artifact 13056500336](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056500336) |

All four used cargo-fuzz 0.13.1, built by 1.95.0 with --locked, and nightly-2026-05-27 target compiler. Binary cache key: `Linux-X64-cargo-fuzz-v2-0.13.1-build-1.95.0-locked-default`; blank hit output means miss (install ran), not an unknown warm hit. Rust target caches missed for these replay jobs.

## Measured shard timings

Seconds from diagnostic status files, not total workflow wall time (which includes runner queueing). `warm` means primary Rust cache hit in the start metadata, not faster execution. Hit samples cover only 12 pointer and 6 assembly shards; **no VM warm sample** in this run. Misses do not prove an empty entire runner filesystem. These are observational samples, not an upper bound on future timing or proof of runtime soundness. Prior cold attempts on 0bedd2e0 reached pointer execution 1658s and assembly 1396s; do not replace those maxima with this faster run. Budgets remain unchanged.

| Group/cache | Shards | Setup min–max | Compile min–max (individual phase) | Guard min–max | Execute min–max |
|---|---:|---:|---:|---:|---:|
| pointer/miss | 54 | 12.567–22.198 | 19.192–59.618 | — | 310.545–1039.753 |
| pointer/warm | 12 | 13.095–20.504 | 4.156–34.188 | — | 345.014–820.099 |
| assembly/miss | 32 | 12.703–22.276 | 13.544–33.741 | 5.125–9.782 | 2.001–967.304 |
| assembly/warm | 6 | 15.569–20.285 | 6.803–14.713 | 7.067–9.417 | 2.901–985.964 |
| VM no-features/miss | 8 | 13.641–20.481 | 23.003–39.662 | 15.252–24.454 | 492.103–798.104 |
| VM multithreading/miss | 8 | 15.795–20.397 | 25.939–39.285 | 18.787–27.419 | 376.020–794.122 |

Pointer caps: job 45min, shared inner deadline 42min from diagnostics start, setup 600s, native discovery 1200s, Miri build 1200s, execute 2700s (also constrained by remaining shared deadline and script timeout). Advisory singleton caps: job 70min, shared inner deadline 67min, setup/compile/guard 300s each, execute 3000s. The deadline calculation subtracts three minutes for evidence from the first step's start; earlier runner startup can reduce the actual window, and hard cancellation can prevent upload. Individual caps cannot be added as though all could consume their full budget. No timeout was raised or workload reduced. All measured executions fit their effective deadlines. Cache misses remain common; cache residency is not assumed or guaranteed.

## Library and native gates

- Blocking value: default library 74 passed/3 ignored, additional integration binary 1 passed/0 ignored; focused no-feature `pointer::tests::heap_access_`/`ptr_arith_`/`owner_retention_` selections 3/8/11 passed, 0 ignored.
- Blocking frame controls, each no-features and multithreading: raw-block 7 passed/0 ignored and frame-allocation 8 passed/0 ignored. Owner retention: 8 passed/0 ignored in each configuration.
- Advisory retained value/default: 74 passed/3 ignored plus integration 1 passed; utils/no-features: 32 passed/0 ignored. This corrects the swapped value/utils counts in the preceding 4.4 memory entry.
- Native fixture selectors `gc_finalizer_after_entrypoint_0` and `finalizer_runs_after_entrypoint_completes` both report `ok`, not ignored, in normal and fuzzing (also all other matrix legs).
- Hosted Documentation and Source Policy Checks passed the exact `DOTNET_SKIP_BUILD=1 RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps --no-default-features` gate. Private-link warning remains nonfatal; this gate does not deny every warning.

## Deferred diagnostics (no runtime repairs authorized)

- Current Fuzz job [124644025720](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237425991/job/124644025720): three completed value campaigns: roundtrip 4,737,014 callbacks, offset 19,348,852, raw 27,862,754, each `Done ... in 61 second(s)`. Initialization 142/172/19 respectively. Rust and driver caches missed; stable driver install 19s. Step wall times including builds: roundtrip 129s, offset 62s, raw 62s; executor 313s, minimization 178s. These are not separated phase budgets.
- Executor loaded 252 seeds and reached #505 INITED, then at 20:17:57Z panicked at `crates/dotnet-vm/src/stack/call_ops_impl.rs:386:75`: `call argument consumption exceeded current frame stack height: VM invariant violated`; `ERROR: libFuzzer: deadly signal`, target exit 77. No final DONE/callback total, so do not claim the 300-second campaign completed successfully. API step conclusion is success only because continue-on-error tolerates failure.
- [Current fuzz artifact 13055518930](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237425991/artifacts/13055518930) contains 14 files: original 115-byte `crash-40a43da48f9af34322f8c335a1f1ca18043e956e` and 13 minimization outputs, down to 5 bytes (`minimized-from-9c2dfcac80d5c7229ce03e889ce5ff0cc57032bf`). Downloaded under `/tmp/ci-step51/fuzz-art`. Root cause/input validity uninvestigated; deferred to runtime/fuzz owners outside this CI package.
- Historical executor stack_ops_impl.rs:23:14 panic and call_ops_impl.rs:386:75 panic are retained in prior memory; [historical artifact 13015555143](https://github.com/nickbclifford/dotnet-rs/actions/runs/41133149130/artifacts/13015555143). Historical `timeout-9ef65340e0aa7b9c713001ca0799997155c93c07` (1752s, exit 70) from [run 41005354582](https://github.com/nickbclifford/dotnet-rs/actions/runs/41005354582) remains an unexplained timeout, not a current regression or UB proof.
- Diagnostic-only p09 remains reported-UB inventory evidence, not a blocking control; no ignored diagnostic probe was run natively. No new Miri UB report was observed in these accepted controls/selections.
- Rustdoc warning: public documentation for `clear` links to private item `Self::invalidate_after_collection` at managed_ptr_decode_cache.rs:70. Deferred, not suppressed.
- Monitoring hit a temporary GitHub core API rate limit at 20:57Z from default 3-second gh run watch polling; watchers stopped, polling resumed after 21:07:32Z reset. No hosted jobs were cancelled or rerun. Final snapshots/logs/artifacts were fetched after completion. Use slower watch intervals in future.

## Complete job ledger

Every listed job concluded success; for Fuzz, see the advisory failure above. Wall seconds exclude queued time. Links lead to job logs and summaries.

| Workflow/job | Job link | Wall seconds |
|---|---|---:|
| CI: Build .NET Fixtures | [124644014725](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644014725) | 61 |
| CI: Build Script Regression Probes | [124644014961](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644014961) | 141 |
| CI: Clippy Check | [124644097529](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644097529) | 27 |
| CI: Clippy Check (fuzzing) | [124644097501](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644097501) | 35 |
| CI: Clippy Check (generic-constraint-validation) | [124644097474](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644097474) | 51 |
| CI: Clippy Check (memory-validation) | [124644097669](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644097669) | 40 |
| CI: Clippy Check (metadata-validation) | [124644097563](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644097563) | 34 |
| CI: Clippy Check (multithreading) | [124644097568](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644097568) | 42 |
| CI: Clippy Check (multithreading,memory-validation) | [124644097525](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644097525) | 37 |
| CI: Clippy Check (multithreading,simd) | [124644097608](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644097608) | 47 |
| CI: Clippy Check (multithreading,validation-all) | [124644097504](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644097504) | 28 |
| CI: Clippy Check (simd) | [124644097574](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644097574) | 36 |
| CI: Documentation and Source Policy Checks | [124644014913](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644014913) | 61 |
| CI: Format Check | [124644015003](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015003) | 18 |
| CI: Fuzz fuzz_executor Corpus | [124644015037](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015037) | 263 |
| CI: Fuzz fuzz_managed_ptr_offset Corpus | [124644015065](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015065) | 127 |
| CI: Fuzz fuzz_managed_ptr_roundtrip Corpus | [124644015008](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015008) | 103 |
| CI: Fuzz fuzz_raw_memory_access Corpus | [124644014938](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644014938) | 146 |
| CI: Loom STW Model | [124644014937](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644014937) | 101 |
| CI: Miri (dotnet-value) | [124644014947](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644014947) | 154 |
| CI: Miri frame blocks | [124644014925](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644014925) | 101 |
| CI: Miri owner retention | [124644015011](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015011) | 70 |
| CI: Miri pointer controls (multithreading, shard 1) | [124644015070](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015070) | 861 |
| CI: Miri pointer controls (multithreading, shard 10) | [124644015040](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015040) | 883 |
| CI: Miri pointer controls (multithreading, shard 11) | [124644015069](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015069) | 903 |
| CI: Miri pointer controls (multithreading, shard 12) | [124644015139](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015139) | 1060 |
| CI: Miri pointer controls (multithreading, shard 13) | [124644015007](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015007) | 783 |
| CI: Miri pointer controls (multithreading, shard 14) | [124644015297](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015297) | 904 |
| CI: Miri pointer controls (multithreading, shard 15) | [124644015107](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015107) | 657 |
| CI: Miri pointer controls (multithreading, shard 16) | [124644015052](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015052) | 836 |
| CI: Miri pointer controls (multithreading, shard 17) | [124644015083](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015083) | 740 |
| CI: Miri pointer controls (multithreading, shard 18) | [124644014971](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644014971) | 521 |
| CI: Miri pointer controls (multithreading, shard 19) | [124644014991](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644014991) | 924 |
| CI: Miri pointer controls (multithreading, shard 2) | [124644015120](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015120) | 1051 |
| CI: Miri pointer controls (multithreading, shard 20) | [124644015012](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015012) | 821 |
| CI: Miri pointer controls (multithreading, shard 21) | [124644015131](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015131) | 825 |
| CI: Miri pointer controls (multithreading, shard 22) | [124644015124](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015124) | 849 |
| CI: Miri pointer controls (multithreading, shard 23) | [124644015165](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015165) | 680 |
| CI: Miri pointer controls (multithreading, shard 24) | [124644015048](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015048) | 588 |
| CI: Miri pointer controls (multithreading, shard 25) | [124644015097](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015097) | 678 |
| CI: Miri pointer controls (multithreading, shard 26) | [124644015046](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015046) | 670 |
| CI: Miri pointer controls (multithreading, shard 27) | [124644015095](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015095) | 923 |
| CI: Miri pointer controls (multithreading, shard 28) | [124644015086](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015086) | 978 |
| CI: Miri pointer controls (multithreading, shard 29) | [124644015098](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015098) | 842 |
| CI: Miri pointer controls (multithreading, shard 3) | [124644015084](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015084) | 615 |
| CI: Miri pointer controls (multithreading, shard 30) | [124644015076](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015076) | 746 |
| CI: Miri pointer controls (multithreading, shard 31) | [124644015045](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015045) | 1046 |
| CI: Miri pointer controls (multithreading, shard 32) | [124644017626](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644017626) | 1055 |
| CI: Miri pointer controls (multithreading, shard 33) | [124644017766](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644017766) | 858 |
| CI: Miri pointer controls (multithreading, shard 4) | [124644014992](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644014992) | 754 |
| CI: Miri pointer controls (multithreading, shard 5) | [124644015071](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015071) | 690 |
| CI: Miri pointer controls (multithreading, shard 6) | [124644015142](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015142) | 576 |
| CI: Miri pointer controls (multithreading, shard 7) | [124644015159](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015159) | 411 |
| CI: Miri pointer controls (multithreading, shard 8) | [124644015104](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015104) | 879 |
| CI: Miri pointer controls (multithreading, shard 9) | [124644014979](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644014979) | 583 |
| CI: Miri pointer controls (no-features, shard 1) | [124644015172](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015172) | 783 |
| CI: Miri pointer controls (no-features, shard 10) | [124644015082](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015082) | 839 |
| CI: Miri pointer controls (no-features, shard 11) | [124644015394](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015394) | 1184 |
| CI: Miri pointer controls (no-features, shard 12) | [124644014969](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644014969) | 842 |
| CI: Miri pointer controls (no-features, shard 13) | [124644015066](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015066) | 728 |
| CI: Miri pointer controls (no-features, shard 14) | [124644015085](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015085) | 742 |
| CI: Miri pointer controls (no-features, shard 15) | [124644015044](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015044) | 475 |
| CI: Miri pointer controls (no-features, shard 16) | [124644014976](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644014976) | 945 |
| CI: Miri pointer controls (no-features, shard 17) | [124644015058](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015058) | 951 |
| CI: Miri pointer controls (no-features, shard 18) | [124644015227](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015227) | 813 |
| CI: Miri pointer controls (no-features, shard 19) | [124644015027](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015027) | 736 |
| CI: Miri pointer controls (no-features, shard 2) | [124644015217](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015217) | 1176 |
| CI: Miri pointer controls (no-features, shard 20) | [124644015134](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015134) | 990 |
| CI: Miri pointer controls (no-features, shard 21) | [124644014998](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644014998) | 683 |
| CI: Miri pointer controls (no-features, shard 22) | [124644015034](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015034) | 881 |
| CI: Miri pointer controls (no-features, shard 23) | [124644015164](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015164) | 768 |
| CI: Miri pointer controls (no-features, shard 24) | [124644015151](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015151) | 682 |
| CI: Miri pointer controls (no-features, shard 25) | [124644015049](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015049) | 416 |
| CI: Miri pointer controls (no-features, shard 26) | [124644015093](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015093) | 410 |
| CI: Miri pointer controls (no-features, shard 27) | [124644015072](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015072) | 947 |
| CI: Miri pointer controls (no-features, shard 28) | [124644015273](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015273) | 659 |
| CI: Miri pointer controls (no-features, shard 29) | [124644015100](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015100) | 904 |
| CI: Miri pointer controls (no-features, shard 3) | [124644015043](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015043) | 1075 |
| CI: Miri pointer controls (no-features, shard 30) | [124644015031](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015031) | 604 |
| CI: Miri pointer controls (no-features, shard 31) | [124644015088](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015088) | 446 |
| CI: Miri pointer controls (no-features, shard 32) | [124644015191](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015191) | 425 |
| CI: Miri pointer controls (no-features, shard 33) | [124644015000](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015000) | 787 |
| CI: Miri pointer controls (no-features, shard 4) | [124644015080](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015080) | 779 |
| CI: Miri pointer controls (no-features, shard 5) | [124644015026](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015026) | 938 |
| CI: Miri pointer controls (no-features, shard 6) | [124644015016](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015016) | 689 |
| CI: Miri pointer controls (no-features, shard 7) | [124644015115](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015115) | 1150 |
| CI: Miri pointer controls (no-features, shard 8) | [124644015102](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015102) | 1045 |
| CI: Miri pointer controls (no-features, shard 9) | [124644015074](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644015074) | 875 |
| CI: Resolve Feature Matrices | [124644014899](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644014899) | 18 |
| CI: Test Suite | [124644263691](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644263691) | 167 |
| CI: Test Suite (fuzzing) | [124644263682](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644263682) | 109 |
| CI: Test Suite (generic-constraint-validation) | [124644263672](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644263672) | 199 |
| CI: Test Suite (memory-validation) | [124644263679](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644263679) | 134 |
| CI: Test Suite (metadata-validation) | [124644263728](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644263728) | 209 |
| CI: Test Suite (multithreading) | [124644263699](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644263699) | 252 |
| CI: Test Suite (multithreading,memory-validation) | [124644263678](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644263678) | 276 |
| CI: Test Suite (multithreading,simd) | [124644263687](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644263687) | 230 |
| CI: Test Suite (multithreading,validation-all) | [124644263681](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644263681) | 185 |
| CI: Test Suite (simd) | [124644263670](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/job/124644263670) | 251 |
| Miri: Miri (dotnet-assemblies, default-features, A01) | [124644301947](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644301947) | 400 |
| Miri: Miri (dotnet-assemblies, default-features, A02) | [124644301878](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644301878) | 363 |
| Miri: Miri (dotnet-assemblies, default-features, A03) | [124644301879](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644301879) | 848 |
| Miri: Miri (dotnet-assemblies, default-features, A04) | [124644301874](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644301874) | 833 |
| Miri: Miri (dotnet-assemblies, default-features, A05) | [124644301924](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644301924) | 638 |
| Miri: Miri (dotnet-assemblies, default-features, A06) | [124644301910](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644301910) | 185 |
| Miri: Miri (dotnet-assemblies, default-features, A07) | [124644301843](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644301843) | 96 |
| Miri: Miri (dotnet-assemblies, default-features, A08) | [124644301929](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644301929) | 98 |
| Miri: Miri (dotnet-assemblies, default-features, A09) | [124644301937](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644301937) | 84 |
| Miri: Miri (dotnet-assemblies, default-features, A10) | [124644302095](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302095) | 74 |
| Miri: Miri (dotnet-assemblies, default-features, A11) | [124644302030](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302030) | 89 |
| Miri: Miri (dotnet-assemblies, default-features, A12) | [124644301880](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644301880) | 87 |
| Miri: Miri (dotnet-assemblies, default-features, A13) | [124644302007](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302007) | 1059 |
| Miri: Miri (dotnet-assemblies, default-features, A14) | [124644302031](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302031) | 691 |
| Miri: Miri (dotnet-assemblies, default-features, A15) | [124644301954](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644301954) | 83 |
| Miri: Miri (dotnet-assemblies, default-features, A16) | [124644302043](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302043) | 440 |
| Miri: Miri (dotnet-assemblies, default-features, A17) | [124644302009](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302009) | 1065 |
| Miri: Miri (dotnet-assemblies, default-features, A18) | [124644302052](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302052) | 787 |
| Miri: Miri (dotnet-assemblies, default-features, A19) | [124644302019](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302019) | 765 |
| Miri: Miri (dotnet-assemblies, default-features, A20) | [124644302042](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302042) | 651 |
| Miri: Miri (dotnet-assemblies, default-features, A21) | [124644302124](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302124) | 699 |
| Miri: Miri (dotnet-assemblies, default-features, A22) | [124644302067](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302067) | 520 |
| Miri: Miri (dotnet-assemblies, default-features, A23) | [124644302055](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302055) | 93 |
| Miri: Miri (dotnet-assemblies, default-features, A24) | [124644302029](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302029) | 713 |
| Miri: Miri (dotnet-assemblies, default-features, A25) | [124644302020](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302020) | 528 |
| Miri: Miri (dotnet-assemblies, default-features, A26) | [124644302131](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302131) | 662 |
| Miri: Miri (dotnet-assemblies, default-features, A27) | [124644301975](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644301975) | 747 |
| Miri: Miri (dotnet-assemblies, default-features, A28) | [124644302075](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302075) | 867 |
| Miri: Miri (dotnet-assemblies, default-features, A29) | [124644302102](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302102) | 591 |
| Miri: Miri (dotnet-assemblies, default-features, A30) | [124644302141](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302141) | 641 |
| Miri: Miri (dotnet-assemblies, default-features, A31) | [124644302021](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302021) | 889 |
| Miri: Miri (dotnet-assemblies, default-features, A32) | [124644301995](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644301995) | 67 |
| Miri: Miri (dotnet-assemblies, default-features, A33) | [124644302062](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302062) | 66 |
| Miri: Miri (dotnet-assemblies, default-features, A34) | [124644301989](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644301989) | 73 |
| Miri: Miri (dotnet-assemblies, default-features, A35) | [124644301974](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644301974) | 81 |
| Miri: Miri (dotnet-assemblies, default-features, A36) | [124644302038](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302038) | 100 |
| Miri: Miri (dotnet-assemblies, default-features, A37) | [124644302071](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302071) | 96 |
| Miri: Miri (dotnet-assemblies, default-features, A38) | [124644302041](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302041) | 73 |
| Miri: Miri (dotnet-utils, no-features) | [124644019849](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644019849) | 70 |
| Miri: Miri (dotnet-value, default-features) | [124644019881](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644019881) | 112 |
| Miri: Miri (dotnet-vm, multithreading, V01) | [124644302239](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302239) | 764 |
| Miri: Miri (dotnet-vm, multithreading, V02) | [124644302122](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302122) | 920 |
| Miri: Miri (dotnet-vm, multithreading, V03) | [124644302118](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302118) | 702 |
| Miri: Miri (dotnet-vm, multithreading, V04) | [124644302167](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302167) | 846 |
| Miri: Miri (dotnet-vm, multithreading, V05) | [124644302093](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302093) | 619 |
| Miri: Miri (dotnet-vm, multithreading, V06) | [124644302125](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302125) | 473 |
| Miri: Miri (dotnet-vm, multithreading, V07) | [124644302145](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302145) | 818 |
| Miri: Miri (dotnet-vm, multithreading, V08) | [124644302135](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302135) | 730 |
| Miri: Miri (dotnet-vm, no-features, V01) | [124644302164](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302164) | 912 |
| Miri: Miri (dotnet-vm, no-features, V02) | [124644302059](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302059) | 685 |
| Miri: Miri (dotnet-vm, no-features, V03) | [124644302151](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302151) | 717 |
| Miri: Miri (dotnet-vm, no-features, V04) | [124644302066](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302066) | 611 |
| Miri: Miri (dotnet-vm, no-features, V05) | [124644302107](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302107) | 688 |
| Miri: Miri (dotnet-vm, no-features, V06) | [124644302092](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302092) | 630 |
| Miri: Miri (dotnet-vm, no-features, V07) | [124644302069](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302069) | 655 |
| Miri: Miri (dotnet-vm, no-features, V08) | [124644302063](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644302063) | 576 |
| Miri: advisory-matrix | [124644019742](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/job/124644019742) | 7 |
| Fuzz: fuzz | [124644025720](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237425991/job/124644025720) | 830 |

## Diagnostic artifact ledger

Each artifact contains phase start/status JSON, combined logs and summary; pointer artifacts also contain per-probe logs and discovery guard output, while advisory guard logs contain the Miri listings. Uploaded diagnostics expire after 14 days; durable measured results are transcribed here. Full downloaded evidence: `/tmp/ci-step51/` (ephemeral local storage). Table timings are phase totals in seconds (multiple compile or execute phases are summed); detailed per-phase bounds above use individual phases. All classifications success.

| Diagnostic case/artifact | Cache | Setup | Compile | Guard | Execute |
|---|---|---:|---:|---:|---:|
| miri-frame-blocks [artifact 13056156843](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056156843) | miss | 15.703 | 17.081 | 0.000 | 12.097 |
| miri-owner-retention [artifact 13056825721](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056825721) | miss | 13.553 | 12.394 | 0.000 | 16.465 |
| miri-value [artifact 13056206981](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056206981) | miss | 20.108 | 18.824 | 0.000 | 84.476 |
| pointer-multithreading-shard-1 [artifact 13056498027](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056498027) | miss | 19.021 | 77.563 | 0.000 | 713.348 |
| pointer-multithreading-shard-10 [artifact 13056324425](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056324425) | miss | 20.169 | 83.825 | 0.000 | 740.983 |
| pointer-multithreading-shard-11 [artifact 13056956760](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056956760) | miss | 20.283 | 80.507 | 0.000 | 756.917 |
| pointer-multithreading-shard-12 [artifact 13057691121](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057691121) | miss | 19.838 | 83.789 | 0.000 | 914.666 |
| pointer-multithreading-shard-13 [artifact 13056144682](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056144682) | warm | 19.841 | 40.996 | 0.000 | 685.295 |
| pointer-multithreading-shard-14 [artifact 13057358241](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057358241) | miss | 21.766 | 86.714 | 0.000 | 754.945 |
| pointer-multithreading-shard-15 [artifact 13057846430](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057846430) | miss | 13.217 | 59.015 | 0.000 | 550.324 |
| pointer-multithreading-shard-16 [artifact 13056538243](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056538243) | miss | 19.613 | 89.673 | 0.000 | 683.786 |
| pointer-multithreading-shard-17 [artifact 13057030031](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057030031) | warm | 20.180 | 39.458 | 0.000 | 642.492 |
| pointer-multithreading-shard-18 [artifact 13056439704](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056439704) | miss | 14.368 | 57.398 | 0.000 | 408.367 |
| pointer-multithreading-shard-19 [artifact 13056538203](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056538203) | warm | 20.504 | 39.376 | 0.000 | 820.099 |
| pointer-multithreading-shard-2 [artifact 13057970766](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057970766) | miss | 21.037 | 89.071 | 0.000 | 898.023 |
| pointer-multithreading-shard-20 [artifact 13056832237](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056832237) | warm | 19.578 | 33.985 | 0.000 | 725.123 |
| pointer-multithreading-shard-21 [artifact 13056199903](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056199903) | miss | 19.825 | 82.854 | 0.000 | 684.282 |
| pointer-multithreading-shard-22 [artifact 13057541912](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057541912) | miss | 20.253 | 90.412 | 0.000 | 694.480 |
| pointer-multithreading-shard-23 [artifact 13057535120](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057535120) | warm | 16.329 | 31.827 | 0.000 | 580.654 |
| pointer-multithreading-shard-24 [artifact 13056795247](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056795247) | miss | 13.593 | 63.837 | 0.000 | 463.850 |
| pointer-multithreading-shard-25 [artifact 13056418437](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056418437) | miss | 14.416 | 58.568 | 0.000 | 567.252 |
| pointer-multithreading-shard-26 [artifact 13055768469](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13055768469) | miss | 16.366 | 71.543 | 0.000 | 545.655 |
| pointer-multithreading-shard-27 [artifact 13056959296](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056959296) | miss | 19.856 | 88.955 | 0.000 | 776.084 |
| pointer-multithreading-shard-28 [artifact 13057686843](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057686843) | miss | 20.894 | 87.898 | 0.000 | 825.101 |
| pointer-multithreading-shard-29 [artifact 13056979772](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056979772) | miss | 19.883 | 82.741 | 0.000 | 695.439 |
| pointer-multithreading-shard-3 [artifact 13056661962](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056661962) | miss | 14.900 | 74.447 | 0.000 | 481.422 |
| pointer-multithreading-shard-30 [artifact 13056334013](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056334013) | miss | 20.254 | 83.062 | 0.000 | 598.927 |
| pointer-multithreading-shard-31 [artifact 13057297665](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057297665) | miss | 22.198 | 92.593 | 0.000 | 885.955 |
| pointer-multithreading-shard-32 [artifact 13058425523](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13058425523) | miss | 17.147 | 73.001 | 0.000 | 921.979 |
| pointer-multithreading-shard-33 [artifact 13058251529](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13058251529) | miss | 17.282 | 64.022 | 0.000 | 733.620 |
| pointer-multithreading-shard-4 [artifact 13056174643](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056174643) | warm | 20.065 | 37.574 | 0.000 | 657.552 |
| pointer-multithreading-shard-5 [artifact 13057033713](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057033713) | miss | 16.307 | 65.401 | 0.000 | 569.528 |
| pointer-multithreading-shard-6 [artifact 13057292291](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057292291) | miss | 15.985 | 57.274 | 0.000 | 458.232 |
| pointer-multithreading-shard-7 [artifact 13056834919](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056834919) | miss | 12.830 | 55.336 | 0.000 | 310.559 |
| pointer-multithreading-shard-8 [artifact 13057451443](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057451443) | miss | 19.561 | 81.818 | 0.000 | 735.075 |
| pointer-multithreading-shard-9 [artifact 13055943207](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13055943207) | warm | 14.692 | 23.510 | 0.000 | 500.466 |
| pointer-no-features-shard-1 [artifact 13057677210](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057677210) | miss | 19.520 | 85.987 | 0.000 | 634.172 |
| pointer-no-features-shard-10 [artifact 13058006458](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13058006458) | miss | 19.382 | 80.808 | 0.000 | 697.065 |
| pointer-no-features-shard-11 [artifact 13058216971](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13058216971) | miss | 20.700 | 83.922 | 0.000 | 1039.753 |
| pointer-no-features-shard-12 [artifact 13056766122](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056766122) | miss | 16.906 | 69.697 | 0.000 | 713.794 |
| pointer-no-features-shard-13 [artifact 13056529338](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056529338) | miss | 16.238 | 62.383 | 0.000 | 609.642 |
| pointer-no-features-shard-14 [artifact 13057911240](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057911240) | miss | 19.397 | 80.204 | 0.000 | 599.710 |
| pointer-no-features-shard-15 [artifact 13056282512](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056282512) | warm | 14.204 | 24.056 | 0.000 | 402.621 |
| pointer-no-features-shard-16 [artifact 13056790857](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056790857) | miss | 16.556 | 72.628 | 0.000 | 804.304 |
| pointer-no-features-shard-17 [artifact 13057780251](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057780251) | miss | 17.985 | 73.230 | 0.000 | 818.420 |
| pointer-no-features-shard-18 [artifact 13058066535](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13058066535) | miss | 19.995 | 82.376 | 0.000 | 672.236 |
| pointer-no-features-shard-19 [artifact 13057935678](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057935678) | miss | 19.983 | 74.898 | 0.000 | 593.180 |
| pointer-no-features-shard-2 [artifact 13058415776](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13058415776) | miss | 21.971 | 85.527 | 0.000 | 1023.013 |
| pointer-no-features-shard-20 [artifact 13057940107](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057940107) | miss | 20.478 | 85.411 | 0.000 | 840.662 |
| pointer-no-features-shard-21 [artifact 13055708738](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13055708738) | warm | 14.322 | 26.986 | 0.000 | 605.318 |
| pointer-no-features-shard-22 [artifact 13055668847](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13055668847) | warm | 20.114 | 40.002 | 0.000 | 777.099 |
| pointer-no-features-shard-23 [artifact 13057525632](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057525632) | miss | 17.089 | 65.345 | 0.000 | 648.501 |
| pointer-no-features-shard-24 [artifact 13057463080](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057463080) | miss | 16.407 | 65.453 | 0.000 | 561.076 |
| pointer-no-features-shard-25 [artifact 13056620355](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056620355) | warm | 13.095 | 22.997 | 0.000 | 345.014 |
| pointer-no-features-shard-26 [artifact 13057156277](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057156277) | miss | 12.567 | 53.784 | 0.000 | 310.545 |
| pointer-no-features-shard-27 [artifact 13056184527](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056184527) | miss | 20.609 | 83.363 | 0.000 | 792.872 |
| pointer-no-features-shard-28 [artifact 13058400087](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13058400087) | miss | 16.072 | 62.712 | 0.000 | 540.186 |
| pointer-no-features-shard-29 [artifact 13056589255](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056589255) | miss | 18.707 | 80.285 | 0.000 | 761.330 |
| pointer-no-features-shard-3 [artifact 13056527187](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056527187) | miss | 19.731 | 81.252 | 0.000 | 927.537 |
| pointer-no-features-shard-30 [artifact 13057130413](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057130413) | miss | 15.216 | 64.671 | 0.000 | 483.289 |
| pointer-no-features-shard-31 [artifact 13057022973](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057022973) | miss | 14.842 | 60.204 | 0.000 | 336.761 |
| pointer-no-features-shard-32 [artifact 13057100153](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057100153) | miss | 14.167 | 58.918 | 0.000 | 316.888 |
| pointer-no-features-shard-33 [artifact 13056840137](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056840137) | miss | 18.479 | 73.906 | 0.000 | 651.669 |
| pointer-no-features-shard-4 [artifact 13057755084](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057755084) | miss | 20.161 | 81.451 | 0.000 | 628.936 |
| pointer-no-features-shard-5 [artifact 13056848128](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056848128) | miss | 20.618 | 83.215 | 0.000 | 790.779 |
| pointer-no-features-shard-6 [artifact 13056555843](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056555843) | warm | 18.111 | 31.328 | 0.000 | 602.103 |
| pointer-no-features-shard-7 [artifact 13058130872](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13058130872) | miss | 20.971 | 85.178 | 0.000 | 1000.488 |
| pointer-no-features-shard-8 [artifact 13057039107](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13057039107) | miss | 20.255 | 82.754 | 0.000 | 897.884 |
| pointer-no-features-shard-9 [artifact 13056893273](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056893273) | miss | 19.890 | 87.674 | 0.000 | 727.534 |
| replay-fuzz_executor [artifact 13056389634](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056389634) | miss | 0.000 | 118.502 | 0.000 | 102.313 |
| replay-fuzz_managed_ptr_offset [artifact 13056505066](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056505066) | miss | 18.712 | 65.239 | 0.000 | 0.309 |
| replay-fuzz_managed_ptr_roundtrip [artifact 13056337557](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056337557) | miss | 13.695 | 50.565 | 0.000 | 0.239 |
| replay-fuzz_raw_memory_access [artifact 13056500336](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237422062/artifacts/13056500336) | miss | 18.178 | 64.851 | 0.000 | 0.261 |
| miri-dotnet-assemblies-default-features-A01 [artifact 13057015346](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057015346) | miss | 14.149 | 21.491 | 6.080 | 324.732 |
| miri-dotnet-assemblies-default-features-A02 [artifact 13056413931](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13056413931) | miss | 13.286 | 17.882 | 5.125 | 295.340 |
| miri-dotnet-assemblies-default-features-A03 [artifact 13057015318](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057015318) | miss | 13.767 | 18.279 | 5.503 | 779.643 |
| miri-dotnet-assemblies-default-features-A04 [artifact 13057030890](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057030890) | miss | 20.643 | 33.741 | 9.091 | 728.874 |
| miri-dotnet-assemblies-default-features-A05 [artifact 13056288864](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13056288864) | miss | 19.518 | 26.978 | 8.682 | 542.791 |
| miri-dotnet-assemblies-default-features-A06 [artifact 13056561153](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13056561153) | miss | 20.031 | 27.065 | 8.735 | 85.526 |
| miri-dotnet-assemblies-default-features-A07 [artifact 13055672809](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13055672809) | miss | 17.136 | 26.108 | 7.109 | 6.947 |
| miri-dotnet-assemblies-default-features-A08 [artifact 13056518688](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13056518688) | miss | 18.802 | 21.212 | 8.672 | 10.578 |
| miri-dotnet-assemblies-default-features-A09 [artifact 13056741280](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13056741280) | miss | 16.015 | 17.058 | 7.291 | 7.841 |
| miri-dotnet-assemblies-default-features-A10 [artifact 13057757258](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057757258) | miss | 14.145 | 19.094 | 5.885 | 2.413 |
| miri-dotnet-assemblies-default-features-A11 [artifact 13056676250](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13056676250) | miss | 16.499 | 22.663 | 7.067 | 3.603 |
| miri-dotnet-assemblies-default-features-A12 [artifact 13056352783](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13056352783) | warm | 20.285 | 14.052 | 9.417 | 3.725 |
| miri-dotnet-assemblies-default-features-A13 [artifact 13057470394](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057470394) | warm | 17.513 | 14.713 | 7.563 | 985.964 |
| miri-dotnet-assemblies-default-features-A14 [artifact 13056942466](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13056942466) | miss | 16.050 | 15.585 | 5.842 | 620.748 |
| miri-dotnet-assemblies-default-features-A15 [artifact 13056369473](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13056369473) | warm | 20.229 | 10.852 | 9.097 | 3.680 |
| miri-dotnet-assemblies-default-features-A16 [artifact 13057088084](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057088084) | miss | 14.313 | 14.975 | 5.743 | 372.065 |
| miri-dotnet-assemblies-default-features-A17 [artifact 13058283672](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13058283672) | miss | 20.784 | 30.920 | 9.532 | 967.304 |
| miri-dotnet-assemblies-default-features-A18 [artifact 13057770750](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057770750) | miss | 20.221 | 26.211 | 8.913 | 696.428 |
| miri-dotnet-assemblies-default-features-A19 [artifact 13057116413](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057116413) | miss | 14.377 | 18.917 | 5.875 | 689.925 |
| miri-dotnet-assemblies-default-features-A20 [artifact 13058371712](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13058371712) | miss | 17.574 | 25.864 | 7.095 | 567.807 |
| miri-dotnet-assemblies-default-features-A21 [artifact 13058376355](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13058376355) | miss | 19.245 | 27.181 | 8.821 | 607.224 |
| miri-dotnet-assemblies-default-features-A22 [artifact 13057359077](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057359077) | miss | 14.945 | 15.268 | 6.331 | 438.228 |
| miri-dotnet-assemblies-default-features-A23 [artifact 13057132413](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057132413) | miss | 20.227 | 22.868 | 9.782 | 3.669 |
| miri-dotnet-assemblies-default-features-A24 [artifact 13056722976](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13056722976) | warm | 19.861 | 6.803 | 9.067 | 640.927 |
| miri-dotnet-assemblies-default-features-A25 [artifact 13056508719](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13056508719) | warm | 15.569 | 11.358 | 7.067 | 459.160 |
| miri-dotnet-assemblies-default-features-A26 [artifact 13057109840](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057109840) | miss | 19.394 | 21.929 | 9.089 | 575.729 |
| miri-dotnet-assemblies-default-features-A27 [artifact 13057475582](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057475582) | miss | 19.941 | 27.410 | 9.655 | 651.942 |
| miri-dotnet-assemblies-default-features-A28 [artifact 13058103433](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13058103433) | miss | 20.198 | 22.688 | 9.414 | 773.911 |
| miri-dotnet-assemblies-default-features-A29 [artifact 13057639004](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057639004) | miss | 22.276 | 18.763 | 7.049 | 503.129 |
| miri-dotnet-assemblies-default-features-A30 [artifact 13057848355](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057848355) | miss | 16.487 | 18.223 | 8.068 | 561.234 |
| miri-dotnet-assemblies-default-features-A31 [artifact 13057722625](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057722625) | miss | 16.545 | 22.637 | 7.140 | 807.692 |
| miri-dotnet-assemblies-default-features-A32 [artifact 13057108533](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057108533) | miss | 13.089 | 13.736 | 5.216 | 2.082 |
| miri-dotnet-assemblies-default-features-A33 [artifact 13057146141](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057146141) | miss | 12.703 | 13.544 | 5.406 | 2.001 |
| miri-dotnet-assemblies-default-features-A34 [artifact 13056907242](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13056907242) | miss | 13.236 | 19.973 | 5.755 | 2.102 |
| miri-dotnet-assemblies-default-features-A35 [artifact 13056438825](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13056438825) | warm | 16.967 | 7.270 | 7.136 | 2.901 |
| miri-dotnet-assemblies-default-features-A36 [artifact 13056744920](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13056744920) | miss | 20.900 | 23.061 | 9.153 | 3.551 |
| miri-dotnet-assemblies-default-features-A37 [artifact 13057037311](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057037311) | miss | 20.630 | 21.214 | 8.978 | 3.402 |
| miri-dotnet-assemblies-default-features-A38 [artifact 13058006427](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13058006427) | miss | 14.814 | 18.897 | 5.268 | 2.140 |
| miri-dotnet-utils-no-features [artifact 13055569229](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13055569229) | miss | 20.890 | 5.718 | 0.000 | 5.803 |
| miri-dotnet-value-default-features [artifact 13056118519](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13056118519) | miss | 19.756 | 17.872 | 0.000 | 39.700 |
| miri-dotnet-vm-multithreading-V01 [artifact 13057859334](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057859334) | miss | 16.356 | 25.939 | 19.572 | 667.390 |
| miri-dotnet-vm-multithreading-V02 [artifact 13058162288](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13058162288) | miss | 20.109 | 38.236 | 27.419 | 794.122 |
| miri-dotnet-vm-multithreading-V03 [artifact 13058002050](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13058002050) | miss | 20.397 | 39.285 | 26.545 | 580.872 |
| miri-dotnet-vm-multithreading-V04 [artifact 13057983748](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057983748) | miss | 20.090 | 34.094 | 27.375 | 722.993 |
| miri-dotnet-vm-multithreading-V05 [artifact 13058306153](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13058306153) | miss | 17.018 | 30.251 | 21.498 | 512.606 |
| miri-dotnet-vm-multithreading-V06 [artifact 13057407434](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057407434) | miss | 15.795 | 29.519 | 18.787 | 376.020 |
| miri-dotnet-vm-multithreading-V07 [artifact 13058223379](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13058223379) | miss | 16.169 | 29.475 | 20.704 | 716.028 |
| miri-dotnet-vm-multithreading-V08 [artifact 13057875651](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057875651) | miss | 17.021 | 29.009 | 21.220 | 625.271 |
| miri-dotnet-vm-no-features-V01 [artifact 13058795291](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13058795291) | miss | 19.187 | 30.882 | 23.942 | 798.104 |
| miri-dotnet-vm-no-features-V02 [artifact 13056874630](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13056874630) | miss | 19.582 | 39.662 | 24.063 | 561.552 |
| miri-dotnet-vm-no-features-V03 [artifact 13058207375](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13058207375) | miss | 20.481 | 37.992 | 24.454 | 599.521 |
| miri-dotnet-vm-no-features-V04 [artifact 13057805920](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057805920) | miss | 15.959 | 33.916 | 18.928 | 502.136 |
| miri-dotnet-vm-no-features-V05 [artifact 13058061656](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13058061656) | miss | 16.660 | 27.380 | 20.027 | 591.002 |
| miri-dotnet-vm-no-features-V06 [artifact 13058350842](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13058350842) | miss | 16.286 | 34.756 | 19.108 | 523.938 |
| miri-dotnet-vm-no-features-V07 [artifact 13056583742](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13056583742) | miss | 14.529 | 39.534 | 16.588 | 526.282 |
| miri-dotnet-vm-no-features-V08 [artifact 13057261473](https://github.com/nickbclifford/dotnet-rs/actions/runs/41237423847/artifacts/13057261473) | miss | 13.641 | 23.003 | 15.252 | 492.103 |
