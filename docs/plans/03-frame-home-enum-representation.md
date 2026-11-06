# Repair enum and named scalar frame homes

**Status:** Complete

**Outcome:** Address-taken enum and named primitive locals and arguments retain
scalar storage semantics and declared identity across assignment and stack growth.

## Problem and implementation

The pre-repair runtime selected struct homes for enum arguments represented on the
evaluation stack as scalars. It rejected argument installation before executing a
constrained enum call, including EF Core's `CoreEventId.MakeUpdateId`.

Frame-home selection now resolves declared identity before normalization. Enums and
named primitives use resolver scalar layouts and retain separate declared type
metadata for address-taking. Ordinary structs retain field-based reconstruction;
reference, byref, TypedReference, and special pointer policies are unchanged.

Argument installation, `starg`, and `stloc` share scalar-home conversion. Matching
scalar or matching value-type inputs are accepted; value-type decoding checks type,
layout size, and actual borrowed buffer length before reading. Numeric stores retain
existing conversion/truncation and layout-driven reads retain extension behavior.
No public API or 24-byte managed-pointer encoding change is required.

## Specification and scope

- ECMA-335 I.12.1.2 requires signed/unsigned extension on narrow loads and truncation
  on stores and argument assignment.
- II.14.3 permits enum/underlying scalar interchange while preserving identity for
  binding and boxing. This repair's acceptance covers the eight C# integer underlying
  types; additional CLI enum underlying types are outside this repair.
- III.2.1 defines the managed receiver and boxing behavior of constrained calls;
  III.3.44 defines aligned managed local addresses.
- II.14.3 prohibits direct enum `initobj`. Generic `Default<T>` instantiated with an
  enum is tested against CLR as compatibility evidence, not a direct-enum initialization
  requirement.

## Regression and acceptance

`enum_frame_homes_42` has an automated CLR oracle covering constrained enum argument
calls, declared identity, saved references, generic defaults/copies, narrow signed and
unsigned values, 32/64-bit patterns, primitive substitutions, and a reference-containing
struct across collection.

p43 explicitly exercises initial scalar and enum `ValueType` argument copies,
`ldarga`/`starg`, `ldloca`/`stloc`, saved local and argument addresses across observed
nested evaluation-stack growth, stable home addresses/serials, native access, and
`ldobj`/`stobj`. Both invocation results are independently checked. Existing p37/p38
remain struct-home assignment controls. Scalar rejection tests cover wrong identity,
wrong layout size, and shorter/longer payloads without changing destination bytes.

## Completion validation

Completion commit: `2b239a5f`. Rust 1.95.0 and .NET SDK 10.0.114 were used.
`./check.sh --fast` passed clippy and all three suites: 632 tests / 7 skipped without
features, 744 / 5 with multithreading, and 759 / 7 with validation-all added.
The enum fixture's CLR oracle and scalar rejection tests pass in both configurations.
Pinned nightly-2026-05-27 Tree Borrows Miri passed p37, p38, p43 and both scalar rejection
tests in both configurations. Runner contracts passed 21 tests, batch verification
covered 44 controls once, and doc drift passed 49 checks. Exposed-provenance warnings
limit Miri detection; these results are not a complete pointer-safety proof.

Actual fixture IL confirms Name uses `ldarga` and `constrained. EventId`; Replace<T>
uses `ldarga`, `starg` and `ldobj !!T`; Identity<T> uses `constrained. !!T`; Default<T>
uses `ldloca` and `initobj !!T`. p43 constructs explicit object operations and asserts
both scalar and value-type call results independently.

Fresh isolated revision `c743e4d0` rejected the identical enum fixture with
`Expected ValueType for Struct write` (exit 1); CLR and the repaired runtime returned 42.
Historical EF test mode failed with the same argument-0 layout error (exit 101).
Repaired `ef_inmemory --test` passed with default and no-default features using copies
of the original baseline cache. All 14 fixture/dependency hashes matched before and
after execution; the complete manifests are retained in git at `2b239a5f`.
These smoke tests establish correctness, not performance.

Reproduce focused tests with `cargo test -p dotnet-cli --test integration_tests
fields_enum_frame_homes_42_matches_dotnet` and `cargo test -p dotnet-vm --lib
scalar_home_rejects`, adding `--no-default-features` for the second configuration.
For pinned Miri, use `bash scripts/pointer_provenance.sh controls --configuration CONFIG
--shard SHARD`: p37=35, p38=37, p43=43. For scalar Miri use
`MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"
cargo +nightly-2026-05-27 miri test -p dotnet-vm --lib scalar_home_rejects`, with and
without `--no-default-features`. EF smoke: build the `end_to_end` benchmark with
`cargo bench --profile bench-fat -p dotnet-benchmarks --bench end_to_end --no-run`,
then run its executable with `ef_inmemory --test`, preserving fixture bytes when comparing.

The fast gate also exposed a catch-entry defect through the enabled Substring delegate
regression: logical stack height was reset while backing operands remained. Catch entry
now truncates those operands before pushing the exception, as required by ECMA-335
I.12.4.2.5. The exception/retry CLR regression passes in all three fast configurations.

Plan02's completed benchmark matrix and accepted primitive cost satisfy the separate
performance dependency. EF test-mode smoke results here claim no performance timings.
