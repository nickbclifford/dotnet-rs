# Pointer provenance evidence inventory

**Scope:** M1a heap-access containment is implemented and its specific library/VM evidence is recorded below; this is not completion of M0 or the broader M1/M3 exits. The [plan](../plans/01-pointer-provenance.md) remains in progress. The current [owning VM module](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) and [catalog/runner](../../scripts/pointer_provenance.sh) are executable sources; outcomes and limits are summarized below. The original pass-1/2/3 review directories, logs, lab sources and patches linked by older documents—including the 3c2 patch—remain absent from the repository. Search at kickoff found no Git history for the pass-3 archive and no source in the cited scratch pass-1 probes, lab or FFI directories; an external backup has not been ruled out. The two pass-1 library tests below are independent recreations; they make no identity claim about the missing originals.

Evidence vocabulary: **VM-M** real VM under Miri; **LIB-M** library Miri; **N** native reproduction; **MODEL** standalone model, not an end-to-end VM result; **R/A** source review/audit, requiring confirmation. A historical report below is not a fresh run or proof that the current baseline had the same outcome. Recorded outcomes include the M1a baseline and final tests, the M1b kickoff baseline, and the Phase 6 M1b control promotions below. Probe rows explicitly marked unknown remain unverified. Sources can support a diagnosis without proving an execution result.

## Source provenance

At Phase 0 the four volatile scratch files were verified against these SHA-256 values before vendoring; paths and line numbers describe the original inputs, not durable links. The adaptations are summarized below. Current module SHA-256 at inventory creation: `0cf183a37c63b59236effd4f828d3f80c45ef14cce5b8395e8d929d6ee0d46e3`; this is a snapshot hash, not the hash of any whole original file.

| Input | Original scratch path | Lines | SHA-256 | Use |
|---|---|---:|---|---|
| 3 | `/tmp/provenance-3/crates/dotnet-vm/src/provenance_probe_tests.rs` | 1062 | `985c09e585f1c5d0c4d36452724cd8db80deb4f69b36660ba06bcebef20f0ed4` | p00–p19 and bounded harness |
| 3b | `/tmp/provenance-3b/crates/dotnet-vm/src/provenance_probe_tests.rs` | 887 | `5f6e421ffa9cbc618d514d58e7be954dcba00b94382b40555363303352842d73` | comparison only; common prefix |
| 3c | `/tmp/provenance-3c/crates/dotnet-vm/src/provenance_probe_tests.rs` | 933 | `b00950a74273197dfa3a434904a33abb0b5742655f3220b23bef585f2d29a51a` | p20 only; tree contained runtime patches, not copied |
| pass-2 | `/tmp/dotnet-provenance-review-2/src/crates/dotnet-vm/src/provenance_probe_tests.rs` | 186 | `acc73b82c8abb0cdd680462deafef696a4dad0e5e34b03c0a88a26adb8a0058b` | two unique probes, adapted to bounded runner |

p20's patched source environment does **not** establish its historical pass-1 outcome or identify an unpatched original run. However, step 1.1/1.2 separately ran the integrated p20 probe against the pre-runtime tree recorded in the baseline table below; those current-tree runs were Miri-UB. Pass-2's other four probes overlap p01/p02/p04/p06 but are not byte-identical: pass-2 initializes slot 42, writes 99 before double-allocation growth, and its arithmetic case reads u1 expecting 0; pass-3's corresponding cases differ. The two unique tests preserve their IL and assertions, including the null case's typed `ValuePointer(vec![], None)` local. Full mock-corlib primitives are retained. `probe_perf_addressing_modes` and `probe_perf_field_modes` and their exclusive/unbounded helpers were excluded; benchmark recovery and baseline capture remain follow-ups.

## Probe catalog

Each linked name is an exact `pointer_provenance_tests::<name>` filter in the owning source. Original source references below are `variant:line`, not links to available archives. The current catalog has **31 entries: 24 enabled controls and 7 diagnostic-only probes** (p13 additionally has a `memory-validation`-conditional ignore because its deliberately far managed offset is rejected by that debug trap). **C** = blocking native and permissive-TB control; **U** = known or suspected host UB, diagnostic permissive TB only; **S** = semantic prediction, diagnostic only; **P** = reported native panic, diagnostic only; **X** = baseline unverified, diagnostic permissive TB only. Only p19 is explicitly permitted for native diagnostics. The script runs each diagnostic in a bounded separate process. Never run known UB natively or use ignored-test flags outside its diagnostic entrypoint. Review group-UB reports lack surviving per-probe transcripts; U alone does not establish an individual UB result. The p17/p18 baseline failures and subsequent repairs are recorded below; p19 is a reported panic, not a fabricated expected value.

