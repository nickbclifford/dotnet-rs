# Plan 02 — Falsifier portfolio

**Gate:** a `loom` leg exercising the stop-the-world handshake is blocking in
`ci.yml`; all four fuzz targets are blocking; Kani harnesses exist for the
F3/F4/F9 value-level facts; and a guard-off leg proves the feature-gated
validation hooks are not silently load-bearing.

**Status:** in progress — instruments 1–2 complete (2026-09-18), instruments 3–5 pending.

## Premise

For the three families the terminated proof study called the keystone — F1 rooting
and stop-the-world liveness, F7 immutable-field publication, F8 lock order and
safepoint discipline — no verification tool establishes the premise on real
code. The AWS/Rust Foundation `verify-rust-std` campaign ran 16 months with
450+ pull requests from 21+ contributors across four institutions and closed
neither of its two concurrency challenges. The corresponding rely-guarantee
theorem was costed in the study at one to three months and would still rest on
a hand-written model of the protocol rather than on the protocol.

Systematic interleaving exploration reaches the same invariants, on the actual
code, this week. It proves nothing and finds bugs — which at this project's
scale is the better trade.

## Current state (Instrument 2 completed 2026-09-18)

- **Miri**: `miri-value` is blocking in `ci.yml`. `miri.yml` runs a matrix with
  `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`;
  its multithreading leg runs `--no-default-features --features multithreading
  -- --test-threads=1` over four named test groups. Interleavings explored are
  whatever those tests happen to produce — nothing systematic.
- **Fuzzing**: all four targets — `fuzz_managed_ptr_roundtrip`,
  `fuzz_managed_ptr_offset`, `fuzz_raw_memory_access` (in `dotnet-value`) and
  `fuzz_executor` (in `dotnet-vm`) — replay committed, nonempty corpora in a
  blocking pinned-nightly `ci.yml` matrix with `-runs=0`. Exploratory
  duration-based fuzzing remains advisory in `fuzz.yml`.
- **Differential**: 7 `diff_test!` fixtures in
  `crates/dotnet-cli/tests/integration_tests_impl/diff_harness.rs`, against 475
  C# fixtures in the tree.
- **`loom` facade**: `cfg(loom)` selects the private `loom_compat` adapter in
  `crates/dotnet-utils/src/sync.rs`, with loom lock, condition-variable, and
  atomic implementations while preserving the facade's public API shape.
- **`loom` model**: `crates/dotnet-vm/tests/loom_stw.rs` exhaustively models
  normal STW parking/resume, cross-arena lease teardown, and the two panic-guard
  paths. It bounds the model to one collector/model thread and two mutators
  (`max_threads = 3`) without preemption, permutation, or duration cutoffs.
- **`loom` CI**: an independent blocking `loom` job runs
  `RUSTFLAGS="--cfg loom" cargo nextest run -p dotnet-vm --test loom_stw --no-default-features`.
  The model faithfully mirrors production sequencing; it does not directly run
  the feature-gated production threaded implementation.
- **Kani**: not present.
- **The `loom` seam now exists.** `crates/dotnet-utils/src/sync.rs` has a
  `loom_compat` arm alongside mutually exclusive `parking_lot` and `compat`
  arms. Dynamic synchronization sites use the facade; the reviewed direct
  `std::sync` exemptions are constrained by a 70-token source-policy ceiling.

## Instrument 1 — `loom` leg (highest value)

`loom` runs a test repeatedly, permuting concurrent executions under the C11
memory model with partial-order reduction. Instrument 1 is complete; these
checks record the implemented, bounded falsifier rather than a proof of the
feature-gated production threaded implementation.

- [x] Add the `cfg(loom)` adapter arm to `dotnet-utils/src/sync.rs`, preserving
  the facade's `Mutex`, `RwLock`, condition-variable, mapped-guard, and atomic
  API shapes over loom primitives.
- [x] Route the scoped dynamic synchronization users through the facade and add
  `scripts/check_std_sync_ceiling.sh` so the reviewed direct `std::sync`
  exemptions cannot regress beyond the 70-token source-policy ceiling.
- [x] Add faithful STW and cross-arena lease models: two mutators plus one
  collector/model thread, directly asserting parked-mutator and live-lease
  teardown safety predicates. The model mirrors production sequencing because
  the feature-gated production threaded types are disabled in the required
  no-default-features loom configuration.
- [x] Model `ResumeOnPanic` and `CommandCompletionGuard` unwind paths and assert
  their resume and completion visibility after injected panics are caught.
