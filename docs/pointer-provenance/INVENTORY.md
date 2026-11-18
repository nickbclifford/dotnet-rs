# Pointer provenance evidence inventory

Evidence record. Not instructions; current work is in [docs/plans/](../plans/README.md).

This file is the single place where the review's IDs are defined:

- **Probe IDs** (`p00`…`p44`, `p_null…`, `p_wide…`) are the rows of the [probe catalog](#probe-catalog);
  each names an exact test in the [owning VM module](../../crates/dotnet-vm/src/pointer_provenance_tests.rs),
  run through the [catalog/runner](../../scripts/pointer_provenance.sh).
- **Finding IDs** (`R01`–`R13`) are the rows of the [finding map](#finding-to-work-map).
- **E1–E13** name standalone models from the original review; their sources are not in the repository.
- **M1a–M1d** label the milestones of a retired plan under which evidence below was gathered
  (heap access, pointer semantics, frame-local `localloc`, owner/guard retention). They are labels
  for this record only.

Evidence vocabulary: **VM-M** real VM under Miri; **LIB-M** library Miri; **N** native reproduction; **MODEL** standalone model, not an end-to-end VM result; **R/A** source review/audit, requiring confirmation. A historical report below is not a fresh run. Probe rows marked unknown remain unverified. Sources can support a diagnosis without proving an execution result.

The original pass-1/2/3 review directories, logs, lab sources and patches (including the 3c2
patch) are absent from the repository; an external backup has not been ruled out. Tests written
to reproduce their reported intents are **independent recreations**: they make no identity claim
about the missing originals. That applies to every "recreation" below.

Miri results use the lane described under [Evidence limits](#evidence-limits).

## Probe catalog

Each name is an exact `pointer_provenance_tests::<name>` filter in the owning source. The Original column gives `input:line` in the review inputs (3, 3b, 3c and pass-2 scratch files) vendored into the owning module; `new` marks probes written for the repairs. The current catalog is in `scripts/pointer_provenance.sh`; the lane layout is in [CI](../CI.md#blocking-miri-pointer-controls-lane). **C** = blocking native and permissive-TB control; **U** = known or suspected host UB, diagnostic permissive TB only; **S** = semantic prediction, diagnostic only; **P** = reported native panic, diagnostic only; **X** = baseline unverified, diagnostic permissive TB only. The remaining diagnostic-only probes have no native permission. The script runs each diagnostic in a bounded separate process. Do not run diagnostic-only known-UB probes natively or use ignored-test flags outside their diagnostic entrypoint. Review group-UB reports lack surviving per-probe transcripts; U alone does not establish an individual UB result. The negative-index control asserts the managed exception after checked element-stride arithmetic; its [cleanup record](../plans/27-audit/cleanup-vm.md) distinguishes the reproduced overflow from successful native and Miri verification.

| Exact probe (owning source) | Original | Finding | Status, evidence and outcome |
|---|---|---|---|
| [`p00_smoke`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:191 | R13 | C; VM-M control passed in both configurations (final shard lane) |
| [`p01_ldloca_convu_stind_i4`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:210 | R02 | C; with `FrameHome` byrefs to frame homes, VM-M control passed in both configurations, as it did with the earlier `Unmanaged`-origin home byrefs. Before homes: the diagnostic was `miri-ub` in both configurations, write through `<wildcard>` forbidden at `dotnet-runtime-memory/src/access.rs:1175` (the address was derived from a shared borrow of the `Vec` slot) |
| [`p02_ldloca_stind_i4_control`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:229 | R02 | C; VM-M control passed in both configurations (final shard lane) |
| [`p03_ldloca_convu_ldind_i4`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:247 | R02 | C; VM-M read control passed in both configurations (final shard lane) |
| [`p04_double_localloc_write_first`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:266 | R05 | C; M1c repair passed in both configurations; pre-repair VM-M `miri-ub` at `dotnet-runtime-memory/src/access.rs:1172:25` |
| [`p05_single_localloc_roundtrip`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:291 | R04, R05 | C; VM-M control passed in both configurations; not a general serde fix |
| [`p06_localloc_minus_one_then_plus_one`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:313 | R05 | C; Phase 6 control passed in both configurations; pre-repair VM-M `miri-ub` |
| [`p07_localloc_one_past_end`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:340 | R05 | C; VM-M formation-only control passed in both configurations |
| [`p08_localloc_overshoot`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:360 | R05 | C; Phase 6 control passed in both configurations; pre-repair VM-M `miri-ub` |
| [`p09_ldloca_convu_survives_callee_stack_growth`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:464 | R02 | C; with frame homes and `FrameHome` byrefs, VM-M control passed in both configurations. Before homes: the diagnostic was `miri-ub` in both configurations, 4-byte access through a dangling address with no provenance at `dotnet-runtime-memory/src/access.rs:1175` after the `Vec` reallocated |
| [`p10_ldelema_convu_stind_heap_array`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:498 | R01 | C; M1a VM-M control passed in both configurations after repair; pre-repair miri-ub |
| [`p11_ldelema_stind_heap_array_control`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:534 | R01 | C; VM-M control passed in both configurations (final shard lane) |
| [`p12_ldsflda_convu_stind_static`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:569 | R03 | U; pinned Miri alignment `debug_assert` panic in both configurations, not UB and not a teardown proof |
| [`p13_ldelema_managed_add_far_out_of_bounds_then_conv_u`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:596 | R01 | C; M1a VM-M control passed in both configurations |
| [`p14_initblk_heap_array_via_managed_ptr`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:798 | R01, R10 | C; M1a VM-M control passed in both configurations for its short valid block |
| [`p15_initblk_stack_local_via_managed_ptr`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:829 | R10 | C; native and pinned Tree Borrows Miri pass in both configurations after resolving the complete frame-home range before access |
| [`p16_cpblk_heap_arrays_via_managed_ptr`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:849 | R01, R10 | C; M1a VM-M control passed in both configurations for its short valid block |
| [`p17_localloc_ptr_ceq_self`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:894 | R05 | C; Phase 6 control passed in both configurations; pre-repair native and VM-M assertions failed |
| [`p18_localloc_ptr_clt`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:914 | R05 | C; Phase 6 control passed in both configurations; pre-repair native and VM-M assertions failed |
| [`p19_ldelem_negative_index`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3:936 | R11 | C; checked element-stride overflow produces managed `IndexOutOfRangeException`; native and pinned Miri pass in both configurations ([evidence](../plans/27-audit/cleanup-vm.md)) |
| [`p20_heap_convu_then_typed_write_then_raw_store`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | 3c:893 | R01 | C; baseline miri-ub and post-repair VM-M control pass in both configurations; patched-source history is not used as evidence |
| [`p21_localloc_signed_extreme_offsets_roundtrip`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05 | C; Phase 6 control passed in both configurations; pre-repair no-features diagnostic was Miri UB |
| [`p22_localloc_wrap_to_zero_roundtrip`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05 | C; Phase 6 control passed in both configurations; pre-repair no-features diagnostic was Miri UB |
| [`p23_localloc_pointer_loop_unsigned_branch_compare`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05 | C; Phase 6 control passed in both configurations; pre-repair no-features diagnostic was an assertion failure |
| [`p24_unsafe_byte_offset_direction_and_deref`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05, R11 | C; Phase 6 control passed in both configurations; pre-repair ByteOffset direction assertion failed |
| [`p25_convi_and_stind_i_pointer_roundtrip_deref`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R11 | C; Phase 6 control passed in both configurations; pre-repair conversion/store diagnostic failed |
| [`p26_native_int_boundary_store_load_roundtrip`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R11 | C; Phase 6 control passed in both configurations; the pre-repair no-features diagnostic passed |
| [`p27_stind_i_fresh_pointer_through_local_slot_dereferences`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new (step 5.4) | R11 | C; Phase 6 control passed in both configurations; no step 0.1 probe baseline |
| [`p28_stind_i_fresh_pointer_through_heap_memory_dereferences`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new (step 5.4) | R11 | C; Phase 6 control passed in both configurations; no step 0.1 probe baseline |
| [`p29_localloc_blocks_survive_growth_and_are_16_byte_aligned`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05 | C; VM-M control passed in both configurations; stable blocks survive later allocations and are 16-byte aligned |
| [`p30_localloc_block_survives_nested_callee_localloc`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05 | C; VM-M control passed in both configurations; caller block remains readable after nested callee allocation |
| [`p31_localloc_blocks_released_on_return_and_pooling`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05 | C; VM-M control passed in both configurations; frame blocks and homes released on return and pooling |
| [`p32_localloc_blocks_released_on_tail_call_replacement`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05 | C; VM-M control passed in both configurations; replaced-frame blocks and homes released on tail call |
| [`p33_localloc_blocks_released_on_exception_unwind`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05 | C; VM-M control passed in both configurations; unwound-frame blocks and homes released before catch |
| [`p34_localloc_blocks_released_on_executor_teardown`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R05 | C; VM-M control passed in both configurations; live blocks and homes released on executor teardown |
| [`p35_ldflda_on_stack_value_type_then_ldind`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R03 | C; post-repair VM-M pass in both configurations; pre-repair exception resolution failure, not UB; throw site inferred only; snapshot read, not stack-home write-through |
| [`p36_static_address_survives_forced_collection`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R03 | C; one-byte direct static access after forced collection passed VM-M in both configurations; refutes only the tested lifetime hypothesis |
| [`p37_ldloca_value_type_survives_stloc`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R02 | C; VM-M control passed in both configurations with local frame homes, and again with `FrameHome` byrefs; without homes the same probe was `miri-ub` (dangling 4-byte access after `stloc` replaced the value-type slot's buffer) |
| [`p38_ldarga_value_type_survives_starg`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R02 | C; argument analogue of p37 (`ldarga`, `starg`, `conv.u`, `stind`); VM-M control passed in both configurations with argument homes |
| [`p39_frame_homes_survive_filter_suspension`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R02 | C; VM-M passed in both configurations; filter acceptance/rejection, collection of an array rooted only in a suspended home, checked reads/writes and bounds rejection; retired serial rejected after unwind |
| [`p40_jmp_forwards_modified_home_argument`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R02 | C; VM-M passed in both configurations; scalar and value-type homes forward current arguments across `jmp`; replaced serial rejected and target serial fresh |
| [`p41_constructor_receiver_survives_nested_growth`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R02 | C; native and VM-M passed in both configurations; constructor receiver homes survive native-address writes across nested evaluation-stack growth, collection with an array rooted only in the receiver home, and constructor `jmp` completion |
| [`p42_typed_reference_address_policy`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R02 | C; local/argument address-taking rejected for initialized and uninitialized typedrefs, including named System.TypedReference signatures; ordinary copying, type extraction and dereferencing remain supported. Native and pinned Tree Borrows Miri pass in both configurations, including collection with an array rooted only through a typedref and wrong-type rejection. |
| [`p_null_through_pointer_local_address`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | pass-2:168 | R05, R11 | C; step 0.1 and Phase 6 VM-M controls passed in both configurations; see M1b outcomes below |
| [`p43_enum_home_scalar_roundtrip`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R02 | C; scalar and enum `ValueType` arguments copied into their homes; saved local/argument addresses and serials survive reassignment and observed nested stack growth; explicit `ldobj`/`stobj` and native-address reads/writes preserve values. Native and pinned Tree Borrows Miri pass in both configurations. |
| [`p44_native_substring_gc_resume`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | new | R02 | C; native Substring progress and source rooting across collection and resumption; both configurations exercise the async copy. |
| [`p_wide_store_into_int_local`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) | pass-2:179 | R11 | X; VM-M current outcome unknown. Natively, with frame homes the 8-byte `stind.i8` into the 4-byte homed local fails the frame-home bounds check and raises `System.AccessViolationException`, which the mock corlib cannot resolve; the source assertion (`0x55`) is not reached |

For an individual diagnostic-only probe, use e.g. `bash scripts/pointer_provenance.sh diagnostic --configuration no-features --probe p01_ldloca_convu_stind_i4`; this is an opt-in diagnostic, not a CI gate. Do not run known-UB diagnostics natively. `bash scripts/pointer_provenance.sh list` lists the exact catalog; see the script for options.

## Native effects and cross-arena pointer inventory

These entries identify provenance-sensitive paths, not additional probe IDs. The predicates and
their premises are defined in the [invariant registry](../INVARIANT_REGISTRY.md); the source
observations below are R/A, while the linked tests supply bounded execution evidence.

| Path | Pointer or carrier and lifetime boundary | Evidence / limits |
|---|---|---|
| [`poll_native_impl` and `NativeControl::enter`](../../crates/dotnet-vm-ops/src/effects.rs) | A `Cell<*mut ()>` holds the lifetime-erased `NativeRegion` only during a poll. The reset guard clears it on return or unwind; `enter` prevents recursive mutable entry and reborrows the same type with erased lifetimes. See `F6.NativePollAdapter`. | Bridge unit tests and the native-bridge Miri rows in [CI](../../.github/workflows/ci.yml); erasure requires all four premises in the registry. |
| [`RootedValue::stash` / `fetch`](../../crates/dotnet-vm-ops/src/effects.rs), [`Adapter::frame_home_block`](../../crates/dotnet-vm/src/stack/native_effects.rs) | A traced `Gc<RootCell>` in the activation's `DynamicRootSet` stores origin, type, offset and pinning, not a cached branded address. Fetch checks the root set and reconstructs: `FrameHome` resolves the serial in the active stack or `parked_finalizer_caller`, rejecting a missing home or out-of-range offset; `Heap` and MT `CrossArenaObjectRef` use `observe_address`; `Static` is re-resolved by type and generics and bounds-checked on each access; `Unmanaged` addresses keep the guest's `F10.RawMemoryAccessValid` obligation. The traced origin records foreign references; a cross-arena access also needs the live owner's lease. `Transient` is rejected. See `F6.NativePointerDescriptor`. | Frame-home retirement and heap/foreign-owner controls in the CI matrix; a resolved address alone is not an access authorization. |
| [`NativeRegion::write_monitor_flag`](../../crates/dotnet-vm-ops/src/effects.rs), [`Adapter::write_monitor_flag`](../../crates/dotnet-vm/src/stack/native_effects.rs) | The region fetches the rooted descriptor again before the backend writes one byte through `RawMemoryOps::write_bytes`, which checks the live owner and full range. The caller must supply synchronized writable bool storage without overlapping reference slots. See `F10.RawMemoryAccessValid`. | Monitor frame-home, heap, alias and collection controls in the CI matrix; bounds checks do not establish the caller's storage/aliasing contract. |
| [`StaticStorageManager::mark_failed_with_exception` / `failed_exception`](../../crates/dotnet-vm/src/statics.rs) | A one-reference `FieldStorage` with a GC descriptor carries the wrapper; the existing exception root remains live until the carrier is published before the failed state. `StaticStorage::trace` visits it, and `failed_exception` decodes during a live arena mutation. The wrapper retains its inner exception. See `F6.InitializerFailureRoot`. | Local and retired-owner identity tests across collection: [slice 4 verification](../plans/08-async-effects/slice-4-static-initialization.md#verification). |
| [`SharedGcHandles`](../../crates/dotnet-runtime-memory/src/handles.rs) | VM-wide slots retain `ObjectPtr` provenance; strong and pinned targets are traced by their owner, weak pointers are cleared before sweep, and dependent keys and values are activated, traced, and cleared by their respective owners. Reads rebrand a pointer only during an active mutator, which must root the result before a safepoint; retired owners remain registered through collection. | Cross-thread GC-handle and dependent-handle diff fixtures and executor collection controls; cross-arena resurrection and long-weak fixtures. |
| [`Finalizers`](../../crates/dotnet-vm/src/finalizers.rs) | Pending and active pointers are traced by their owners until the dedicated worker completes; the final root barrier retraces publications. Weak suppression and re-registration requests are consumed by the owner before sweep. Retired owners remain registered while managed code can collect. | Finalizer-thread identity, re-registration and cross-arena collection fixtures; finalizer handoff Loom model. |
| [`ObjectRef::trace`](../../crates/dotnet-value/src/object/mod.rs), [`Executor::serve_retired`](../../crates/dotnet-vm/src/executor.rs), [`WorkerRegistry::shutdown`](../../crates/dotnet-vm/src/threading/workers.rs) | MT tracing reads the immutable owner ID through the object pointer before recording a foreign root. Retired owners service GC until collection is excluded and workers/finalizers are quiescent; unregister precedes lease drain and freeing. Shutdown waits for managed workers to quiesce before signalling retired owners and joins all workers. On abort pending finalizers are abandoned. See `F1.RetiredCollectionExclusion` and `F1.StwParked`. | Retired-owner and shutdown controls in CI; the owner-ID read relies on this teardown ordering, not on a lease alone. |
| [`test_utils::foreign_reference`](../../crates/dotnet-vm/src/test_utils/foreign_reference.rs) | Test `encode_reference` uses the production `FieldStorage` reference codec and transports pointer-bearing storage intact; `decode_reference` reads through a temporary object in the destination mutation, then clears its field. Tests must separately keep the owner and reference live; the temporary carrier is not an extra collection root. | Used by foreign-owner and retired-initializer tests; it is a test transport, not a standalone rooting or lifetime guarantee. |

## Evidence limits

- **Lane.** Pointer-lane and library Miri results use the pinned `nightly-2026-05-27` toolchain
  with `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`: filtered,
  permissive Tree Borrows with leaks ignored. That is not strict-provenance evidence, a full VM
  Miri suite, leak checking or a soundness proof. Only isolated library cores run with
  `-Zmiri-strict-provenance` ([CI](../CI.md#strict-provenance)).
- **Configurations.** "Both configurations" means `--no-default-features` with and without
  `--features multithreading`. Cross-arena paths exist only under `multithreading`.
- **Runner.** `bash scripts/pointer_provenance.sh list | controls | diagnostic | verify-controls`
  reproduces any row; the script pins the toolchain and flags. Diagnostic-only probes run one per
  process through `diagnostic`.
- **History.** Pre-repair baselines, per-probe wall times and milestone transcripts were removed
  from this file; they are in git. A historical result is not a fresh run.
- **Recreations.** The original pass-1/2/3 review archives are absent; tests that reproduce their
  reported intents make no identity claim about the originals.
- **Models.** E1–E13 are standalone models (sources absent) and MODEL rows are not VM results.

## Finding-to-work map

Each row defines a finding ID, the evidence gathered for it and the scope of what was repaired.
Model, library and source-review evidence is kept distinct from real VM runs. Problems without a
current plan are listed under [Not yet planned](../plans/README.md#not-yet-planned).

| ID | Finding | Evidence | Scoped result |
|---|---|---|---|
| R01 | Oversized heap slice, shared-derived writable address, safe out-of-bounds formation | Baseline p10/p13/p14/p16/p20 each miri-ub at the oversized slice; after the M1a repair all five pass in both configurations; p11 is a passing control. Three LIB-M recreations also pass. | Heap access repaired for the tested paths. Stack and static targets are not covered. |
| R02 | Assigning to a local invalidates an earlier byref; evaluation-stack growth invalidates an address held as a native integer | p01/p09/p37/p38 controls with p02/p03; LIB-M absent; models E12b/E13 distinct | Repaired for locals and arguments by [frame homes](../GC_AND_MEMORY_SAFETY.md#frame-owned-homes); byrefs to homes resolve through the frame check (`FrameHome`). Constructor receivers also use caller-owned temporary homes. Typedref address-taking is rejected under the chosen strict ECMA policy; the Stack origin and fixup have been removed. Fixed: every control in the catalog passes natively and under Miri in both configurations. See [Plan 02](../plans/02-stable-frame-homes.md#completion-validation) for validation limits and reproduction. |
| R03 | Transient owner destroyed while in use; expired static/arena guards | Pre-repair native liveness/clone failures and Miri dangling accesses in both configurations; repaired tests pass. Controlled cross-arena teardown gave Miri UAF before repair, rejection and in-flight lease passes after (multithreading only). p36 refutes the static allocation-lifetime hypothesis for its direct one-byte case; p12 panicked on alignment, not UB. | Owner/guard retention repaired for the tested paths. Fill reachability is source-inferred. Open: p12, tracked as "Static field addresses" and "Instance field alignment" in [Not yet planned](../plans/README.md#not-yet-planned). |
| R04 | Unmanaged serde exposure; integer GC transport | Historical LIB-M sources absent; p05 VM-M control refutes unconditional failure only. Retained-transport recreation and isolated strict-core experiment pass. | Retained GC transport implemented. Open: unmanaged pointer serialization is unchanged (p05 is not a general serde fix); tracked as "Unmanaged pointer serialization" in Not yet planned. |
| R05 | Repeated localloc; pointer arithmetic, null, wrapping and comparison | Step 0.1 measured p06/p08 Miri UB and p17/p18 assertion failures, p_null passed; Phase 6 controls p06–p08, p17/p18, p21–p24 and p_null passed in both configurations. M1c p04 and p29–p34 passed in both configurations. | Arithmetic, comparison and null paths repaired; stable frame-local `localloc` storage and its tested lifecycle paths repaired. |
| R06 | Pinned payload address; FFI return overwrite | Heap-origin P/Invoke address derivation uses the observer under existing pins/guards; no native-call return/copy regression added. Historical Rust/libffi sources missing. | No P/Invoke lifetime, ABI or return contract repaired. Open: "Native call boundary" in Not yet planned. |
| R07 | By-ref/out copy, temporary growth, null and string ABI | Existing P/Invoke crate tests pass in both configurations; no end-to-end native ABI evidence. | Not repaired. |
| R08 | Cross-arena root barrier and teardown; arena/address packing | Library model, ID40000 codec, registry/library ABA, real-VM collection and teardown recreations; see [R08 tests](#r08-tests). | Fixed: root barrier, retained transport, identity/token checks and executor teardown ownership. The real-VM collection tests transfer production-encoded field storage and decode through receiver-owned instance fields. The 65,535 arena-ID cap of the compact codec remains ("Retired-worker arena-ID capacity" in Not yet planned). |
| R09 | Host races and alignment, mixed-size overlap, static atomics | MODEL E9/E10 reports; VM paths R/A only | Open: "Shared raw-memory access" and "Safe byte writes into reference slots" in Not yet planned. |
| R10 | Block-operation barriers, count width, yield state | p14 `initblk` and p16 `cpblk` pass in both configurations for short valid scalar ranges; p15 checked frame-home `initblk` passes in both configurations; `span_static_storage_bulk_42` covers static targets | Partly fixed: `RetainedPtr::checked_range` resolves and bounds-checks heap, cross-arena, static, transient and frame-home targets before block operations; `SpanHelpers.Memmove`, `ClearWithoutReferences` and byte `Fill` are resumable. Open: GC write barriers for reference slots and the count width ("Resumable bulk operations and barriers" and "Safe byte writes into reference slots" in Not yet planned). |
| R11 | Negative index, `ByteOffset` sign, `conv.i` and native-int stores | Phase 6 controls p24–p28 and p_null pass in both configurations; p19 verifies the managed negative-index exception; `pointer_stobj_writes_native_word` and the `pointer_slot_writes_42` fixture cover native-word pointer slots; `p_wide_store_into_int_local` remains unverified. | `ByteOffset` direction, pointer conversion/store and native-integer round trips repaired for the asserted paths. Negative Int32 element loading is covered by p19 in native and pinned Miri configurations. Open: `p_wide_store_into_int_local` (Not yet planned). |
| R12 | `volatile.` ignored, Interlocked overflow, sync-block race, `refanytype` lifetime | R/A only; no integrated evidence | Open: "Shared raw-memory access" in Not yet planned. |
| R13 | Atomic backing aliasing, strict-provenance dependencies, blocking VM coverage | Historical LIB-M report unlocated. The two historically ignored atomic tests (`test_atomic_load_store`, `test_concurrent_atomic_load_store`): the Tree Borrows UB was test backing aliasing, and the Stacked Borrows ICE is a Miri mixed-size store-buffer bug. `provenance-lints` is a blocking deny gate over every exposed-provenance cast. | Atomic test pair repaired. Strict-provenance VM Miri: won't do. It is infeasible because dependencies (`bitvec` via dotnetdll, crossbeam) reconstruct pointers from integers; Miri runs Tree Borrows instead. |

### R08 tests

All are independent recreations; library tests are not VM runs.

- **Compact codec and IDs:** boundary, unavailable-owner, short-storage and managed-ID exhaustion
  tests. The multithreaded tag-5 codec supports registered owner IDs 0..=65535 and nonzero
  8-byte-aligned addresses below 2^48; the VM reserves ID 0 and allocates IDs 1..=65535 without
  recycling, failing loudly on exhaustion. The one-word `ObjectRef` keeps its unsafe
  live-original-allocation contract.
- **Executor teardown:** `executor::executor_teardown_tests::extracted_executor_tls_ownership`,
  `arena_lifetime_teardown_stress` and the normal/extracted/unwind active-lease witnesses failed
  before the ownership and drain-before-free repair ([plan 01](../plans/01-executor-teardown-ownership.md))
  and pass after it. They do not establish object rooting or all concurrent schedules.
- **Real-VM collection:** `executor::executor_collection_tests::arena_lifetime_vm_collection` and
  its controls ([plan 06](../plans/06-cross-arena-collection-fixture.md)) use production
  coordinator commands and the production field-storage codec for both cross edges.
- **Registry:** duplicate, fast-bit, exhaustion, reuse and identity/token tests; stale tokens and
  identities cannot acquire or unregister a successor.
- **Transport:** `gc::opaque_transport_tests::arena_lifetime_opaque_transport` and
  `object::tests::arena_lifetime_opaque_gc_transport`; `CrossArenaRef` carries the retained pointer
  with its exact identity, and installation checks the destination identity under a lease.
- **Origins:** `pointer::tests::arena_identity_in_memory_origin_aba`,
  `access::tests::arena_identity_memory_owner_aba` and `call::tests::arena_identity_native_escape_preflight`.
  Cross-arena typedrefs and non-ref/out managed-pointer native escapes are rejected by P/Invoke
  before marshalling; by-value byref-like aggregates are rejected in both configurations.

Strict-provenance selectors for the VM were never recovered or run; see R13.