| Exact probe (owning source) | Original | Rows / work | Status, evidence and current outcome |
|---|---|---|---|
| [`p00_smoke`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L226) | 3:191 | R13 / M0 | C; VM-M control passed in both configurations (final shard lane) |
| [`p01_ldloca_convu_stind_i4`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L246) | 3:210 | R02 / M3 | U; suspected UB, exact current outcome unknown |
| [`p02_ldloca_stind_i4_control`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L265) | 3:229 | R02 / M3 | C; VM-M control passed in both configurations (final shard lane) |
| [`p03_ldloca_convu_ldind_i4`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L283) | 3:247 | R02 / M3 | C; VM-M read control passed in both configurations (final shard lane) |
| [`p04_double_localloc_write_first`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L303) | 3:266 | R05 / M1c | U; group UB was historically reported; exact current outcome unknown |
| [`p05_single_localloc_roundtrip`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L328) | 3:291 | R04, R05 / M1c/M1e/M3 | C; VM-M control passed in both configurations; not a general serde fix |
| [`p06_localloc_minus_one_then_plus_one`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L360) | 3:313 | R05 / M1b | C; Phase 6 control passed in both configurations; pre-repair VM-M `miri-ub`, see M1b outcomes below |
| [`p07_localloc_one_past_end`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L378) | 3:340 | R05 / M1b | C; VM-M formation-only control passed in both configurations |
| [`p08_localloc_overshoot`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L407) | 3:360 | R05 / M1b | C; Phase 6 control passed in both configurations; pre-repair VM-M `miri-ub`, see M1b outcomes below |
| [`p09_ldloca_convu_survives_callee_stack_growth`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L510) | 3:464 | R02 / M3 | U; VM-M/N was historically reported; exact current outcome unknown |
| [`p10_ldelema_convu_stind_heap_array`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L544) | 3:498 | R01 / M1a/M3 | C; M1a VM-M control passed in both configurations after repair; baseline miri-ub below |
| [`p11_ldelema_stind_heap_array_control`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L580) | 3:534 | R01 / M1a/M3 | C; VM-M control passed in both configurations (final shard lane) |
| [`p12_ldsflda_convu_stind_static`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L616) | 3:569 | R03 / M3/M1d | U; suspected UB, exact current outcome unknown; not a teardown proof |
| [`p13_ldelema_managed_add_far_out_of_bounds_then_conv_u`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L648) | 3:596 | R01 / M1a/M3 | C; M1a VM-M control passed in both configurations; validation-all conditionally ignored |
| [`p14_initblk_heap_array_via_managed_ptr`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L833) | 3:798 | R01, R10 / M1a/M1f/M5 | C; M1a VM-M control passed in both configurations for its short valid block |
| [`p15_initblk_stack_local_via_managed_ptr`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L868) | 3:829 | R10 / M3/M1f/M5 | U; suspected UB, exact current outcome unknown |
| [`p16_cpblk_heap_arrays_via_managed_ptr`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L891) | 3:849 | R01, R10 / M1a/M1f/M5 | C; M1a VM-M control passed in both configurations for its short valid block |
| [`p17_localloc_ptr_ceq_self`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L950) | 3:894 | R05 / M1b | C; Phase 6 control passed in both configurations; pre-repair native and VM-M assertions failed, see M1b outcomes below |
| [`p18_localloc_ptr_clt`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L970) | 3:914 | R05 / M1b | C; Phase 6 control passed in both configurations; pre-repair native and VM-M assertions failed, see M1b outcomes below |
| [`p19_ldelem_negative_index`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L987) | 3:936 | R11 / M1f | P; native panic historically reported, not rerun; no fabricated i32 result |
| [`p20_heap_convu_then_typed_write_then_raw_store`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L1011) | 3c:893 | R01 / M1a/M3 | C; baseline miri-ub and post-repair VM-M control pass in both configurations; patched-source history is not used as evidence |
| [`p21_localloc_signed_extreme_offsets_roundtrip`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L1061) | new | R05 / M1b | C; Phase 6 control passed in both configurations; pre-repair no-features diagnostic was Miri UB, see M1b outcomes below |
| [`p22_localloc_wrap_to_zero_roundtrip`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L1111) | new | R05 / M1b | C; Phase 6 control passed in both configurations; pre-repair no-features diagnostic was Miri UB, see M1b outcomes below |
| [`p23_localloc_pointer_loop_unsigned_branch_compare`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L1163) | new | R05 / M1b | C; Phase 6 control passed in both configurations; pre-repair no-features diagnostic was an assertion failure, see M1b outcomes below |
| [`p24_unsafe_byte_offset_direction_and_deref`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L1232) | new | R05, R11 / M1b | C; Phase 6 control passed in both configurations; pre-repair ByteOffset direction assertion failed, see M1b outcomes below |
| [`p25_convi_and_stind_i_pointer_roundtrip_deref`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L1472) | new | R11 / M1b | C; Phase 6 control passed in both configurations; pre-repair conversion/store diagnostic failed, see M1b outcomes below |
| [`p26_native_int_boundary_store_load_roundtrip`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L1560) | new | R11 / M1b | C; Phase 6 control passed in both configurations; the pre-repair no-features diagnostic passed, see M1b outcomes below |
| [`p27_stind_i_fresh_pointer_through_local_slot_dereferences`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L1406) | new (step 5.4) | R11 / M1b | C; Phase 6 control passed in both configurations; no step 0.1 probe baseline, see M1b outcomes below |
| [`p28_stind_i_fresh_pointer_through_heap_memory_dereferences`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L1430) | new (step 5.4) | R11 / M1b | C; Phase 6 control passed in both configurations; no step 0.1 probe baseline, see M1b outcomes below |
| [`p_null_through_pointer_local_address`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L1696) | pass-2:168 | R05, R11 / M1b/M3 | C; step 0.1 and Phase 6 VM-M controls passed in both configurations; see M1b outcomes below |
| [`p_wide_store_into_int_local`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L1088) | pass-2:179 | R11 / M3 | X; VM-M current outcome unknown; source assertion only |

For an individual ignored probe, use e.g. `bash scripts/pointer_provenance.sh diagnostic --configuration no-features --probe p04_double_localloc_write_first`; this is an opt-in diagnostic expected to fail if the reported issue reproduces, not a CI gate. Do not run it natively. `bash scripts/pointer_provenance.sh list` lists the exact catalog; see the script for options. Future repairs must keep the original assertions and separately validate actual access, not just address formation.

## M1a before/after outcomes and repaired scope

### Pre-repair baseline (steps 1.1/1.2)

Each cell is an observed, separate `scripts/pointer_provenance.sh diagnostic --configuration C --probe NAME --lane permissive-tb` process using `nightly-2026-05-27` and `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`. The no-features baseline source commit was `775886fc7554f9d5b58014f2b3db30c2e17d1982`; the multithreading baseline commit was `9524740e01953416a5d68eeee377eb18c9f51851` (the intervening difference was tracking-only, with no runtime edits). Full summaries/logs and metadata were archived under `target/pointer-provenance/m1a-before/` as untracked evidence.

