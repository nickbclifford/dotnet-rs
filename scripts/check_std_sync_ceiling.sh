#!/usr/bin/env bash
# Ratchet contiguous `std::sync` tokens in Rust sources. The retained uses are
# the F-SYNC-002-reviewed test/build-script, std Arc/Weak, static/raw-atomic,
# and Once/LazyLock-family exemptions; this lexical check is not a policy parser.
set -euo pipefail

CEILING=70
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
