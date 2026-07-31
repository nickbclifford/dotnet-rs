#!/usr/bin/env bash
# Enforce the repository-wide multithreading cfg-occurrence budget.
set -euo pipefail

CEILING=403
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
