# Pointer / provenance review evidence

Evidence record. Not instructions; current work is in [docs/plans/](../plans/README.md).

This folder records measured outcomes of the pointer/provenance review and its repairs.
[INVENTORY.md](INVENTORY.md) defines the probe IDs (p00…), finding IDs (R01–R13) and evidence
vocabulary, and holds the detailed tables. The section names below use the milestone labels
(M1a–M1d) under which the evidence was gathered; they are labels for this record only. The
current runtime contracts are in [GC and Memory Safety](../GC_AND_MEMORY_SAFETY.md) and
[Architecture](../ARCHITECTURE.md#evaluation-stack-locals-and-managed-pointers).

Several original review archives (pass-1/2/3 sources, logs, lab/FFI sources and the 3c2 patch)
are absent from this repository. Tests written to reproduce their reported intents are
independent recreations, and make no identity claim about the originals.

All pointer-lane Miri results here come from a **filtered permissive Tree Borrows lane** with
leaks ignored. That is not strict-provenance evidence, a full VM Miri suite, leak checking or a
soundness proof. Native benchmark numbers are performance measurements, not Miri evidence.

## M1a: heap access

The pre-repair Miri baseline covered p10, p13, p14, p16 and p20 in no-features and multithreading. All ten separate runs were classified **miri-ub** (exit 1): Miri reported a dangling, out-of-allocation `&[u8]` in `ManagedPtr::with_data`; the caller was the `conv.u` handler for p10/p13/p20 and `StackValue::as_ptr` for p14/p16. No-features wall times were 268/233/236/241/245 seconds, respectively; multithreading times were 243/277/258/259/244 seconds. The source commits were `775886fc7554f9d5b58014f2b3db30c2e17d1982` (no-features) and `9524740e01953416a5d68eeee377eb18c9f51851` (multithreading; intervening changes were tracking-only). The full per-probe table is in [INVENTORY.md](INVENTORY.md#pre-repair-baseline-steps-1112).

After the heap repairs, all five probes passed their individual diagnostics in both configurations. They were promoted as blocking controls without changing their VM probe bodies, IL or assertions (p10=7, p13=1, p14/p16=`0x01010101`, p20=7). The p13 test is ignored under `memory-validation` because that feature's bounds trap rejects its deliberate 4096-byte managed offset; the Miri control lane does not enable memory-validation. The separate far-OOB `conv.u8` regression is ignored under the same feature for the same reason; it passed pinned Miri in both configurations without that feature.

Three library regressions in `crates/dotnet-value/src/pointer/tests.rs` passed individually in both feature configurations, each with 1 passed/0 failed/0 ignored:

- `pointer::tests::heap_access_one_byte_heap_read`: reads exactly one byte (`0xA5`) from a one-byte heap payload.
- `pointer::tests::heap_access_safe_oob_address_observation`: checks the wrapping address at logical offset 4096 through both `PointerLike::pointer()` and `StackValue::ManagedPtr::as_ptr()` without dereferencing.
- `pointer::tests::heap_access_heap_pointer_survives_typed_write`: for array, object and boxed payloads, checks that a retained heap base survives a guarded typed write of 5, a raw write of 7, and a guarded readback of 7.

The first two are independent recreations; the typed-write test is new.

The repairs: heap `ManagedPtr::with_data` validates checked ranges against actual payload extent and creates only the requested slice; heap address observation is separate from access, using retained allocation bases and wrapping formation; heap `StackValue::as_ptr` and VM `resolve_address` go through that observer; `conv.u` and `conv.u8`, heap-origin `AsPointer`/Unsafe and buffer address helpers were fixed; retained-base derivation does not depend on a payload-slice reborrow, so pointers survive ordinary typed writes.

## M1b: pointer semantics

At M1b completion the catalog contained **31 entries: 24 controls and 7 diagnostic-only probes**, in 12 shards with documented passing outcomes in both configurations (shards 1–5 from the earlier final-lane measurements; shards 6–12 from Phase 6 results).

| Shard | Controls |
|---:|---|
| 1 | p00, p02 |
| 2 | p03, p05 |
| 3 | p07, p11 |
| 4 | p10, p20 |
| 5 | p13, p14 |
| 6 | p16, `p_null_through_pointer_local_address` |
| 7 | p06, p08 |
| 8 | p17, p18 |
| 9 | p21, p22 |
| 10 | p23, p24 |
| 11 | p25, p26 |
| 12 | p27, p28 |

The eight `dotnet-value` `pointer::tests::ptr_arith_` regressions passed natively and under pinned Miri in both configurations (8 passed, 0 failed, 0 ignored per run). With `memory-validation`, 7 passed and 1 was ignored because that feature's bounds trap rejects its deliberate out-of-bounds offset.

Shard 12's fresh-pointer `stind.i` controls were run using `bash scripts/pointer_provenance.sh controls --configuration CFG --shard 12`: p27 passed in 296 s (no-features) and 348 s (multithreading); p28 passed in 302 s and 321 s, respectively. These are per-probe times in the filtered Miri lane, not native timings. Details are in the [inventory](INVENTORY.md#m1b-phase-6-control-outcome-comparison-2026-10-29).

## M1c: frame-local `localloc` storage

M1c replaced the growing frame `Vec<u8>` used by IL `localloc` with frame-owned, non-resizing raw allocation blocks. p04's pre-repair guarded Miri baseline was `miri-ub` in both configurations, first reported in `dotnet-runtime-memory/src/access.rs` (239 s no-features, 224 s multithreading); after the repair it passed at 255 s and 240 s. p05, the unchanged single-allocation control, passed in shard 2 (235 s and 231 s). The new p29–p34 growth, nested-call and release probes passed diagnostic Miri in both configurations; p31's timings are for its final source after the observer assertions were tightened:

| Probe | no-features VM-M wall (s) | multithreading VM-M wall (s) |
|---|---:|---:|
| p29 stable blocks and alignment | 341 | 330 |
| p30 nested callee allocation | 329 | 292 |
| p31 return and pooling cleanup (final source) | 365 | 315 |
| p32 tail-call replacement cleanup | 387 | 314 |
| p33 exception-unwind cleanup | 357 | 314 |
| p34 executor-teardown cleanup | 283 | 248 |

At M1c completion the promoted catalog had **37 entries: 31 blocking controls and 6 diagnostic-only probes** in 16 shards. `raw_block::` passed 7 tests in each configuration (1.413 s no-features; 1.475 s multithreading), and the initial `frame_alloc::` run passed 7 in each (0.958 s; 0.969 s). The expanded `frame_alloc::` suite passed 8 in each configuration, including data-structure-level suspension/restoration. p33 observed no frame suspension in an ordinary catch; the nested-filter suspension path was not exercised. Detailed rows are in the [M1c inventory](INVENTORY.md#m1c-baseline-and-outcomes).

## M1d: owner and guard retention

Independent pre-repair recreations showed native Transient owner-liveness and clone-identity failures and pinned-Miri dangling accesses after normalization, reconstruction and stack pop in both configurations. After repair those library regressions passed natively and under pinned Miri.

A controlled multithreading cross-arena teardown produced a pre-repair Miri use-after-free; post-repair Miri tests passed for rejection after unregister and for an in-flight closure protected by a lease. The byte Fill fast path was repaired after a controlled reproduction of its observation-to-access gap; end-to-end reachability is unconfirmed. A one-byte direct-static p36 access after forced collection passed in both configurations, refuting that particular allocation-lifetime hypothesis only. p12 instead hit a four-byte alignment `debug_assert` in both configurations: not Miri UB and not a teardown proof. p35 passed post-repair in both configurations; its pre-repair exception type-resolution failure was not a Miri UB report, and its snapshot read does not show stack-home write-through.

The catalog has **40 entries: 35 blocking controls and 5 diagnostic-only probes** (see the [inventory](INVENTORY.md#probe-catalog)). At M1d, all 33 controls of that time passed in both configurations in the paired-shard pinned-Miri sweep. The current lane layout is described in [CI](../CI.md#blocking-miri-pointer-controls-lane). See the [M1d inventory](INVENTORY.md#m1d-baseline-and-outcomes) for commands, counts and times.

## Executable records

- [`INVENTORY.md`](INVENTORY.md) records outcomes and limits, the finding map, all 39 catalog entries and control results.
- [`pointer_provenance_tests.rs`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) is the owning VM test source; [`pointer_provenance.sh`](../../scripts/pointer_provenance.sh) owns catalog discovery, guarded control shards and bounded diagnostics.
- Only p19 is permitted for native diagnostics. Never run known-UB probes natively or ignored tests outside the runner's diagnostic entrypoint.
