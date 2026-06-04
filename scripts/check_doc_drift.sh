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

# ---------------------------------------------------------------------------
# Check table
# Format: "DOC_FILE|IDENTIFIER|SOURCE_GLOB"
#   DOC_FILE    — path relative to DOCS_DIR
#   IDENTIFIER  — literal string that must appear in the doc AND in source
#   SOURCE_GLOB — glob passed to grep -r --include to narrow the source search
# ---------------------------------------------------------------------------
CHECKS=(
  # --- GC_AND_MEMORY_SAFETY.md ---
  "GC_AND_MEMORY_SAFETY.md|CollectionSession|*.rs"
  "GC_AND_MEMORY_SAFETY.md|GCCoordinator|*.rs"
  "GC_AND_MEMORY_SAFETY.md|begin_collection|*.rs"
  "GC_AND_MEMORY_SAFETY.md|MarkPhaseCommand|*.rs"
  "GC_AND_MEMORY_SAFETY.md|SweepPhaseCommand|*.rs"
  "GC_AND_MEMORY_SAFETY.md|MarkObjectPointers|*.rs"
  "GC_AND_MEMORY_SAFETY.md|GcLifetime|*.rs"
  "GC_AND_MEMORY_SAFETY.md|MemoryOwner|*.rs"
  "GC_AND_MEMORY_SAFETY.md|ThreadSafeLock|*.rs"
  "GC_AND_MEMORY_SAFETY.md|WriteBarrierPanicFlushGuard|*.rs"
  "GC_AND_MEMORY_SAFETY.md|validate_magic|*.rs"
  "GC_AND_MEMORY_SAFETY.md|validate_arena_id|*.rs"

  # --- THREADING_AND_SYNCHRONIZATION.md ---
  "THREADING_AND_SYNCHRONIZATION.md|StopTheWorldGuard|*.rs"
  "THREADING_AND_SYNCHRONIZATION.md|GcScopeGuard|*.rs"
  "THREADING_AND_SYNCHRONIZATION.md|CommandCompletionGuard|*.rs"
  "THREADING_AND_SYNCHRONIZATION.md|ResumeOnPanic|*.rs"
  "THREADING_AND_SYNCHRONIZATION.md|SyncBlockManager|*.rs"
  "THREADING_AND_SYNCHRONIZATION.md|TRACER_CHANNEL_CAPACITY|*.rs"
  "THREADING_AND_SYNCHRONIZATION.md|LockResult|*.rs"
  "THREADING_AND_SYNCHRONIZATION.md|ThreadManagerOps|*.rs"
  "THREADING_AND_SYNCHRONIZATION.md|SyncBlockOps|*.rs"
  "THREADING_AND_SYNCHRONIZATION.md|execute_gc_command_for_current_thread|*.rs"
  "THREADING_AND_SYNCHRONIZATION.md|record_found_cross_arena_refs|*.rs"

  # --- BUILD_TIME_CODE_GENERATION.md ---
  "BUILD_TIME_CODE_GENERATION.md|process_instruction_file|*.rs"
  "BUILD_TIME_CODE_GENERATION.md|generate_instruction_table|*.rs"
  "BUILD_TIME_CODE_GENERATION.md|process_intrinsic_file|*.rs"
  "BUILD_TIME_CODE_GENERATION.md|generate_intrinsic_phf|*.rs"
  "BUILD_TIME_CODE_GENERATION.md|ParsedSignature|*.rs"

  # --- FUZZING.md ---
  "FUZZING.md|FuzzProgram|*.rs"
  "FUZZING.md|FuzzInstruction|*.rs"
  "FUZZING.md|execute_cil_program|*.rs"
  "FUZZING.md|ManagedPtrInfo|*.rs"
  "FUZZING.md|AtomicAccess|*.rs"

  # --- EXCEPTION_HANDLING.md ---
  "EXCEPTION_HANDLING.md|ExceptionHandlingSystem|*.rs"
  "EXCEPTION_HANDLING.md|ManagedException|*.rs"
  "EXCEPTION_HANDLING.md|SearchState|*.rs"
  "EXCEPTION_HANDLING.md|UnwindState|*.rs"
  "EXCEPTION_HANDLING.md|ExceptionState|*.rs"

  # --- DELEGATES_AND_DISPATCH.md ---
  "DELEGATES_AND_DISPATCH.md|MulticastState|*.rs"
  "DELEGATES_AND_DISPATCH.md|try_delegate_dispatch|*.rs"
  "DELEGATES_AND_DISPATCH.md|unified_dispatch|*.rs"

  # --- TYPE_RESOLUTION_AND_CACHING.md ---
  "TYPE_RESOLUTION_AND_CACHING.md|GlobalCaches|*.rs"
  "TYPE_RESOLUTION_AND_CACHING.md|ResolutionContext|*.rs"
  "TYPE_RESOLUTION_AND_CACHING.md|GenericLookup|*.rs"
  "TYPE_RESOLUTION_AND_CACHING.md|StaticStorageManager|*.rs"
  "TYPE_RESOLUTION_AND_CACHING.md|VmLayoutFactory|*.rs"

  # --- ARCHITECTURE.md ---
  "ARCHITECTURE.md|dotnet-vm|*.toml"
  "ARCHITECTURE.md|dotnet-utils|*.toml"
  "ARCHITECTURE.md|multithreading|*.toml"
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

echo ""
echo "doc-drift check: $PASS passed, $FAIL failed."

if [[ $FAIL -gt 0 ]]; then
  echo ""
  echo "Fix the mismatches above: update the doc to match the current identifier name,"
  echo "or update the check table in scripts/check_doc_drift.sh if a rename was intentional."
  exit 1
fi
