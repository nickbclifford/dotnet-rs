# Pointer / provenance review evidence and current status

The pointer/provenance plan remains **In progress — scoped M1a heap-access containment, M1b pointer semantics, M1c stable frame-local `localloc` storage, and M1d owner/guard work complete; partial M0 (VM probe harness and controls)**. These scoped implementations and evidence do not complete M0, the broader M1 exits, or M3. See the [unified plan](../plans/01-pointer-provenance.md) and the [durable evidence inventory](INVENTORY.md) for the baseline table, probe catalog, R-row task status, and detailed results.

## M1a outcomes

The pre-repair Miri baseline covered p10, p13, p14, p16 and p20 in no-features and multithreading. All ten separate runs were classified **miri-ub** (exit 1): Miri reported a dangling, out-of-allocation `&[u8]` at `crates/dotnet-value/src/pointer/mod.rs:505:37`; the caller was `conversions.rs:101:21` for p10/p13/p20 and `stack_value.rs:574:30` for p14/p16. No-features wall times were 268/233/236/241/245 seconds, respectively; multithreading times were 243/277/258/259/244 seconds. The source commits were `775886fc7554f9d5b58014f2b3db30c2e17d1982` (no-features) and `9524740e01953416a5d68eeee377eb18c9f51851` (multithreading; intervening changes were tracking-only). The full per-probe table is in [INVENTORY.md](INVENTORY.md#pre-repair-baseline-steps-112).

After the heap repairs, all five probes passed their individual diagnostics in both configurations under pinned permissive Tree Borrows. They were promoted as blocking controls without changing their VM probe bodies, IL or assertions (p10=7, p13=1, p14/p16=`0x01010101`, p20=7). The p13 test retains a `memory-validation`-conditional ignore because the deliberate 4096-byte managed offset is rejected by that debug trap; the Miri control lane does not enable memory-validation. The separate far-OOB `conv.u8` regression is conditionally ignored under the same feature for the same trap; it passed pinned Miri in both configurations without that feature.

Three library regressions in `crates/dotnet-value/src/pointer/tests.rs` passed individually under the same pinned lane in both feature configurations, each with 1 passed/0 failed/0 ignored:

- `pointer::tests::m1a_one_byte_heap_read`: reads exactly one byte (`0xA5`) from a one-byte heap payload.
- `pointer::tests::m1a_safe_oob_address_observation`: checks the wrapping address at logical offset 4096 through both `PointerLike::pointer()` and `StackValue::ManagedPtr::as_ptr()` without dereferencing.
- `pointer::tests::m1a_heap_pointer_survives_typed_write`: for array, object and boxed payloads, checks that a retained heap base survives a guarded typed write of 5, a raw write of 7, and a guarded readback of 7.

The original pass-1 library sources and pass archives remain unavailable. The first two tests independently recreate the reported intents and make **no identity claim** about the missing originals; the typed-write test is a new regression.

At the M1a stage, the pointer runner had **11 exact controls in 6 shards** across no-features and multithreading. `verify-controls` discovered 11 exact controls in both configurations. At M1b completion, the lane had 24 controls in 12 shards and the 31-entry catalog had 7 diagnostic-only probes. The M1a-era lane measurements and Phase 6 additions passed; the inventory identifies which shard outcomes came from each run and records measured walls. `miri-value` CI includes the three M1a library selectors in both its default multithreading run and a separate no-features step. See the [M1b outcomes](#m1b-outcomes) and [inventory](INVENTORY.md#m1b-phase-6-control-outcome-comparison-2026-10-29).

## M1b outcomes

At M1b completion, the catalog contained **31 entries: 24 controls and 7 diagnostic-only probes**. Those 24 controls were grouped as follows; all 12 shards had documented passing outcomes in both no-features and multithreading configurations (shards 1–5 use the earlier final-lane measurements; shards 6–12 use Phase 6 results). At M1c completion the catalog had 37 entries (31 controls, 6 diagnostic-only probes) across 16 shards; see [M1c outcomes](#m1c-outcomes). The current M1d catalog is described below.

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

The eight `dotnet-value` `pointer::tests::m1b_` regressions passed natively and under pinned permissive-TB Miri in both configurations (8 passed, 0 failed, 0 ignored per run). With `memory-validation`, 7 passed and 1 was intentionally ignored by the F-TEST-003 guard.

Shard 12's fresh-pointer `stind.i` controls were run using `bash scripts/pointer_provenance.sh controls --configuration CFG --shard 12`: p27 passed in 296 s (no-features) and 348 s (multithreading); p28 passed in 302 s and 321 s, respectively. Each configuration reported 2 passed, 0 failed. These are per-probe times in the filtered permissive Tree Borrows lane, not native timings. The lane is **not strict-provenance evidence**, **not a full VM Miri suite**, and **not a soundness proof**; the inventory records pre-repair outcomes and per-shard evidence details.

## M1c outcomes

M1c replaces the growing frame `Vec<u8>` used by IL `localloc` with frame-owned, non-resizing raw allocation blocks. p04's pre-repair guarded Miri baseline was `miri-ub` in both configurations, first reported at `crates/dotnet-runtime-memory/src/access.rs:1172:25` (239 s no-features, 224 s multithreading); after the repair it passed at 255 s and 240 s. p05 remains the unchanged single-allocation control and passed in shard 2 (235 s and 231 s). The new p29–p34 growth, nested-call and release probes passed diagnostic Miri in both configurations; p31's reported timings are for its final source after the observer assertions were tightened:

| Probe | no-features VM-M wall (s) | multithreading VM-M wall (s) |
|---|---:|---:|
| p29 stable blocks and alignment | 341 | 330 |
| p30 nested callee allocation | 329 | 292 |
| p31 return and pooling cleanup (final source) | 365 | 315 |
| p32 tail-call replacement cleanup | 387 | 314 |
| p33 exception-unwind cleanup | 357 | 314 |
| p34 executor-teardown cleanup | 283 | 248 |

At M1c completion the promoted catalog had **37 entries: 31 blocking controls and 6 diagnostic-only probes** in 16 shards; `verify-controls` discovered all 31 in both configurations. p04 and p29–p34 are in blocking shards 13–16. Under pinned permissive-Tree-Borrows Miri, `raw_block::` passed 7 tests in each configuration (1.413 s no-features; 1.475 s multithreading), and the initial `frame_alloc::` run passed 7 in each (0.958 s; 0.969 s), with those walls measured for test-plus-log/count pipelines. The finalizer's expanded `frame_alloc::` suite passed 8 in each configuration, including data-structure-level suspension/restoration. Detailed rows and measured outcomes are in the [M1c inventory](INVENTORY.md#m1c-baseline-and-outcomes).

M1c did **not** close the p09 stack-home lifetime or p12 static-address/storage obligations, owner/guard escape work, or final pointer representation/codecs. M1d subsequently completed its scoped owner/guard work; the remaining stack/static and codec obligations are M3. The possible nested-filter suspended-frame overwrite noted as F-LIFE-004 was identified by inspection, not reproduced or fixed; p33 observed no suspension in an ordinary catch. These filtered permissive-TB passes are not strict-provenance evidence, not a full VM Miri suite, and not a soundness proof.

## M1d outcomes

Independent pre-repair recreations (not recovered pass-1 tests) showed native Transient owner-liveness and clone-identity failures and pinned-Miri dangling accesses after normalization, reconstruction and stack pop in both configurations. After repair those library regressions passed natively and under pinned Miri. Transient origins share an `Arc`-owned object across clones, normalization and `ManagedPtr::from_info_full`; access-performing pops retain their `StackValue` via `RetainedPtr`, while observation-only pops return addresses. Fallible stores, native-integer conversion and CLI-to-CTS numeric narrowing reject unsupported Transient ownership escape. Resolution borrows the origin; an address alone retains no owner or access guard.

A controlled multithreading cross-arena teardown produced a pre-repair Miri use-after-free; post-repair Miri tests passed for rejection after unregister and an in-flight closure protected by a lease. `RetainedPtr` holds a cross-arena lease for synchronous block operations. The byte Fill fast path was repaired after a controlled reproduction of its observation-to-access gap, but end-to-end reachability remains unconfirmed. A one-byte direct-static p36 access after forced collection passed in both configurations, refuting that particular allocation-lifetime hypothesis, not proving the nested resolver or synchronization safe. p12 instead hit a four-byte alignment `debug_assert` in both configurations: not Miri UB and not a teardown proof. p35 passed post-repair in both configurations; its pre-repair exception type-resolution failure was not a Miri UB report, and its snapshot read does not prove stack-home write-through.

The current catalog has **39 entries: 33 blocking controls and 6 diagnostic-only probes** in 17 shards. All 33 controls passed in both configurations in the final pinned-Miri sweep; p35 joins p34 in shard 16 and p36 is in shard 17. Library M1d selectors and cross-arena tests also passed in their applicable configurations. See the [M1d inventory](INVENTORY.md#m1d-baseline-and-outcomes) for commands, counts, times and evidence limits. These are filtered permissive Tree Borrows results, not strict-provenance evidence or a soundness proof.

## Repaired heap paths and remaining boundaries

M1a repaired heap `ManagedPtr::with_data` to validate checked ranges against actual payload extent and create only the requested slice; separated heap address observation from access using retained allocation bases and wrapping formation; routed heap `StackValue::as_ptr` and VM `resolve_address` through that observer; fixed both distinct integer conversions, `conv.u` and `conv.u8`; fixed heap-origin `AsPointer`/Unsafe and buffer address helpers; and kept retained-base derivation independent of a payload-slice reborrow so pointers survive ordinary typed writes.

Remaining obligations, not fixed by the scoped M1d work, include:

- **M3:** p09 stack-home relocation, Stack/Static observation and stack fallback, final pointer codecs and struct layout, and static `GuestBlock` alignment (p12); p36 addresses only the tested static allocation lifetime.
- **M1e/M1g:** GC transport, root publication and arena-ID/generation identity. A lease does not itself establish a GC root, range, synchronization or an origin's generation.
- **M4:** a shared-memory/atomic protocol, including overlapping and static/raw accesses.
- **M2:** P/Invoke pin/guard lifetime, cross-arena access, ABI and return/copy semantics.
- **M1f/M5:** block-operation count width, complete ranges, barriers/rooting and `cpblk`/`initblk`/`Memmove` yield/resumption; negative-index exception behavior also remains open.
- **F-LIFE-004:** the source-inferred nested-filter suspended-frame overwrite, neither reproduced nor fixed.
- **M0/M6:** strict-provenance evidence; no strict lane was run for M1d.

The writable contract for interned Arc-backed strings, M3 signed/wider managed offsets, and remaining narrow/overflow-checking `conv.*` pointer cases remain open; negative or beyond-compact managed offsets still return an interim `NotImplemented` error. The repeated M1a no-features performance regressions remain an open PERF item. See the [R-row map](INVENTORY.md#finding-to-work-map) for scoped results and follow-ups.

## Historical evidence and limits

The original pass-1/2/3 archives and raw pass logs, standalone lab/FFI sources, and the 3c2 patch are **still absent** from this checkout; an external backup has not been ruled out. In particular, p20's historical patched source is not evidence of an original pass. The unpatched-tree p20 baseline above was independently observed in steps 1.1/1.2.

All pointer-lane Miri results here are from a **filtered permissive Tree Borrows lane**. This is not strict-provenance evidence, not a full VM Miri suite, and not a proof of soundness. Native benchmark numbers are performance measurements, not Miri evidence. The step 5.1 benchmark review recorded native Criterion comparisons and repeated no-features regressions; those regressions remain an open performance follow-up, not a Miri result.

## Current executable records and safe operation

- [`INVENTORY.md`](INVENTORY.md) records the outcomes and limits, R01–R13 task updates, all 39 catalog entries (33 controls and 6 diagnostic-only), refreshed source line anchors, control results and still-open work.
- [`pointer_provenance_tests.rs`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) is the owning VM test source; [`pointer_provenance.sh`](../../scripts/pointer_provenance.sh) owns catalog discovery, guarded control shards and bounded diagnostics.
- Only p19 is permitted for native diagnostics. Never run known-UB probes natively or ignored tests outside the runner's diagnostic entrypoint.
- The source test anchors in the inventory are manually refreshed; they are not machine-checked.
