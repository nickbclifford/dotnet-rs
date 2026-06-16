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
#       system libraries (libc, ld.so) are shipped WITHOUT frame pointers, so when
#       a sample's leaf is inside libc the unwind STOPS at the libc boundary and
#       loses the Rust callers above it (you lose "who called malloc").  Use dwarf
#       when allocator attribution matters.  Remaining "[unknown] ([unknown])"
#       frames with ffffffffa… addresses are PMU interrupt-skid artefacts (the
#       hardware fires the counter during a user→kernel transition); they require
#       PEBS and cannot be resolved with any call-graph flag.
#
# NOTE on libc *symbol names* (a separate axis from unwinding): libc's internal
#      functions (_int_malloc, __memmove_avx512_unaligned_erms, …) live only in
#      the debuginfo .symtab, not the stripped on-disk .dynsym.  perf never pulls
#      those names from debuginfod — NOT in `perf script` and NOT even in `perf
#      report` — so they print as "[unknown]" or a raw offset regardless of call
#      graph mode.  resolve_perf_syms.py (run in postprocess) closes that gap with
#      +dsoff + addr2line against the debuginfod-cached debug file.
#
# dwarf: walks DWARF .eh_frame/.debug_frame instead of frame pointers, so it
#        unwinds cleanly through libc and keeps the allocator's Rust callers.
#        Requires kernel.perf_event_mlock_kb ≥ 8192 (default is 516).
#        One-time setup (persists until next boot):
#          sudo sysctl kernel.perf_event_mlock_kb=65536
#        Or permanently via /etc/sysctl.d/99-perf.conf:
#          kernel.perf_event_mlock_kb = 65536
#        dwarf,8192 captures 8 KiB of user stack per sample — enough for
#        ~250 frames at 32 bytes/frame, well above dotnet-rs stack depth.
#        dwarf,16384 is an option for deeply recursive test cases.
#
# auto: (set below) picks fp — dense and overflow-free, with libc leaf names recovered in
#       post-processing.  dwarf is opt-in (--call-graph dwarf,8192) for callchains through libc.
CALL_GRAPH="auto"
CARGO_PROFILE=""
FEATURES=""
NO_DEFAULT_FEATURES=0
SAMPLE_SIZE="30"
PREWARM=1
# Capture backend: "auto" (default), "perf", or "samply".
#   samply: frame-pointer sampling profiler with a native Firefox Profiler backend.  The build
#           sets -Cforce-frame-pointers=yes, so it unwinds the full stack cheaply at every
#           sample — far denser captures than perf's throttled DWARF path, with near-zero empty
#           stacks, and it symbolicates libc/system libs via debuginfod itself.  Needs
#           kernel.perf_event_paranoid <= 1.
#   perf:   perf record → perf script → resolve_perf_syms.py → inferno.  Works at paranoid 2;
#           produces a self-contained perf.script text dump and a perf.data for other tools.
# auto picks samply when it is installed and paranoid allows it, else perf.
BACKEND="auto"
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
  --backend <name>        Capture backend: auto (default), perf, or samply. auto prefers
                          samply (denser, fully-symbolicated, native Firefox Profiler) when
                          it is installed and kernel.perf_event_paranoid <= 1, else perf.
  --no-prewarm            Skip the untraced warm-up run (which builds the fixture DLL and
                          warms caches OUTSIDE the trace; without it, a first-time `dotnet
                          build` subprocess pollutes the capture — usually keep it on)
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
      --backend)
        BACKEND="$2"
        shift 2
        ;;
      --no-prewarm)
        PREWARM=0
        shift
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
  command -v cargo >/dev/null 2>&1 || fail "cargo not found on PATH"
  if [[ "$BACKEND" == "samply" ]]; then
    command -v samply >/dev/null 2>&1 || fail "samply not found on PATH (cargo install samply)"
  else
    command -v perf >/dev/null 2>&1 || fail "perf not found on PATH"
  fi
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

# Emit `perf script` for $1 (perf.data) with the fields we need, falling back gracefully on
# older perf that lacks `+dsoff`.  Remaining args (e.g. --no-inline) are forwarded.
#  +pid    — the pid field the Firefox Profiler importer requires.
#  +dsoff  — each frame's DSO-relative offset, so resolve_perf_syms.py can recover stripped
#            system-library symbol names (libc/ld.so internals) from debuginfod.  perf itself
#            never pulls those names from debuginfod — not in `perf script` and not even in
#            `perf report` — so without this they stay `[unknown]`.
emit_perf_script() {
  local data="$1"
  shift
  DEBUGINFOD_URLS="${DEBUGINFOD_URLS:-https://debuginfod.archlinux.org}" \
    perf script -i "$data" -F +pid,+dsoff "$@" 2>/dev/null && return 0
  DEBUGINFOD_URLS="${DEBUGINFOD_URLS:-https://debuginfod.archlinux.org}" \
    perf script -i "$data" -F +pid "$@"
}

