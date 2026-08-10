#!/usr/bin/env fish

# Capture one machine-readable perf trace per end_to_end benchmark.  Positional arguments select
# cases; without them the complete end_to_end suite is captured.

function usage
    printf '%s\n' \
        'Usage: ./bench.fish [options] [benchmark ...]' \
        '' \
        'Options:' \
        '  -o, --output-dir DIR       Trace corpus directory (must be new unless --resume)' \
        '  -b, --backend NAME         perf (default), samply, or auto' \
        '  -f, --frequency HZ         Sampling frequency for normal-duration cases (default: 999)' \
        '      --slow-frequency HZ    Sampling frequency for slow cases (default: 199)' \
        '      --call-graph MODE      perf call graph mode (default: fp)' \
        '      --rayon-threads N      Rayon workers for metadata decoding (default: 4)' \
        '  -s, --sample-size N        Criterion sample size (default: 10)' \
        '      --standard-sample-size N  Samples for an optional clean standard baseline (default: 10)' \
        '  -p, --profile NAME         Cargo profile (default: profiling)' \
        '      --features LIST        Cargo feature list passed unchanged to Cargo' \
        '      --no-default-features  Disable Cargo default features' \
        '      --min-free-kb N        Stop starting cases below this free-space floor (default: 2097152)' \
        '      --resume               Skip cases already marked ok in runs.tsv' \
        '      --standard-baseline NAME  Archive target/criterion and save a fresh bench-fat baseline' \
        '  -h, --help                 Show this help' \
        '' \
        'Environment equivalents: TRACE_ROOT, TRACE_BACKEND, TRACE_STANDARD_FREQUENCY,' \
        'TRACE_SLOW_FREQUENCY, TRACE_CALL_GRAPH, TRACE_RAYON_THREADS, TRACE_SAMPLE_SIZE,' \
        'TRACE_STANDARD_SAMPLE_SIZE, TRACE_STANDARD_BASELINE, TRACE_PROFILE, TRACE_FEATURES,' \
        'TRACE_NO_DEFAULT_FEATURES, TRACE_MIN_FREE_KB, TRACE_RESUME, TRACE_BENCHES,' \
        'and TRACE_SLOW_BENCHES (the two benchmark-list variables are comma-separated).'
end

function require_positive_integer
    set -l label $argv[1]
    set -l value $argv[2]
    if not string match -qr '^[1-9][0-9]*$' -- "$value"
        printf 'error: %s must be a positive integer; got %s\n' "$label" "$value" >&2
        exit 2
    end
end

function already_completed
    set -l benchmark $argv[1]
    set -l run_index $argv[2]
    command awk -F '\t' -v target="$benchmark" '
        NR == 1 {
            for (i = 1; i <= NF; i++) column[$i] = i
            benchmark_column = column["benchmark"]
            result_column = column["result"]
            next
        }
        benchmark_column && result_column && $benchmark_column == target && $result_column == "ok" {
            found = 1
        }
        END { exit(found ? 0 : 1) }
    ' "$run_index"
end

function available_kb
    command df -Pk "$argv[1]" | command awk 'NR == 2 { print $4 }'
end

function migrate_run_index
    set -l run_index $argv[1]
    set -l temporary_index (mktemp "$run_index.XXXXXX"); or return 1
    command awk -F '\t' '
        BEGIN {
            OFS = "\t"
            print "benchmark", "frequency_hz", "sample_size", "started_at", "finished_at", "result", "profile_exit_code", "log_exit_code", "trace_dir", "quality_file", "workflow_log"
        }
        NR == 1 {
            for (i = 1; i <= NF; i++) column[$i] = i
            next
        }
        {
            profile_exit = column["profile_exit_code"] ? $column["profile_exit_code"] : (column["exit_code"] ? $column["exit_code"] : "-")
            log_exit = column["log_exit_code"] ? $column["log_exit_code"] : "-"
            print \
                $column["benchmark"], \
                $column["frequency_hz"], \
                $column["sample_size"], \
                $column["started_at"], \
                $column["finished_at"], \
                $column["result"], \
                profile_exit, \
                log_exit, \
                $column["trace_dir"], \
                $column["quality_file"], \
                $column["workflow_log"]
        }
    ' "$run_index" > "$temporary_index"
    or begin
        rm -f "$temporary_index"
        return 1
    end
    command mv "$temporary_index" "$run_index"
