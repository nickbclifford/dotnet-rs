#!/usr/bin/env bash
# check_doc_drift.sh — CI doc-to-code drift detector
#
# For each entry in the CHECKS table below the script verifies that a key
# identifier (struct name, constant, enum variant, etc.) appears in BOTH the
# specified documentation file AND somewhere in the Rust source tree.
#
# If the identifier has been renamed or removed in the code but the doc still
# references the old name — or vice-versa — the check fails and prints a
# diagnostic.  This catches the most common form of doc drift: a refactor that
# forgets to update the corresponding documentation.
#
# Usage:
#   ./scripts/check_doc_drift.sh          # run all checks
#   ./scripts/check_doc_drift.sh --list   # print the check table and exit
#
# Exit code: 0 if all checks pass, 1 if any check fails.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATES_DIR="$REPO_ROOT/crates"
DOCS_DIR="$REPO_ROOT/docs"
REGISTRY_PATH="$DOCS_DIR/INVARIANT_REGISTRY.md"

# Limit invariant-citation checks to Rust files owned by workspace packages
# beneath crates/. Cargo metadata is authoritative for workspace membership;
# do not fall back to a partial hand-maintained list if discovery fails.
if ! SAFETY_CRATE_ROOTS_TEXT="$(
  cargo metadata --no-deps --format-version 1 |
    python3 -c '
import json
import os
import sys

crates_dir = os.path.realpath(sys.argv[1])
metadata = json.load(sys.stdin)
workspace_members = set(metadata["workspace_members"])
roots = set()

for package in metadata["packages"]:
    if package["id"] not in workspace_members:
        continue
    root = os.path.realpath(os.path.dirname(package["manifest_path"]))
    if os.path.commonpath((crates_dir, root)) == crates_dir:
        roots.add(root)

if not roots:
    raise RuntimeError("cargo metadata found no workspace packages under crates/")

print("\n".join(sorted(roots)))
' "$CRATES_DIR"
)"; then
  echo "[INVARIANT] failed to discover workspace package roots with cargo metadata"
  exit 1
fi

if [[ -z "$SAFETY_CRATE_ROOTS_TEXT" ]]; then
  echo "[INVARIANT] cargo metadata found no workspace packages under crates/"
  exit 1
fi

mapfile -t SAFETY_CRATE_ROOTS <<< "$SAFETY_CRATE_ROOTS_TEXT"

# ---------------------------------------------------------------------------
# Check table
# Format: "DOC_FILE|IDENTIFIER|SOURCE_GLOB"
#   DOC_FILE    — path relative to DOCS_DIR
#   IDENTIFIER  — literal string that must appear in the doc AND in source
#   SOURCE_GLOB — glob passed to grep -r --include to narrow the source search
# ---------------------------------------------------------------------------
CHECKS=(
  # One row per core concept a doc is built around. A row fails when the doc or
  # the code drops the name, so a rename forces the doc to be revisited. Keep the
  # list short; rustdoc intra-doc links cover ordinary symbol references.

  # --- GC_AND_MEMORY_SAFETY.md ---
  "GC_AND_MEMORY_SAFETY.md|GCCoordinator|*.rs"
  "GC_AND_MEMORY_SAFETY.md|CollectionSession|*.rs"
  "GC_AND_MEMORY_SAFETY.md|MemoryOwner|*.rs"
  "GC_AND_MEMORY_SAFETY.md|ThreadSafeLock|*.rs"
  "GC_AND_MEMORY_SAFETY.md|PointerOrigin|*.rs"

  # --- THREADING_AND_SYNCHRONIZATION.md ---
  "THREADING_AND_SYNCHRONIZATION.md|StopTheWorldGuard|*.rs"
  "THREADING_AND_SYNCHRONIZATION.md|GcScopeGuard|*.rs"
  "THREADING_AND_SYNCHRONIZATION.md|SyncBlockManager|*.rs"
  "THREADING_AND_SYNCHRONIZATION.md|define_lock_order_dag!|*.rs"

  # --- ARCHITECTURE.md ---
  "ARCHITECTURE.md|EvaluationStack|*.rs"
  "ARCHITECTURE.md|frame_home_window|*.rs"

  # --- BUILD_TIME_CODE_GENERATION.md ---
  "BUILD_TIME_CODE_GENERATION.md|generate_instruction_table|*.rs"
  "BUILD_TIME_CODE_GENERATION.md|generate_intrinsic_phf|*.rs"
  "BUILD_TIME_CODE_GENERATION.md|SupportSlotOps|*.rs"

  # --- EXCEPTION_HANDLING.md ---
  "EXCEPTION_HANDLING.md|ExceptionHandlingSystem|*.rs"
  "EXCEPTION_HANDLING.md|SearchState|*.rs"
  "EXCEPTION_HANDLING.md|UnwindState|*.rs"

  # --- DELEGATES_AND_DISPATCH.md ---
  "DELEGATES_AND_DISPATCH.md|unified_dispatch|*.rs"
  "DELEGATES_AND_DISPATCH.md|MulticastState|*.rs"

  # --- TYPE_RESOLUTION_AND_CACHING.md ---
  "TYPE_RESOLUTION_AND_CACHING.md|GlobalCaches|*.rs"
  "TYPE_RESOLUTION_AND_CACHING.md|ResolutionContext|*.rs"

  # --- FUZZING.md ---
  "FUZZING.md|FuzzProgram|*.rs"
  "FUZZING.md|execute_cil_program|*.rs"
)