| Probe | no-features: classification, exit, wall, first offending frame | multithreading: classification, exit, wall, first offending frame |
|---|---|---|
| p10 `p10_ldelema_convu_stind_heap_array` | miri-ub; 1; 268 s; `dotnet-value/src/pointer/mod.rs:505:37` (caller `dotnet-vm/src/instructions/conversions.rs:101:21`) | miri-ub; 1; 243 s; same frame/caller |
| p13 `p13_ldelema_managed_add_far_out_of_bounds_then_conv_u` | miri-ub; 1; 233 s; `dotnet-value/src/pointer/mod.rs:505:37` (caller `dotnet-vm/src/instructions/conversions.rs:101:21`) | miri-ub; 1; 277 s; same frame/caller |
| p14 `p14_initblk_heap_array_via_managed_ptr` | miri-ub; 1; 236 s; `dotnet-value/src/pointer/mod.rs:505:37` (caller `dotnet-value/src/stack_value.rs:574:30`) | miri-ub; 1; 258 s; same frame/caller |
| p16 `p16_cpblk_heap_arrays_via_managed_ptr` | miri-ub; 1; 241 s; `dotnet-value/src/pointer/mod.rs:505:37` (caller `dotnet-value/src/stack_value.rs:574:30`) | miri-ub; 1; 259 s; same frame/caller |
| p20 `p20_heap_convu_then_typed_write_then_raw_store` | miri-ub; 1; 245 s; `dotnet-value/src/pointer/mod.rs:505:37` (caller `dotnet-vm/src/instructions/conversions.rs:101:21`) | miri-ub; 1; 244 s; same frame/caller |

All ten logs reported `Undefined Behavior: constructing invalid value of type &[u8]: encountered a dangling reference (going beyond the bounds of its allocation)`. The common first failure was the oversized `size_bytes()`-sized slice in `ManagedPtr::with_data`; therefore these results do not establish the later address, typed-write or block-operation failure modes. Wall times are warm-run observations; the baseline commits are the source commit metadata, not claims that the missing historical source archives have been recovered.

### M1b pre-repair baseline (2026-10-28)

The following are kickoff measurements at source commit `082fb56b`, not new results from the M1a baseline above. The VM-M and LIB-M lane used `nightly-2026-05-27` with `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`: filtered permissive Tree Borrows, not strict-provenance evidence, a full VM Miri suite, or a soundness proof. Diagnostic probes used `bash scripts/pointer_provenance.sh diagnostic --configuration CFG --probe NAME` (add `--lane native` for native diagnostic rows); controls used `bash scripts/pointer_provenance.sh controls --configuration CFG --shard 3`. Walls are seconds; shard walls include discovery and build. Allocation IDs in the quoted Miri diagnostics are abbreviated with `…`; they are excerpts, not complete original lines.

| Probe / check | Configuration | Evidence lane | Outcome, first diagnostic and Rust location | Wall (s) |
|---|---|---|---|---:|
| `p06_localloc_minus_one_then_plus_one` | no-features | VM-M | `miri-ub` (exit 1); `Undefined Behavior: in-bounds pointer arithmetic failed: attempting to offset pointer by -1 bytes, but got alloc… which is at the beginning of the allocation`, `crates/dotnet-value/src/stack_value.rs:1092:39` | 368 |
| p06 | multithreading | VM-M | `miri-ub` (exit 1); same diagnostic and location | 318 |
| `p08_localloc_overshoot` | no-features | VM-M | `miri-ub` (exit 1); `Undefined Behavior: in-bounds pointer arithmetic failed: attempting to offset pointer by 32 bytes, but got alloc… which is only 16 bytes from the end of the allocation`, `crates/dotnet-value/src/stack_value.rs:1048:43` | 318 |
| p08 | multithreading | VM-M | `miri-ub` (exit 1); same diagnostic and location | 266 |
| `p17_localloc_ptr_ceq_self` | no-features | N: native diagnostic | `assertion-or-panic` (exit 101); `assertion left == right failed: p == p must be true`, `left: 0 right: 1`, `crates/dotnet-vm/src/pointer_provenance_tests.rs:958:5` | 2 |
| p17 | multithreading | N: native diagnostic | `assertion-or-panic` (exit 101); same assertion and location | 2 |
| `p18_localloc_ptr_clt` | no-features | N: native diagnostic | `assertion-or-panic` (exit 101); `p < p+8 must be true`, `left: 0 right: 1`, `crates/dotnet-vm/src/pointer_provenance_tests.rs:981:5` | 2 |
| p18 | multithreading | N: native diagnostic | `assertion-or-panic` (exit 101); same assertion and location | 2 |
| p17 | no-features | VM-M | `assertion-or-panic` (exit 101), no UB reported; `p == p must be true`, `left: 0 right: 1`, `pointer_provenance_tests.rs:958:5` | 322 |
| p17 | multithreading | VM-M | `assertion-or-panic` (exit 101), no UB reported; same assertion and location | 282 |
| p18 | no-features | VM-M | `assertion-or-panic` (exit 101), no UB reported; `p < p+8 must be true`, `left: 0 right: 1`, `pointer_provenance_tests.rs:981:5` | 321 |
| p18 | multithreading | VM-M | `assertion-or-panic` (exit 101), no UB reported; same assertion and location | 294 |
| `p_null_through_pointer_local_address` | no-features | VM-M | `pass` (exit 0; `1 passed; 0 failed; 0 ignored`) | 329 |
| `p_null_through_pointer_local_address` | multithreading | VM-M | `pass` (exit 0) | 284 |
| shard 3: `p07_localloc_one_past_end`, `p11_ldelema_stind_heap_array_control` | no-features | VM-M | `total: 2 passed, 0 failed`; p07 `pass` 283 s, p11 `pass` 270 s | 556 |
| shard 3: p07, p11 | multithreading | VM-M | `total: 2 passed, 0 failed`; p07 `pass` 299 s, p11 `pass` 336 s | 639 |
| `verify-controls` | no-features | native discovery | `control discovery: 11 exact non-ignored controls` | 7 |
| `verify-controls` | multithreading | native discovery | `control discovery: 11 exact non-ignored controls` | 5 |
| `pointer::tests::m1a_` | no-features | LIB-M | `3 passed; 0 failed; 0 ignored` | 1 (warm) |
| `pointer::tests::m1a_` | multithreading (default features) | LIB-M | `3 passed; 0 failed; 0 ignored` | 3 (warm) |

