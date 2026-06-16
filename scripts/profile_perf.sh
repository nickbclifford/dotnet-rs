#!/usr/bin/env bash

# Capture Linux perf traces for focused fixture or benchmark runs.
#
# The script builds the relevant Cargo target first, then profiles the produced
# test/bench executable directly so build work does not pollute perf.data.

set -euo pipefail

MODE=""
NAME=""
OUTPUT_DIR=""
FREQUENCY="3997"
# Call-graph mode: "fp" (default) or "dwarf[,size]".
#
# fp:   reliable at any kernel.perf_event_mlock_kb setting because frame-pointer
#       unwinding requires no auxiliary locked memory.  All Rust code is compiled
#       with -Cforce-frame-pointers=yes so our frames unwind cleanly.  The gap:
#       system libraries (libc, ld.so) are shipped WITHOUT frame pointers, so
#       any sample whose leaf instruction is inside libc shows up as "[unknown]"
#       in perf script output — the stack terminates at the libc boundary.
#       These frames are typically allocator internals (realloc, _int_malloc,
#       malloc_consolidate) called from Vec growth.  perf report resolves them
#       correctly because it uses DWARF; the Firefox Profiler / perf script path
#       does not.  Remaining "[unknown] ([unknown])" frames with ffffffffa…
#       addresses are PMU interrupt-skid artefacts (the hardware fires the counter
#       during a user→kernel transition); they require PEBS and cannot be resolved
#       with any call-graph flag.
#
# dwarf: walks DWARF .eh_frame/.debug_frame instead of frame pointers, so it
#        unwinds cleanly through libc and fully resolves the allocator frames.
#        Requires kernel.perf_event_mlock_kb ≥ 8192 (default is 516).
#        One-time setup (persists until next boot):
#          sudo sysctl kernel.perf_event_mlock_kb=65536
#        Or permanently via /etc/sysctl.d/99-perf.conf:
#          kernel.perf_event_mlock_kb = 65536
#        dwarf,8192 captures 8 KiB of user stack per sample — enough for
#        ~250 frames at 32 bytes/frame, well above dotnet-rs stack depth.
#        dwarf,16384 is an option for deeply recursive test cases.
#
# auto: (set below) picks dwarf,8192 when mlock_kb ≥ 8192, fp otherwise.
CALL_GRAPH="auto"
CARGO_PROFILE=""
FEATURES=""
NO_DEFAULT_FEATURES=0
SAMPLE_SIZE="30"
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
  --frequency <hz>        perf sample frequency (default: 3997)
  --call-graph <mode>     perf call graph mode (default: fp)
  --features <features>   Cargo features to enable
  --no-default-features   Disable Cargo default features
  --profile <profile>     Cargo profile (default: profiling — full debug info + bench-fat opts)
  --sample-size <n>       Criterion sample size for bench mode (default: 30)
  --extra-args [--] ...   Forward remaining args to the test/bench executable
  -h, --help              Show this message