if [[ "${1:-}" == "--list" ]]; then
  printf "%-45s %-35s %s\n" "DOC FILE" "IDENTIFIER" "SOURCE GLOB"
  printf "%-45s %-35s %s\n" "---------" "----------" "-----------"
  for entry in "${CHECKS[@]}"; do
    IFS='|' read -r doc ident glob <<< "$entry"
    printf "%-45s %-35s %s\n" "$doc" "$ident" "$glob"
  done
  exit 0
fi

FAIL=0
PASS=0

check() {
  local doc="$1" ident="$2" glob="$3"
  local doc_path="$DOCS_DIR/$doc"
  local in_doc=0 in_src=0

  if grep -qF "$ident" "$doc_path" 2>/dev/null; then
    in_doc=1
  fi

  if grep -rq --include="$glob" -F "$ident" "$CRATES_DIR" 2>/dev/null; then
    in_src=1
  fi

  if [[ $in_doc -eq 1 && $in_src -eq 1 ]]; then
    PASS=$((PASS + 1))
    return
  fi

  FAIL=$((FAIL + 1))
  if [[ $in_doc -eq 0 && $in_src -eq 1 ]]; then
    echo "[DRIFT] '$ident' exists in source but is MISSING from docs/$doc"
  elif [[ $in_doc -eq 1 && $in_src -eq 0 ]]; then
    echo "[DRIFT] '$ident' is referenced in docs/$doc but NOT FOUND in source (renamed/removed?)"
  else
    echo "[DRIFT] '$ident' is missing from BOTH docs/$doc and source (check entry is stale)"
  fi
}

for entry in "${CHECKS[@]}"; do
  IFS='|' read -r doc ident glob <<< "$entry"
  check "$doc" "$ident" "$glob"
done

# ---------------------------------------------------------------------------
# Invariant-citation checks
#
# Registry rows use `F<n>.<PredicateName>` identifiers. Source cites them with
# `// SAFETY: F<n>.<Name> ...` above an unsafe operation, or
# `// INVARIANT: F<n>.<Name> ...` where safe code relies on or maintains the
# invariant. Every marker must cite an existing predicate, and every predicate
# must be cited by at least one marker, so the registry is a complete
# inventory of the non-obvious facts the code relies on.
# ---------------------------------------------------------------------------
if [[ ! -f "$REGISTRY_PATH" ]]; then
  echo "[INVARIANT] missing docs/INVARIANT_REGISTRY.md"
  exit 1
fi

mapfile -t REGISTRY_PREDICATES < <(
  sed -nE 's/^\| `(F[1-9][0-9]*\.[A-Za-z][A-Za-z0-9]*)` \|.*/\1/p' "$REGISTRY_PATH"
)

if [[ ${#REGISTRY_PREDICATES[@]} -eq 0 ]]; then
  echo "[INVARIANT] no predicate rows found in docs/INVARIANT_REGISTRY.md"
  exit 1
fi

MARKER_RE='^[[:space:]]*// (SAFETY|INVARIANT):'

while IFS= read -r marker_site; do
  marker="SAFETY"
  [[ "$marker_site" == *"// INVARIANT:"* ]] && marker="INVARIANT"
  citation="${marker_site#*// $marker:}"
  if [[ ! "$citation" =~ F[1-9][0-9]*\.[A-Za-z][A-Za-z0-9]* ]]; then
    FAIL=$((FAIL + 1))
    echo "[INVARIANT] $marker comment lacks a registry citation: $marker_site"
  fi
done < <(grep -rnE --include='*.rs' --exclude-dir=target "$MARKER_RE" "${SAFETY_CRATE_ROOTS[@]}" 2>/dev/null || true)

mapfile -t CITED_PREDICATES < <(
  grep -rhE --include='*.rs' --exclude-dir=target "$MARKER_RE" "${SAFETY_CRATE_ROOTS[@]}" 2>/dev/null \
    | grep -oE 'F[1-9][0-9]*\.[A-Za-z][A-Za-z0-9]*' \
    | sort -u
)

for predicate in "${CITED_PREDICATES[@]}"; do
  if ! printf '%s\n' "${REGISTRY_PREDICATES[@]}" | grep -qxF "$predicate"; then
    FAIL=$((FAIL + 1))
    echo "[INVARIANT] source cites '$predicate', but it is absent from docs/INVARIANT_REGISTRY.md"
  else
    PASS=$((PASS + 1))
  fi
done

for predicate in "${REGISTRY_PREDICATES[@]}"; do
  if ! printf '%s\n' "${CITED_PREDICATES[@]}" | grep -qxF "$predicate"; then
    FAIL=$((FAIL + 1))
    echo "[INVARIANT] registry predicate '$predicate' has no SAFETY or INVARIANT citation"
  fi
done

# Only the harness's live, randomly named probe file can suppress recursion.
# In particular, an unrelated inherited DOC_DRIFT_NEGATIVE_PROBE value (such
# as `1`) must not silently skip the blocking negative test.
is_negative_probe_child() {
  local probe="${DOC_DRIFT_NEGATIVE_PROBE:-}"
  [[ "$probe" == "$REPO_ROOT"/crates/*/.check_doc_drift_negative.*.rs ]] &&
    [[ -f "$probe" ]]
}

# Exercise the real gate's negative invariant paths. The harness invokes this
# script with its live probe path, so that child invocation remains non-recursive.
if ! is_negative_probe_child; then
  if bash "$REPO_ROOT/scripts/tests/check_doc_drift_negative.sh"; then
    PASS=$((PASS + 1))
  else
    FAIL=$((FAIL + 1))
  fi
fi

echo ""
echo "doc-drift check: $PASS passed, $FAIL failed."

if [[ $FAIL -gt 0 ]]; then
  echo ""
  echo "Fix the mismatches above: reconcile source and docs. Update this script's"
  echo "check table after intentional renames."
  exit 1
fi