- [x] Make the exhaustive model a blocking `ci.yml` job. The shared builder
  caps the population at three total threads but leaves preemption, permutation,
  and duration exploration uncapped; no `LOOM_MAX_PREEMPTIONS` cap is used.

Instruments 2–5 below are independent portfolio work and remain pending.

## Instrument 2 — promote the existing fuzz targets

Complete (2026-09-18). All four targets replay committed, nonempty corpora in
a blocking `ci.yml` matrix using cargo-fuzz 0.13.1 and
`nightly-2026-05-27`; each command uses `-runs=0`. The two managed-pointer
targets now construct their assertion subjects from live storage, assert origin
and derived-address preservation, and cover checksum-valid unsupported serde
subtags as `UnknownSubtag` errors. This is Plan 08 step 7 coverage only; it
does not add a strict-provenance Miri leg or reopen Plan 08's parked gate.

## Instrument 3 — Kani harnesses for the value-level facts

Kani is not a verifier for this codebase — no concurrency, no unwinding, no
Stacked/Tree Borrows, no provenance UB — but it is a total decision procedure
for small pure functions, which is exactly what the F3/F4/F9 premises reduce
to. Worthwhile targets, in order:

- `dotnet_utils::validate_alignment` and the `is_aligned`-style width dispatch
  in `dotnet-utils/src/lib.rs` (the `1/2/4/8/_` match) — prove the dispatch is
  exhaustive and correct for all widths, not just the tested ones.
- `bucket_range` in `dotnet-runtime-memory/src/heap.rs` — saturating arithmetic
  over the full `usize` range, where the shift-based bucketing is easy to get
  wrong at the boundaries.
- The width/dispatch consistency in `dotnet-intrinsics-threading/src/interlocked.rs`:
  each `InterlockedAtomicTypeDispatch` arm passes a literal width to
  `compare_exchange_atomic`, and nothing today checks the literal matches the
  arm. Kani can, per arm.

That last one is worth flagging: it is a *proof that a refactor is needed*
rather than a permanent fixture. Once the width is a type parameter
([plan 03](03-width-generic-atomics.md)), the harness becomes redundant, which
is the correct outcome.

## Instrument 4 — guard-off leg

The terminated proof study's most consequential concrete finding was that a
feature-gated no-op was cited as an alignment witness — `validate_alignment` is
a real check under `memory-validation` and `#[inline(always)]` nothing without
it. The general defect is: code whose correctness silently depends on a
validation feature that release builds disable.

Add a CI leg that runs the test suite with every validation feature **off** and
asserts it still passes. Any test that fails is either testing the guard (fine,
move it under the feature) or depending on the guard for correctness (a defect
of exactly the class Phase 0 fixed twice).

## Instrument 5 — differential fixture ratchet

7 of 475 fixtures are differentially compared against real .NET. The
`diff_test!` macro requires a fixture whose expected exit code is 42, so not all
475 are eligible — but the eligible set is far larger than 7. Add fixtures in
bulk, and add a counting ratchet in the shape of
`scripts/check_mt_cfg_ceiling.sh` with the comparison inverted: a **floor**, not
a ceiling, so the differential count cannot regress. Keep the existing rejected-
candidate comment block in `diff_harness.rs` as the record of known,
intentional divergences — that list is itself an assurance artifact and feeds
plan 04.

## Not in scope

- RustMC / GenMC. Right shape for F1/F7/F8, but mixed-size atomic accesses
  require the MIXER extension, unmerged at publication, and this VM's atomics
  are 1/2/4/8-byte accesses into shared object memory. Revisit if MIXER lands.
- Strict-provenance Miri legs — those are plan 08's gate, not this one's.
- Any attempt to make Kani cover the concurrent code. It will warn and compile
  threads sequentially, producing a green result that means nothing. If a Kani
  harness touches threading, that is a bug in the harness.

## Related

- [`docs/plans/README.md`](README.md)
- [`docs/CI.md`](../CI.md) — current gate inventory
- [`docs/FUZZING.md`](../FUZZING.md) — existing fuzz workflow and corpus policy
- [`docs/VALIDATION_FEATURES.md`](../VALIDATION_FEATURES.md) — the feature
  matrix instrument 4 tests against
- [`docs/ASSURANCE_BACKGROUND.md`](../ASSURANCE_BACKGROUND.md) — why a
  falsifier beats a proof for F1/F7/F8
- Archived study on the STW protocol theorem this replaces:
  [`08-risks.tex`](https://github.com/nickbclifford/dotnet-rs/blob/b5a5f65d67345b0682def83867b816ea86fa3152/docs/proof-dsl-feasibility/sections/08-risks.tex) §8.4