end

argparse -n bench.fish \
    'h/help' \
    'o/output-dir=' \
    'b/backend=' \
    'f/frequency=' \
    'slow-frequency=' \
    'call-graph=' \
    'rayon-threads=' \
    's/sample-size=' \
    'standard-sample-size=' \
    'p/profile=' \
    'features=' \
    'no-default-features' \
    'min-free-kb=' \
    'resume' \
    'standard-baseline=' \
    -- $argv
or begin
    usage >&2
    exit 2
end

if set -q _flag_help
    usage
    exit 0
end

set -l script_dir (cd (dirname (status filename)); and pwd -P)
if not command git -C "$script_dir" rev-parse --show-toplevel >/dev/null 2>&1
    echo 'error: bench.fish must run from a Git worktree' >&2
    exit 2
end
set -l repo_root (command git -C "$script_dir" rev-parse --show-toplevel)
cd "$repo_root"; or exit 2

for dependency in git cargo rustc dotnet tee df awk mktemp
    if not type -q "$dependency"
        printf 'error: required command not found: %s\n' "$dependency" >&2
        exit 2
    end
end

set -l default_benches \
    json arithmetic gc alloc_throughput gc_cross_arena \
    dispatch generics stack span span_equality memory unsafe_buffer \
    string reflection json_dom linq_pipeline ef_inmemory
set -l benches $default_benches
if set -q TRACE_BENCHES; and test -n "$TRACE_BENCHES"
    set benches (string split ',' -- "$TRACE_BENCHES")
end
if test (count $argv) -gt 0
    set benches $argv
end

set -l slow_benches generics ef_inmemory
if set -q TRACE_SLOW_BENCHES; and test -n "$TRACE_SLOW_BENCHES"
    set slow_benches (string split ',' -- "$TRACE_SLOW_BENCHES")
end

set -l backend perf
set -q TRACE_BACKEND; and set backend $TRACE_BACKEND[1]
set -q _flag_backend; and set backend $_flag_backend
if not contains -- "$backend" auto perf samply
    printf 'error: backend must be auto, perf, or samply; got %s\n' "$backend" >&2
    exit 2
end

set -l standard_frequency 999
set -q TRACE_STANDARD_FREQUENCY; and set standard_frequency $TRACE_STANDARD_FREQUENCY[1]
set -q _flag_frequency; and set standard_frequency $_flag_frequency

set -l slow_frequency 199
set -q TRACE_SLOW_FREQUENCY; and set slow_frequency $TRACE_SLOW_FREQUENCY[1]
set -q _flag_slow_frequency; and set slow_frequency $_flag_slow_frequency

set -l call_graph fp
set -q TRACE_CALL_GRAPH; and set call_graph $TRACE_CALL_GRAPH[1]
set -q _flag_call_graph; and set call_graph $_flag_call_graph
if test -z "$call_graph"
    echo 'error: --call-graph cannot be empty' >&2
    exit 2
end

# The benchmark executor itself is intentionally single-threaded.  Rayon is only used while
# decoding metadata; four workers match the production CLI cap and beat all-core decoding on the
# measured many-core host.  Override explicitly for a scaling experiment.
set -l rayon_threads 4
set -q TRACE_RAYON_THREADS; and set rayon_threads $TRACE_RAYON_THREADS[1]
set -q _flag_rayon_threads; and set rayon_threads $_flag_rayon_threads