postprocess_perf_data() {
  local perf_data="$OUTPUT_DIR/perf.data"
  local perf_script="$OUTPUT_DIR/perf.script"
  local perf_inline_script="$OUTPUT_DIR/perf.inline.script"
  local folded="$OUTPUT_DIR/stacks.folded"
  local svg="$OUTPUT_DIR/flamegraph.svg"
  local resolver
  resolver="$(dirname "$0")/resolve_perf_syms.py"

  # Resolution + quality reporting is a best-effort filter: if python3 or the resolver is
  # missing we fall back to writing raw `perf script` output unchanged.
  local resolve=("cat")
  if command -v python3 >/dev/null 2>&1 && [[ -f "$resolver" ]]; then
    resolve=(python3 "$resolver" --perf-data "$perf_data" --target-comm "${TARGET_COMM:-}")
  fi

  # `--no-inline` keeps one physical frame per line: with fat LTO every physical frame sits
  # inside inlined code, so the default expansion produces only `(inlined)` entries with no
  # root frame to anchor them in Firefox Profiler.  Stats (the [quality] lines) are printed
  # to the terminal from this pass.
  if emit_perf_script "$perf_data" --no-inline | "${resolve[@]}" > "$perf_script" 2> "$OUTPUT_DIR/quality.txt"; then
    [[ -s "$OUTPUT_DIR/quality.txt" ]] && cat "$OUTPUT_DIR/quality.txt" >&2
    echo "[perf] wrote $perf_script (symbol-resolved; load directly in Firefox Profiler: https://profiler.firefox.com)"
  else
    echo "[perf] perf script failed" >&2
    return
  fi

  # Second script with inline expansion enabled for richer folded stacks and finer
  # flamegraphs.  Same resolution; quality summary suppressed (identical to the pass above).
  if emit_perf_script "$perf_data" | "${resolve[@]}" 2>/dev/null > "$perf_inline_script"; then
    echo "[perf] wrote $perf_inline_script (inline-expanded)"
  else
    echo "[perf] inline-expanded perf script failed" >&2
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
  else
    echo "[perf] no flamegraph tool found — install one for an SVG: cargo install inferno"
    echo "[perf]   (perf.script already loads in the Firefox Profiler without it)"
  fi
}

run_perf() {
  local exe="$1"
  shift
  local target_name="$1"
  shift
  local perf_data="$OUTPUT_DIR/perf.data"

  # The process comm perf records; resolve_perf_syms.py uses it to flag samples that landed
  # in OTHER processes (subprocess pollution).  perf truncates comm to 15 chars.
  TARGET_COMM="$(basename "$exe")"

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

run_samply() {
  local exe="$1"
  shift
  local target_name="$1"
  shift
  local profile="$OUTPUT_DIR/profile.json.gz"

  TARGET_COMM="$(basename "$exe")"
  write_metadata "$exe" "$target_name"

  echo "[samply] output: $OUTPUT_DIR"
  echo "[samply] executable: $exe"
  echo "[samply] target: $target_name"

  # samply unwinds with frame pointers (the build sets -Cforce-frame-pointers=yes), so it
  # needs no call-graph / mlock / ring-buffer tuning and short-lived threads still unwind
  # cleanly.  --save-only writes a self-contained profile; symbolication (binary debuginfo +
  # libc/system libs via debuginfod) happens lazily when the profile is loaded in the UI.
  DEBUGINFOD_URLS="${DEBUGINFOD_URLS:-https://debuginfod.archlinux.org}" \
    samply record --save-only -o "$profile" --rate "$FREQUENCY" -- "$exe" "$@"

  echo "[samply] wrote $profile"
  echo "[samply] view it (opens the Firefox Profiler, symbolicates on load): samply load $profile"
}

# Dispatch to the selected capture backend.  Both implementations take (exe, target_name, args…).
run_capture() {
  if [[ "$BACKEND" == "samply" ]]; then
    run_samply "$@"
  else
    run_perf "$@"
  fi
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

  # Warm up untraced: materialize on-demand fixture DLLs and warm the file cache so the
  # traced run does not pay (and record) a one-time `dotnet build` or cold-cache I/O.
  if [[ "$PREWARM" -eq 1 ]]; then
    echo "[profile] pre-warming (untraced) so fixture build / cold I/O stays out of the trace"
    "$exe" "$test_name" --exact --nocapture --test-threads=1 "${EXTRA_ARGS[@]}" >/dev/null 2>&1 || true
  fi

  run_capture "$exe" "$test_name" "$test_name" --exact --nocapture --test-threads=1 "${EXTRA_ARGS[@]}"
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

  # Warm up untraced.  The harness lazily runs a one-time `dotnet build` to compile the
  # fixture .cs into a DLL on the first iteration; if that lands inside `perf record` the
  # whole MSBuild/Roslyn/CoreCLR process tree is captured (dozens of unwindable managed
  # threads) and drowns the actual interpreter signal.  Running the case briefly here builds
  # the DLL to its on-disk cache so the traced run finds it fresh and never shells out.
  if [[ "$PREWARM" -eq 1 ]]; then
    echo "[profile] pre-warming (untraced) so the fixture's dotnet build stays out of the trace"
    "$exe" "$bench_case" --warm-up-time 0.3 --measurement-time 0.3 --sample-size 10 >/dev/null 2>&1 || true
  fi

  # `--bench` is essential: run directly without it, a Criterion binary executes in libtest
  # "test mode" (one iteration per case, to support `cargo test`) and barely runs the workload
  # — yielding a sparse, startup-dominated trace.  `cargo bench` passes `--bench` for us; when
  # we exec the binary ourselves we must pass it explicitly to get the real measurement loop.
  run_capture "$exe" "$bench_case" --bench "$bench_case" --sample-size "$SAMPLE_SIZE" "${EXTRA_ARGS[@]}"
}

parse_args "$@"
case "$BACKEND" in
  auto|perf|samply) ;;
  *) fail "unknown backend: $BACKEND (expected auto, perf, or samply)" ;;
