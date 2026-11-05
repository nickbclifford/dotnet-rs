#!/usr/bin/env bash
# Ratchet the count of `std::sync` tokens in Rust sources so new code uses the repository's
# synchronization wrappers; see docs/CI.md for how to change the ceiling.
set -euo pipefail

CEILING=81
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

COUNT="$(
    cd "$REPO_ROOT"
    { grep -rno --exclude-dir=target --include='*.rs' 'std::sync' crates/ || [ "$?" -eq 1 ]; } | wc -l
)"

echo "std::sync occurrences: $COUNT (ceiling: $CEILING)"

if (( COUNT > CEILING )); then
    echo "error: std::sync occurrences exceed the ceiling" >&2
    exit 1
fi