set -l sample_size 10
set -q TRACE_SAMPLE_SIZE; and set sample_size $TRACE_SAMPLE_SIZE[1]
set -q _flag_sample_size; and set sample_size $_flag_sample_size

set -l standard_sample_size 10
set -q TRACE_STANDARD_SAMPLE_SIZE; and set standard_sample_size $TRACE_STANDARD_SAMPLE_SIZE[1]
set -q _flag_standard_sample_size; and set standard_sample_size $_flag_standard_sample_size

set -l standard_baseline ''
set -q TRACE_STANDARD_BASELINE; and set standard_baseline $TRACE_STANDARD_BASELINE[1]
set -q _flag_standard_baseline; and set standard_baseline $_flag_standard_baseline

set -l cargo_profile profiling
set -q TRACE_PROFILE; and set cargo_profile $TRACE_PROFILE[1]
set -q _flag_profile; and set cargo_profile $_flag_profile

set -l features ''
set -q TRACE_FEATURES; and set features $TRACE_FEATURES[1]
set -q _flag_features; and set features $_flag_features

set -l no_default_features 0
set -q TRACE_NO_DEFAULT_FEATURES; and set no_default_features $TRACE_NO_DEFAULT_FEATURES[1]
set -q _flag_no_default_features; and set no_default_features 1

set -l minimum_free_kb 2097152
set -q TRACE_MIN_FREE_KB; and set minimum_free_kb $TRACE_MIN_FREE_KB[1]
set -q _flag_min_free_kb; and set minimum_free_kb $_flag_min_free_kb

set -l resume 0
set -q TRACE_RESUME; and set resume $TRACE_RESUME[1]
set -q _flag_resume; and set resume 1

for setting in \
    "--frequency $standard_frequency" \
    "--slow-frequency $slow_frequency" \
    "--rayon-threads $rayon_threads" \
    "--sample-size $sample_size" \
    "--standard-sample-size $standard_sample_size" \
    "--min-free-kb $minimum_free_kb"
    set -l parts (string split ' ' -- "$setting")
    require_positive_integer $parts[1] $parts[2]
end

if not string match -qr '^[01]$' -- "$no_default_features"
    printf 'error: TRACE_NO_DEFAULT_FEATURES must be 0 or 1; got %s\n' "$no_default_features" >&2
    exit 2
end
if not string match -qr '^[01]$' -- "$resume"
    printf 'error: TRACE_RESUME must be 0 or 1; got %s\n' "$resume" >&2
    exit 2
end

set -l trace_root ''
set -q TRACE_ROOT; and set trace_root $TRACE_ROOT[1]
set -q _flag_output_dir; and set trace_root $_flag_output_dir
if test -z "$trace_root"
    set -l trace_parent "$repo_root/target/perf-traces"
    mkdir -p "$trace_parent"; or exit 2
    set trace_root (mktemp -d "$trace_parent/e2e-all-"(date -u +%Y%m%dT%H%M%SZ)'-XXXXXX'); or exit 2
else if test -e "$trace_root"
    if test "$resume" -ne 1
        printf 'error: output directory already exists: %s (pass --resume to reuse it)\n' "$trace_root" >&2
        exit 2
    end
else
    mkdir -p "$trace_root"; or exit 2
end

set -l run_index "$trace_root/runs.tsv"
set -l current_run "$trace_root/current-run.tsv"
set -l session_log "$trace_root/session.log"
if not test -f "$run_index"
    printf 'benchmark\tfrequency_hz\tsample_size\tstarted_at\tfinished_at\tresult\tprofile_exit_code\tlog_exit_code\ttrace_dir\tquality_file\tworkflow_log\n' \
        > "$run_index"
else if not head -n 1 "$run_index" | string match -q '*profile_exit_code*'
    echo 'upgrading legacy runs.tsv to the current schema' >&2
    migrate_run_index "$run_index"; or exit 2
end

