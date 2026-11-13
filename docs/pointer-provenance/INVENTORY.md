# Pointer provenance evidence inventory

Evidence record. Not instructions; current work is in [docs/plans/](../plans/README.md).

This file is the single place where the review's IDs are defined:

- **Probe IDs** (`p00`…`p40`, `p_null…`, `p_wide…`) are the rows of the [probe catalog](#probe-catalog);
  each names an exact test in the [owning VM module](../../crates/dotnet-vm/src/pointer_provenance_tests.rs),
  run through the [catalog/runner](../../scripts/pointer_provenance.sh).
- **Finding IDs** (`R01`–`R13`) are the rows of the [finding map](#finding-to-work-map).
- **E1–E13** name standalone models from the original review; their sources are not in the repository.
- **M0–M6, M1a–M1g** label the milestones of a retired plan under which evidence below was
  gathered. They are labels for this record only and do not describe current work.

Evidence vocabulary: **VM-M** real VM under Miri; **LIB-M** library Miri; **N** native reproduction; **MODEL** standalone model, not an end-to-end VM result; **R/A** source review/audit, requiring confirmation. A historical report below is not a fresh run. Probe rows marked unknown remain unverified. Sources can support a diagnosis without proving an execution result.

The original pass-1/2/3 review directories, logs, lab sources and patches (including the 3c2
patch) are absent from the repository; an external backup has not been ruled out. Tests written
to reproduce their reported intents are **independent recreations**: they make no identity claim
about the missing originals. That applies to every "recreation" below.

Unless stated otherwise, Miri results use the pinned `nightly-2026-05-27` toolchain with
`MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`: filtered
permissive Tree Borrows with leaks ignored. That lane is not strict-provenance evidence, a full VM
Miri suite, leak checking or a soundness proof.

## Source provenance

At Phase 0 four volatile scratch files from the review were verified against these SHA-256 values before vendoring into the owning module. Module SHA-256 at inventory creation: `0cf183a37c63b59236effd4f828d3f80c45ef14cce5b8395e8d929d6ee0d46e3` (a snapshot hash, not the hash of any original file).

| Input | Lines | SHA-256 | Use |
|---|---:|---|---|
| 3 | 1062 | `985c09e585f1c5d0c4d36452724cd8db80deb4f69b36660ba06bcebef20f0ed4` | p00–p19 and bounded harness |
| 3b | 887 | `5f6e421ffa9cbc618d514d58e7be954dcba00b94382b40555363303352842d73` | comparison only; common prefix |
| 3c | 933 | `b00950a74273197dfa3a434904a33abb0b5742655f3220b23bef585f2d29a51a` | p20 only; tree contained runtime patches, not copied |
| pass-2 | 186 | `acc73b82c8abb0cdd680462deafef696a4dad0e5e34b03c0a88a26adb8a0058b` | two unique probes, adapted to bounded runner |

p20's patched source environment does **not** establish its historical pass-1 outcome. Steps 1.1/1.2 separately ran the integrated p20 probe against the pre-repair tree (see the baseline table below); those runs were Miri-UB. Pass-2's other four probes overlap p01/p02/p04/p06 but are not byte-identical: pass-2 initializes slot 42, writes 99 before double-allocation growth, and its arithmetic case reads u1 expecting 0. The two unique tests preserve their IL and assertions, including the null case's typed `ValuePointer(vec![], None)` local. `probe_perf_addressing_modes` and `probe_perf_field_modes` were excluded.

## Probe catalog

Each name is an exact `pointer_provenance_tests::<name>` filter in the owning source. The Original column gives `input:line` in the vendored review inputs above. The current catalog has **41 entries: 37 enabled controls and 4 diagnostic-only probes**; the current lane layout is in [CI](../CI.md#blocking-miri-pointer-controls-lane) (p13 additionally has a `memory-validation`-conditional ignore because its deliberately far managed offset is rejected by that debug trap). **C** = blocking native and permissive-TB control; **U** = known or suspected host UB, diagnostic permissive TB only; **S** = semantic prediction, diagnostic only; **P** = reported native panic, diagnostic only; **X** = baseline unverified, diagnostic permissive TB only. Only p19 is explicitly permitted for native diagnostics. The script runs each diagnostic in a bounded separate process. Do not run diagnostic-only known-UB probes natively or use ignored-test flags outside their diagnostic entrypoint. Review group-UB reports lack surviving per-probe transcripts; U alone does not establish an individual UB result. The p17/p18 baseline failures and subsequent repairs are recorded below; p19 is a reported panic, not a fabricated expected value.

| Exact probe (owning source) | Original | Finding | Status, evidence and outcome |
|---|---|---|---|
| [`p00_smoke`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:191 | R13 | C; VM-M control passed in both configurations (final shard lane) |
| [`p01_ldloca_convu_stind_i4`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:210 | R02 | C; with `FrameHome` byrefs to frame homes, VM-M control passed in both configurations (2026-11-04, singleton shard), as it did with the earlier `Unmanaged`-origin home byrefs. Before homes: 2026-11-04 diagnostic on 525ed3c4 was `miri-ub` in both configurations, write through `<wildcard>` forbidden at `dotnet-runtime-memory/src/access.rs:1175` (the address was derived from a shared borrow of the `Vec` slot) |
| [`p02_ldloca_stind_i4_control`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:229 | R02 | C; VM-M control passed in both configurations (final shard lane) |
| [`p03_ldloca_convu_ldind_i4`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:247 | R02 | C; VM-M read control passed in both configurations (final shard lane) |
| [`p04_double_localloc_write_first`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:266 | R05 | C; M1c repair passed in both configurations; pre-repair VM-M `miri-ub` at `dotnet-runtime-memory/src/access.rs:1172:25` |
| [`p05_single_localloc_roundtrip`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:291 | R04, R05 | C; VM-M control passed in both configurations; not a general serde fix |
| [`p06_localloc_minus_one_then_plus_one`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:313 | R05 | C; Phase 6 control passed in both configurations; pre-repair VM-M `miri-ub`, see M1b outcomes below |
| [`p07_localloc_one_past_end`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:340 | R05 | C; VM-M formation-only control passed in both configurations |
| [`p08_localloc_overshoot`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:360 | R05 | C; Phase 6 control passed in both configurations; pre-repair VM-M `miri-ub`, see M1b outcomes below |
| [`p09_ldloca_convu_survives_callee_stack_growth`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:464 | R02 | C; with frame homes and `FrameHome` byrefs, VM-M control passed in both configurations (2026-11-04, singleton shard 36). Before homes: 2026-11-04 diagnostic on 525ed3c4 was `miri-ub` in both configurations, 4-byte access through a dangling address with no provenance at `dotnet-runtime-memory/src/access.rs:1175` after the `Vec` reallocated |
| [`p10_ldelema_convu_stind_heap_array`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:498 | R01 | C; M1a VM-M control passed in both configurations after repair; baseline miri-ub below |
| [`p11_ldelema_stind_heap_array_control`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:534 | R01 | C; VM-M control passed in both configurations (final shard lane) |
| [`p12_ldsflda_convu_stind_static`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:569 | R03 | U; pinned Miri alignment `debug_assert` panic in both configurations, not UB and not a teardown proof; see M1d below |
| [`p13_ldelema_managed_add_far_out_of_bounds_then_conv_u`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:596 | R01 | C; M1a VM-M control passed in both configurations; validation-all conditionally ignored |
| [`p14_initblk_heap_array_via_managed_ptr`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:798 | R01, R10 | C; M1a VM-M control passed in both configurations for its short valid block |
| [`p15_initblk_stack_local_via_managed_ptr`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:829 | R10 | Control; 2026-11-04: native and pinned Tree Borrows Miri pass in both configurations after resolving the complete frame-home range before access |
| [`p16_cpblk_heap_arrays_via_managed_ptr`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:849 | R01, R10 | C; M1a VM-M control passed in both configurations for its short valid block |
| [`p17_localloc_ptr_ceq_self`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:894 | R05 | C; Phase 6 control passed in both configurations; pre-repair native and VM-M assertions failed, see M1b outcomes below |
| [`p18_localloc_ptr_clt`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:914 | R05 | C; Phase 6 control passed in both configurations; pre-repair native and VM-M assertions failed, see M1b outcomes below |
| [`p19_ldelem_negative_index`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:936 | R11 | P; native panic historically reported, not rerun; no fabricated i32 result |
| [`p20_heap_convu_then_typed_write_then_raw_store`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3c:893 | R01 | C; baseline miri-ub and post-repair VM-M control pass in both configurations; patched-source history is not used as evidence |
| [`p21_localloc_signed_extreme_offsets_roundtrip`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05 | C; Phase 6 control passed in both configurations; pre-repair no-features diagnostic was Miri UB, see M1b outcomes below |
| [`p22_localloc_wrap_to_zero_roundtrip`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05 | C; Phase 6 control passed in both configurations; pre-repair no-features diagnostic was Miri UB, see M1b outcomes below |
| [`p23_localloc_pointer_loop_unsigned_branch_compare`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05 | C; Phase 6 control passed in both configurations; pre-repair no-features diagnostic was an assertion failure, see M1b outcomes below |
| [`p24_unsafe_byte_offset_direction_and_deref`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05, R11 | C; Phase 6 control passed in both configurations; pre-repair ByteOffset direction assertion failed, see M1b outcomes below |
| [`p25_convi_and_stind_i_pointer_roundtrip_deref`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R11 | C; Phase 6 control passed in both configurations; pre-repair conversion/store diagnostic failed, see M1b outcomes below |
| [`p26_native_int_boundary_store_load_roundtrip`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R11 | C; Phase 6 control passed in both configurations; the pre-repair no-features diagnostic passed, see M1b outcomes below |
| [`p27_stind_i_fresh_pointer_through_local_slot_dereferences`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new (step 5.4) | R11 | C; Phase 6 control passed in both configurations; no step 0.1 probe baseline, see M1b outcomes below |
| [`p28_stind_i_fresh_pointer_through_heap_memory_dereferences`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new (step 5.4) | R11 | C; Phase 6 control passed in both configurations; no step 0.1 probe baseline, see M1b outcomes below |
| [`p29_localloc_blocks_survive_growth_and_are_16_byte_aligned`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05 | C; VM-M control passed in both configurations; stable blocks survive later allocations and are 16-byte aligned |
| [`p30_localloc_block_survives_nested_callee_localloc`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05 | C; VM-M control passed in both configurations; caller block remains readable after nested callee allocation |
| [`p31_localloc_blocks_released_on_return_and_pooling`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05 | C; VM-M control passed in both configurations; frame blocks and homes released on return and pooling |
| [`p32_localloc_blocks_released_on_tail_call_replacement`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05 | C; VM-M control passed in both configurations; replaced-frame blocks and homes released on tail call |
| [`p33_localloc_blocks_released_on_exception_unwind`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05 | C; VM-M control passed in both configurations; unwound-frame blocks and homes released before catch |
| [`p34_localloc_blocks_released_on_executor_teardown`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05 | C; VM-M control passed in both configurations; live blocks and homes released on executor teardown |
| [`p35_ldflda_on_stack_value_type_then_ldind`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R03 | C; post-repair VM-M pass in both configurations; pre-repair exception resolution failure, not UB; throw site inferred only; snapshot read, not stack-home write-through |
| [`p36_static_address_survives_forced_collection`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R03 | C; one-byte direct static access after forced collection passed VM-M in both configurations; refutes only the tested lifetime hypothesis |
| [`p37_ldloca_value_type_survives_stloc`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R02 | C; VM-M control passed in both configurations with local frame homes (2026-11-04, singleton shard), and again with `FrameHome` byrefs; without homes the same probe was `miri-ub` (dangling 4-byte access after `stloc` replaced the value-type slot's buffer) |
| [`p38_ldarga_value_type_survives_starg`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R02 | C; argument analogue of p37 (`ldarga`, `starg`, `conv.u`, `stind`); VM-M control passed in both configurations with argument homes (2026-11-04, singleton shard 37) |
| [`p39_frame_homes_survive_filter_suspension`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R02 | C; VM-M passed in both configurations; filter acceptance/rejection, collection of an array rooted only in a suspended home, checked reads/writes and bounds rejection; retired serial rejected after unwind |
| [`p40_jmp_forwards_modified_home_argument`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R02 | C; VM-M passed in both configurations; scalar and value-type homes forward current arguments across `jmp`; replaced serial rejected and target serial fresh |
| [`p41_constructor_receiver_survives_nested_growth`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R02 | C; native and VM-M passed in both configurations (2026-11-05); constructor receiver homes survive native-address writes across nested evaluation-stack growth, collection with an array rooted only in the receiver home, and constructor `jmp` completion |
| [`p42_typed_reference_address_policy`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R02 | Native control passes (2026-11-05): local/argument address-taking rejected for initialized and uninitialized typedrefs, including named System.TypedReference signatures; ordinary copying, type extraction and dereferencing remain supported. Pinned Tree Borrows Miri passes in both configurations, including collection with an array rooted only through a typedref and wrong-type rejection. |
| [`p_null_through_pointer_local_address`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | pass-2:168 | R05, R11 | C; step 0.1 and Phase 6 VM-M controls passed in both configurations; see M1b outcomes below |
| [`p43_enum_home_scalar_roundtrip`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R02 | C; scalar and enum `ValueType` arguments copied into their homes; saved local/argument addresses and serials survive reassignment and observed nested stack growth; explicit `ldobj`/`stobj` and native-address reads/writes preserve values. Native and pinned Tree Borrows Miri pass in both configurations (2026-11-05). |
| [`p44_native_substring_gc_resume`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R02 | C; native Substring progress and source rooting across collection and resumption; both configurations exercise the async copy. |
| [`p_wide_store_into_int_local`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | pass-2:179 | R11 | X; VM-M current outcome unknown. Natively, with frame homes the 8-byte `stind.i8` into the 4-byte homed local fails the frame-home bounds check and raises `System.AccessViolationException`, which the mock corlib cannot resolve (2026-11-04); the source assertion (`0x55`) is not reached |

For an individual diagnostic-only probe, use e.g. `bash scripts/pointer_provenance.sh diagnostic --configuration no-features --probe p01_ldloca_convu_stind_i4`; this is an opt-in diagnostic, not a CI gate. Do not run known-UB diagnostics natively. `bash scripts/pointer_provenance.sh list` lists the exact catalog; see the script for options. 

## Native effects and cross-arena pointer inventory

These entries identify provenance-sensitive paths, not additional probe IDs. The predicates and
their premises are defined in the [invariant registry](../INVARIANT_REGISTRY.md); the source
observations below are R/A, while the linked tests supply bounded execution evidence.

| Path | Pointer or carrier and lifetime boundary | Evidence / limits |
|---|---|---|
| [`poll_native_impl` and `NativeControl::enter`](../../crates/dotnet-vm-ops/src/effects.rs) | A `Cell<*mut ()>` holds the lifetime-erased `NativeRegion` only during a poll. The reset guard clears it on return or unwind; `enter` prevents recursive mutable entry and reborrows the same type with erased lifetimes. See `F6.NativePollAdapter`. | Bridge unit tests and the native-bridge Miri rows in [CI](../../.github/workflows/ci.yml); erasure requires all four premises in the registry. |
| [`RootedValue::stash` / `fetch`](../../crates/dotnet-vm-ops/src/effects.rs), [`Adapter::frame_home_block`](../../crates/dotnet-vm/src/stack/native_effects.rs) | A traced `Gc<RootCell>` in the activation's `DynamicRootSet` stores origin, type, offset and pinning, not a cached branded address. Fetch checks the root set and reconstructs: `FrameHome` resolves the serial in the active stack or `parked_finalizer_caller`, rejecting a missing home or out-of-range offset; `Heap` and MT `CrossArenaObjectRef` use `observe_address`. The traced origin records foreign references; a cross-arena access also needs the live owner's lease. Other origins are rejected. See `F6.NativePointerDescriptor`. | Frame-home retirement and heap/foreign-owner controls in the CI matrix; a resolved address alone is not an access authorization. |
| [`NativeRegion::write_monitor_flag`](../../crates/dotnet-vm-ops/src/effects.rs), [`Adapter::write_monitor_flag`](../../crates/dotnet-vm/src/stack/native_effects.rs) | The region fetches the rooted descriptor again before the backend writes one byte through `RawMemoryOps::write_bytes`, which checks the live owner and full range. The caller must supply synchronized writable bool storage without overlapping reference slots. See `F10.RawMemoryAccessValid`. | Monitor frame-home, heap, alias and collection controls in the CI matrix; bounds checks do not establish the caller's storage/aliasing contract. |
| [`StaticStorageManager::mark_failed_with_exception` / `failed_exception`](../../crates/dotnet-vm/src/statics.rs) | A one-reference `FieldStorage` with a GC descriptor carries the wrapper; the existing exception root remains live until the carrier is published before the failed state. `StaticStorage::trace` visits it, and `failed_exception` decodes during a live arena mutation. The wrapper retains its inner exception. See `F6.InitializerFailureRoot`. | Local and retired-owner identity tests across collection: [slice 4 verification](../plans/08-async-effects/slice-4-static-initialization.md#verification-2026-11-09). |
| [`SharedGcHandles`](../../crates/dotnet-runtime-memory/src/handles.rs) | VM-wide slots retain `ObjectPtr` provenance; strong and pinned targets are traced by their owner, and weak pointers are cleared before sweep. Reads rebrand a pointer only during an active mutator, which must root the result before a safepoint; retired owners remain registered through collection. | Cross-thread GC-handle diff fixture and executor collection controls; long-weak cross-arena resurrection remains in plan 20 slice 4. |
| [`ObjectRef::trace`](../../crates/dotnet-value/src/object/mod.rs), [`Executor::serve_retired`](../../crates/dotnet-vm/src/executor.rs), [`WorkerRegistry::shutdown`](../../crates/dotnet-vm/src/threading/workers.rs) | MT tracing reads the immutable owner ID through the object pointer before recording a foreign root. Retired owners service GC and finalizers until collection is excluded and workers/finalizers are quiescent; unregister precedes lease drain and freeing. Shutdown waits for managed workers to quiesce before signalling retired owners and joins all workers. On abort pending finalizers are abandoned. See `F1.RetiredCollectionExclusion` and `F1.StwParked`. | Retired-owner and shutdown controls in CI; the owner-ID read relies on this teardown ordering, not on a lease alone. |
| [`test_utils::foreign_reference`](../../crates/dotnet-vm/src/test_utils/foreign_reference.rs) | Test `encode_reference` uses the production `FieldStorage` reference codec and transports pointer-bearing storage intact; `decode_reference` reads through a temporary object in the destination mutation, then clears its field. Tests must separately keep the owner and reference live; the temporary carrier is not an extra collection root. | Used by foreign-owner and retired-initializer tests; it is a test transport, not a standalone rooting or lifetime guarantee. |

## Plan 02 (stable frame homes) slice 3 verification (2026-11-04)

The extended p31–p34 controls and new p39/p40 controls passed as guarded Miri singletons in
both no-features and multithreading configurations, using the pinned nightly and Tree Borrows
flags from `scripts/pointer_provenance.sh`. The final suspended-array version of p33/p39 was
also rerun without features. The eight `frame_alloc::` library tests passed under Miri in both
configurations, including home lookup across suspension/restoration and invalidation on discard.

`./check.sh --fast` passed on the final test version: 614, 734 and 749 tests passed in its three
configurations (7, 6 and 8 expected skips). The pointer manifest's 21 tests, batch membership
verification and doc-drift checks passed. This completes lifecycle slice 3; block-operation
validation and evaluation-stack fixup deletion remain in slice 4. The existing nested-filter
suspension limitation below is not claimed resolved by these controls.

## M1d baseline and outcomes

The missing pass-1 owner probes were recreated. The table records consolidated pre-repair runs and the later final-audit conversion regression; historical locations and commands refer to the trees on which they were measured. Both configurations mean `--no-default-features`, with or without `--features multithreading`. Pinned Miri uses `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"` and `cargo +nightly-2026-05-27 miri`. Native liveness tests used `cargo test -p dotnet-value --lib --no-default-features [--features multithreading] pointer::tests::<name> -- --exact --ignored --test-threads=1` (or `-p dotnet-vm-data ... stack::tests::<name>`); the pre-repair access tests used the matching pinned Miri commands with `--ignored`, not native execution. Historical locations refer to the pre-repair tree.

| Pre-repair check | Lane / configurations | Measured result and wall seconds |
|---|---|---|
| normalize owner / access | N / LIB-M, both | Native owner count 1 instead of 2, `Unmanaged`, exit 101 (0.075 / 0.062 s); Miri dangling read, exit 1, `pointer/tests.rs:128`, freed at `origin.rs:117` (0.910 / 0.898 s). |
| from-info owner / access | N / LIB-M, both | Native owner count 1 instead of 2, exit 101 (0.065 / 0.069 s); Miri dangling reference, exit 1 via `pointer/mod.rs:698` (0.869 / 0.900 s). |
| Transient clone identity | N, both | Different storage addresses, assertion exit 101 at `pointer/tests.rs:187` (0.058 / 0.054 s). |
| pop owner / access | N / LIB-M, both | Native owner count 1 instead of 2, exit 101 (0.063 / 0.064 s); Miri dangling read, exit 1 at `stack.rs:492`, freed at `stack.rs:293` (0.614 / 0.603 s). |
| p35 | VM-M diagnostic, both | Exception type-resolution failure at `test_utils.rs:59:37`, exit 1, no Miri UB reported (363 / 282 s). The exact origin and throw site were not established. |
| p12 | VM-M diagnostic, both | Four-byte alignment `debug_assert` panic at `storage.rs:339:9` (368 / 339 s); no UB reported, not a teardown proof. |
| Cross-arena teardown | LIB-M, multithreading only | Exit 1, use-after-free borrowing freed lock via `ObjectPtr::as_heap_storage` (`object/mod.rs:242`); no-features has no CrossArena variant. Wall not recorded. |
| Fill pre-repair pattern | LIB-M, multithreading only | Deliberate post-teardown write reported dangling-pointer UB. Wall not recorded; actual IL reachability is source-inferred, not VM-confirmed. |
| CLI-to-CTS Transient conversion audit | Native, both | Before the final audit repair, `cargo test -p dotnet-value --lib --no-default-features [--features multithreading] pointer::tests::owner_retention_cli_to_cts_rejects_transient_integer_escape -- --exact --test-threads=1` exited 101 in each configuration: the `convert_u32` rejection assertion failed at `pointer/tests.rs:187`. No dangling access was performed. |

The VM diagnostics used `bash scripts/pointer_provenance.sh diagnostic --configuration <cfg> --probe <name>`; the cross-arena teardown used the exact ignored `pointer::tests::owner_retention_cross_arena::owner_retention_cross_arena_access_after_owner_teardown_is_rejected` selector with pinned Miri and `--features multithreading`. The intentionally dangling Fill-pattern diagnostic remains ignored and must never be run natively.

**Post-repair measured:** normalize, from-info, clone and ported pop real-access regressions passed native and pinned Miri in both configurations. Transient clones share the owner; `normalize` and `from_info_full` preserve it; `RetainedPtr` holds the popped value and an optional cross-arena lease. Fallible memory stores, native-integer conversions and CLI-to-CTS numeric narrowing reject unsupported Transient ownership escape. `resolve_address` borrows its origin; an observed address alone is not a lasting lease or access authorization. The cross-arena rejection and in-flight lease tests passed pinned Miri with multithreading; fresh accesses fail after unregister, while an existing lease drains. p36 passed its one-byte direct-static forced-collection diagnostic (test durations 377.66 / 308.91 s), refuting allocation-lifetime failure for that scenario only; it does not test the nested resolver arm. p35 passed after repair (runner walls 627 / 623 s), reading a snapshot, not proving stack-home write-through. Both were promoted after passing in both configurations. The fixed Fill fast path itself was **not run** under an end-to-end Miri fixture.

Before the final-audit regression was added, blocking library commands used the flags above with `cargo +nightly-2026-05-27 miri test -p dotnet-value --lib --no-default-features pointer::tests::owner_retention_ -- --test-threads=1` (10 passed, zero ignored; test time 0.72 s), and the default-feature selector (13 passed, one intentionally ignored historical UB diagnostic). `-p dotnet-vm-data --lib --no-default-features [--features multithreading] stack::tests::owner_retention_ -- --test-threads=1` passed 8, zero ignored in each configuration (0.59 / 0.65 s). Full default `dotnet-value` Miri: 73 unit tests passed, 3 ignored, plus 1 integration test passed. After the audit repair, the same `owner_retention_` selector passed pinned Miri with 11 passed / 0 ignored without features and 14 passed / 1 ignored with multithreading; the ignored test is the historical known-UB diagnostic. The new test checks null and non-null Transient rejection and supported unmanaged and Heap observations.

The historical M1d `bash scripts/pointer_provenance.sh controls --configuration <cfg> --shard <1..17>` sweep passed all 33 controls in each configuration, retaining the original controls; in that paired layout shard 16 had p34/p35 and shard 17 p36. The current lane layout is in CI.md. A maximum of four shard commands ran concurrently, total elapsed about 131 min 29 s for both configurations. The following outer command walls include discovery; hosted timing was **not measured**.

| Shard | No-features: passed / wall seconds | Multithreading: passed / wall seconds |
|---:|---:|---:|
| 1 | 2 / 1024.19 | 2 / 1027.40 |
| 2 | 2 / 1045.77 | 2 / 870.72 |
| 3 | 2 / 898.15 | 2 / 922.40 |
| 4 | 2 / 1035.36 | 2 / 1002.37 |
| 5 | 2 / 941.15 | 2 / 898.35 |
| 6 | 2 / 985.14 | 2 / 979.27 |
| 7 | 2 / 995.76 | 2 / 871.42 |
| 8 | 2 / 954.44 | 2 / 926.70 |
| 9 | 2 / 968.90 | 2 / 958.39 |
| 10 | 2 / 931.95 | 2 / 933.13 |
| 11 | 2 / 983.81 | 2 / 906.24 |
| 12 | 2 / 921.41 | 2 / 902.34 |
| 13 | 2 / 1082.66 | 2 / 869.93 |
| 14 | 2 / 969.34 | 2 / 982.96 |
| 15 | 2 / 868.75 | 2 / 861.09 |
| 16 | 2 / 877.42 | 2 / 871.30 |
| 17 | 1 / 418.10 | 1 / 364.21 |

**Evidence limits:** Strict-provenance lanes were **not run**. The lease tests do not establish GC roots, ranges, synchronization, arena generations or yield resumption. The static test refutes only its controlled lifetime hypothesis; p12 does not establish teardown safety. Native outcomes and Miri outcomes are distinct.

## M1c baseline and outcomes

The pre-repair p04 baseline was measured at kickoff through the guarded diagnostic runner; the post-repair observations use the same pinned `nightly-2026-05-27` permissive-Tree-Borrows lane with `-Zmiri-ignore-leaks`. Runner wall times are per probe, not outer command wall. The pre-repair message's first Rust location was `crates/dotnet-runtime-memory/src/access.rs:1172:25`; the dangling-pointer reconstruction failure was observed before replacing the growing `Vec` storage. p05 remains the single-allocation control and its probe body is unchanged. p29–p34 have passing diagnostic runs in both configurations; p31's reported runs use the final source after its observer assertions were tightened, replacing earlier preliminary passes. The final promoted control runs are separately described in the CI/control records. These are permissive-TB results, not strict-provenance, leak-check, or full-VM Miri evidence.

| Probe / check | Configuration | Evidence lane | Outcome, first diagnostic and Rust location | Wall (s) |
|---|---|---|---|---:|
| p04 `p04_double_localloc_write_first` baseline | no-features | VM-M | `miri-ub` (exit 1); dangling pointer with no provenance, `crates/dotnet-runtime-memory/src/access.rs:1172:25` | 239 |
| p04 baseline | multithreading | VM-M | `miri-ub` (exit 1); same diagnostic and location | 224 |
| p04 after repair | no-features | VM-M | `pass` (1 passed, 0 failed, 0 ignored); no diagnostic | 255 |
| p04 after repair | multithreading | VM-M | `pass` (1 passed, 0 failed, 0 ignored); no diagnostic | 240 |
| p05 `p05_single_localloc_roundtrip` control | no-features | VM-M | `pass` in shard 2 (1 passed, 0 failed, 0 ignored); unchanged single-allocation control | 235 |
| p05 control | multithreading | VM-M | `pass` in shard 2 (1 passed, 0 failed, 0 ignored); unchanged single-allocation control | 231 |
| p29 `p29_localloc_blocks_survive_growth_and_are_16_byte_aligned` first pass | no-features | VM-M | `pass` (1 passed, 0 failed, 0 ignored); no diagnostic | 341 |
| p29 first pass | multithreading | VM-M | `pass` (1 passed, 0 failed, 0 ignored); no diagnostic | 330 |
| p30 `p30_localloc_block_survives_nested_callee_localloc` first pass | no-features | VM-M | `pass` (1 passed, 0 failed, 0 ignored); no diagnostic | 329 |
| p30 first pass | multithreading | VM-M | `pass` (1 passed, 0 failed, 0 ignored); no diagnostic | 292 |
| p31 `p31_localloc_blocks_released_on_return_and_pooling` final-source pass | no-features | VM-M | `pass` (1 passed, 0 failed, 0 ignored); no diagnostic | 365 |
| p31 final-source pass | multithreading | VM-M | `pass` (1 passed, 0 failed, 0 ignored); no diagnostic | 315 |
| p32 `p32_localloc_blocks_released_on_tail_call_replacement` first pass | no-features | VM-M | `pass` (1 passed, 0 failed, 0 ignored); no diagnostic | 387 |
| p32 first pass | multithreading | VM-M | `pass` (1 passed, 0 failed, 0 ignored); no diagnostic | 314 |
| p33 `p33_localloc_blocks_released_on_exception_unwind` first pass | no-features | VM-M | `pass` (1 passed, 0 failed, 0 ignored); no diagnostic | 357 |
| p33 first pass | multithreading | VM-M | `pass` (1 passed, 0 failed, 0 ignored); no diagnostic | 314 |
| p34 `p34_localloc_blocks_released_on_executor_teardown` first pass | no-features | VM-M | `pass` (1 passed, 0 failed, 0 ignored); no diagnostic | 283 |
| p34 first pass | multithreading | VM-M | `pass` (1 passed, 0 failed, 0 ignored); no diagnostic | 248 |
| `raw_block::` (7 tests) | no-features | LIB-M | `7 passed, 0 failed, 0 ignored`; no diagnostic | 1.413 |
| `raw_block::` (7 tests) | multithreading | LIB-M | `7 passed, 0 failed, 0 ignored`; no diagnostic | 1.475 |
| `frame_alloc::` (7 tests) | no-features | LIB-M | `7 passed, 0 failed, 0 ignored`; no diagnostic | 0.958 |
| `frame_alloc::` (7 tests) | multithreading | LIB-M | `7 passed, 0 failed, 0 ignored`; no diagnostic | 0.969 |
| `frame_alloc::` finalizer (8 tests) | no-features | LIB-M | `8 passed, 0 failed, 0 ignored`; includes suspension/restoration | 1.105 |
| `frame_alloc::` finalizer (8 tests) | multithreading | LIB-M | `8 passed, 0 failed, 0 ignored`; includes suspension/restoration | 1.036 |

The initial library test walls above measure the test-plus-log/count pipelines used for the blocking `miri-frame-blocks` job; the finalizer walls are direct command durations. The new test moves an owning frame through `FrameStack::suspend_above` and `restore_suspended`, allocates in the active frame, reads the earlier block after restoration, and checks cleanup. Exception-filter dispatch and nested suspension are now covered by plan 07 regressions. The guarded p04 and p29–p34 runs passed in both configurations and were promoted to blocking controls in shards 13–16. These results do not cover p09 (stack homes), p12 (static addresses) or native activations (plan 08). The nested-filter overwrite is fixed: suspended segments and a traced active-filter stack retain enclosing state through nested search, catch, rethrow and escaping filters. CLR differential fixtures `nested_filter_catches_42.cs`, `nested_filter_throws_42.cs`, `nested_filter_byref_42.cs`, `nested_filter_depth_42.cs` and `nested_filter_rethrow_42.cs` cover this path. The frame-allocation test restores and discards multiple nonempty segments; the complete 22-test VM-data library passed pinned Miri in both configurations.

### M1c final verification

On 2026-10-29, `./check.sh --fast` exited 0 in 88.442 s: verify-slots, fixture build and all three clippy configurations passed. Nextest reported 554 passed / 9 skipped (no features), 625 / 7 (multithreading), and 639 / 9 (multithreading,validation-all); the cfg and `std::sync` ceilings remained 421/421 and 70/70. The runner syntax/Python self-test also passed (11 tests). These counts are native, not Miri results.

The M1c final guarded Miri sweep discovered 31 exact non-ignored controls in each configuration and passed all 31, with zero failures. Each shard ran `bash scripts/pointer_provenance.sh controls --configuration CFG --shard N`; the following are outer command walls, including discovery. Shards 1–15 each passed two controls; shard 16 passed one. Each exact libtest invocation reported 1 passed, 0 failed, 0 ignored.

| Shard | no-features: passed / failed, wall (s) | multithreading: passed / failed, wall (s) |
|---:|---|---|
| 1 | 2 / 0, 1001.428 | 2 / 0, 1009.529 |
| 2 | 2 / 0, 964.572 | 2 / 0, 999.673 |
| 3 | 2 / 0, 1055.937 | 2 / 0, 1011.151 |
| 4 | 2 / 0, 801.331 | 2 / 0, 1009.351 |
| 5 | 2 / 0, 1065.894 | 2 / 0, 964.773 |
| 6 | 2 / 0, 1060.059 | 2 / 0, 954.213 |
| 7 | 2 / 0, 1075.993 | 2 / 0, 967.081 |
| 8 | 2 / 0, 1049.387 | 2 / 0, 925.699 |
| 9 | 2 / 0, 1094.971 | 2 / 0, 969.584 |
| 10 | 2 / 0, 1071.742 | 2 / 0, 965.297 |
| 11 | 2 / 0, 1103.821 | 2 / 0, 843.516 |
| 12 | 2 / 0, 1069.344 | 2 / 0, 973.538 |
| 13 | 2 / 0, 986.274 | 2 / 0, 806.386 |
| 14 | 2 / 0, 968.631 | 2 / 0, 908.883 |
| 15 | 2 / 0, 955.443 | 2 / 0, 920.158 |
| 16 | 1 / 0, 525.170 | 1 / 0, 498.876 |

The four sequential batches of four parallel shards took 4222.037 s and 3871.943 s respectively, excluding the preceding standalone discovery (4.415 s and 5.342 s). The slowest individual shard walls were 1103.821 s and 1011.151 s, below the 2700 s CI leg limit. These warm local timings include contention and discovery/build waits; additional Miri processes of unestablished origin were observed during the no-features sweep. Hosted cold-build/cache timings remain unmeasured. The lane is filtered permissive Tree Borrows with leaks ignored, not strict-provenance, leak-check, full-VM Miri, or soundness evidence.

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
| `pointer::tests::heap_access_` | no-features | LIB-M | `3 passed; 0 failed; 0 ignored` | 1 (warm) |
| `pointer::tests::heap_access_` | multithreading (default features) | LIB-M | `3 passed; 0 failed; 0 ignored` | 3 (warm) |

Control discovery used `bash scripts/pointer_provenance.sh verify-controls --configuration CFG`; it establishes the identities and non-ignored count, not a Miri pass. Only shard 3 was re-run at kickoff; the other nine controls' prior M1a outcomes are in the historical M1a final-shard table later in this section. The library measurements used the `pointer::tests::heap_access_` selector, no-features and default-feature multithreading respectively; the review does not preserve the full command lines.

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

The Phase 6 runs above report each promoted test as `1 passed; 0 failed; 0 ignored`; each two-control shard totals `2 passed, 0 failed`. The M1b final count was checked with `bash scripts/pointer_provenance.sh list` (31 catalog entries: 24 controls, 7 diagnostic-only) and `bash scripts/pointer_provenance.sh verify-controls --configuration no-features` plus the `multithreading` variant (each `control discovery: 24 exact non-ignored controls`). Discovery is native metadata, not test execution.

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

The final audit repaired arithmetic from a canonical unmanaged-origin managed null and widened checked managed-offset calculations so a compact `u32` offset is not sign-flipped on a 32-bit host. The extended `ptr_arith_managed_ptr_checked_offset_reports_errors` regression covers the compact boundary and an exposed address reconstructed and dereferenced via both `checked_offset` and `StackValue` addition. The eight-test selector passed natively and under pinned permissive-TB Miri in both no-features and multithreading configurations on x86_64; `memory-validation` passed 7 with the existing 1 intentional ignore. Miri warned about exposed-provenance reconstruction. Guarded VM shard 5 was rerun after these repairs: p13 and p14 passed in no-features (275 s and 271 s) and multithreading (338 s and 289 s), with 2 passed and 0 failed per configuration. Other VM shards were not rerun. A 32-bit target was not run, and this is not strict-provenance evidence.

The eight `pointer::tests::ptr_arith_` regressions are `ptr_arith_unmanaged_ptr_null_contract`, `ptr_arith_managed_ptr_checked_offset_reports_errors`, `ptr_arith_unmanaged_offsets_wrap_and_round_trip_with_deref`, `ptr_arith_unmanaged_arithmetic_wraps_to_explicit_null`, `ptr_arith_min_signed_operands_do_not_overflow`, `ptr_arith_pointer_comparisons_are_value_bearing`, `ptr_arith_managed_ptr_far_oob_compare_and_difference`, and `ptr_arith_managed_ptr_arithmetic_boundaries_report_explicit_errors`.

- Native, no-features: `cargo test -p dotnet-value --lib --no-default-features pointer::tests::ptr_arith_ -- --test-threads=1` → 8 passed, 0 failed, 0 ignored.
- Native, multithreading: `cargo test -p dotnet-value --lib --no-default-features --features multithreading pointer::tests::ptr_arith_ -- --test-threads=1` → 8 passed, 0 failed, 0 ignored.
- LIB-M, pinned permissive-TB: `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" cargo +nightly-2026-05-27 miri test -p dotnet-value --lib --no-default-features pointer::tests::ptr_arith_ -- --test-threads=1` → 8 passed, 0 failed, 0 ignored; the same command with `--no-default-features` omitted (default multithreading configuration) → 8 passed, 0 failed, 0 ignored. Both were measured at step 6.2. Current no-features, default/multithreading and `memory-validation` `--list` selectors each discover exactly 8 tests.
- Native, `memory-validation`: `cargo test -p dotnet-value --lib --features memory-validation pointer::tests::ptr_arith_ -- --test-threads=1` → 7 passed, 0 failed, 1 ignored. The only ignored selector is the deliberate heap-owner out-of-bounds arithmetic case ignored because that feature's bounds trap rejects its deliberate out-of-bounds offset.

#### M1b validation limits

The finished-tree M1b Criterion comparison was not measured on an idle host and does
not establish an isolated regression. Detailed historical comparisons are retained
in git at `747ac78d`.

The VM controls use the **filtered permissive Tree Borrows** lane: `nightly-2026-05-27` with `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`. This is **not strict-provenance evidence**, **not a full VM Miri suite**, and **not a soundness proof**. The 24 controls are an explicitly selected bounded lane; passing them establishes only their asserted paths, not every pointer, storage, ownership or ABI contract. Native discovery (`verify-controls`) is not execution. The Phase 6 control runs were not native executions; the pre-repair native diagnostics for p17/p18 are recorded separately above.

<a id="recreated-library-regressions-and-final-vm-passes"></a>

### Library regressions and final VM passes

The first two tests in [`crates/dotnet-value/src/pointer/tests.rs`](../../crates/dotnet-value/src/pointer/tests.rs) are recreations of the reported test intents; the third is a new typed-write interleaving regression:

| Test | What it asserts | Pinned Miri outcome |
|---|---|---|
| `pointer::tests::heap_access_one_byte_heap_read` | A one-byte `UInt8` heap vector returns exactly one byte (`0xA5`) from `with_data(1, ...)`. | 1 passed / 0 failed / 0 ignored in no-features and multithreading |
| `pointer::tests::heap_access_safe_oob_address_observation` | At logical offset 4096, `PointerLike::pointer()` and `StackValue::ManagedPtr::as_ptr()` report the retained-base address plus wrapping offset; it does not dereference. | 1 passed / 0 failed / 0 ignored in both configurations |
| `pointer::tests::heap_access_heap_pointer_survives_typed_write` | For array, object and boxed storage, an observed heap base remains usable across a guarded typed write of 5; raw write of 7 followed by guarded read returns 7 (including the exposed-integer reconstruction path). | 1 passed / 0 failed / 0 ignored in both configurations |

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

Not changed by the M1a repairs: Stack/Static observation and the `ManagedPtr::with_data` stack fallback; the string write contract (no writable access to interned Arc-backed data); P/Invoke pinning/guard lifetime, ABI and return/copy semantics; and general `cpblk`/`initblk` correctness (count width, checked complete ranges, barriers/rooting and yield/resumption state). At that point the legacy `StackValue` managed-pointer-versus-native-integer comparisons and managed-pointer subtraction called `managed_ptr_to_raw`, whose Heap arm used in-bounds `.add` even for a far-out-of-bounds logical offset; the M1b work addressed those.

**Evidence limits:** Native Criterion benchmark numbers are performance measurements, not Miri evidence. Historical native comparisons recorded repeated no-features regressions; those
measurements remain in git at `747ac78d` and must not be treated as pointer-safety evidence.

## Initial six-control calibration (2026-10-27)

On **2026-10-27**, each Phase 2 cell below executed one exact ignored control via `bash scripts/pointer_provenance.sh diagnostic --configuration CONFIG --probe NAME`, separately, with `nightly-2026-05-27`, `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`, `--no-default-features`, and optionally `--features multithreading`. At that time the controls were still ignored; they are now enabled, so use `controls` rather than `diagnostic` to rerun them. All twelve were exit 0, one executed/passed and zero ignored; wall seconds are script measurements. The pre-finalizer session log can be consulted in branch history with `git show a2ac27d1:AGENT_MEMORY.md` if that history is retained; it records exact commands, results and timestamps.

| Control | no-features wall | multithreading wall |
|---|---:|---:|
| p00 | 241 s | 228 s |
| p02 | 236 s | 228 s |
| p03 | 238 s | 228 s |
| p05 | 237 s | 224 s |
| p07 | 230 s | 228 s |
| p11 | 228 s | 229 s |

An earlier p00 raw pass was rejected by the diagnostic recognizer because warnings interrupted its status line; after the recognizer fix it was rerun successfully (recorded in the historical session log entries 2.1 and 2.7). Native nextest passed the original six controls in each configuration, with 17 ignored at that time. Enabled Miri shard 1 separately passed p00/p02 in both configurations: no-features outer wall 453.927s (226s/224s child), multithreading 483.700s (240s/240s child). The initial blocking matrix then paired p00/p02, p03/p05, p07/p11; subsequent promotions later produced an 11-control/six-shard M1a-era layout, superseded by the 24-control/twelve-shard Phase 6 lane above. Local results do not establish hosted CI cold-build/cache timings or hosted job success. All these are filtered **permissive** TB results, not full VM Miri, strict Miri, or runtime soundness.

## Finding-to-work map

Each row defines a finding ID, the evidence gathered for it and the scope of what was repaired.
Model, library and source-review evidence is kept distinct from real VM runs. Problems without a
current plan are listed under [Not yet planned](../plans/README.md#not-yet-planned).

| ID | Finding | Evidence | Scoped result |
|---|---|---|---|
| R01 | Oversized heap slice, shared-derived writable address, safe out-of-bounds formation | Baseline p10/p13/p14/p16/p20 each miri-ub at the oversized slice; after the M1a repair all five pass in both configurations; p11 is a passing control. Three LIB-M recreations also pass. | Heap access repaired for the tested paths. Stack and static targets are not covered. |
| R02 | Assigning to a local invalidates an earlier byref; evaluation-stack growth invalidates an address held as a native integer | p01/p09/p37/p38 controls with p02/p03; LIB-M absent; models E12b/E13 distinct | Repaired for locals and arguments by [frame homes](../GC_AND_MEMORY_SAFETY.md#frame-owned-homes); byrefs to homes resolve through the frame check (`FrameHome`). Constructor receivers also use caller-owned temporary homes. Typedref address-taking is rejected under the chosen strict ECMA policy; the Stack origin and fixup have been removed. Resolved: native fast checks and all 42 pointer controls under Miri pass in both configurations; the subsequent completion audit passed all 44 controls in both configurations. See [Plan 02](../plans/02-stable-frame-homes.md#completion-validation) for validation limits and reproduction. |
| R03 | Transient owner destroyed while in use; expired static/arena guards | Pre-repair native liveness/clone failures and Miri dangling accesses in both configurations; repaired tests pass. Controlled cross-arena teardown gave Miri UAF before repair, rejection and in-flight lease passes after (multithreading only). p36 refutes the static allocation-lifetime hypothesis for its direct one-byte case; p12 panicked on alignment, not UB. | Owner/guard retention repaired for the tested paths. Fill reachability is source-inferred. |
| R04 | Unmanaged serde exposure; integer GC transport | Historical LIB-M sources absent; p05 VM-M control refutes unconditional failure only. Retained-transport recreation and isolated strict-core experiment pass. | Retained GC transport implemented. Unmanaged serde not changed; p05 is not a general serde fix. |
| R05 | Repeated localloc; pointer arithmetic, null, wrapping and comparison | Step 0.1 measured p06/p08 Miri UB and p17/p18 assertion failures, p_null passed; Phase 6 controls p06–p08, p17/p18, p21–p24 and p_null passed in both configurations. M1c p04 and p29–p34 passed in both configurations. | Arithmetic, comparison and null paths repaired; stable frame-local `localloc` storage and its tested lifecycle paths repaired. |
| R06 | Pinned payload address; FFI return overwrite | Heap-origin P/Invoke address derivation uses the observer under existing pins/guards; no native-call return/copy regression added. Historical Rust/libffi sources missing. | No P/Invoke lifetime, ABI or return contract repaired. |
| R07 | By-ref/out copy, temporary growth, null and string ABI | Existing P/Invoke crate tests pass in both configurations; no end-to-end native ABI evidence. | Not repaired. |
| R08 | Cross-arena root barrier and teardown; arena/address packing | Library model, ID40000 codec, registry/library ABA, real-VM collection and teardown recreations; details below. | Root barrier, retained transport, identity/token checks and executor teardown ownership implemented. The real-VM collection tests transfer production-encoded field storage and decode through receiver-owned instance fields. |
| R09 | Host races and alignment, mixed-size overlap, static atomics | MODEL E9/E10 reports; VM paths R/A only | Not repaired. |
| R10 | Block-operation barriers, count width, yield state | p14 `initblk` and p16 `cpblk` pass in both configurations for short valid scalar ranges; p15 checked frame-home `initblk` passes in both configurations | Heap address formation and checked frame-home ranges only. Chunked yield/retry, general count width and barriers not established. |
| R11 | Negative index, `ByteOffset` sign, `conv.i` and native-int stores | Phase 6 controls p24–p28 and p_null pass in both configurations; p19 is a reported native panic; `p_wide_store_into_int_local` unverified. | `ByteOffset` direction, pointer conversion/store and native-integer round trips repaired for the asserted paths. Negative array index not covered. |
| R12 | `volatile.` ignored, Interlocked overflow, sync-block race, `refanytype` lifetime | R/A only; no integrated evidence | Not confirmed or repaired. |
| R13 | Atomic backing aliasing, strict-provenance dependencies, blocking VM coverage | Historical LIB-M report unlocated. The two historically ignored atomic tests (`test_atomic_load_store`, `test_concurrent_atomic_load_store`) were diagnosed 2026-10-31: the Tree Borrows UB was test backing aliasing, and the Stacked Borrows ICE is a Miri mixed-size store-buffer bug. A provenance-lint audit the same day found 59 hits, all migrated; `provenance-lints` is a blocking deny gate. | Atomic test pair repaired. Strict-provenance dependency diagnosis not done. |

R08 evidence categories: the independent E11-style **library model** showed a premature drop
without the barrier in both configurations; it is not a VM test. The ID40000 **library codec**
recreation failed by null assertion before unsigned decoding and passes after repair, including
release. The **actual Executor** teardown witness confirmed early normal destruction before drain;
the repaired normal and extracted paths pass their measured tests. The **real-VM** collection
recreation uses production coordinator commands, payload access and eventual reclamation.
Plan 06 replaces raw-pointer fixture branding with owner-side encoding and receiver-side
instance-field decoding through the production codec for both cross edges. Temporary carrier
fields are cleared immediately after decoding. The earlier two-collection pre-fix run was
invalidated by a Miri dangling-pointer diagnostic; it remains historical evidence, not a
production-codec result. No-features supplies single-arena/local controls;
registry and cross-arena transport are absent.

R08 compact codec: the boundary, unavailable-owner, short-storage and managed-ID exhaustion tests are recreations. The interim multithreaded Tag-5 codec supports 64-bit pointers, registered owner IDs 0..=65535 and nonzero 8-byte-aligned addresses below 2^48; INVALID remains untagged. The VM reserves ID 0 and allocates monotonic IDs 1..=65535 without recycling, so it can create at most 65,535 managed thread arenas over a process lifetime before a host invariant panic. Unrepresentable or unavailable managed objects likewise fail loudly rather than becoming null. The generation-qualified origin and retained transport do not remove the compact ObjectRef's unsafe live-original-allocation contract.

R08 extracted-Executor TLS ownership: `executor::executor_teardown_tests::extracted_executor_tls_ownership` is a recreation. Before repair its multithreading assertion failed because a second Executor was accepted after extraction and before the first Executor dropped; a separate actual-Executor witness observed the successor arena destroyed while its lease was held. The regression is enabled and passes with the ownership, affinity and panic-retention repair ([plan 01](../plans/01-executor-teardown-ownership.md)). No-features has no extracted/TLS backend and uses the single-arena cleanup control.

R08 teardown stress: `executor::executor_teardown_tests::arena_lifetime_teardown_stress` is a recreation, not a complete R08 proof. Before teardown repair, repeated real two-Executor coordinated collections followed by a held-lease exit exposed early destruction in the normal path, while extracted-arena exits preserved the witness; the no-features counterpart repeats single-arena collection and cleanup. The enabled MT test covers normal/extracted ownership and held/released leases over 20 native rounds or four Miri rounds. Worker owners release leases and channels before joining, including on unwind; an outer process timeout is still needed for a genuine deadlock. The teardown witness is allocated after collection.

The 5.3–5.5 nested-creation, self-held-lease and external-remover recreations were written with the repair. Step 8.6 retrospectively overlaid that post-fix test source on `cdb7b642`, the pre-repair parent of `ec0b7ac3`; each failed natively as intended, while extracted teardown passed as a control. These are retrospective results, not contemporaneous pre-fix or Miri runs.

R08 Executor teardown order: the normal, extracted and unwind active-lease witness tests in `executor::executor_teardown_tests` are recreations. Normal destruction and unwind dropped the actual Executor arena before its held lease drained; those tests are enabled after the owner-held registration repair, which drains before freeing (or leaks on a self-held lease). The no-features test is a single-arena cleanup control. These tests do not establish object rooting, all concurrent schedules, nested Executor TLS ownership or R08 completion.

Baseline source reading supports but does not execute these diagnoses: [block operations and localloc](../../crates/dotnet-vm/src/instructions/memory.rs), [growing allocation pool](../../crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs), [conversions](../../crates/dotnet-vm/src/instructions/conversions.rs), [array handlers](../../crates/dotnet-vm/src/instructions/objects/arrays.rs), [static field handler](../../crates/dotnet-vm/src/instructions/objects/fields.rs), [pointer representation/access](../../crates/dotnet-value/src/pointer/mod.rs), and [stack pointer arithmetic/comparison](../../crates/dotnet-value/src/stack_value.rs). The controlled cross-arena teardown defect and limited static refutation are recorded under R03.

## Strict exclusions and candidate selectors

These are **candidate commands**, not recovered historical filters or claims of successful runs. Dependency traces and exact historical strict-core filters are missing. Record setup, configuration, actual first failure and logs when independently investigated; distinguish intentional exposed reconstruction from dependency blockers.

| Boundary | Candidate command / task | Limitation |
|---|---|---|
| VM parser (`bitvec`/dotnetdll) | `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-strict-provenance -Zmiri-disable-isolation -Zmiri-ignore-leaks" cargo +nightly-2026-05-27 miri test -p dotnet-vm --lib --no-default-features -- --exact pointer_provenance_tests::p00_smoke --test-threads=1` | Proposed exact smoke selector; earlier docs report dependency integer reconstruction, not a recovered current stack trace. |
| multithreading / parking_lot, crossbeam-epoch, rayon | Same p00 command with `--no-default-features --features multithreading` | Dependency blockers reported, exact current first-failure frame unavailable; legacy advisory `jmp_tests`, `tail_calls`, `fault_tests` are substring filters, not exact selectors. |
| Exposed native-int reconstruction | Same strict flags and command, exact `pointer_provenance_tests::p05_single_localloc_roundtrip`; repeat with multithreading if investigated | Permissive p05 pass does not imply strict reconstruction works. |
| Managed strict core | Same strict flags; candidate discovery `cargo +nightly-2026-05-27 miri test -p dotnet-runtime-memory --no-default-features -- --list` | Recover library source/results and select actual exact names later; no invented historical allowlist. |

Whole-VM strict Miri, dependency repairs, supplemental Stacked Borrows and unrelated VM Miri expansion are not established by these selectors or this harness.

## R08 registry, transport and origin recreations

R08 registry: the duplicate, fast-bit, exhaustion, reuse and identity/token tests are recreations. The MT registry issues non-reused tokens and exact identities; stale tokens and identities cannot acquire a successor lease or unregister it. In step 0.6 duplicate replacement, premature fast-bit clearing and generation wrap failed by assertion before repair; old-drain/new-registration already passed. Step 6.1 checks the contended removal checkpoint deterministically and verifies the successor's bit, identity and token survive completion of the old drain. Production counter allocation is tested on isolated registries; the separate checked-counter control is a model. The no-features registry is absent. These are library tests, not VM runs.

R08 retained command primitive: `gc::opaque_transport_tests::arena_lifetime_opaque_transport` and `object::tests::arena_lifetime_opaque_gc_transport` are recreations. The former checks the command wrapper structurally and transports a Box stand-in through an MT command with strict-provenance Miri; the latter uses a real GC allocation in a local mutation, not an end-to-end VM collection. `OpaqueObjectPtr` retains a pointer, and `CrossArenaRef` carries it with the exact identity through producers, harvest and commands. The sentinel and numeric GC reconstruction were removed; installation checks the destination identity under a lease and publishes through the root barrier.

R08 ordinary origins (step 3.1): `pointer::tests::arena_identity_in_memory_origin_aba`, `access::tests::arena_identity_memory_owner_aba`, and `call::tests::arena_identity_native_escape_preflight` are recreations. Scoped accesses reject a successor registration before reading the original object. ManagedPtr word 1 carries the non-reused checked registration token; the enabled origin/serialized ABA recreation rejects a successor registration. Retained GC transport also carries identity, but the one-word ObjectRef still requires the original live allocation. With maintainer approval, cross-arena typedrefs and non-ref/out ManagedPtr native escapes are temporarily rejected by P/Invoke before argument marshalling; ref-copy arguments retain their existing path. The ID-only `StackValue::CrossArenaObjectRef` variant was retired. By-value byref-like aggregates are separately rejected in both configurations. Native-call lifetime, callback and ABI contracts are not covered. The no-features configuration rejects the cross-arena tag and has no cross-arena MemoryOwner/P/Invoke preflight.
