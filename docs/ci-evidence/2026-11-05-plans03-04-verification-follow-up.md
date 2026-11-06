# Plans 03 and 04 verification follow-up

Starting revision: `36c59e23`. This is a partial verification record, not completion certification. Both plans remain In progress.

## Changes

The enabled `substring_fallback_42` fixture now catches exceptions from both delegate overloads, prints their parameter names for the CLR oracle, and performs subsequent successful delegate calls with exact content checks. This restores the previously removed exploratory sequence without suppressing failures.

## Fresh completed checks

- `cargo test -p dotnet-vm --lib --no-default-features --features fuzzing native_substring_tests`: 10 passed, 1 manual timing test ignored.
- `python3 scripts/tests/test_pointer_provenance.py`: 21 passed.
- `bash scripts/pointer_provenance.sh verify-batches`: 44 controls exactly once in four batches.
- `bash scripts/check_doc_drift.sh`: 49 passed, including negative regression.
- `git diff --check`: passed.

## Pending evidence

The default-feature Substring differential invocation was started with output in `/tmp/plans0304-default.log`. A sequential verification job was also started for no-feature differential tests, scalar rejection tests in both configurations, and the full fast gate, with outputs `/tmp/plans0304-st.log`, `/tmp/plans0304-scalar-st.log`, `/tmp/plans0304-scalar-default.log`, and `/tmp/plans0304-fast.log`. These are volatile transcripts; a started command is not a passing result.

Still required: actual generated-IL requirement mapping and missing opcode controls; deterministic enum nested-growth control; fresh historical enum/identical-cache EF reproduction; actual managed-finalizer ordering control; indirect/tail/engine-call admission controls; fresh pinned Miri matrix; measured resource accounting and isolated performance reproduction after correctness passes. No performance measurement or completion status change is justified by this partial record.

Plan02 working-tree edits and its untracked audit are pre-existing and were left untouched.
