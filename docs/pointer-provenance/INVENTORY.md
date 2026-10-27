# Pointer provenance evidence inventory

**Scope:** partial M0 harness and filtered controls, not runtime repair or completed M0. The [plan](../plans/01-pointer-provenance.md) retains the milestone exits. No row below is resolved by adding a probe. The current [owning VM module](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) and [catalog/runner](../../scripts/pointer_provenance.sh) are executable sources; provenance and recorded results are summarized below. The original pass-1/2/3 review directories, logs, lab sources and patches linked by older documents are absent from the repository. Search at kickoff found no Git history for the pass-3 archive and no source in the cited scratch pass-1 probes, lab or FFI directories; an external backup has not been ruled out. Recover it or reproduce the evidence; do not treat summaries as transcripts.

Evidence vocabulary: **VM-M** real VM under Miri; **LIB-M** library Miri; **N** native reproduction; **MODEL** standalone model, not an end-to-end VM result; **R/A** source review/audit, requiring confirmation. A historical report below is not a fresh run or proof that the current baseline has the same outcome. Except for the controls documented below, exact current probe outcomes have not been revalidated. Sources can support a diagnosis without proving an execution result.

## Source provenance

At Phase 0 the four volatile scratch files were verified against these SHA-256 values before vendoring; paths and line numbers describe the original inputs, not durable links. The adaptations are summarized below. Current module SHA-256 at inventory creation: `0cf183a37c63b59236effd4f828d3f80c45ef14cce5b8395e8d929d6ee0d46e3`; this is a snapshot hash, not the hash of any whole original file.

| Input | Original scratch path | Lines | SHA-256 | Use |
|---|---|---:|---|---|
| 3 | `/tmp/provenance-3/crates/dotnet-vm/src/provenance_probe_tests.rs` | 1062 | `985c09e585f1c5d0c4d36452724cd8db80deb4f69b36660ba06bcebef20f0ed4` | p00–p19 and bounded harness |
| 3b | `/tmp/provenance-3b/crates/dotnet-vm/src/provenance_probe_tests.rs` | 887 | `5f6e421ffa9cbc618d514d58e7be954dcba00b94382b40555363303352842d73` | comparison only; common prefix |
| 3c | `/tmp/provenance-3c/crates/dotnet-vm/src/provenance_probe_tests.rs` | 933 | `b00950a74273197dfa3a434904a33abb0b5742655f3220b23bef585f2d29a51a` | p20 only; tree contained runtime patches, not copied |
| pass-2 | `/tmp/dotnet-provenance-review-2/src/crates/dotnet-vm/src/provenance_probe_tests.rs` | 186 | `acc73b82c8abb0cdd680462deafef696a4dad0e5e34b03c0a88a26adb8a0058b` | two unique probes, adapted to bounded runner |

p20's patched source environment does **not** establish a pass or failure on unpatched HEAD; no surviving run establishes its original outcome. Pass-2's other four probes overlap p01/p02/p04/p06 but are not byte-identical: pass-2 initializes slot 42, writes 99 before double-allocation growth, and its arithmetic case reads u1 expecting 0; pass-3's corresponding cases differ. The two unique tests preserve their IL and assertions, including the null case's typed `ValuePointer(vec![], None)` local. Full mock-corlib primitives are retained. `probe_perf_addressing_modes` and `probe_perf_field_modes` and their exclusive/unbounded helpers were excluded; benchmark recovery and baseline capture remain follow-ups.

## Probe catalog

Each linked name is an exact `pointer_provenance_tests::<name>` filter in the owning source. Original source references below are `variant:line`, not links to available archives. All controls are enabled; all other probes remain ignored. **C** = blocking native and permissive TB control; **U** = known or suspected host UB, diagnostic permissive TB only; **S** = semantic prediction, diagnostic only; **P** = reported native panic, diagnostic only; **X** = baseline unverified, diagnostic permissive TB only. Only p17/p18/p19 are explicitly permitted for native diagnostics. The script runs each diagnostic in a bounded separate process. Never run known UB natively or use ignored-test flags outside its diagnostic entrypoint. Review group-UB reports lack surviving per-probe transcripts; U does not establish a fresh individual UB result. p17/p18 wrong results are predictions from source, not reruns; p19 is a reported panic with no i32 assertion, not a fabricated expected value.