printf 'key\tvalue\n' > "$trace_root/trace-config.tsv"
printf 'backend\t%s\n' "$backend" >> "$trace_root/trace-config.tsv"
printf 'standard_frequency_hz\t%s\n' "$standard_frequency" >> "$trace_root/trace-config.tsv"
printf 'slow_frequency_hz\t%s\n' "$slow_frequency" >> "$trace_root/trace-config.tsv"
printf 'call_graph\t%s\n' "$call_graph" >> "$trace_root/trace-config.tsv"
printf 'rayon_threads\t%s\n' "$rayon_threads" >> "$trace_root/trace-config.tsv"
printf 'sample_size\t%s\n' "$sample_size" >> "$trace_root/trace-config.tsv"
printf 'standard_sample_size\t%s\n' "$standard_sample_size" >> "$trace_root/trace-config.tsv"
printf 'standard_baseline\t%s\n' "$standard_baseline" >> "$trace_root/trace-config.tsv"
printf 'cargo_profile\t%s\n' "$cargo_profile" >> "$trace_root/trace-config.tsv"
printf 'features\t%s\n' "$features" >> "$trace_root/trace-config.tsv"
printf 'no_default_features\t%s\n' "$no_default_features" >> "$trace_root/trace-config.tsv"
printf 'minimum_free_kb\t%s\n' "$minimum_free_kb" >> "$trace_root/trace-config.tsv"
printf 'resume\t%s\n' "$resume" >> "$trace_root/trace-config.tsv"
printf 'slow_benches\t%s\n' (string join ',' $slow_benches) >> "$trace_root/trace-config.tsv"
printf 'benchmarks\t%s\n' (string join ',' $benches) >> "$trace_root/trace-config.tsv"

command git rev-parse HEAD > "$trace_root/git-commit.txt"; or printf 'unknown\n' > "$trace_root/git-commit.txt"
command git status --porcelain=v1 > "$trace_root/git-status.txt"; or true
command rustc -Vv > "$trace_root/rustc-version.txt"; or true
command dotnet --info > "$trace_root/dotnet-info.txt" 2>&1; or true
command uname -a > "$trace_root/uname.txt"; or true
printf 'trace corpus started at %s\n' (date -u +%Y-%m-%dT%H:%M:%SZ) > "$session_log"

set -l failures 0
for bench in $benches
    if test "$resume" -eq 1; and already_completed "$bench" "$run_index"
        echo "=== skipping completed $bench ==="
        continue
    end

    set -l frequency $standard_frequency
    if contains -- "$bench" $slow_benches
        set frequency $slow_frequency
    end

    set -l started_at (date -u +%Y-%m-%dT%H:%M:%SZ)
    set -l run_dir "$trace_root/$bench"
    set -l workflow_log "$run_dir/workflow.log"
    set -l free_kb (available_kb "$trace_root")
    if string match -qr '^[0-9]+$' -- "$free_kb"; and test "$free_kb" -lt "$minimum_free_kb"
        printf 'error: refusing to start %s: only %s KiB free (floor: %s KiB)\n' "$bench" "$free_kb" "$minimum_free_kb" >&2
        printf '%s\t%s\t%s\t%s\t%s\tskipped_low_disk\t-\t-\t%s\t%s\t%s\n' \
            "$bench" "$frequency" "$sample_size" "$started_at" (date -u +%Y-%m-%dT%H:%M:%SZ) \
            "$run_dir" "$run_dir/quality.txt" "$workflow_log" >> "$run_index"
        set failures 1
        continue
    else if not string match -qr '^[0-9]+$' -- "$free_kb"
        printf 'warning: unable to determine free disk space before %s\n' "$bench" >&2
    end

    mkdir -p "$run_dir"
    if test $status -ne 0
        printf 'error: unable to create trace directory for %s\n' "$bench" >&2
        set failures 1
        continue
    end

    printf 'benchmark\tstarted_at\tfrequency_hz\tsample_size\ttrace_dir\n%s\t%s\t%s\t%s\t%s\n' \
        "$bench" "$started_at" "$frequency" "$sample_size" "$run_dir" > "$current_run"
    echo "=== tracing $bench (frequency=$frequency Hz, sample_size=$sample_size) ==="

    set -l profile_args \
        bench --name "$bench" --backend "$backend" --frequency "$frequency" \
        --call-graph "$call_graph" --sample-size "$sample_size" --profile "$cargo_profile" \
        --output-dir "$run_dir"
    if test -n "$features"
        set -a profile_args --features "$features"
    end
    if test "$no_default_features" -eq 1
        set -a profile_args --no-default-features
    end

    env RAYON_NUM_THREADS="$rayon_threads" "$repo_root/scripts/profile_perf.sh" $profile_args 2>&1 \
        | tee "$workflow_log" \
        | tee -a "$session_log"
    set -l pipeline_status $pipestatus
    set -l profile_exit $pipeline_status[1]
    set -l log_exit $pipeline_status[2]
    if test "$log_exit" -eq 0
        set log_exit $pipeline_status[3]
    end
    set -l finished_at (date -u +%Y-%m-%dT%H:%M:%SZ)
    set -l result ok
    if test "$profile_exit" -ne 0; or test "$log_exit" -ne 0
        set result failed
        set failures 1
    end

    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$bench" "$frequency" "$sample_size" "$started_at" "$finished_at" "$result" \
        "$profile_exit" "$log_exit" "$run_dir" "$run_dir/quality.txt" "$workflow_log" \
        >> "$run_index"
    rm -f "$current_run"
