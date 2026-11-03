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
# 478 -> 479 (2026-11-02): one module-level gate making the concurrent atomic
# load/store test multithreading-only; the no-features VM is single-threaded by
# contract and its atomics lower to plain accesses, so the test raced under Miri.
# 479 -> 481 (plan 03 step 1.1): one test-support module gate and one MT-only
# failure-injection regression. The worker protocols require real MT Executors;
# the helper is unused in the single-thread configuration. No production cfgs added.
# 481 -> 488 (plan 03 step 2.1): net seven ownership/retention gates
# including the private owner field, retained Box helper, Drop-entry arming,
# test-only fault state and ownership regression module. Existing TLS re-export
# gate removed; no unrelated cfgs changed. std::sync remains 81.
CEILING=488
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
