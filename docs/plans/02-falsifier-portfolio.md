# Plan 02 — Falsifier portfolio

**Gate:** a `loom` leg exercising the stop-the-world handshake is blocking in
`ci.yml`; all four fuzz targets are blocking; Kani harnesses exist for the
F3/F4/F9 value-level facts; and a guard-off leg proves the feature-gated
validation hooks are not silently load-bearing.

**Status:** not started.

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

## Current state (measured at `6ebda249`)

- **Miri**: `miri-value` is blocking in `ci.yml`. `miri.yml` runs a matrix with
  `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`;
  its multithreading leg runs `--no-default-features --features multithreading
  -- --test-threads=1` over four named test groups. Interleavings explored are
  whatever those tests happen to produce — nothing systematic.
- **Fuzzing**: four targets — `fuzz_managed_ptr_roundtrip`,
  `fuzz_managed_ptr_offset`, `fuzz_raw_memory_access` (in `dotnet-value`) and
  `fuzz_executor` (in `dotnet-vm`). Only `fuzz_raw_memory_access` is blocking;
  the other three are `continue-on-error`.
- **Differential**: 7 `diff_test!` fixtures in
  `crates/dotnet-cli/tests/integration_tests_impl/diff_harness.rs`, against 475
  C# fixtures in the tree.
- **`loom` / `shuttle`**: not present anywhere in the workspace.
- **Kani**: not present.
- **The `loom` seam already exists.** `crates/dotnet-utils/src/sync.rs` has a
  `compat` module providing `Mutex`/`RwLock` shims selected by
  `#[cfg(not(feature = "multithreading"))]`. A third arm is the natural
  insertion point. 39 files across the workspace reference `std::sync`.

## Instrument 1 — `loom` leg (highest value)

`loom` runs a test repeatedly, permuting concurrent executions under the C11
memory model with partial-order reduction. It requires substituting
`loom::sync::*` and `loom::thread` for the std types, which is exactly what the
existing `compat` module is shaped for.

1. Add a `loom` cfg arm to `dotnet-utils/src/sync.rs`'s `compat` module,
   re-exporting `loom::sync::{Mutex, RwLock, Condvar}` and `loom::sync::atomic`.
2. Route the 39 `std::sync` users through `compat`. This is a mechanical,
   bounded, supervised-refactor-shaped change; it also has standalone value as
   an abstraction cleanup. Add a `check_doc_drift`-style ceiling on direct
   `std::sync` imports so the routing cannot regress.
3. Write the first model as the smallest thing that can be wrong: two mutator
   threads plus one collector, exercising `GCCoordinator::begin_collection`, the
   thread-manager handshake, and `unregister_arena`'s wait for
   `active_leases == 0`. Assert the F1 predicate directly — no mutator is
   running while a collection session is active, and no arena is torn down with
   a live lease.
4. Add `ResumeOnPanic` and `CommandCompletionGuard` unwind paths, since `loom`
   explores those too and Kani cannot (no unwinding support).
5. Make the leg blocking in `ci.yml` once green. `loom` runs are slow; scope the
   first leg to 2–3 threads, which is where the study said specification errors
   surface cheaply, and cap with `LOOM_MAX_PREEMPTIONS`.

If exhaustive exploration proves too slow, `shuttle` (randomized, unsound but
scalable) is the fallback for the larger configurations, with `loom` retained
for the small ones. Do not replace `loom` with `shuttle` — losing exhaustiveness
at 2–3 threads loses the whole point.

## Instrument 2 — promote the existing fuzz targets

Three of four targets are `continue-on-error`, and two of them
(`fuzz_managed_ptr_roundtrip`, `fuzz_managed_ptr_offset`) test precisely the
code plan 08 rewrites. Promote all four to blocking. If a target is too flaky
or slow to block, that is a finding about the target, to be fixed or documented,
not a reason to leave it advisory indefinitely. Extend the two managed-pointer
targets with the provenance-preservation assertions from plan 08, step 7.

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