Control discovery used `bash scripts/pointer_provenance.sh verify-controls --configuration CFG`; it establishes the identities and non-ignored count, not a Miri pass. Only shard 3 was re-run at kickoff; the other nine controls' prior M1a outcomes are in the historical M1a final-shard table later in this section. The library measurements used the `pointer::tests::m1a_` selector, no-features and default-feature multithreading respectively; the review does not preserve the full command lines.

**Spot-check (VM-M):** `git diff 082fb56b -- crates scripts` was empty; only `bash scripts/pointer_provenance.sh diagnostic --configuration no-features --probe p06_localloc_minus_one_then_plus_one` was re-run for this subsection. It reported `miri-ub (exit=1, wall=242s)`; the first diagnostic was `error: Undefined Behavior: in-bounds pointer arithmetic failed: attempting to offset pointer by -1 bytes, but got alloc4138538 which is at the beginning of the allocation` at `crates/dotnet-value/src/stack_value.rs:1092:39`. This spot-check is separate from the kickoff table; the other rows were not re-run in this step.

**R/A source claim:** p06 performs `UnmanagedPtr - 1` and p08 performs `UnmanagedPtr + 32`; the respective unmanaged arithmetic arms call allocation-bounded `offset` at the cited locations. p06 returns and dereferences the pointer after the out-and-back sequence; p08 is a formation-only probe, not evidence of a dereference after overshoot. p17/p18 assertion results are value-bearing comparisons. None of these results resolves the separate M1c growing-`localloc` storage obligation.

### M1b Phase 6 control outcome comparison (2026-10-29)

The pre-repair column uses the step 0.1 baseline table above where applicable; later-added probes use their pre-repair diagnostic observations from steps 0.4–0.6, and p27/p28 did not exist at step 0.1. Final results are the Phase 6 guarded control runs, using `bash scripts/pointer_provenance.sh controls --configuration CFG --shard N` with the pinned `nightly-2026-05-27` permissive Tree Borrows flags (no `--ignored`). Reported walls are per-probe runner seconds, not shard aggregate times. The associated control command is identified by each row's shard. Pre-repair VM diagnostics used the matching `bash scripts/pointer_provenance.sh diagnostic --configuration CFG --probe NAME` command; native baseline diagnostics for p17/p18 added `--lane native`.

| Probe (shard) | Pre-repair outcome / evidence | Phase 6 no-features control | Phase 6 multithreading control |
|---|---|---|---|
| p06 `p06_localloc_minus_one_then_plus_one` (7) | Step 0.1: VM-M `miri-ub` in both; 368 s / 318 s | `pass`, 340 s | `pass`, 380 s |
| p08 `p08_localloc_overshoot` (7) | Step 0.1: VM-M `miri-ub` in both; 318 s / 266 s | `pass`, 307 s | `pass`, 329 s |
| p17 `p17_localloc_ptr_ceq_self` (8) | Step 0.1: native and VM-M `assertion-or-panic` in both; native 2 s each, VM-M 322 s / 282 s | `pass`, 334 s | `pass`, 337 s |
| p18 `p18_localloc_ptr_clt` (8) | Step 0.1: native and VM-M `assertion-or-panic` in both; native 2 s each, VM-M 321 s / 294 s | `pass`, 332 s | `pass`, 339 s |
| p21 `p21_localloc_signed_extreme_offsets_roundtrip` (9) | Not in step 0.1; step 0.4 no-features diagnostic `miri-ub`, 284 s; multithreading pre-repair not run | `pass`, 358 s | `pass`, 357 s |
| p22 `p22_localloc_wrap_to_zero_roundtrip` (9) | Not in step 0.1; step 0.4 no-features diagnostic `miri-ub`, 246 s; multithreading pre-repair not run | `pass`, 313 s | `pass`, 336 s |
| p23 `p23_localloc_pointer_loop_unsigned_branch_compare` (10) | Not in step 0.1; step 0.5 no-features diagnostic `assertion-or-panic`, 235 s; multithreading pre-repair not run | `pass`, 304 s | `pass`, 457 s |
| p24 `p24_unsafe_byte_offset_direction_and_deref` (10) | Not in step 0.1; step 0.5 no-features diagnostic `assertion-or-panic`, 238 s; step 4.1 multithreading run before ByteOffset repair `assertion-or-panic`, 321 s | `pass`, 429 s | `pass`, 418 s |
| p25 `p25_convi_and_stind_i_pointer_roundtrip_deref` (11) | Not in step 0.1; step 0.6 no-features diagnostic `assertion-or-panic`, 236 s | `pass`, 437 s | `pass`, 477 s |
| p26 `p26_native_int_boundary_store_load_roundtrip` (11) | Not in step 0.1; step 0.6 no-features diagnostic passed, 233 s; step 5.3 multithreading diagnostic passed, 340 s | `pass`, 515 s | `pass`, 459 s |
| p27 `p27_stind_i_fresh_pointer_through_local_slot_dereferences` (12) | New at step 5.4; no step 0.1 baseline | `pass`, 296 s | `pass`, 348 s |
| p28 `p28_stind_i_fresh_pointer_through_heap_memory_dereferences` (12) | New at step 5.4; no step 0.1 baseline | `pass`, 302 s | `pass`, 321 s |
| `p_null_through_pointer_local_address` (6) | Step 0.1: VM-M `pass` in both; 329 s / 284 s | `pass`, 274 s | `pass`, 344 s |
| Existing shard-6 control p16 `p16_cpblk_heap_arrays_via_managed_ptr` (6) | Existing M1a control; not an M1b step 0.1 row | `pass`, 269 s | `pass`, 344 s |

