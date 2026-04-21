#!/usr/bin/env bash

# Optional two-pass PGO workflow for benchmark binaries.
# Pass 1: compile+run benchmarks with -Cprofile-generate
# Pass 2: compile+run with -Cprofile-use from merged profdata

set -euo pipefail

BENCH_PROFILE="bench-fat"
PACKAGE="dotnet-benchmarks"
BENCH_NAME="end_to_end"
SAMPLE_SIZE="10"
TARGET_DIR="target/pgo-bench"
RUN_TESTS=1

print_usage() {
  cat <<USAGE
Usage: $0 [options] [-- <extra criterion args>]

Options:
  --profile <name>       Cargo profile for bench runs (default: bench-fat)
  --package <name>       Cargo package to benchmark (default: dotnet-benchmarks)
  --bench <name>         Bench target name (default: end_to_end)
  --sample-size <n>      Criterion sample size for both passes (default: 10)
  --target-dir <path>    Cargo target directory for PGO artifacts (default: target/pgo-bench)
  --no-tests             Skip PGO-use validation test build step
  -h, --help             Show this message

Examples:
  $0
  $0 --profile bench-thin --sample-size 20
  $0 -- --measurement-time 5
USAGE
}

EXTRA_CRITERION_ARGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      BENCH_PROFILE="$2"
      shift 2
      ;;
    --package)
      PACKAGE="$2"
      shift 2
      ;;
    --bench)
      BENCH_NAME="$2"
      shift 2
      ;;
    --sample-size)
      SAMPLE_SIZE="$2"
      shift 2
      ;;
    --target-dir)
      TARGET_DIR="$2"
      shift 2
      ;;
    --no-tests)
      RUN_TESTS=0
      shift
      ;;
    -h|--help)
      print_usage
      exit 0
      ;;
    --)
      shift
      EXTRA_CRITERION_ARGS=("$@")
      break
      ;;
    *)
      echo "Unknown option: $1" >&2
      print_usage >&2
      exit 1
      ;;
  esac
done

if ! [[ "$SAMPLE_SIZE" =~ ^[0-9]+$ ]]; then
  echo "error: --sample-size must be an integer (got: $SAMPLE_SIZE)" >&2
  exit 1
fi

if (( SAMPLE_SIZE < 10 )); then
  echo "error: --sample-size must be >= 10 for Criterion compatibility (got: $SAMPLE_SIZE)" >&2
  exit 1
fi

find_llvm_profdata() {
  if command -v rustup >/dev/null 2>&1; then
    local sysroot host rustup_profdata
    sysroot="$(rustc --print sysroot)"
    host="$(rustc -vV | sed -n 's/^host: //p')"
    rustup_profdata="$sysroot/lib/rustlib/$host/bin/llvm-profdata"
    if [[ -x "$rustup_profdata" ]]; then
      echo "$rustup_profdata"
      return
    fi

    echo "" >&2
    echo "error: llvm-profdata for the active Rust toolchain was not found." >&2
    echo "Install it with: rustup component add llvm-tools-preview" >&2
    exit 1
  fi

  if command -v llvm-profdata >/dev/null 2>&1; then
    command -v llvm-profdata
    return
  fi

  echo "" >&2
  echo "error: could not find llvm-profdata on PATH or via rustup." >&2
  echo "Install it with rustup (default for official toolchains) or add it to PATH." >&2
  exit 1
}

append_rustflag() {
  local current="$1"
  local extra="$2"
  if [[ -n "$current" ]]; then
    printf '%s %s' "$current" "$extra"
  else
    printf '%s' "$extra"
  fi
}

run_bench_pass() {
  local rustflags="$1"
  CARGO_TARGET_DIR="$TARGET_DIR" \
  RUSTFLAGS="$rustflags" \
  cargo bench \
    --profile "$BENCH_PROFILE" \
    -p "$PACKAGE" \
    --bench "$BENCH_NAME" \
    -- \
    --sample-size "$SAMPLE_SIZE" \
    "${EXTRA_CRITERION_ARGS[@]}"
}

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PGO_ROOT="$ROOT_DIR/$TARGET_DIR/pgo"
RAW_DIR="$PGO_ROOT/raw"
MERGED_PROFDATA="$PGO_ROOT/merged.profdata"
LLVM_PROFDATA_BIN="$(find_llvm_profdata)"

mkdir -p "$RAW_DIR"
rm -f "$RAW_DIR"/*.profraw "$MERGED_PROFDATA"

echo "[pgo] Pass 1/2: collecting profiles"
echo "[pgo] benchmark profile: $BENCH_PROFILE"
echo "[pgo] benchmark target:  $PACKAGE::$BENCH_NAME"
echo "[pgo] criterion sample-size: $SAMPLE_SIZE"
echo "[pgo] llvm-profdata: $LLVM_PROFDATA_BIN"
echo "[pgo] raw profile dir: $RAW_DIR"

GEN_RUSTFLAGS="$(append_rustflag "${RUSTFLAGS:-}" "-Cprofile-generate=$RAW_DIR")"
run_bench_pass "$GEN_RUSTFLAGS"

mapfile -t PROFRAW_FILES < <(find "$RAW_DIR" -type f -name '*.profraw' | sort)
if [[ "${#PROFRAW_FILES[@]}" -eq 0 ]]; then
  echo "error: no .profraw files generated in $RAW_DIR" >&2
  exit 1
fi

echo "[pgo] Merging ${#PROFRAW_FILES[@]} profile files"
"$LLVM_PROFDATA_BIN" merge -o "$MERGED_PROFDATA" "${PROFRAW_FILES[@]}"

if [[ ! -f "$MERGED_PROFDATA" ]]; then
  echo "error: expected merged profile at $MERGED_PROFDATA" >&2
  exit 1
fi

USE_RUSTFLAGS="$(append_rustflag "${RUSTFLAGS:-}" "-Cprofile-use=$MERGED_PROFDATA")"

if [[ "$RUN_TESTS" -eq 1 ]]; then
  echo "[pgo] Validating PGO-use build/test path"
  CARGO_TARGET_DIR="$TARGET_DIR" \
  RUSTFLAGS="$USE_RUSTFLAGS" \
  cargo test -p "$PACKAGE" --no-run
fi

echo "[pgo] Pass 2/2: running benchmark with merged profile"
run_bench_pass "$USE_RUSTFLAGS"

echo "[pgo] Completed successfully"
echo "[pgo] merged profile: $MERGED_PROFDATA"
