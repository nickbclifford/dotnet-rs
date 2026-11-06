# Nested filters keep earlier suspended frames

**Status:** Complete
**Outcome:** A filter whose callee catches an exception through its own filter, entered while
frames are already suspended, resumes the original search with every suspended frame and
evaluation-stack slot intact, matching CLR output, in both configurations.

## Problem

When exception search reaches a filter, `dotnet-exceptions/src/lib.rs:415–447` suspends the
frames and evaluation stack above the filter's frame:

- `FrameStack::suspend_above` (`dotnet-vm-data/src/stack.rs:696–698`) **assigns**
  `self.suspended_frames = self.frames.split_off(index + 1)`.
- `EvaluationStack::suspend_above` (`stack.rs:351–353`) assigns `suspended_stack` the same way.

If code run by that filter throws, and the nested search reaches another filter, the second
call replaces the first suspended set. That set is dropped, along with its frame homes and
localloc blocks. `endfilter` (`dotnet-vm/src/stack/exception_ops_impl.rs:174–175`) and
`complete_nested_filter_unwind` (`lib.rs:523–524`) then restore only the inner set. The outer
search resumes over a stack missing the frames between the throw and the outer filter. Home
lookup also walks suspended frames (`stack.rs:620, 628`), so a byref into a dropped frame
resolves to nothing, or to the wrong frame.

ECMA-335 §I.12.4.2.7 allows this: "Functions called from within a filter can contain
exception handling." The existing fixture
`crates/dotnet-cli/tests/fixtures/exceptions/nested_filter_42.cs` throws from the frame that
owns the filter, so the first suspended set is empty and the overwrite never happens.
`docs/pointer-provenance/INVENTORY.md` (M1c outcomes) records it as suspected from source and
not reproduced.

[Plan 08](08-async-effects.md) has suspended segments carry native activations. They must
not be dropped this way.

## Scope

In:
- Suspension nests: each filter entry pushes a segment, and each `endfilter` or nested-filter
  completion pops exactly the segment it pushed.
- Home and byref resolution across all suspended segments.
- Rethrow and the existing "an exception escaping a filter means false" policy at every
  depth.

Out:
- Changing filter timing or two-pass semantics.
- Native activations in segments (plan 08 slice 2).

## Design

Replace the single `suspended_frames` / `suspended_stack` vectors with a stack of segments.
Each segment records the frame index and stack base it split at. `FilterState` already nests
through `nested_filter`. Tie each `FilterState` to its segment depth, and check the depth on
restore so a mismatch is an internal error instead of a silent mis-restore. `clear_suspended`
clears every segment on unhandled or terminal paths, as it does today.

## Slices

1. Write fixtures that reproduce the overwrite, then fix it:
   - throw from a frame below the filter owner, with a filter that calls a method containing
     its own `try`/`filter`, which catches;
   - the same, but the inner filter throws;
   - a byref to a local in a suspended frame, read after the nested filter returns;
   - three levels deep.

   Compare each with CLR output. Enabled when the fixtures pass in both configurations and
   the `frame_alloc::` suspension tests are extended to more than one segment.

## Done when

- The new fixtures and `nested_filter_42.cs` pass with and without `multithreading`.
- The `frame_alloc::` tests cover multi-segment suspend, restore and discard under pinned
  Miri.
- The INVENTORY note says the nested-filter overwrite is fixed and names the fixtures.

## Ask the maintainer only if

- CLR output for a fixture contradicts current filter-timing behavior beyond this defect.


## Completion evidence

Suspended storage uses flat owning vectors with LIFO segment boundaries. A traced
`FrameStack::active_filters` retains filter state when throw/rethrow replaces the current
exception mode; returning to its owner resumes the enclosing filter. Both completion paths
pop their filter and check segment depth. Terminal clearing drops all records.

The five `nested_filter_{catches,throws,byref,depth,rethrow}_42.cs` fixtures compare exit
codes and stdout with CLR. The byref fixture retains a byref argument into a suspended
worker local and reads/writes it during unwind after nested filtering. The existing
`nested_filter_42.cs` remains enabled. CLR returns 7 for the former
`exception_filter_5.cs` (filter 1 + finally 2 + catch 4); it is renamed
`exception_filter_7.cs`.

Validation (both with `--no-default-features` and with
`--no-default-features --features multithreading`):
- `DOTNET_TEST_FILTER=exceptions cargo test -p dotnet-cli --test integration_tests nested_filter`
  plus the configuration flags: 11 passed.
- `cargo +nightly-2026-05-27 miri test -p dotnet-vm-data --lib` plus configuration
  flags and `-- --test-threads=1`: 22 passed, using
  `-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks`.
- The exception fixture selector `exceptions`: 25 passed in both configurations,
  including a CLR exit-code comparison for `exception_filter_7.cs`.
- Clippy for `dotnet-exceptions`, `dotnet-vm`, and `dotnet-vm-data` with
  `-D warnings`; formatting and diff whitespace checks.

An unfiltered integration invocation was interrupted while running unrelated fixtures;
it is not recorded as a full-suite pass.