end

if test -n "$standard_baseline"
    set -l standard_log "$trace_root/standard-benchmark.log"
    set -l criterion_dir "$repo_root/target/criterion"
    set -l archived_criterion "$repo_root/target/criterion.pre-baseline-"(date -u +%Y%m%dT%H%M%SZ)
    set -l baseline_status "$trace_root/standard-baseline.tsv"
    printf 'baseline\tsample_size\trayon_threads\tarchived_criterion\tresult\tcargo_exit_code\tlog_exit_code\tlog_file\n' \
        > "$baseline_status"

    if test -d "$criterion_dir"
        command mv "$criterion_dir" "$archived_criterion"
        if test $status -ne 0
            printf '%s\t%s\t%s\t%s\tarchive_failed\t-\t-\t%s\n' \
                "$standard_baseline" "$standard_sample_size" "$rayon_threads" "$archived_criterion" "$standard_log" \
                >> "$baseline_status"
            echo 'error: failed to archive target/criterion; standard baseline was not run' >&2
            set failures 1
        end
    end

    if not test -f "$baseline_status"; or not string match -q '*archive_failed*' < "$baseline_status"
        echo "=== saving clean standard baseline $standard_baseline ==="
        env RAYON_NUM_THREADS="$rayon_threads" \
            cargo bench --profile bench-fat -p dotnet-benchmarks --bench end_to_end -- \
            --sample-size "$standard_sample_size" --save-baseline "$standard_baseline" 2>&1 \
            | tee "$standard_log" \
            | tee -a "$session_log"
        set -l baseline_pipeline $pipestatus
        set -l cargo_exit $baseline_pipeline[1]
        set -l baseline_log_exit $baseline_pipeline[2]
        if test "$baseline_log_exit" -eq 0
            set baseline_log_exit $baseline_pipeline[3]
        end
        set -l baseline_result ok
        if test "$cargo_exit" -ne 0; or test "$baseline_log_exit" -ne 0
            set baseline_result failed
            set failures 1
        end
        printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
            "$standard_baseline" "$standard_sample_size" "$rayon_threads" "$archived_criterion" \
            "$baseline_result" "$cargo_exit" "$baseline_log_exit" "$standard_log" >> "$baseline_status"
    end
end

echo "Trace corpus: $trace_root"
echo "Run index: $run_index"

if test "$failures" -ne 0
    exit 1
end
