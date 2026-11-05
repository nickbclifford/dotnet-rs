#!/usr/bin/env bash
# Ratchet the repository-wide count of `feature = "multithreading"` cfgs so feature gating
# stays at module boundaries; see docs/CI.md for how to change the ceiling.
set -euo pipefail

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
