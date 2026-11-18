# Native continuations: resumable Substring

**Status:** Complete (historical; the runtime path was superseded by [plan 08](08-async-effects.md))
**Outcome:** Both `System.String.Substring` overloads retain copy progress across real
single-threaded GC safepoints, consume operands once and copy each requested UTF-16 code unit
once.

## What it proved

The first native continuation retained a traced source, output buffer, offset and caller identity
for large `Substring` calls, copied at most 64K code units per executor turn, and returned to the
executor between chunks so collection, abort and budget checks ran. It established the scheduler
contract that plan 08 generalized: Substring now runs as `substring_async` on the native
activation stack (`crates/dotnet-intrinsics-string/src/search.rs`), with finalizers in isolated
segments while the copy is parked.

## Safety requirements for any native continuation

- Futures are `'static` and hold no unconfined GC-branded values or heap borrows. Anything
  carried across suspension is a traced root.
- Nothing held while parked prevents collection (no lock, lease or GC access scope).
- Operands are consumed once; progress, result and IP ownership are unambiguous.
- Cancellation, failure, return and teardown release futures, buffers and roots.

## Accepted cost

The maintainer accepted the large-call throughput regression: +4.30% mean over four
CPU-pinned pairs (pairs +1.62%, +6.76%, +5.84%, +2.98%) on a workload of 80 calls requesting
12,000,000 units, which adds 160 scheduler turns (0.524 ms per run, about 3.28 µs per turn if
attributed entirely to scheduling). Controls showed no repeatable >1% regression: empty/small
Substring +0.17%, dispatch −0.28%, string −1.97%.

With `bench-instrumentation`, a 131,089-unit call copies 131,089 units with one buffer
allocation, no reallocation and a 262,178-byte largest buffer. The native latency control
measured 199.748 µs without and 196.417 µs with two collections (diagnostic only). Massif peak
live heap was 238,036,434 bytes before and 239,091,826 bytes after (whole process, diagnostic).

## Checks

The tests live in `executor::native_substring_tests` (`crates/dotnet-vm/src/native_substring_tests.rs`):
`pending_finalizer_runs_in_separate_segment_during_copy`,
`substring_dispatch_admits_only_inline_calls`,
`indirect_opcode_refuses_invalid_target_without_consuming_operands`, and the ignored release
control `measure_whole_copy_with_and_without_collection`.

```bash
cargo test -p dotnet-vm --no-default-features --features bench-instrumentation --lib native_substring_tests -- --nocapture
bash scripts/pointer_provenance.sh controls --configuration CONFIG --shard 44   # p44_native_substring_gc_resume
```

Throughput comparisons build `cargo bench --profile bench-fat -p dotnet-benchmarks
--no-default-features --bench substring --bench end_to_end --no-run` and run each executable under
`taskset -c 0` with `--sample-size 10 --warm-up-time 1 --measurement-time 4`.
