# Plans 03 and 04 completion audit

Audit base: `98f04a7709303917e085a82e17764fa4bc2cd8d7`, Rust 1.95.0;
pointer checks use nightly-2026-05-27 Tree Borrows through the existing runner.
The pre-existing untracked plan02 audit was left untouched.

## Verdict

Audit complete: neither plan qualifies for completion certification. Plan 03 has an
unproved scalar-buffer safety premise and incomplete requirement coverage. Plan 04
has a reproduced CLR compatibility defect and incomplete finalizer/performance
evidence. Both status lines now say In progress. Runtime repairs are deliberately
left to follow-up work; the failing CLR regression remains enabled.

## Fresh results

| Check | Result |
| --- | --- |
| ST native Substring tests, `--no-default-features --features fuzzing` | 10 passed, one manual timing test ignored |
| CLR differential suite, no default features | 11 passed before ParamName output was added; superseded for Substring range validation by the failing test below |
| CLR differential suite, default features | 11 passed before ParamName output was added; superseded for Substring range validation by the failing test below |
| Pointer-runner Python contract tests | 21 passed |
| Pointer batch guard | 44 controls exactly once in four batches |
| Doc drift including negative regression | 49 passed |
| Pinned Miri p37, p38, p43, p44 | All eight runs passed: no features and multithreading |
| Scalar-home negative test | Passed with no features and default features; rejects wrong identity and layout size without modifying destination |
| Substring ParamName differential, no features and default features | Failed in both; VM emits seven empty names, CLR emits startIndex/startIndex/startIndex/length/length/startIndex/length |
| `./check.sh --fast` | Failed: no-feature suite 629 passed, 1 failed, 7 skipped; failure is the new ParamName regression |
| VM all-target no-feature clippy with warnings denied | Passed after audit test additions |
| Multithreading nextest, excluding known failing ParamName test | 742 passed, 6 skipped |
| Multithreading + validation-all nextest, excluding known failing ParamName test | 757 passed, 8 skipped |
| Formatting and whitespace | Passed |

Commands: `cargo test -p dotnet-vm --lib --no-default-features --features fuzzing native_substring_tests`;
`cargo test -p dotnet-cli --test integration_tests [--no-default-features] matches_dotnet`;
`python3 scripts/tests/test_pointer_provenance.py`;
`bash scripts/pointer_provenance.sh verify-batches`;
`bash scripts/check_doc_drift.sh`;
`bash scripts/pointer_provenance.sh controls --configuration CONFIG --shard SHARD`.
Shard 43 is p43; shard 44 is p44; p37/p38 are shards 35/37.

Local transcripts use `/tmp/plan03-04-*.log`. These paths are volatile; the table
records completed results rather than implying the logs will remain available.

## Requirement assessment

| Plan | Requirement | Evidence and remaining work |
| --- | --- | --- |
| 03 | Scalar layout with separate declared identity | `frame_homes::variable_home` selects scalar layouts and retains `scalar_type`; enum fixture and p43 pass |
| 03 | Shared installation/assignment conversion | `write_home_value` uses the full home and checks descriptor and layout size; new scalar-home test verifies wrong descriptor and wrong layout-size rejection; short byte payload remains unchecked |
| 03 | Widths, generic values, identity, reference-bearing controls | Existing enum fixture covers these and now has an automated CLR oracle |
| 03 | Explicit opcode and nested-growth coverage | Full requirement-to-generated-IL mapping and additional controls remain outstanding |
| 03 | Historical failure and identical-cache EF reproduction | Not rerun; requires isolated pre-repair build and fixture hash comparison |
| 03 | Plan02 performance dependency | Plan02 is now Complete with accepted primitive cost; plan03's closing note predates that closure |
| 04 | Real collection, bounded progress, rooting and single completion | Existing native tests pass, including no-pressure yield and weak-source reclamation |
| 04 | Abort, budget, mismatch and teardown | Existing native controls pass in the focused fuzzing configuration |
| 04 | Both overloads, UTF-16, ranges and handler contexts | Existing managed fixtures pass automated CLR comparisons |
| 04 | Exception parameter names | Fixture now emits ParamName for differential comparison; post-edit comparison fails under both feature configurations |
| 04 | Finalizer execution ordering | Existing test queues a string, proving retention/drainage only; actual managed finalizer test remains needed |
| 04 | All fallback call shapes | Delegate/reflection fixture and MT Miri pass; indirect/tail/engine-call admission coverage still needs explicit mapping/tests |
| 04 | Performance and resource measurements | Historical report distinguishes calculations and observed capacity from counters; the historical paired compact-representation results show no repeatable >1% unaffected-control regression; end-to-end allocation/copy/peak counts are calculated rather than measured. Fresh timing is deferred until correctness gates pass |