esac
if ! [[ "$FREQUENCY" =~ ^[0-9]+$ ]]; then
  fail "--frequency must be an integer"
fi
if ! [[ "$SAMPLE_SIZE" =~ ^[0-9]+$ ]]; then
  fail "--sample-size must be an integer"
fi

# Resolve "auto" backend before ensure_tools so we verify the recorder we will actually use.
# Prefer samply (frame-pointer unwinding → far denser, fully-symbolicated captures) when it is
# installed and perf_event_paranoid allows it; otherwise fall back to the perf path.
if [[ "$BACKEND" == "auto" ]]; then
  paranoid="$(cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null || echo 2)"
  if command -v samply >/dev/null 2>&1 && [[ "$paranoid" =~ ^-?[0-9]+$ && "$paranoid" -le 1 ]]; then
    BACKEND="samply"
    echo "[profile] backend=samply (frame-pointer unwinding, native Firefox Profiler; paranoid=$paranoid)"
  else
    BACKEND="perf"
    if command -v samply >/dev/null 2>&1; then
      echo "[profile] backend=perf — samply needs kernel.perf_event_paranoid<=1 (currently ${paranoid})"
      echo "[profile]   make it permanent: echo 'kernel.perf_event_paranoid=1' | sudo tee /etc/sysctl.d/99-perf.conf"
    else
      echo "[profile] backend=perf — install samply for a denser, fully-symbolicated capture: cargo install samply"
    fi
  fi
fi

ensure_tools

# Call-graph resolution (perf backend only).
if [[ "$BACKEND" == "perf" ]]; then
  # auto → fp.  Frame-pointer unwinding is the right default: the build sets
  # -Cforce-frame-pointers=yes so every dotnet-rs frame unwinds, fp samples are tiny (~200 B)
  # so the ring buffer never overflows even on a dense bench (measured 0% empty stacks at
  # 3997 Hz), and resolve_perf_syms.py names libc/system leaves afterward.  DWARF only adds the
  # ability to unwind the Rust callers ABOVE a libc-internal leaf, but its 8 KiB/sample stack
  # dumps force a low rate and overflow on dense workloads — measured 80% empty / 87%
  # unresolved on `bench json`.  So DWARF is opt-in, not the default.
  if [[ "$CALL_GRAPH" == "auto" ]]; then
    CALL_GRAPH="fp"
    echo "[perf] call-graph=fp (dense, no ring-buffer loss; libc leaves named in post-processing)"
    echo "[perf]   (pass --call-graph dwarf,8192 to unwind callchains THROUGH libc — much lower density)"
  fi
  # When DWARF is explicitly requested, keep it usable: it needs a large mlock budget and a low
  # sample rate or it drops most stacks.
  if [[ "$CALL_GRAPH" == dwarf* ]]; then
    mlock_kb="$(cat /proc/sys/kernel/perf_event_mlock_kb 2>/dev/null || echo 516)"
    if [[ "$mlock_kb" -lt 8192 ]]; then
      echo "[perf] WARNING: DWARF needs kernel.perf_event_mlock_kb>=8192 (currently ${mlock_kb}); stacks will be truncated"
      echo "[perf]   sudo sysctl kernel.perf_event_mlock_kb=65536"
    fi
    if [[ "$FREQUENCY" == "3997" ]]; then
      FREQUENCY="299"
      echo "[perf] DWARF mode: lowering frequency 3997→299 Hz to limit ring-buffer overflow (--frequency overrides)"
    fi
    echo "[perf] call-graph=${CALL_GRAPH} — note: still expect heavy sample loss on dense workloads"
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
