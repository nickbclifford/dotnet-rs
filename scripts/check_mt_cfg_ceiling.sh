#!/usr/bin/env bash
# Enforce the repository-wide multithreading cfg-occurrence budget.
set -euo pipefail

# Raised 424 -> 477 (reviewed and approved by the maintainer 2026-11-01, step 8.4).
# Growth is lexical and spans both gated tests (value object/pointer/teardown/
# collection tests) and production cfgs (codec repair, pointer origin, write
# barrier, pinvoke, VM executor, utils gc); it is not tests-only. Some files
# shrank (e.g. stack_value -8). Further growth needs a new review.
# 477 -> 478 (approved 2026-11-01): the single ignored pending-repair
# recreation for plan 03 (extracted-Executor TLS ownership).
CEILING=478
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

COUNT="$(
    cd "$REPO_ROOT"
    { grep -rno 'feature = "multithreading"' crates/ --include='*.rs' || [ "$?" -eq 1 ]; } | wc -l
)"

echo "multithreading cfg occurrences: $COUNT (ceiling: $CEILING)"

if (( COUNT > CEILING )); then
    echo "error: multithreading cfg occurrences exceed the ceiling" >&2
    exit 1
fi
