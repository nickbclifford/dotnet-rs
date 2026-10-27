# Pointer / provenance review evidence and current status

The pointer/provenance plan is **In progress — partial M0 (VM probe harness and blocking controls)**.
This repository now contains an owning VM probe module, a guarded diagnostic/control runner, a 13-row
evidence inventory, and a blocking filtered Miri controls job. It does **not** contain runtime repairs,
model integration, or a complete M0 result. The six enabled controls have twelve recorded exact-name
passes (six probes in each of no-features and multithreading configurations); this is permissive Tree
Borrows evidence for a narrow control corpus, not a full VM Miri suite, strict-provenance result, or
proof of runtime soundness. See the [unified plan](../plans/01-pointer-provenance.md) and
[durable inventory](INVENTORY.md).

## Historical evidence availability

The original pass-1, pass-2 and pass-3 archive directories, their raw Miri/native logs, standalone lab
and FFI source trees, and experimental patch sources are **absent from this checkout**. The external
backup location is unknown. The plan and inventory preserve selected claims and their limits; they are
not substitutes for missing source files or transcripts. Recover those artifacts if possible or create
separately identified reproductions. Do not infer a result from
a review summary. In particular, the integrated p20 probe was sourced from a tree with runtime patches;
its unpatched-baseline outcome remains unknown.

## Current executable records

- [`INVENTORY.md`](INVENTORY.md) records R01–R13, all 23 exact probe names, source hashes and variants,
  control outcomes, evidence types, strict exclusions and unresolved tasks.
- [`pointer_provenance_tests.rs`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) is the
  durable owning test source; [`pointer_provenance.sh`](../../scripts/pointer_provenance.sh) owns
  catalog discovery, guarded control shards and bounded diagnostics.
- `miri-pointer-controls` in [the blocking CI workflow](../../.github/workflows/ci.yml) runs
  no-features and multithreading configurations, each split into three explicit paired shards, using
  nightly-2026-05-27 and `-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks`.
  The six enabled tests are the controls; the other 17 remain ignored and diagnostic-only. The
  45-minute timeout is provisional and has not been tuned from hosted cold-build measurements.
- Only `p17`, `p18`, and `p19` are permitted for native diagnostics. Never run known-UB probes
  natively, and never run ignored tests outside the diagnostic entrypoint.

## Outstanding M0 obligations and follow-ups

None of the following is completed by the harness/control lane:

- Recover/reproduce and integrate pass-1 library probes and strict-core evidence, standalone E1–E13
  models and `gc_model::e11*`; recover the missing address and libffi fixtures; investigate p20 on an
  unpatched baseline. Preserve the distinction between library, model, native and real-VM evidence.
- Identify the two historically reported ignored atomic tests, distinguish backing-storage aliasing
  from historical Miri ICE reasons, then repair and validate them (M0/M6). Their identities remain
  unknown.
- Inventory and migrate the **reported** 73 provenance-lint hits before introducing
  `fuzzy_provenance_casts` / `lossy_provenance_casts` deny gates; the count is not a current audit.
- Recover excluded addressing/field performance probes as needed and capture native
  memory/unsafe_buffer/span/dispatch and field/byref/native-int baselines under consistent profiles
  and noise controls.
- Diagnose missing dependency first-failure traces; recover/re-establish strict-core evidence and
  exact selectors. Whole-VM strict Miri is not a target of this slice. Any broader unrelated VM Miri
  coverage requires a separate scoped follow-up. Observe hosted timings before changing the timeout.
- Complete the pointer/runtime/storage/GC/ABI/concurrency repairs and acceptance exits in M1–M6.
  These remain future engineering work; this harness makes no runtime-soundness claim.
