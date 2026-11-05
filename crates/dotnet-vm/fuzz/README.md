# Fuzzing dotnet-vm

This directory holds the `fuzz_executor` cargo-fuzz target, its committed corpus
(`corpus/fuzz_executor/`), the CIL dictionary (`cil.dict`), libFuzzer defaults
(`fuzz_executor.options`) and the corpus generators (`tools/`).

See [docs/FUZZING.md](../../../docs/FUZZING.md) for the toolchain, how to run and replay the
target, corpus management and crash triage.
