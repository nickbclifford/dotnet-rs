# Plan 03 completion follow-up

Plan 03 is **Complete**. Validation uses Rust 1.95.0, .NET SDK 10.0.114, and
nightly-2026-05-27 for Tree Borrows Miri. Repaired source is the working tree based
on `36c59e23`, including the changes described below.

## Changes

p43 saves local and argument addresses before reassignment and a nested call with
4096 evaluation-stack slots. The observer checks actual capacity growth and stable
addresses/serials for both homes in the same invocation. Native-address reads feed
explicit enum `stobj`/`ldobj`; both scalar and enum `ValueType` invocations must
independently produce 42. Scalar rejection coverage includes payload lengths
0, 1, 2, 3, 5 and 8 for a four-byte home, preserving destination bytes on failure.

The full gate exposed the enabled Substring delegate exception-and-retry regression.
Catch entry reset logical height without truncating backing evaluation operands.
Catch entry now truncates to the frame's evaluation base before pushing the exception.
ECMA-335 I.12.4.2.5 requires intermediate evaluation results to be lost and the
exception object to be the first item on catch entry; this was checked with ecma-search.
The existing managed fixture and CLR oracle pass with no features after this repair.
No Substring implementation change was retained.

## Requirement and instruction mapping

Inspected the actual fixture assembly with:
`ilspycmd -il target/debug/dotnet-fixtures/fields/enum_frame_homes_42/SingleFile.dll`.

| Requirement | Executable evidence |
| --- | --- |
| Scalar argument installation and enum value input | p43 calls EnumRoundtrip first with ldc.i4 10000, then an unhomed enum ldloc; both results are checked |
| Homed argument assignment | p43 ldarga/starg/ldarg; fixture Replace<T>: IL_0001 ldarga.s, IL_0005 starg.s, IL_000e ldobj !!T |
| Homed local assignment and saved pointer | p43 ldloca/stloc/ldloc, saved before nested growth and later accessed through conv.u/ldind.i4 |
| Explicit object operations | p43 stobj HomeEnum and ldobj HomeEnum; their value comes from the native read |
| Constrained call | fixture Name: IL_0001 ldarga.s, IL_0003 constrained. EventId, IL_0009 callvirt Object.ToString |
| Declared identity and generic substitution | fixture Identity<T>: constrained. !!T and Object.GetType, asserted against typeof(EventId) |
| Default initialization | fixture Default<T>: IL_0001 ldloca.s, IL_0003 initobj !!T; p43 initialized local home |
| Narrow signed/unsigned extension and wide bit patterns | fixture Replace<T> assertions for sbyte, byte, short, ushort, int, uint, long and ulong |
| Named primitives | fixture Replace<int> and Replace<double> |
| Struct copying and reference tracing | fixture Replace<ReferenceValue>, GC.Collect, ReferenceEquals and Number assertion; p37/p38 struct assignment controls |
| Nested growth | p43 helper maxstack 4096; observer asserts actual growth and unchanged local/argument home addresses and serials |

## Specification grounding

- ECMA-335 I.12.1.2: short loads extend according to signedness; stores and argument
  assignment truncate.
- II.14.3: unboxed enums interconvert with underlying types; binding and boxing retain
  distinct identity. Acceptance here covers the eight C# integer underlying types.
- III.2.1: constrained calls require a managed pointer to the constrained type.
- III.3.44: local addresses are aligned managed pointers.
- II.14.3 prohibits direct enum `initobj`. Generic Default<T> is retained as CLR
  compatibility coverage, not proof that direct `initobj Enum` is conforming.

## Fresh historical reproduction

Source revision: `c743e4d0c948c08839d4610f5cc0c6b264c7484f`, extracted using
`git archive` into `/tmp/plan03-historical-src`. Runtime code was not patched.
Build: `DOTNET_SKIP_BUILD=1 CARGO_TARGET_DIR=target/plan03-history-prebuilt cargo build
--manifest-path /tmp/plan03-historical-src/Cargo.toml --locked --offline -p dotnet-cli`.
The skip flag suppresses managed fixture rebuilding; the support artifact was already
built in this target. An initial build attempting full managed-fixture generation
was superseded; an attempted reuse of the current prebuilt fixture catalog was correctly
rejected because its catalog hash differs from the historical source catalog.

The identical current enum fixture assembly returns 42 under CLR and the repaired
runtime, and exits 1 under the historical runtime with:
`argument 0 does not match its parameter type ... Expected ValueType for Struct write`.
The first fixture assertion is the constrained enum argument call. Assembly SHA-256 and
outputs are recorded in `2026-11-05-plan03-historical-enum.json`.

Historical EF build: the same source/target, `cargo bench --locked --offline --profile dev
-p dotnet-benchmarks --bench end_to_end --no-run`, with `DOTNET_SKIP_BUILD=1`.
Run its generated `end_to_end-049f8b3a6a0cfc55 ef_inmemory --test`: exit 101 with the
same argument-0 struct-write layout error. The repaired benchmark was built with
`--profile bench-fat` in `target/plan03-completion-build`; both default and
`--no-default-features` `ef_inmemory --test` runs succeed (exit 0).
These are correctness smoke runs, not comparable performance measurements.

All three EF runs use copies of the original
`target/plan01-slice5/baseline-build/bench-fat/dotnet-bench-fixtures/ef_inmemory` cache.
Fourteen fixture/dependency hashes match the original baseline manifest before and
after execution; see `2026-11-05-plan03-ef-hashes.json`. Output DLL timestamps alone
were refreshed to prevent the benchmark harness from rebuilding the project; bytes
and hashes were unchanged.

## Verification

- Final expanded p43 passes with default features; the full gate covers all final
  runtime/test changes in no-feature, multithreading, and validation configurations.
- `./check.sh --fast`: passed clippy and all three suites, respectively 632 passed /
  7 skipped, 744 / 5, and 759 / 7. No tests were excluded.
- Scalar identity/layout and actual buffer-length rejection tests pass with default
  and no-default features; oversize payloads leave the destination unchanged.
- Pointer runner contract tests: 21 passed. Batch verification: 44 controls once in
  four batches. Doc drift: 49 passed. Formatting and whitespace checks pass.
- Pinned nightly-2026-05-27 Tree Borrows Miri: p37, p38, and final expanded p43
  passed in both no-feature and multithreading configurations (six controls).
  Both scalar rejection tests passed in both configurations (four test executions).
  Commands: `bash scripts/pointer_provenance.sh controls --configuration CONFIG
  --shard SHARD`, with p37 shard 35, p38 shard 37, and p43 shard 43. Scalar tests:
  `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"
  cargo +nightly-2026-05-27 miri test -p dotnet-vm --lib [--no-default-features]
  scalar_home_rejects`.
- Miri emitted exposed-provenance/integer-to-pointer warnings. Passing runs retain
  those detection limitations; no claim of complete pointer-bug detection is made.

Logs are under `/tmp/plan03-*` and are volatile. Pre-existing Plan02 and Substring
fixture working-tree changes were preserved.
