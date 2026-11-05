# Substring continuation: performance evidence

Baseline: `43663231e420c490e8afde27298b71ca5d09de3b` exported with `git archive`.
Candidate: working-tree continuation implementation, built after the direct-call
eligibility change. Both used `bench-fat`, `--no-default-features`, the same Rust
toolchain, and identical benchmark sources and compiled fixture assemblies. Each
executable was prewarmed and pinned to CPU 0. Criterion used 10 samples, a one-second
warmup, and a three-second target (long cases required longer). Two rounds were run
in opposite orders. Other fast-check and Miri work was active during measurements,
so these results are preliminary and do not establish an isolated regression.

| Workload | Baseline round 1 / 2 | Candidate round 1 / 2 | Round 2 difference |
| --- | --- | --- | --- |
| Empty and small substring | 108.98 / 108.39 ms | 112.73 / 112.18 ms | +3.5% |
| 150,000-unit substring | 13.115 / 12.962 ms | 14.913 / 13.474 ms | +4.0% |
| Existing dispatch | 368.02 / 370.23 ms | 377.10 / 372.97 ms | +0.7% |
| Existing string workload | 364.62 / 359.61 ms | 358.35 / 360.56 ms | +0.3% |

Values are Criterion point estimates per fixture invocation, not per substring call.
The small control regressed in both rounds; dispatch's second-round intervals overlap.
Large timings varied markedly between rounds. Further isolated measurements and
investigation are needed before accepting the small-path regression. The direct-call
eligibility writes and changes to the intrinsic are possible causes, not established
by these measurements.

An additional source experiment replaced the intrinsic eligibility check's allocated
type-name comparison with generated intrinsic-ID matching. With the same settings,
candidate then baseline measured 112.40 vs 108.78 ms for empty/small (+3.3%) and
374.61 vs 383.39 ms for dispatch (-2.3%). The candidate's small result was essentially
unchanged from the previous 112.18 ms. The dispatch baseline interval was wide
(372.90–397.73 ms); fast-check compilation was active during this round too. This
experiment does not resolve the small-path difference or establish its cause.

The subsequent compact `CallState` experiment groups existing call state behind one
context pointer and stores the activation in an `Option<Box<_>>`. It retained the
intrinsic-ID check. With no project checks running concurrently, the same CPU affinity
and 10 samples, using a four-second target, gave these paired point estimates:

| Workload | Baseline | Compact candidate | Difference |
| --- | --- | --- | --- |
| Empty and small, baseline first | 106.30 ms | 107.18 ms | +0.83% |
| Empty and small, candidate first | 106.76 ms | 106.97 ms | +0.20% |
| Dispatch, baseline first | 362.01 ms | 362.24 ms | +0.06% |
| Dispatch, candidate first | 362.75 ms | 366.37 ms | +1.00% (rounded) |
| Large substring | 12.025 ms | 12.737 ms | +5.92% |
| Existing string workload | 353.92 ms | 328.80 ms | -7.10% |

The unaffected controls did not show a repeatable difference above 1% in these two
pairs. The large path incurs bounded scheduling turns; the observed difference also
includes all fixture and executor overhead. The existing string result varied much
more than the other controls, so its apparent improvement should not be attributed to
this change without further investigation. These are point-estimate comparisons of
independent runs, not paired statistical confidence intervals.

The small fixture performs 20,000 empty and 20,000 eight-unit calls. The large fixture
performs 80 calls of 150,000 units each. In a run without retries, these request
12,000,000 copied units and 80 result buffers; each large buffer has capacity for
300,000 bytes of UTF-16 data. These are workload and capacity calculations, not
measured allocation, copy or peak-retention counters for that end-to-end workload.
Ordinary runs may collect and were not instrumented to prove absence of GC.

## Forced collection and retained storage

The final compact representation was measured separately using the ignored
`executor::native_substring_tests::measure_forced_collection_copy` diagnostic. It
creates the synthetic assembly, executor and source, installs the activation, and
copies the first chunk **before** timing. Each timed interval forces two real full
collections and resumes the remaining 65,553 UTF-16 code units. It includes the
production collection checks, copy/completion path and one progress snapshot;
setup, the initial chunk, post-timing result assertions and executor destruction are
excluded. This is parked-suffix latency, not whole-call throughput or an estimate of
GC overhead against the pre-change path (which cannot retain progress).

Thirty serial CPU-0-pinned `bench-fat` samples gave a median of
192.274 microseconds (range 189.219–317.341).
The observed destination capacity plus `size_of::<NativeSubstringActivation>()` was
262,258 bytes per parked activation. This excludes the existing source object,
allocator overhead, and GC bookkeeping. Each activation contains one traced source
field; this is not a measurement of all VM roots. Each sample checked the unchanged
destination address at the intermediate boundary, exact progress, one completion,
and the exact final UTF-16 result. The separate rooting test verifies source survival
through each collection and reclamation after completion.

The admitted path constructs one destination `Vec::with_capacity(length)` and one
boxed activation. Copies stay within that reserved capacity; the stable-address
check exercises that invariant. These are allocation sites and storage-capacity
observations, not global allocator instrumentation or a heap-wide peak measurement.
They account for the new owned storage without attributing unrelated collector
allocations to Substring.

Reproduce the manual diagnostic after building the optimized unit-test executable:

```sh
CARGO_TARGET_DIR=/tmp/substring-candidate-target cargo test -p dotnet-vm --profile bench-fat --no-default-features measure_forced_collection_copy --lib --no-run
taskset -c 0 <target>/bench-fat/deps/dotnet_vm-<hash> executor::native_substring_tests::measure_forced_collection_copy --exact --ignored --nocapture --test-threads=1
```

The local raw diagnostic output is `/tmp/substring-forced-timing.log`; source and
reported interval definition are retained here so the measurement can be repeated.

Reproduction (after exporting the baseline and copying the new benchmark sources to
it):

```sh
CARGO_TARGET_DIR=/tmp/substring-baseline-target cargo bench --profile bench-fat -p dotnet-benchmarks --no-default-features --bench substring --bench end_to_end --no-run
CARGO_TARGET_DIR=/tmp/substring-candidate-target cargo bench --profile bench-fat -p dotnet-benchmarks --no-default-features --bench substring --bench end_to_end --no-run
taskset -c 0 <target>/bench-fat/deps/substring-<hash> --bench --warm-up-time 1 --measurement-time 3 --sample-size 10 --noplot
taskset -c 0 <target>/bench-fat/deps/end_to_end-<hash> 'dispatch|string' --bench --warm-up-time 1 --measurement-time 3 --sample-size 10 --noplot
```

The fixture builds succeeded with NuGet vulnerability-feed warnings due to restricted
network access. The compiled baseline fixture directories were copied to the candidate
target before prewarming so both measured the same assemblies.
