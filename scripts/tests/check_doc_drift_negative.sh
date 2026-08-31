#!/usr/bin/env bash
# Verify that check_doc_drift.sh rejects both a missing SAFETY citation and an
# otherwise parseable citation whose predicate is absent from the registry.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CRATES_DIR="$REPO_ROOT/crates"
probe_file=""
output_file=""

cleanup() {
  [[ -z "$probe_file" ]] || rm -f -- "$probe_file"
  [[ -z "$output_file" ]] || rm -f -- "$output_file"
}
trap cleanup EXIT INT TERM

fail() {
  echo "doc-drift negative regression failed: $*" >&2
  [[ -z "$output_file" || ! -f "$output_file" ]] || {
    echo "--- check_doc_drift.sh output ---" >&2
    cat "$output_file" >&2
    echo "----------------------------------" >&2
  }
  exit 1
}

# Use the same Cargo-metadata workspace membership rule as the gate and select
# a package source directory. A hidden standalone file is scanned by the gate
# but is not compiled as a module by the selected crate.
if ! probe_dir="$(
  cargo metadata --no-deps --format-version 1 |
    python3 -c '
import json
import os
import sys

crates_dir = os.path.realpath(sys.argv[1])
metadata = json.load(sys.stdin)
workspace_members = set(metadata["workspace_members"])
roots = []

for package in metadata["packages"]:
    if package["id"] not in workspace_members:
        continue
    root = os.path.realpath(os.path.dirname(package["manifest_path"]))
    if os.path.commonpath((crates_dir, root)) == crates_dir:
        roots.append(root)

for root in sorted(set(roots)):
    source_dir = os.path.join(root, "src")
    if os.path.isdir(source_dir):
        print(source_dir)
        break
else:
    raise RuntimeError("cargo metadata found no workspace package source directory under crates/")
' "$CRATES_DIR"
)"; then
  fail "could not discover a workspace package source directory"
fi

[[ -n "$probe_dir" ]] || fail "cargo metadata returned an empty package source directory"
probe_file="$(mktemp "$probe_dir/.check_doc_drift_negative.XXXXXX.rs")"
output_file="$(mktemp "${TMPDIR:-/tmp}/dotnet-rs-check-doc-drift-negative.XXXXXX")"

cat >"$probe_file" <<'EOF'
// SAFETY: This deliberate regression probe omits its registry citation.
// SAFETY: F99.DeliberatelyUndefined This deliberate regression probe uses a parseable undefined predicate.
EOF

if DOC_DRIFT_NEGATIVE_PROBE="$probe_file" bash "$REPO_ROOT/scripts/check_doc_drift.sh" >"$output_file" 2>&1; then
  status=0
else
  status=$?
fi

[[ $status -eq 1 ]] || fail "expected check_doc_drift.sh to exit 1, got $status"

grep -Fq \
  "[INVARIANT] SAFETY comment lacks a registry citation: $probe_file:" \
  "$output_file" \
  || fail "missing-citation diagnostic was not emitted for the temporary probe"

grep -Fq \
  "[INVARIANT] source cites 'F99.DeliberatelyUndefined', but it is absent from docs/INVARIANT_REGISTRY.md" \
  "$output_file" \
  || fail "undefined-predicate diagnostic was not emitted for the temporary probe"

echo "doc-drift negative regression: both expected diagnostics observed."