The Phase 6 runs above report each promoted test as `1 passed; 0 failed; 0 ignored`; each two-control shard totals `2 passed, 0 failed`. The final count was checked with `bash scripts/pointer_provenance.sh list` (31 catalog entries: 24 controls, 7 diagnostic-only) and `bash scripts/pointer_provenance.sh verify-controls --configuration no-features` plus the `multithreading` variant (each `control discovery: 24 exact non-ignored controls`). Discovery is native metadata, not test execution.

All 12 shards and their measurements are summarized below. For shards 1–5, the shard wall is the earlier final 11-control lane measurement already reported above (not a Phase 6 rerun); for shards 6–12, times shown are the Phase 6 per-probe walls from the guarded commands just described. The aggregate wall for shard 6 was not preserved in the Phase 6 notes.

| Shard | Controls | no-features result / measured wall | multithreading result / measured wall |
|---:|---|---|---|
| 1 | p00, p02 | 2 passed; shard 1037.430 s | 2 passed; shard 1327.492 s |
| 2 | p03, p05 | 2 passed; shard 1048.719 s | 2 passed; shard 1323.221 s |
| 3 | p07, p11 | 2 passed; shard 1043.840 s | 2 passed; shard 1118.221 s |
| 4 | p10, p20 | 2 passed; shard 1046.018 s | 2 passed; shard 1305.717 s |
| 5 | p13, p14 | 2 passed; shard 1036.255 s | 2 passed; shard 1328.707 s |
| 6 | p16, `p_null_through_pointer_local_address` | p16 pass 269 s; p_null pass 274 s; 2 passed | p16 pass 344 s; p_null pass 344 s; 2 passed |
| 7 | p06, p08 | p06 pass 340 s; p08 pass 307 s; 2 passed | p06 pass 380 s; p08 pass 329 s; 2 passed |
| 8 | p17, p18 | p17 pass 334 s; p18 pass 332 s; 2 passed | p17 pass 337 s; p18 pass 339 s; 2 passed |
| 9 | p21, p22 | p21 pass 358 s; p22 pass 313 s; 2 passed | p21 pass 357 s; p22 pass 336 s; 2 passed |
| 10 | p23, p24 | p23 pass 304 s; p24 pass 429 s; 2 passed | p23 pass 457 s; p24 pass 418 s; 2 passed |
| 11 | p25, p26 | p25 pass 437 s; p26 pass 515 s; 2 passed | p25 pass 477 s; p26 pass 459 s; 2 passed |
| 12 | p27, p28 | p27 pass 296 s; p28 pass 302 s; 2 passed | p27 pass 348 s; p28 pass 321 s; 2 passed |

### M1b library regressions and validation limits

The final audit repaired arithmetic from a canonical unmanaged-origin managed null and widened checked managed-offset calculations so a compact `u32` offset is not sign-flipped on a 32-bit host. The extended `m1b_managed_ptr_checked_offset_reports_errors` regression covers the compact boundary and an exposed address reconstructed and dereferenced via both `checked_offset` and `StackValue` addition. The eight-test selector passed natively and under pinned permissive-TB Miri in both no-features and multithreading configurations on x86_64; `memory-validation` passed 7 with the existing 1 intentional ignore. Miri warned about exposed-provenance reconstruction. Guarded VM shard 5 was rerun after these repairs: p13 and p14 passed in no-features (275 s and 271 s) and multithreading (338 s and 289 s), with 2 passed and 0 failed per configuration. Other VM shards were not rerun. A 32-bit target was not run, and this is not strict-provenance evidence.

The eight `pointer::tests::m1b_` regressions are `m1b_unmanaged_ptr_null_contract`, `m1b_managed_ptr_checked_offset_reports_errors`, `m1b_unmanaged_offsets_wrap_and_round_trip_with_deref`, `m1b_unmanaged_arithmetic_wraps_to_explicit_null`, `m1b_min_signed_operands_do_not_overflow`, `m1b_pointer_comparisons_are_value_bearing`, `m1b_managed_ptr_far_oob_compare_and_difference`, and `m1b_managed_ptr_arithmetic_boundaries_report_explicit_errors`.

- Native, no-features: `cargo test -p dotnet-value --lib --no-default-features pointer::tests::m1b_ -- --test-threads=1` → 8 passed, 0 failed, 0 ignored.
- Native, multithreading: `cargo test -p dotnet-value --lib --no-default-features --features multithreading pointer::tests::m1b_ -- --test-threads=1` → 8 passed, 0 failed, 0 ignored.
- LIB-M, pinned permissive-TB: `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" cargo +nightly-2026-05-27 miri test -p dotnet-value --lib --no-default-features pointer::tests::m1b_ -- --test-threads=1` → 8 passed, 0 failed, 0 ignored; the same command with `--no-default-features` omitted (default multithreading configuration) → 8 passed, 0 failed, 0 ignored. Both were measured at step 6.2. Current no-features, default/multithreading and `memory-validation` `--list` selectors each discover exactly 8 tests.
- Native, `memory-validation`: `cargo test -p dotnet-value --lib --features memory-validation pointer::tests::m1b_ -- --test-threads=1` → 7 passed, 0 failed, 1 ignored. The only ignored selector is the deliberate heap-owner out-of-bounds arithmetic case guarded by the F-TEST-003 memory-validation rationale.

#### M1b validation limits

The VM controls use the **filtered permissive Tree Borrows** lane: `nightly-2026-05-27` with `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`. This is **not strict-provenance evidence**, **not a full VM Miri suite**, and **not a soundness proof**. The 24 controls are an explicitly selected bounded lane; passing them establishes only their asserted paths, not every pointer, storage, ownership or ABI contract. Native discovery (`verify-controls`) is not execution. The Phase 6 control runs were not native executions; the pre-repair native diagnostics for p17/p18 are recorded separately above.

<a id="recreated-library-regressions-and-final-vm-passes"></a>

### Library regressions and final VM passes

The original pass-1 library source files remain absent. The first two tests in [`crates/dotnet-value/src/pointer/tests.rs`](../../crates/dotnet-value/src/pointer/tests.rs) independently recreate the reported test intents without claiming source identity; the third is a new typed-write interleaving regression:

| Test | What it asserts | Pinned Miri outcome |
|---|---|---|
| `pointer::tests::m1a_one_byte_heap_read` | A one-byte `UInt8` heap vector returns exactly one byte (`0xA5`) from `with_data(1, ...)`. | 1 passed / 0 failed / 0 ignored in no-features and multithreading |
| `pointer::tests::m1a_safe_oob_address_observation` | At logical offset 4096, `PointerLike::pointer()` and `StackValue::ManagedPtr::as_ptr()` report the retained-base address plus wrapping offset; it does not dereference. | 1 passed / 0 failed / 0 ignored in both configurations |
| `pointer::tests::m1a_heap_pointer_survives_typed_write` | For array, object and boxed storage, an observed heap base remains usable across a guarded typed write of 5; raw write of 7 followed by guarded read returns 7 (including the exposed-integer reconstruction path). | 1 passed / 0 failed / 0 ignored in both configurations |

All three exact selectors ran individually under `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" cargo +nightly-2026-05-27 miri test -p dotnet-value --lib --no-default-features [--features multithreading] ... -- --exact --test-threads=1`; none used `--ignored` after promotion.

The five repaired VM probes p10, p13, p14, p16 and p20 each passed their individual permissive-TB diagnostics in **both** configurations (1 passed / 0 failed / 0 ignored) during the M1a phase. Their original probe bodies, IL and assertions were not changed: p10 asserts 7; p13 asserts 1; p14 and p16 assert `0x01010101`; p20 asserts 7 after the typed-write-then-raw-store interleaving. The p13 validation-feature conditional ignore remains to accommodate the intentional `memory-validation` bounds trap; its Miri diagnostic/control lane does not enable that feature. The separate `conv_u8_after_far_oob_managed_add_does_not_panic` regression has the same conditional ignore for its deliberate 4096-byte managed add; it passed pinned Miri in both configurations without that feature. At that M1a stage, the runner discovered 11 controls in each configuration and passed all 11 exact tests without `--ignored` (22 control executions total); Phase 6 subsequently grew the lane to 24 controls in 12 shards, as summarized above.

Historical M1a shard walls (seconds, including runner discovery) were measured by the step 5.2 rerun. At that stage shards 1–5 each contained two exact controls and shard 6 contained one. All individual results were `1 passed; 0 failed; 0 ignored`; the expanded Phase 6 12-shard lane is tabulated above.

| Configuration | shard 1 | shard 2 | shard 3 | shard 4 | shard 5 | shard 6 | verify-controls |
|---|---:|---:|---:|---:|---:|---:|---:|
| no-features | 1037.430 | 1048.719 | 1043.840 | 1046.018 | 1036.255 | 554.252 | pass; 11 controls |
| multithreading | 1327.492 | 1323.221 | 1118.221 | 1305.717 | 1328.707 | 537.128 | pass; 11 controls |

The longest measured shard was 1328.707 s, below the CI matrix leg's 2700 s (45-minute) per-shard timeout; the runner also applies a separate 2700 s timeout to each probe. These were warm-target, contended local runs in two concurrent waves; they exclude fresh CI checkout, toolchain/sysroot setup, network and hosted-runner variability and do not guarantee hosted CI timing.

### Changed and unchanged heap paths

M1a repaired heap paths: Heap/CrossArena `ManagedPtr::with_data` checks checked offset-plus-size against actual payload length before making only the requested slice; heap address observation uses retained raw allocation bases with wrapping formation and no payload slice; heap `StackValue::as_ptr` and VM `resolve_address` use that observer; `conv.u` and `conv.u8` use separate slice-free observation and explicit provenance exposure at integer conversion; `Unsafe.AsPointer`/Unsafe helpers and buffer helpers use the heap observer (with exposed provenance only for integer reconstruction); heap pointer derivation uses retained allocation bases so a guest-visible pointer survives an ordinary typed write. Object/array/string payload extent helpers preserve accounting/layout, and string observation/read remains read-only.

Still open or intentionally unchanged: Stack/Static observation and the `ManagedPtr::with_data` stack fallback (M3); the string write contract (no writable access to interned Arc-backed data); P/Invoke pinning/guard lifetime, ABI and return/copy semantics (M2; the relevant heap address derivation is routed, not a P/Invoke contract repair, and the ordinary ManagedPtr CrossArena branch retains its legacy fallback under M1d); and general `cpblk`/`initblk` correctness (M1f/M5: count width, checked complete ranges, barriers/rooting and yield/resumption state). The legacy `StackValue` managed-pointer-versus-native-integer comparisons and managed-pointer subtraction still call `managed_ptr_to_raw`, whose Heap arm uses in-bounds `.add` even for a far-out-of-bounds logical offset. M1a did not validate or repair those operations; a far-OOB operand can violate that arithmetic premise without a dereference (M1b). Broader `conv.i`/pointer semantics (M1b), owner/guard escape including `pop_ptr` and Transient (M1d), GC transport/identity (M1e/M1g), shared-memory/atomic protocol (M4), pointer-codec/layout redesign (M3), and strict-provenance migration/evidence (M0/M6) are not closed by these results.

