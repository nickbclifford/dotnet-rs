#!/usr/bin/env bash

# Capture Linux perf traces for focused fixture or benchmark runs.
#
# The script builds the relevant Cargo target first, then profiles the produced
# test/bench executable directly so build work does not pollute perf.data.

set -euo pipefail

MODE=""
NAME=""
OUTPUT_DIR=""
FREQUENCY="997"
# Frame pointer unwinding is reliable at the default kernel.perf_event_paranoid
# level (2) and doesn't require mlock budget. DWARF mode looks appealing but
# silently drops all stacks on the default mlock_kb=516 budget. Frame pointers
# work here because we compile with -Cforce-frame-pointers=yes.
CALL_GRAPH="fp"
CARGO_PROFILE=""
FEATURES=""
NO_DEFAULT_FEATURES=0
SAMPLE_SIZE="10"
EXTRA_ARGS=()

print_usage() {
  cat <<USAGE
Usage:
  $0 fixture [options] [-- <extra libtest args>]
  $0 bench [options] [-- <extra Criterion args>]

Modes:
  fixture                 Profile a generated dotnet-cli integration fixture test.
  bench                   Profile a dotnet-benchmarks Criterion case.

Common options:
  --name <name>           Fixture substring or benchmark case (default: system_text_json for fixture, json for bench)
  --output-dir <path>     Output directory (default: target/perf-traces/<mode>-<name>/<timestamp>)
  --frequency <hz>        perf sample frequency (default: 997)
  --call-graph <mode>     perf call graph mode (default: fp)
  --features <features>   Cargo features to enable
  --no-default-features   Disable Cargo default features
  --profile <profile>     Cargo profile (default: profiling — full debug info + bench-fat opts)
  --sample-size <n>       Criterion sample size for bench mode (default: 10)
  --extra-args [--] ...   Forward remaining args to the test/bench executable
  -h, --help              Show this message

Examples:
  $0 fixture --name system_text_json
  $0 bench --name json --sample-size 10
  $0 bench --name json -- --measurement-time 5
USAGE
}

fail() {
  echo "error: $*" >&2
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

parse_args() {
  [[ $# -gt 0 ]] || {
    print_usage >&2
    exit 2
  }

  MODE="$1"
  shift
  case "$MODE" in
    fixture|bench) ;;
    -h|--help)
      print_usage
      exit 0
      ;;
    *)
      fail "unknown mode: $MODE"
      ;;
  esac

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --name|--case)
        NAME="$2"
        shift 2
        ;;
      --output-dir)
        OUTPUT_DIR="$2"
        shift 2
        ;;
      --frequency)
        FREQUENCY="$2"
        shift 2
        ;;
      --call-graph)
        CALL_GRAPH="$2"
        shift 2
        ;;
      --features)
        FEATURES="$2"
        shift 2
        ;;
      --no-default-features)
        NO_DEFAULT_FEATURES=1
        shift
        ;;
      --profile)
        CARGO_PROFILE="$2"
        shift 2
        ;;
      --sample-size)
        SAMPLE_SIZE="$2"
        shift 2
        ;;
      --extra-args)
        shift
        if [[ $# -gt 0 && "$1" == "--" ]]; then
          shift
        fi
        EXTRA_ARGS=("$@")
        break
        ;;
      --)
        shift
        EXTRA_ARGS=("$@")
        break
        ;;
      -h|--help)
        print_usage
        exit 0
        ;;
      *)
        fail "unknown option: $1"
        ;;
    esac
  done
}

cargo_feature_args() {
  if [[ "$NO_DEFAULT_FEATURES" -eq 1 ]]; then
    printf '%s\n' "--no-default-features"
  fi
  if [[ -n "$FEATURES" ]]; then
    printf '%s\n' "--features"
    printf '%s\n' "$FEATURES"
  fi
}

ensure_tools() {
  command -v perf >/dev/null 2>&1 || fail "perf not found on PATH"
  command -v cargo >/dev/null 2>&1 || fail "cargo not found on PATH"
}

make_run_dir() {
  local label="$1"
  if [[ -z "$OUTPUT_DIR" ]]; then
    local stamp
    stamp="$(date +%Y%m%d-%H%M%S)"
    OUTPUT_DIR="target/perf-traces/${MODE}-${label}/${stamp}"
  fi
  mkdir -p "$OUTPUT_DIR"
}

extract_executable() {
  local json_file="$1"
  local pattern="$2"
  sed -n 's/.*"executable":"\([^"]*\)".*/\1/p' "$json_file" | grep "$pattern" | tail -n 1
}

write_metadata() {
  local exe="$1"
  local target_name="$2"
  local metadata="$OUTPUT_DIR/command.txt"

  {
    echo "mode: $MODE"
    echo "name: $target_name"
    echo "executable: $exe"
    echo "frequency: $FREQUENCY"
    echo "call_graph: $CALL_GRAPH"
    echo "features: ${FEATURES:-<default>}"
    echo "no_default_features: $NO_DEFAULT_FEATURES"
    echo "cargo_profile: ${CARGO_PROFILE:-<default>}"
    echo "rustflags: $(append_rustflag "${RUSTFLAGS:-}" "-Cforce-frame-pointers=yes")"
    echo "extra_args: ${EXTRA_ARGS[*]:-<none>}"
  } > "$metadata"
}