| Exact probe (owning source) | Original | Rows / work | Status, evidence and current outcome |
|---|---|---|---|
| [`p00_smoke`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L213) | 3:191 | R13 / M0 | C; VM-M control, pass below |
| [`p01_ldloca_convu_stind_i4`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L233) | 3:210 | R02 / M3 | U; suspected UB, unknown |
| [`p02_ldloca_stind_i4_control`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L252) | 3:229 | R02 / M3 | C; VM-M control, pass below |
| [`p03_ldloca_convu_ldind_i4`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L270) | 3:247 | R02 / M3 | C; VM-M read control, pass below |
| [`p04_double_localloc_write_first`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L290) | 3:266 | R05 / M1c | U; VM-M group UB reported, exact current result unknown |
| [`p05_single_localloc_roundtrip`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L315) | 3:291 | R04, R05 / M1c/M1e/M3 | C; VM-M control, pass below, not a general serde fix |
| [`p06_localloc_minus_one_then_plus_one`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L338) | 3:313 | R05 / M1b | U; VM-M group UB reported, exact current result unknown |
| [`p07_localloc_one_past_end`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L365) | 3:340 | R05 / M1b | C; VM-M formation-only control, pass below |
| [`p08_localloc_overshoot`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L386) | 3:360 | R05 / M1b | U; VM-M group UB reported, exact current result unknown |
| [`p09_ldloca_convu_survives_callee_stack_growth`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L497) | 3:464 | R02 / M3 | U; VM-M/N reported, exact current result unknown |
| [`p10_ldelema_convu_stind_heap_array`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L532) | 3:498 | R01 / M1a/M3 | U; VM-M group UB reported, exact current result unknown |
| [`p11_ldelema_stind_heap_array_control`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L568) | 3:534 | R01 / M1a/M3 | C; VM-M control, pass below |
| [`p12_ldsflda_convu_stind_static`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L604) | 3:569 | R03 / M3/M1d | U; suspected UB, unknown; not a teardown proof |
| [`p13_ldelema_managed_add_far_out_of_bounds_then_conv_u`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L633) | 3:596 | R01 / M1a/M3 | U; VM-M group UB reported, exact current result unknown |
| [`p14_initblk_heap_array_via_managed_ptr`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L677) | 3:798 | R01, R10 / M1a/M1f/M5 | U; VM-M group UB reported, exact current result unknown |
| [`p15_initblk_stack_local_via_managed_ptr`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L712) | 3:829 | R10 / M3/M1f/M5 | U; suspected UB, unknown |
| [`p16_cpblk_heap_arrays_via_managed_ptr`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L736) | 3:849 | R01, R10 / M1a/M1f/M5 | U; VM-M group UB reported, exact current result unknown |
| [`p17_localloc_ptr_ceq_self`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L788) | 3:894 | R05 / M1b | S; N wrong result predicted, not rerun |
| [`p18_localloc_ptr_clt`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L809) | 3:914 | R05 / M1b | S; N wrong result predicted, not rerun |
| [`p19_ldelem_negative_index`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L832) | 3:936 | R11 / M1f | P; N panic reported, not rerun; no i32 assertion |
| [`p20_heap_convu_then_typed_write_then_raw_store`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L857) | 3c:893 | R01 / M1a/M3 | X; VM-M baseline unknown; patched source only |
| [`p_null_through_pointer_local_address`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L908) | pass-2:168 | R05, R11 / M1b/M3 | X; VM-M baseline unknown; source assertion only |
| [`p_wide_store_into_int_local`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs#L934) | pass-2:179 | R11 / M3 | X; VM-M baseline unknown; source assertion only |

For an individual ignored probe, use e.g. `bash scripts/pointer_provenance.sh diagnostic --configuration no-features --probe p04_double_localloc_write_first`; this is an opt-in diagnostic expected to fail if the reported issue reproduces, not a CI gate. Do not run it natively. `bash scripts/pointer_provenance.sh list` lists the exact catalog; see the script for options. Future repairs must keep the original assertions and separately validate actual access, not just address formation.

## Measured controls

On **2026-10-27**, each Phase 2 cell below executed one exact ignored control via `bash scripts/pointer_provenance.sh diagnostic --configuration CONFIG --probe NAME`, separately, with `nightly-2026-05-27`, `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`, `--no-default-features`, and optionally `--features multithreading`. At that time the controls were still ignored; they are now enabled, so use `controls` rather than `diagnostic` to rerun them. All twelve were exit 0, one executed/passed and zero ignored; wall seconds are script measurements. Temporary log paths are volatile, not archived artifacts. The pre-finalizer session log can be consulted in branch history with `git show a2ac27d1:AGENT_MEMORY.md` if that history is retained; it records exact commands, results and timestamps.

| Control | no-features pass: wall / log | multithreading pass: wall / log |
|---|---|---|
| p00 | 241s / `/tmp/pointer-provenance.oPGNFTh1/p00_smoke.log` | 228s / `/tmp/pointer-provenance.vfSMfwfC/p00_smoke.log` |
| p02 | 236s / `/tmp/pointer-provenance.80XGMW6y/p02_ldloca_stind_i4_control.log` | 228s / `/tmp/pointer-provenance.Kv4o9Izj/p02_ldloca_stind_i4_control.log` |
| p03 | 238s / `/tmp/pointer-provenance.fNjGbwis/p03_ldloca_convu_ldind_i4.log` | 228s / `/tmp/pointer-provenance.iiqIvhuP/p03_ldloca_convu_ldind_i4.log` |
| p05 | 237s / `/tmp/pointer-provenance.wBQAI1Bf/p05_single_localloc_roundtrip.log` | 224s / `/tmp/pointer-provenance.uxCzRf2j/p05_single_localloc_roundtrip.log` |
| p07 | 230s / `/tmp/pointer-provenance.i8KFhgza/p07_localloc_one_past_end.log` | 228s / `/tmp/pointer-provenance.JHiNHYr2/p07_localloc_one_past_end.log` |
| p11 | 228s / `/tmp/pointer-provenance.ziLBRUf8/p11_ldelema_stind_heap_array_control.log` | 229s / `/tmp/pointer-provenance.539PdeSJ/p11_ldelema_stind_heap_array_control.log` |

An earlier p00 raw pass was rejected by the diagnostic recognizer because warnings interrupted its status line; after the recognizer fix it was rerun successfully (recorded in the historical session log entries 2.1 and 2.7). Native nextest subsequently passed six controls in each configuration, with 17 ignored. Enabled Miri shard 1 separately passed p00/p02 in both configurations: no-features outer wall 453.927s (226s/224s child), multithreading 483.700s (240s/240s child); logs `/tmp/pointer-4-1-controls-{no-features,multithreading}-shard1.log`. The blocking CI matrix uses pairs p00/p02, p03/p05, p07/p11 in each configuration, with a provisional 45-minute job limit. Local results do not establish hosted CI cold-build/cache timings or hosted job success. All these are filtered **permissive** TB results, not full VM Miri, strict Miri, or runtime soundness.

## Finding-to-work map

Each row remains open. Probe IDs refer to the exact catalog above; named tasks are future work, not completed fixes. Model, library and source-review evidence must not be conflated with real VM runs.

| ID / finding | Evidence and existing coverage | Named reproduction/repair task and owner |
|---|---|---|
| R01 Oversized heap slice, shared-derived writable address, safe OOB formation | VM-M reports p10/p13/p14/p16, p11 control; p20 unknown; LIB-M source absent | Value/VM M1a/M3: recover pass-1 one-byte heap read and safe-OOB observation; separately reproduce actual read and checked access. |
| R02 Slot assignment invalidates byref; stack growth invalidates native int | p01/p09 diagnostic, p02/p03 controls; reported VM-M/N p09, LIB-M absent, MODEL distinct | VM stack/value M3: assignment then dereference; recursion/suspend/restore; recover E12b/E13 models separately. |
| R03 Transient destruction; expired static/arena guards | LIB-M report versus R/A resolver claims; p12 static-address smoke only | Value/VM-data M1d/M3: recover pop/normalize/codec owner probes; concurrent teardown fixture with retained roots and post-pop access. |
| R04 Unmanaged serde exposure; integer GC transport | LIB-M reports; p05 VM-M control refutes unconditional failure only | Value/utils GC M1e/M3: recover decode+deref without unrelated exposure and typed GC transport; future strict-core TB. |
| R05 Repeated localloc, arithmetic, null/wrapping/comparison | VM-M/N group reports; p04/p06/p08 diagnostic, p17/p18 predicted, pass-2 null unverified; p05/p07 controls | VM/value M1b/M1c: wrapping-to-zero and signed native conversions; retain both controls as gates. |
| R06 Pinned payload address; FFI return overwrite | R/A pass-2; reported Rust address test and native libffi, sources missing | P/Invoke/core intrinsics M2: recover address and libffi guard-byte sources; IL pinned payload reads for object/array/string/boxed, native widened-scalar and aggregate return guards. |
| R07 By-ref/out, temp growth, null and string ABI | R/A only; no integrated evidence | P/Invoke M2: native fixture with type-sized copies, many arguments forcing temp growth, null and terminated/encoded strings. |
| R08 Cross-arena root barrier/teardown; arena/address packing | MODEL E11 vs LIB-M ID40000 vs R/A; no integrated proof | Utils GC/value M1e/M1g/M5: recover `gc_model::e11*` and ID40000; actual VM incremental/cross-arena collection, lease/ID reuse and boundary codec accesses. |
| R09 Host races/alignment, mixed-size overlap, static atomics | MODEL E9/E10 reports; VM paths R/A | Runtime-memory/value M3/M4: recover E9/E10, seeded VM field/raw/Interlocked/block overlap stress with alignment checks, static path audit; native threaded stress. |
| R10 Block barriers, count width, yield state | p14–p16 short scalar blocks only; broader claims R/A | VM/runtime-memory M1f/M5: forced >1MB chunk-yield/GC retry and fresh-reference copy fixtures, validate barriers in both configs. |
| R11 Negative index, ByteOffset sign, conv.i/native stores | N p19 reported; p_null/p_wide unverified; R/A offsets/conversion, LIB low-level panic report | VM/unsafe intrinsics/value M1b/M1f/M3: exception-aware negative-index regression, signed ByteOffset, native store/load/deref roundtrips. |
| R12 Volatile ignored, Interlocked overflow, sync-block race, refanytype lifetime | R/A only; no integrated evidence | VM/intrinsics M4/M6: ordering litmus, wrapping Add, concurrent identity creation, post-handle-owner-drop access; confirm audits before fixes. |
| R13 Atomic backing aliasing, strict dependencies, blocking VM coverage | Historical LIB-M report unlocated; six filtered VM controls cover only a portion | Value/utils/runtime-memory M0/M6: identify historical two atomic tests and repair backing; diagnose dependencies and re-establish strict-core filters. |

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

- Recover missing pass-1 library sources, review archives/transcripts and patch/lab/FFI sources from backup if available; otherwise create explicit new reproductions. Integrate pass-1 library probes, E1–E13 and `gc_model::e11*` as separately labeled models; investigate p20 on the unpatched baseline through a bounded diagnostic process. Never infer a result from the patched tree.
- M1–M6: pointer/runtime/storage/GC/ABI and concurrency repairs, each with its own regression and milestone exit; R01–R12 name the immediate tasks. No repair is made here.
- M0/M6: locate the two historically reported ignored atomic tests, distinguish backing aliasing from historical Miri ICE reasons, repair and validate; they were not identified at HEAD or the reviewed atomic refactor tip.
- M0: inventory and migrate the **reported** 73 provenance-lint hits before any `fuzzy_provenance_casts` / `lossy_provenance_casts` deny gate; define narrow justified exceptions. The reported count is not a current verified inventory.
- M0: recover the excluded addressing/field performance probes where needed and capture native memory/unsafe_buffer/span/dispatch and field/byref/native-int benchmark baselines under a consistent profile and noise controls.
- M0/M6: recover or re-establish strict-core evidence and selectors after dependency diagnosis; broaden unrelated VM Miri only in a separate scoped follow-up. Whole-VM strict is not a target of this slice. Observe hosted CI cold/cache timings before adjusting the provisional timeout.