**Evidence limits:** The 3c2 patch and pass-1/2/3 archives are still unavailable; the independently recreated library tests make no identity claim. The VM lane is a filtered permissive Tree Borrows lane, not strict-provenance evidence, a full VM Miri suite, or a soundness proof. Native Criterion benchmark numbers are performance measurements, not Miri evidence. The [native benchmark comparisons](../BENCHMARK_WORKFLOW.md#m1a-heap-access-comparison-2026-10-28) record no-features regressions that remain unresolved for follow-up.

## Initial six-control calibration (2026-10-27)

On **2026-10-27**, each Phase 2 cell below executed one exact ignored control via `bash scripts/pointer_provenance.sh diagnostic --configuration CONFIG --probe NAME`, separately, with `nightly-2026-05-27`, `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`, `--no-default-features`, and optionally `--features multithreading`. At that time the controls were still ignored; they are now enabled, so use `controls` rather than `diagnostic` to rerun them. All twelve were exit 0, one executed/passed and zero ignored; wall seconds are script measurements. Temporary log paths are volatile, not archived artifacts. The pre-finalizer session log can be consulted in branch history with `git show a2ac27d1:AGENT_MEMORY.md` if that history is retained; it records exact commands, results and timestamps.

| Control | no-features pass: wall / log | multithreading pass: wall / log |
|---|---|---|
| p00 | 241s / `/tmp/pointer-provenance.oPGNFTh1/p00_smoke.log` | 228s / `/tmp/pointer-provenance.vfSMfwfC/p00_smoke.log` |
| p02 | 236s / `/tmp/pointer-provenance.80XGMW6y/p02_ldloca_stind_i4_control.log` | 228s / `/tmp/pointer-provenance.Kv4o9Izj/p02_ldloca_stind_i4_control.log` |
| p03 | 238s / `/tmp/pointer-provenance.fNjGbwis/p03_ldloca_convu_ldind_i4.log` | 228s / `/tmp/pointer-provenance.iiqIvhuP/p03_ldloca_convu_ldind_i4.log` |
| p05 | 237s / `/tmp/pointer-provenance.wBQAI1Bf/p05_single_localloc_roundtrip.log` | 224s / `/tmp/pointer-provenance.uxCzRf2j/p05_single_localloc_roundtrip.log` |
| p07 | 230s / `/tmp/pointer-provenance.i8KFhgza/p07_localloc_one_past_end.log` | 228s / `/tmp/pointer-provenance.JHiNHYr2/p07_localloc_one_past_end.log` |
| p11 | 228s / `/tmp/pointer-provenance.ziLBRUf8/p11_ldelema_stind_heap_array_control.log` | 229s / `/tmp/pointer-provenance.539PdeSJ/p11_ldelema_stind_heap_array_control.log` |

An earlier p00 raw pass was rejected by the diagnostic recognizer because warnings interrupted its status line; after the recognizer fix it was rerun successfully (recorded in the historical session log entries 2.1 and 2.7). Native nextest passed the original six controls in each configuration, with 17 ignored at that time. Enabled Miri shard 1 separately passed p00/p02 in both configurations: no-features outer wall 453.927s (226s/224s child), multithreading 483.700s (240s/240s child); logs `/tmp/pointer-4-1-controls-{no-features,multithreading}-shard1.log`. The initial blocking matrix then paired p00/p02, p03/p05, p07/p11; subsequent promotions later produced an 11-control/six-shard M1a-era layout, superseded by the 24-control/twelve-shard Phase 6 lane above. Local results do not establish hosted CI cold-build/cache timings or hosted job success. All these are filtered **permissive** TB results, not full VM Miri, strict Miri, or runtime soundness.

## Finding-to-work map

Rows are updated to distinguish the M1a sub-results closed here from adjacent work that remains open. Probe IDs refer to the exact catalog above; only the stated narrowly scoped tasks are closed. Model, library and source-review evidence must not be conflated with real VM runs.

| ID / finding | Evidence and existing coverage | Named reproduction/repair task and owner |
|---|---|---|
| R01 Oversized heap slice, shared-derived writable address, safe OOB formation | Baseline p10/p13/p14/p16/p20 each miri-ub at the oversized slice; after M1a repair all five pass in both configurations; p11 remains a passing control. Three independent LIB-M regressions also pass; original pass-1 sources absent. | **M1a heap reproduction/repair closed** by independently recreated exact-read and OOB-observation tests, a new typed-write test, and the five VM controls. M3 remains open for stack/static and broader pointer semantics; do not infer that all R01-adjacent lifetimes are proven. |
| R02 Slot assignment invalidates byref; stack growth invalidates native int | p01/p09 diagnostic, p02/p03 controls; reported VM-M/N p09, LIB-M absent, MODEL distinct | VM stack/value M3: assignment then dereference; recursion/suspend/restore; recover E12b/E13 models separately. |
| R03 Transient destruction; expired static/arena guards | LIB-M report versus R/A resolver claims; p12 static-address smoke only | Value/VM-data M1d/M3: recover pop/normalize/codec owner probes; concurrent teardown fixture with retained roots and post-pop access. |
| R04 Unmanaged serde exposure; integer GC transport | LIB-M reports; p05 VM-M control refutes unconditional failure only | Value/utils GC M1e/M3: recover decode+deref without unrelated exposure and typed GC transport; future strict-core TB. |
| R05 Repeated localloc, arithmetic, null/wrapping/comparison | Step 0.1 measured p06/p08 Miri UB and p17/p18 assertion failures; p_null passed. Phase 6 controls p06, p08, p17, p18, p21–p24 and p_null passed in both configurations; p07 remains a passing control. | **M1b arithmetic/comparison/null coverage closed** for the listed wrapping, signed-extreme, wrap-to-zero, comparison and dereference paths. **M1c remains open** for stable `localloc` storage (the growing pool `Vec`); **M3 remains open** for final pointer representation and codecs. |
| R06 Pinned payload address; FFI return overwrite | Heap-origin P/Invoke address derivation now uses the observer under existing pins/guards; no new native-call return/copy regression was added. Historical Rust/libffi sources remain missing. | **M2 remains open:** recover/add dedicated pinned payload and libffi guard-byte tests; verify object/array/string/boxed reads and native widened-scalar/aggregate return guards. No P/Invoke lifetime, ABI or return contract is closed by M1a. |
| R07 By-ref/out, temp growth, null and string ABI | Existing P/Invoke crate tests passed in both feature configurations; no new end-to-end native ABI evidence was collected. | **M2 remains open:** native fixture with type-sized copies, many arguments forcing temp growth, null and terminated/encoded strings; retain current pin/guard obligations. |
| R08 Cross-arena root barrier/teardown; arena/address packing | MODEL E11 vs LIB-M ID40000 vs R/A; no integrated proof | Utils GC/value M1e/M1g/M5: recover `gc_model::e11*` and ID40000; actual VM incremental/cross-arena collection, lease/ID reuse and boundary codec accesses. |
| R09 Host races/alignment, mixed-size overlap, static atomics | MODEL E9/E10 reports; VM paths R/A | Runtime-memory/value M3/M4: recover E9/E10, seeded VM field/raw/Interlocked/block overlap stress with alignment checks, static path audit; native threaded stress. |
| R10 Block barriers, count width, yield state | p14 `initblk` and p16 `cpblk` pass in both configurations for their short valid scalar ranges; p15 stack-origin case remains diagnostic-only. This validates only M1a heap address formation. | **M1f/M5 remain open:** test >1MB chunk-yield/GC retry and fresh-reference copy; verify count width, complete checked ranges, barriers/rooting and consumed-operand yield/resumption in both configurations. |
| R11 Negative index, ByteOffset sign, conv.i/native stores | Phase 6 controls p24–p28 and p_null passed in both configurations; p26 checks native-width sign/zero-extension and pointer store/load followed by dereference. p19 remains a reported native panic; `p_wide_store_into_int_local` remains unverified. | **M1b ByteOffset direction, pointer conversion/store acceptance and native-integer round trips closed** for the asserted paths. **M1f remains open** for an exception-aware negative-array-index regression; **M3 remains open** for final pointer representation/codecs. |
| R12 Volatile ignored, Interlocked overflow, sync-block race, refanytype lifetime | R/A only; no integrated evidence | VM/intrinsics M4/M6: ordering litmus, wrapping Add, concurrent identity creation, post-handle-owner-drop access; confirm audits before fixes. |
| R13 Atomic backing aliasing, strict dependencies, blocking VM coverage | Historical LIB-M report unlocated; the final blocking lane has 24 filtered VM controls in each configuration, still only partial coverage. | M0/M6 remains open: identify historical two atomic tests and repair backing; diagnose dependencies and re-establish strict-core filters. |

Baseline source reading supports but does not execute these diagnoses: [block operations and localloc](../../crates/dotnet-vm/src/instructions/memory.rs), [growing allocation pool](../../crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs), [conversions](../../crates/dotnet-vm/src/instructions/conversions.rs), [array handlers](../../crates/dotnet-vm/src/instructions/objects/arrays.rs), [static field handler](../../crates/dotnet-vm/src/instructions/objects/fields.rs), [pointer representation/access](../../crates/dotnet-value/src/pointer/mod.rs), and [stack pointer arithmetic/comparison](../../crates/dotnet-value/src/stack_value.rs). Guard expiry remains an audit question. See also [architecture](../ARCHITECTURE.md) and [GC/memory safety](../GC_AND_MEMORY_SAFETY.md): contracts are not proof that native integers survive reallocation.

## Strict exclusions and candidate selectors

No strict job is authorized by this slice. These are **future candidate commands**, not recovered historical filters or claims of successful runs. Dependency traces and exact historical strict-core filters are missing. Record setup, configuration, actual first failure and logs when independently investigated; distinguish intentional exposed reconstruction from dependency blockers.

| Boundary | Candidate command / task | Limitation |
|---|---|---|
| VM parser (`bitvec`/dotnetdll) | `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-strict-provenance -Zmiri-disable-isolation -Zmiri-ignore-leaks" cargo +nightly-2026-05-27 miri test -p dotnet-vm --lib --no-default-features -- --exact pointer_provenance_tests::p00_smoke --test-threads=1` | Proposed exact smoke selector; earlier docs report dependency integer reconstruction, not a recovered current stack trace. |
| multithreading / parking_lot, crossbeam-epoch, rayon | Same p00 command with `--no-default-features --features multithreading` | Dependency blockers reported, exact current first-failure frame unavailable; legacy advisory `jmp_tests`, `tail_calls`, `fault_tests` are substring filters, not exact selectors. |
| Exposed native-int reconstruction | Same strict flags and command, exact `pointer_provenance_tests::p05_single_localloc_roundtrip`; repeat with multithreading if investigated | Permissive p05 pass does not imply strict reconstruction works. |
| Managed strict core | Same strict flags; candidate discovery `cargo +nightly-2026-05-27 miri test -p dotnet-runtime-memory --no-default-features -- --list` | Recover library source/results and select actual exact names later; no invented historical allowlist. |

Whole-VM strict Miri, dependency repairs, supplemental Stacked Borrows and unrelated VM Miri expansion are not established by these selectors or this harness.

## Outstanding cross-cutting follow-ups

- Recover the missing pass-1 library sources, review archives/transcripts and patch/lab/FFI sources from backup if available. The independently recreated M1a library tests are integrated and passing, but do not identify the original pass-1 tests. Recover E1–E13 and `gc_model::e11*` as separately labeled models. The current pre-runtime p20 baseline was run in steps 1.1/1.2; this does not recover p20's patched historical source/run. Never infer results from that patched tree.
- M1–M6: broader pointer/runtime/storage/GC/ABI/concurrency work still requires its own regressions and milestone exits. The scoped M1a heap repairs are recorded above; R01's stack/static and neighboring obligations remain open.
- M0/M6: locate the two historically reported ignored atomic tests, distinguish backing aliasing from historical Miri ICE reasons, repair and validate; they were not identified at HEAD or the reviewed atomic refactor tip.
- M0: inventory and migrate the **reported** 73 provenance-lint hits before any `fuzzy_provenance_casts` / `lossy_provenance_casts` deny gate; define narrow justified exceptions. The reported count is not a current verified inventory.
- M0: recover excluded addressing/field performance probes where needed. Native benchmark comparisons were captured/reviewed for this package; the repeated no-features regressions remain unresolved and are performance evidence only.
- The finished-tree M1b Criterion comparison was not measured on an idle host; see the [M1b PERF report](../BENCHMARK_WORKFLOW.md#m1b-pointer-semantics-comparison-2026-10-29).
- M0/M6: recover or re-establish strict-core evidence and selectors after dependency diagnosis; broaden unrelated VM Miri only in a separate scoped follow-up. Whole-VM strict is not a target of this slice. Observe hosted CI cold/cache timings before adjusting the provisional timeout.