Examples:
  $0 fixture --name system_text_json
  $0 bench --name json --sample-size 30
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
  local perf_inline_script="$OUTPUT_DIR/perf.inline.script"
  local folded="$OUTPUT_DIR/stacks.folded"
  local svg="$OUTPUT_DIR/flamegraph.svg"

  # `-F +pid` appends the pid field the Firefox Profiler importer requires.
  # `--no-inline` suppresses DWARF inline expansion: with fat LTO every physical
  # frame sits inside inlined code, so the default expansion produces only
  # `(inlined)` entries with no root frame to anchor them in Firefox Profiler.
  # DEBUGINFOD_URLS is set here but NOTE: `perf script` only uses the ELF symbol
  # table for name lookup, not DWARF; debuginfod does not help for libc internals
  # in this output.  Use `perf report` (which uses DWARF) for full resolution, or
  # recapture with --call-graph dwarf after setting mlock_kb ≥ 8192.
  if DEBUGINFOD_URLS="${DEBUGINFOD_URLS:-https://debuginfod.archlinux.org}" \
     perf script -i "$perf_data" -F +pid --no-inline > "$perf_script" 2> "$OUTPUT_DIR/perf-script.stderr"; then
    echo "[perf] wrote $perf_script (load directly in Firefox Profiler: https://profiler.firefox.com)"
  else
    echo "[perf] perf script failed; see $OUTPUT_DIR/perf-script.stderr" >&2
    return
  fi

  # Keep a second script with inline expansion enabled for richer folded stacks
  # and finer flamegraphs.
  if DEBUGINFOD_URLS="${DEBUGINFOD_URLS:-https://debuginfod.archlinux.org}" \
     perf script -i "$perf_data" -F +pid > "$perf_inline_script" 2> "$OUTPUT_DIR/perf-inline-script.stderr"; then
    echo "[perf] wrote $perf_inline_script (inline-expanded)"
  else
    echo "[perf] inline-expanded perf script failed; see $OUTPUT_DIR/perf-inline-script.stderr" >&2
    perf_inline_script="$perf_script"
  fi

  if command -v inferno-collapse-perf >/dev/null 2>&1; then
    inferno-collapse-perf "$perf_inline_script" > "$folded"
    echo "[perf] wrote $folded"

    if command -v inferno-flamegraph >/dev/null 2>&1; then
      inferno-flamegraph "$folded" > "$svg"
      echo "[perf] wrote $svg"
    fi
  elif command -v stackcollapse-perf.pl >/dev/null 2>&1; then
    stackcollapse-perf.pl "$perf_inline_script" > "$folded"
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

  # fp mode: 128-page (512 KiB) ring buffer is sufficient at 3997 Hz.
  # dwarf mode: 8192 bytes/sample × 299 Hz × ~6 threads ≈ 14 MB/s; a 256-page
  # (1 MiB) ring buffer holds ~70 ms of burst before the consumer catches up.
  # mlock budget: 2 × 1 MiB × ~6 CPUs = 12 MiB — well within 64 MiB budget.
  local mmap_pages=128
  if [[ "$CALL_GRAPH" == dwarf* ]]; then
    mmap_pages=256
  fi

  perf record \
    -o "$perf_data" \
    -F "$FREQUENCY" \
    -m "$mmap_pages" \
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

  # In DWARF mode, cap rayon's global pool to match what the production CLI does
  # via P1 (run_cli init).  The integration-test binary bypasses run_cli, so
  # without this all cores are used — multiplying the DWARF sample data rate by
  # 24× and causing massive ring buffer overflow.  Set here, not in the binary,
  # to avoid hardcoding a production-only env var in test code.
  if [[ "$CALL_GRAPH" == dwarf* ]]; then
    export RAYON_NUM_THREADS="${RAYON_NUM_THREADS:-4}"
  fi

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

# Resolve "auto" call-graph mode: use dwarf when mlock budget allows it.
if [[ "$CALL_GRAPH" == "auto" ]]; then
  mlock_kb="$(cat /proc/sys/kernel/perf_event_mlock_kb 2>/dev/null || echo 516)"
  if [[ "$mlock_kb" -ge 8192 ]]; then
    CALL_GRAPH="dwarf,8192"
    # At 3997 Hz, 8192 bytes/sample × 25 rayon+test threads ≈ 800 MB/s of DWARF
    # data — the ring buffer drains in milliseconds and 90%+ of samples are lost.
    # The default frequency only makes sense for fp mode (samples are ~200 bytes).
    # For DWARF, cap at 299 Hz: even with 25 threads that's ~60 MB/s, which a
    # 1 MB ring buffer (-m 256) handles for a 200 ms fixture run.
    # Users who pass --frequency explicitly get their value; we only override the
    # default of 3997, which is never appropriate for DWARF.
    if [[ "$FREQUENCY" == "3997" ]]; then
      FREQUENCY="299"
      echo "[perf] DWARF mode: auto-lowering frequency 3997→299 Hz to prevent ring buffer overflow"
      echo "[perf]   (use --frequency to override; ~60 samples per thread over a 200 ms fixture run)"
    fi
    echo "[perf] mlock_kb=${mlock_kb} — using call-graph dwarf,8192 (full libc symbol resolution)"
  else
    CALL_GRAPH="fp"
    echo "[perf] mlock_kb=${mlock_kb} < 8192 — using call-graph fp (libc internals will be [unknown])"
    echo "[perf] To enable DWARF: sudo sysctl kernel.perf_event_mlock_kb=65536"
  fi
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
