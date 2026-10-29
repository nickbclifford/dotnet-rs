# Pointer / provenance review evidence and current status

The pointer/provenance plan remains **In progress — scoped M1b pointer semantics complete; partial M0 (VM probe harness and controls)**. The M1a heap-access containment implementation and scoped M1b pointer-semantics evidence are recorded, but this does not complete M0, the broader M1 exits, or M3. See the [unified plan](../plans/01-pointer-provenance.md) and the [durable evidence inventory](INVENTORY.md) for the baseline table, probe catalog, R-row task status, and detailed results.

## M1a outcomes

The pre-repair Miri baseline covered p10, p13, p14, p16 and p20 in no-features and multithreading. All ten separate runs were classified **miri-ub** (exit 1): Miri reported a dangling, out-of-allocation `&[u8]` at `crates/dotnet-value/src/pointer/mod.rs:505:37`; the caller was `conversions.rs:101:21` for p10/p13/p20 and `stack_value.rs:574:30` for p14/p16. No-features wall times were 268/233/236/241/245 seconds, respectively; multithreading times were 243/277/258/259/244 seconds. The source commits were `775886fc7554f9d5b58014f2b3db30c2e17d1982` (no-features) and `9524740e01953416a5d68eeee377eb18c9f51851` (multithreading; intervening changes were tracking-only). The full per-probe table is in [INVENTORY.md](INVENTORY.md#pre-repair-baseline-steps-112).

After the heap repairs, all five probes passed their individual diagnostics in both configurations under pinned permissive Tree Borrows. They were promoted as blocking controls without changing their VM probe bodies, IL or assertions (p10=7, p13=1, p14/p16=`0x01010101`, p20=7). The p13 test retains a `memory-validation`-conditional ignore because the deliberate 4096-byte managed offset is rejected by that debug trap; the Miri control lane does not enable memory-validation. The separate far-OOB `conv.u8` regression is conditionally ignored under the same feature for the same trap; it passed pinned Miri in both configurations without that feature.

Three library regressions in `crates/dotnet-value/src/pointer/tests.rs` passed individually under the same pinned lane in both feature configurations, each with 1 passed/0 failed/0 ignored:

- `pointer::tests::m1a_one_byte_heap_read`: reads exactly one byte (`0xA5`) from a one-byte heap payload.
- `pointer::tests::m1a_safe_oob_address_observation`: checks the wrapping address at logical offset 4096 through both `PointerLike::pointer()` and `StackValue::ManagedPtr::as_ptr()` without dereferencing.
- `pointer::tests::m1a_heap_pointer_survives_typed_write`: for array, object and boxed payloads, checks that a retained heap base survives a guarded typed write of 5, a raw write of 7, and a guarded readback of 7.

The original pass-1 library sources and pass archives remain unavailable. The first two tests independently recreate the reported intents and make **no identity claim** about the missing originals; the typed-write test is a new regression.

The current pointer runner has **24 exact controls in 12 shards** across no-features and multithreading; its 31-entry catalog also has 7 diagnostic-only probes. `verify-controls` discovered 24 exact controls in both configurations. The M1a-era lane measurements and Phase 6 additions passed; the inventory identifies which shard outcomes came from each run and records measured walls. `miri-value` CI includes the three M1a library selectors in both its default multithreading run and a separate no-features step. See the [M1b outcomes](#m1b-outcomes) and [inventory](INVENTORY.md#m1b-phase-6-control-outcome-comparison-2026-10-29).

## M1b outcomes

The current catalog contains **31 entries: 24 controls and 7 diagnostic-only probes**. Its 24 controls are grouped as follows; all 12 shards have documented passing outcomes in both no-features and multithreading configurations (shards 1–5 use the earlier final-lane measurements; shards 6–12 use Phase 6 results).

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

## Repaired heap paths and remaining boundaries

M1a repaired heap `ManagedPtr::with_data` to validate checked ranges against actual payload extent and create only the requested slice; separated heap address observation from access using retained allocation bases and wrapping formation; routed heap `StackValue::as_ptr` and VM `resolve_address` through that observer; fixed both distinct integer conversions, `conv.u` and `conv.u8`; fixed heap-origin `AsPointer`/Unsafe and buffer address helpers; and kept retained-base derivation independent of a payload-slice reborrow so pointers survive ordinary typed writes.

The following remain out of scope: Stack/Static observation and the `with_data` stack fallback (M3); any writable contract for strings/interned Arc data; P/Invoke pinning, guards, ABI and return/copy semantics (M2); and general `cpblk`/`initblk` correctness (M1f/M5: count width, full checked ranges, barriers/rooting and yield state). The short valid p14/p16 block probes only validate the M1a address path. M1b's scoped arithmetic, comparison, `Unsafe.ByteOffset`, and pointer/native-integer store/load paths are covered as described below; remaining obligations include M1c stable `localloc` storage, M1d owner/guard escape, M1f/M5 block operations and negative-index exception behavior, M3 signed/wider managed offsets and final pointer-codec/struct layout, M4 shared-memory/atomic protocol, M2 native ABI, and strict-provenance evidence (M0/M6). Negative or beyond-compact managed offsets still return an interim `NotImplemented` error; remaining narrow/overflow-checking `conv.*` pointer cases are unchanged. The repeated M1a no-features performance regressions remain an open PERF item. R01's M1a heap reproduction is closed; broader R01/M3 and R06/R07/R10 follow-ups remain as detailed in the [R-row map](INVENTORY.md#finding-to-work-map).

## Historical evidence and limits

The original pass-1/2/3 archives and raw pass logs, standalone lab/FFI sources, and the 3c2 patch are **still absent** from this checkout; an external backup has not been ruled out. In particular, p20's historical patched source is not evidence of an original pass. The unpatched-tree p20 baseline above was independently observed in steps 1.1/1.2.

All pointer-lane Miri results here are from a **filtered permissive Tree Borrows lane**. This is not strict-provenance evidence, not a full VM Miri suite, and not a proof of soundness. Native benchmark numbers are performance measurements, not Miri evidence. The step 5.1 benchmark review recorded native Criterion comparisons and repeated no-features regressions; those regressions remain an open performance follow-up, not a Miri result.

## Current executable records and safe operation

- [`INVENTORY.md`](INVENTORY.md) records the outcomes and limits, R01–R13 task updates, all 31 catalog entries (24 controls and 7 diagnostic-only), source variants, current line anchors, control results and still-open work.
- [`pointer_provenance_tests.rs`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) is the owning VM test source; [`pointer_provenance.sh`](../../scripts/pointer_provenance.sh) owns catalog discovery, guarded control shards and bounded diagnostics.
- Only p19 is permitted for native diagnostics. Never run known-UB probes natively or ignored tests outside the runner's diagnostic entrypoint.
- The source test anchors in the inventory are manually refreshed; they are not machine-checked.
