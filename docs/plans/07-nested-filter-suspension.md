# Nested filters keep earlier suspended frames

**Status:** Complete
**Outcome:** A filter whose callee catches an exception through its own filter, entered while
frames are already suspended, resumes the original search with every suspended frame and
evaluation-stack slot intact, matching CLR output, in both configurations.

## What was fixed

Filter entry suspended the frames and evaluation stack above the filter's frame by assigning a
single `suspended_frames`/`suspended_stack` pair, so a nested filter (allowed by ECMA-335
§I.12.4.2.7) replaced and dropped the outer suspended set, with its frame homes and `localloc`
blocks. Suspension now nests: `EvaluationStack` and `FrameStack` keep flat owning vectors with LIFO
segment boundaries (`suspended_segments` in `crates/dotnet-vm-data/src/stack.rs`); a traced
`FrameStack::active_filters` retains filter state when a throw or rethrow replaces the current
exception mode; both completion paths (`endfilter` and nested-filter unwind) pop their own
segment and check its depth, so a mismatch is an internal error. Terminal clearing drops every
segment. Home lookup walks all suspended segments.

## Checks

CLR differential fixtures in `crates/dotnet-cli/tests/fixtures/exceptions/`:
`nested_filter_{catches,throws,byref,depth,rethrow}_42.cs`, plus `nested_filter_42.cs` and
`filter_throws_keeps_original_42.cs`. `exception_filter_7.cs` (filter 1 + finally 2 + catch 4)
compares its exit code with CLR. The `frame_alloc::` library tests cover multi-segment suspend,
restore and discard.

```bash
DOTNET_TEST_FILTER=exceptions cargo test -p dotnet-cli --test integration_tests nested_filter --no-default-features   # and --features multithreading
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
  cargo +nightly-2026-05-27 miri test -p dotnet-vm-data --lib --no-default-features -- --test-threads=1
```
