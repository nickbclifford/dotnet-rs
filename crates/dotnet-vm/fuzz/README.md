# Fuzzing dotnet-vm

This directory contains fuzz targets for the dotnet-vm executor using cargo-fuzz (libFuzzer).

## Prerequisites

```bash
# Install cargo-fuzz
cargo install cargo-fuzz

# Ensure you have a nightly Rust toolchain
rustup install nightly
```

## Generating Seed Corpus

Before fuzzing, generate the initial seed corpus:

```bash
# From the fuzz directory
cargo run -p corpus-tools --bin generate_structured_corpus --release
```

This creates hand-crafted seed inputs in `corpus/fuzz_executor/` that exercise interesting code paths.

## Running the Fuzzer

### Basic fuzzing (run indefinitely)

```bash
cargo +nightly fuzz run fuzz_executor
```

### With corpus directory and options

```bash
cargo +nightly fuzz run fuzz_executor \
    corpus/fuzz_executor \
    -- \
    -dict=cil.dict \
    -timeout=2 \
    -rss_limit_mb=2048 \
    -max_len=10000 \
    -use_value_profile=1 \
    -print_coverage=1
```

### Running for a fixed time (e.g., 60 seconds)

```bash
cargo +nightly fuzz run fuzz_executor \
    corpus/fuzz_executor \
    -- \
    -max_total_time=60
```

### With AddressSanitizer (recommended)

```bash
RUSTFLAGS="-Zsanitizer=address" \
    cargo +nightly fuzz run fuzz_executor \
    corpus/fuzz_executor
```

## Checking Coverage

To see what code paths are being exercised:

```bash
cargo +nightly fuzz coverage fuzz_executor corpus/fuzz_executor
```

## Minimizing the Corpus

To reduce the corpus to a minimal set covering all unique paths:

```bash
cargo +nightly fuzz cmin fuzz_executor corpus/fuzz_executor
```

## Reproducing Crashes

If the fuzzer finds a crash, it will save the input to `fuzz/artifacts/fuzz_executor/crash-*`.

To reproduce:

```bash
cargo +nightly fuzz run fuzz_executor fuzz/artifacts/fuzz_executor/crash-<hash>
```

## Fuzzing Strategy

The `fuzz_executor` target:
- Accepts raw CIL bytecode as input
- Creates a minimal execution context with locals, arguments, and simple type definitions
- Executes the bytecode with an instruction budget to prevent infinite loops
- Catches panics and converts them to fuzzer failures

The fuzzer is configured with:
- **Seed corpus**: Hand-crafted inputs exercising arithmetic, memory, control flow, and stack operations
- **CIL dictionary**: Common opcodes and instruction sequences to guide mutation
- **Instruction budget**: 100,000 instructions to prevent timeouts
- **Memory limit**: 2GB RSS limit
- **Timeout**: 2 seconds per test case

## Interpreting Results

The fuzzer will report:
- **Crashes**: Panics, assertion failures, or UB caught by sanitizers
- **Hangs**: Test cases that exceed the timeout (usually infinite loops)
- **OOM**: Out-of-memory failures (allocation bombs)

All findings indicate potential issues in the VM that should be investigated and fixed.

## Integration with CI

See `.github/workflows/fuzz.yml` for continuous fuzzing configuration.