postprocess_perf_data() {
  local perf_data="$OUTPUT_DIR/perf.data"
  local perf_script="$OUTPUT_DIR/perf.script"
  local folded="$OUTPUT_DIR/stacks.folded"
  local svg="$OUTPUT_DIR/flamegraph.svg"

  # `-F +pid` appends the pid field the Firefox Profiler importer requires.
  # `--no-inline` suppresses DWARF inline expansion: with fat LTO every physical
  # frame sits inside inlined code, so the default expansion produces only
  # `(inlined)` entries with no root frame to anchor them in Firefox Profiler.
  # DEBUGINFOD_URLS fetches split debug info for system libraries (libc etc.)
  # via Arch Linux's public debuginfod server so their symbols resolve.
  if DEBUGINFOD_URLS="${DEBUGINFOD_URLS:-https://debuginfod.archlinux.org}" \
     perf script -i "$perf_data" -F +pid --no-inline > "$perf_script" 2> "$OUTPUT_DIR/perf-script.stderr"; then
    echo "[perf] wrote $perf_script (load directly in Firefox Profiler: https://profiler.firefox.com)"
  else
    echo "[perf] perf script failed; see $OUTPUT_DIR/perf-script.stderr" >&2
    return
  fi

  if command -v inferno-collapse-perf >/dev/null 2>&1; then
    inferno-collapse-perf "$perf_script" > "$folded"
    echo "[perf] wrote $folded"

    if command -v inferno-flamegraph >/dev/null 2>&1; then
      inferno-flamegraph "$folded" > "$svg"
      echo "[perf] wrote $svg"
    fi
  elif command -v stackcollapse-perf.pl >/dev/null 2>&1; then
    stackcollapse-perf.pl "$perf_script" > "$folded"
    echo "[perf] wrote $folded"

    if command -v flamegraph.pl >/dev/null 2>&1; then
      flamegraph.pl "$folded" > "$svg"
      echo "[perf] wrote $svg"
    fi
  fi
}

run_perf() {
  local exe="$1"
  shift
  local target_name="$1"
  shift
  local perf_data="$OUTPUT_DIR/perf.data"

  write_metadata "$exe" "$target_name"

  echo "[perf] output: $OUTPUT_DIR"
  echo "[perf] executable: $exe"
  echo "[perf] target: $target_name"

  perf record \
    -o "$perf_data" \
    -F "$FREQUENCY" \
    --call-graph "$CALL_GRAPH" \
    -- "$exe" "$@"

  echo "[perf] wrote $perf_data"
  postprocess_perf_data
}

run_fixture_mode() {
  local fixture_filter="${NAME:-system_text_json}"
  local cargo_json="$OUTPUT_DIR/cargo-test-no-run.json"
  local feature_args=()
  mapfile -t feature_args < <(cargo_feature_args)

  if [[ -z "$CARGO_PROFILE" ]]; then
    CARGO_PROFILE="profiling"
  fi

  local cargo_args=(
    test
    -p dotnet-cli
    --no-run
    --profile "$CARGO_PROFILE"
    --message-format=json
  )
  cargo_args+=("${feature_args[@]}")

  echo "[perf] building integration test binary"
  DOTNET_TEST_FILTER="$fixture_filter" \
  RUSTFLAGS="$(append_rustflag "${RUSTFLAGS:-}" "-Cforce-frame-pointers=yes")" \
  cargo "${cargo_args[@]}" > "$cargo_json"

  local exe
  exe="$(extract_executable "$cargo_json" "integration_tests")"
  [[ -n "$exe" && -x "$exe" ]] || fail "could not discover integration_tests executable from $cargo_json"

  local tests
  tests="$("$exe" --list | sed 's/: test$//' | grep "$fixture_filter" || true)"
  [[ -n "$tests" ]] || fail "no generated tests matched '$fixture_filter'"

  local count
  count="$(printf '%s\n' "$tests" | sed '/^$/d' | wc -l | tr -d ' ')"
  [[ "$count" == "1" ]] || fail "expected one test matching '$fixture_filter', found $count: $tests"

  local test_name
  test_name="$tests"

  run_perf "$exe" "$test_name" "$test_name" --exact --nocapture --test-threads=1 "${EXTRA_ARGS[@]}"
}

run_bench_mode() {
  local bench_case="${NAME:-json}"
  local cargo_json="$OUTPUT_DIR/cargo-bench-no-run.json"
  local feature_args=()
  mapfile -t feature_args < <(cargo_feature_args)

  if [[ -z "$CARGO_PROFILE" ]]; then
    CARGO_PROFILE="profiling"
  fi

  echo "[perf] building benchmark binary"
  RUSTFLAGS="$(append_rustflag "${RUSTFLAGS:-}" "-Cforce-frame-pointers=yes")" \
  cargo bench \
    --profile "$CARGO_PROFILE" \
    -p dotnet-benchmarks \
    --bench end_to_end \
    --no-run \
    --message-format=json \
    "${feature_args[@]}" > "$cargo_json"

  local exe
  exe="$(extract_executable "$cargo_json" "end_to_end")"
  [[ -n "$exe" && -x "$exe" ]] || fail "could not discover end_to_end benchmark executable from $cargo_json"

  run_perf "$exe" "$bench_case" "$bench_case" --sample-size "$SAMPLE_SIZE" "${EXTRA_ARGS[@]}"
}

parse_args "$@"
ensure_tools
if ! [[ "$FREQUENCY" =~ ^[0-9]+$ ]]; then
  fail "--frequency must be an integer"
fi
if ! [[ "$SAMPLE_SIZE" =~ ^[0-9]+$ ]]; then
  fail "--sample-size must be an integer"
fi

DEFAULT_LABEL="${NAME:-}"
if [[ -z "$DEFAULT_LABEL" ]]; then
  if [[ "$MODE" == "fixture" ]]; then
    DEFAULT_LABEL="system_text_json"
  else
    DEFAULT_LABEL="json"
  fi
fi
make_run_dir "$DEFAULT_LABEL"

case "$MODE" in
  fixture) run_fixture_mode ;;
  bench) run_bench_mode ;;
esac
