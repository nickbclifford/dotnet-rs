# CLI test cleanup evidence

This record reconciles CLI-1 through CLI-23 in the [CLI audit](cli.md). Guest fixtures remain
generated and selected by `build.rs` unless the row names a handwritten test. The verification
section records executed checks and links the combined repository verification.

## Finding dispositions

| Finding | Change and retained contract |
|---|---|
| CLI-1 | Replaced the three unhandled stack-trace smokes with caught-exception fixtures. They require the exact exception type and selected no-parameter, parameter, or generic frames through `Exception.StackTrace`; the parameter case accepts the CLR names (`String`, `Int32`) or their exact C# aliases (`string`, `int`) with the same parameter names. The unhandled-exception smoke remains, and CLR differential registrations run alongside the guest assertions. |
| CLI-2 | Removed event-count and source-text selection tests. Delegate multicast cctor/byref fixtures retain guest state, output, and CLR comparison, including the cctor case that must preserve a byref operand. Reflection guest assertions and CLR comparisons remain. The tail-call reflection probe keeps only its positive asynchronous-path observation, where that path is an internal requirement. |
| CLI-3 | Removed fuzz-mode success returns based on exception-message substrings. Fuzz-only static ignores cover fixtures observed to call denied `SystemNative_SetErrNo`, `SystemNative_Malloc`, or `SystemNative_Calloc`; ordinary feature runs retain their normal registrations. Every executed fixture must match its expected exit code. |
| CLI-4 | Removed sync-block, GC-coordinator, and arena-handle field-access smokes. The feature suite retains the stable single-threaded thread-ID check and shared-loader root check. |
| CLI-5 | Arithmetic inputs now come from methods so C# cannot fold the tested expressions. A parsed-IL test requires division, remainder, shifts, bitwise operations, multiply, add, and subtract. Floating infinity division also uses runtime operands. |
| CLI-6 | LINQ materialization checks indexed list/array contents; the ordering case checks every ordered element instead of a permutation-invariant sum. Aggregate, grouping, and laziness assertions remain. |
| CLI-7 | The finalizable generic object is created in a no-inline helper, and the fixture now requires the finalizer to record `Int16`. |
| CLI-8 | `MakeGenericType` maps `GenericConstraintViolation` to managed `ArgumentException` and propagates metadata resolution failures as host `VmError`; five invalid constraints and a valid interface-constrained construction exercise the API boundary. |
| CLI-9 | The contravariant target records its argument and call count; the fixture requires the same object instance to arrive once. |
| CLI-10 | The pending return moved to a helper. `Main` independently requires both the returned value and the post-finally state. |
| CLI-11 | Renamed the negative `GC.Collect` case to `gc_collect_negative_count_smoke_42` and kept its differential exit-only. It is explicitly a smoke, with no stack-trace claim. The known negative-count CLR/VM behavior difference remains separate. |
| CLI-12 | The thread-static finalizer fixture now requires one finalizer run as well as its thread-static identity values. |
| CLI-13 | Resurrection fixtures require the first and second finalization counts, and the suppressed object remains covered by the exact finalizer count. |
| CLI-14 | Every P/Invoke mismatch branch now returns an unambiguous failure code instead of truncating a wrong scalar or struct result to 42. |
| CLI-15 | Removed the machine-dependent `Span.Clear` timing threshold and its output. Byte, integer, reference, struct, and empty-span boundary checks remain. |
| CLI-16 | Cctor success cases check each trigger result independently; recursive initialization checks both `A.Get()` and `B.Y`. Failure branches return codes other than 42. |
| CLI-17 | The volatile-sharing fixture assigns one writer and one reader with `Interlocked`, coordinates reader readiness, publishes data before the volatile flag, and requires the reader to observe the flag and value. The generated registration uses two arenas when multithreading is enabled and is explicitly ignored in single-executor configurations; yield loops are bounded. |
| CLI-18 | The DynamicMethod fixture emits `ldc.i4.s` explicitly, independently calculates the two-target switch's next offset as 17 bytes, then executes the branch and checks its result. |
| CLI-19 | Added direct wrong-type and valid reflection controls. The racing target records calls and rejects any value other than the expected integer; success requires the race to mutate the argument, produce a rejection, and never deliver a wrong value to the target. The test does not claim a specific internal scheduling interleaving. |
| CLI-20 | Multi-arena workers no longer catch and discard panics after reporting entry results. The parent joins every worker and reports teardown panics; a gated post-result panic control exercises that join path. Entrypoint setup errors also report through the result callback. |
| CLI-21 | Cache observability uses a fresh loader, runs the non-console arithmetic fixture, requires its successful guest exit, and checks cache growth from the pre-run baseline. |
| CLI-22 | Generated multi-arena tests accept attributes, and `build.rs` passes the missing-artifact ignore to them just as it does to ordinary fixture tests. Prebuilt mode still rejects missing artifacts. |
| CLI-23 | `run_cli`, handwritten CLR runs, and the F# build use the concurrent, bounded process capture. The partial-output/pipe-drain deadline control remains. |

## Validation

Focused checks passed:

- `CARGO_TARGET_DIR=/tmp/dotnet-rs-27b-cli-reflection-target CARGO_NET_OFFLINE=true cargo test -p dotnet-intrinsics-reflection --lib type_fit_error_tests::reference_type_fit_preserves_resolution_errors_as_host_vm_errors` (1 passed).
- `DOTNET_USE_PREBUILT_FIXTURES=1 DOTNET_FIXTURES_BASE=/home/nick/Desktop/dotnet-rs/target/debug/dotnet-fixtures CARGO_TARGET_DIR=/tmp/dotnet-rs-27b-cli-generic-target CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4 cargo nextest run -p dotnet-cli --no-default-features --features multithreading,generic-constraint-validation -E 'test(generic_constraints_fail_0)'` (1 passed).
- Isolated fuzz nextest passed (230 passed, 181 skipped), and full fuzz Clippy with `-D warnings` passed. Both used `/tmp/dotnet-rs-27b-fuzz-target` and prebuilt fixture DLLs.
- `rustfmt --edition 2024` on changed CLI/reflection Rust files and scoped `git diff --check` passed.

Combined workspace results are recorded in the [test audit ledger](../27-test-audit-ledger.md#combined-cleanup-verification). The fuzz-only static ignores cover fixtures observed to fail on denied native P/Invoke entry points; normal feature registrations remain enabled.

## Evidence limits

The reflection comparer regression exercises the production type-fit helper with valid, mismatched, and missing-type cases, but does not inject a resolution failure through a full interpreter invocation. The `MakeGenericType` fixture checks five invalid constraints and a valid control at the managed API boundary. The reflection argument race remains a stress schedule and does not prove a particular validation/use interleaving. The negative `GC.Collect` behavior divergence is not covered by the renamed smoke and remains a separate runtime finding.

The stack-trace parameter fixture accepts CLR framework names (`String`, `Int32`) or their exact C# aliases (`string`, `int`) because CLR and VM format the same selected method frames differently. The DynamicMethod offset fixture emits `ldc.i4.s` explicitly so its 17-byte oracle follows fixed instruction encodings.