The scalar-store code checks `instance_storage.layout().total_size` before reading
inside `with_data`. The plan03 record also claims an actual byte-buffer-length check;
that explicit check is absent at this audit base. This is a documentation discrepancy,
an unmet safety premise: `FieldStorage::new` stores the supplied Vec without a
length assertion (storage.rs), so the constructor does not establish that invariant.
A malformed same-type, same-layout value can reach an unchecked raw read. This audit
does not claim a production managed-IL reproducer or execute that potentially invalid
read. Add the actual length guard and a short-payload regression in the repair.

## Follow-up required for certification

1. Plan 03: validate the actual byte length before scalar decoding; add a short-payload
   rejection test, retaining the new type/layout negative test. Complete generated-IL
   mapping for nested growth and explicit object operations, and reproduce the historical
   pre-repair failure and identical-cache EF smoke. Existing repair logs and fixture hash
   manifests are present under `target/plan01-slice5/`; they are historical evidence,
   not fresh runs. Plan02's completed performance closure satisfies its separate matrix
   dependency; no new plan03 performance claim is needed.
2. Plan 04: populate ArgumentOutOfRangeException.ParamName rather than passing the name
   only as the exception message (`intrinsic_string_substring` in search.rs). Both overloads
   use this path. Keep the differential test enabled and restore the full fast gate.
3. Plan 04: add an actual managed finalizer execution/order test. The present test
   queues a string and only establishes queue deferral/drainage. Inspecting production
   dispatch establishes non-tail call/callvirt admission and generic-dispatch fallback,
   but indirect/tail/engine-call paths still need explicit executable controls.
4. After correctness passes, complete measured resource accounting and isolated paired
   performance reproduction. The historical report is candid about scope: the forced-GC
   diagnostic measures parked-suffix latency and destination capacity, not whole-call
   throughput or heap-wide peak bytes. Do not substitute those observations for counters.

No new performance run was attempted after correctness rejection: a timing result cannot
change this verdict. No runtime fixes or public API changes were made. The deliberately
excluded differential test remains enabled in normal checks; these supplementary suites
do not constitute a passing fast gate. An initial negative-test harness used an absent
mock corlib and failed during setup; the final test uses explicit synthetic descriptors
and passes, and the multithreading suite was rerun after that correction.

## Plan 03 scalar-buffer follow-up

After this audit, scalar-home decoding was changed to check the actual borrowed byte
length against the scalar layout size before the raw read. The new regression rejects
all shorter Int32 payloads without modifying the destination; the existing identity and
layout-size rejection test is retained. Both tests pass with default features and with
`--no-default-features`; no-feature VM test clippy also passes with warnings denied.
Pinned nightly-2026-05-27 Tree Borrows Miri passed p43 in no-feature and
multithreading configurations and the short-payload test without features. These
runs emitted exposed-provenance warnings, limiting pointer-bug detection.
The plan remains in progress because the opcode/nested-growth mapping and fresh
historical failure and identical-cache EF reproduction still need completion.

## Plan 04 parameter-name follow-up

Substring range exceptions now populate `ArgumentException._paramName`. The final
targeted Substring run passes all four tests with default features and all four
with `--no-default-features`, including the seven-name CLR comparison that failed
during the audit. Doc drift passes (49 checks). These results supersede the
historical ParamName failures above; they do not establish the full fast gate.

The implementation evidence records an exploratory delegate-exception sequence
that failed under default features after returning the correct parameter names.
Its cause and whether it predates this repair remain unverified. Plan 04 remains
in progress for that fallback investigation and the finalizer/performance gaps.
