#!/usr/bin/env bash
# Keep the plan queue's status column synchronized with each plan's Status header.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PLANS_DIR="${1:-$REPO_ROOT/docs/plans}"
QUEUE_FILE="${2:-$PLANS_DIR/README.md}"

if [[ $# -gt 2 ]]; then
  echo "usage: $0 [plans-directory [queue-file]]" >&2
  exit 2
fi

PASS=0
FAIL=0

trim() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "$value"
}

canonical_status() {
  local status="$1"
  status="${status//\*\*/}"
  status="$(trim "$status")"
  status="$(printf '%s' "$status" | tr '[:upper:]' '[:lower:]')"

  case "$status" in
    complete*) printf 'complete' ;;
    not\ started*) printf 'not started' ;;
    in\ progress*) printf 'in progress' ;;
    parked*) printf 'parked' ;;
    *) return 1 ;;
  esac
}

if [[ ! -f "$QUEUE_FILE" ]]; then
  echo "[PLAN-QUEUE] missing queue file: $QUEUE_FILE"
  exit 1
fi

declare -A QUEUE_STATUS
declare -A QUEUE_ROW_COUNT

while IFS='|' read -r _ plan_id _ _ status _ _; do
  plan_id="$(trim "$plan_id")"
  [[ "$plan_id" =~ ^[0-9][0-9]$ ]] || continue

  QUEUE_STATUS["$plan_id"]="$(trim "$status")"
  QUEUE_ROW_COUNT["$plan_id"]=$(( ${QUEUE_ROW_COUNT["$plan_id"]:-0} + 1 ))
done < "$QUEUE_FILE"

declare -A SEEN_PLAN
shopt -s nullglob
plan_files=("$PLANS_DIR"/[0-9][0-9]-*.md)
shopt -u nullglob

if [[ ${#plan_files[@]} -eq 0 ]]; then
  echo "[PLAN-QUEUE] no numbered plan files found in $PLANS_DIR"
  exit 1
fi

for plan_file in "${plan_files[@]}"; do
  plan_name="$(basename "$plan_file")"
  plan_id="${plan_name%%-*}"
  SEEN_PLAN["$plan_id"]=1

  plan_status="$(
    awk '
      /^\*\*Status:\*\*/ {
        line = $0
        sub(/^\*\*Status:\*\*[[:space:]]*/, "", line)
        print line
        exit
      }
    ' "$plan_file"
  )"

  if [[ -z "$plan_status" ]]; then
    FAIL=$((FAIL + 1))
    echo "[PLAN-QUEUE] $plan_name has no canonical '**Status:**' header"
    continue
  fi

  if [[ -z "${QUEUE_STATUS[$plan_id]+present}" ]]; then
    FAIL=$((FAIL + 1))
    echo "[PLAN-QUEUE] $plan_name has no row in $(basename "$QUEUE_FILE")"
    continue
  fi

  if [[ ${QUEUE_ROW_COUNT[$plan_id]} -ne 1 ]]; then
    FAIL=$((FAIL + 1))
    echo "[PLAN-QUEUE] plan $plan_id has ${QUEUE_ROW_COUNT[$plan_id]} queue rows; expected exactly one"
    continue
  fi

  if ! plan_canonical="$(canonical_status "$plan_status")"; then
    FAIL=$((FAIL + 1))
    echo "[PLAN-QUEUE] $plan_name has an unrecognized status: $plan_status"
    continue
  fi

  if ! queue_canonical="$(canonical_status "${QUEUE_STATUS[$plan_id]}")"; then
    FAIL=$((FAIL + 1))
    echo "[PLAN-QUEUE] plan $plan_id has an unrecognized queue status: ${QUEUE_STATUS[$plan_id]}"
    continue
  fi

  if [[ "$plan_canonical" != "$queue_canonical" ]]; then
    FAIL=$((FAIL + 1))
    echo "[PLAN-QUEUE] plan $plan_id status mismatch: file='$plan_canonical', queue='$queue_canonical'"
    continue
  fi

  PASS=$((PASS + 1))
done

for plan_id in "${!QUEUE_STATUS[@]}"; do
  if [[ -z "${SEEN_PLAN[$plan_id]+present}" ]]; then
    FAIL=$((FAIL + 1))
    echo "[PLAN-QUEUE] queue row $plan_id has no matching numbered plan file"
  fi
done

echo "plan-queue check: $PASS passed, $FAIL failed."

if [[ $FAIL -gt 0 ]]; then
  exit 1
fi
