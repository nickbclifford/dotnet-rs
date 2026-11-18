# Fuzzing dotnet-vm

This directory holds the `fuzz_executor` cargo-fuzz target, its committed corpus
(`corpus/fuzz_executor/`), the CIL dictionary (`cil.dict`), libFuzzer defaults
(`fuzz_executor.options`) and the corpus generators (`tools/`).

The `seed_program_*` inputs use the `DVMSEED1` format and decode through the same
`FuzzProgram::arbitrary` implementation as fuzzed inputs. Their named arithmetic,
memory, branch, and error paths have direct decoder and executor regressions. Inputs
named `opaque_fixture_*` and `opaque_legacy_*` are retained as byte-for-byte mutation
material; they do not encode or claim to replay the CIL programs suggested by their
former names. The arithmetic overflow seed exercises unchecked boundary addition; it does
not claim to test a checked overflow exception. `generate_corpus` produces opaque textual
instruction descriptions, while `generate_structured_corpus` writes the checked named
programs.

See [docs/FUZZING.md](../../../docs/FUZZING.md) for the toolchain, how to run and replay the
target, corpus management and crash triage.
