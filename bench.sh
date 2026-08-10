#!/usr/bin/env fish

set -l trace_root target/perf-traces/e2e-all-(date -u +%Y%m%dT%H%M%SZ)
set -l benches \
    json arithmetic gc alloc_throughput gc_cross_arena \
    dispatch generics stack span span_equality memory unsafe_buffer \
    string reflection json_dom linq_pipeline ef_inmemory
set -l failures 0

# Profiling needs enough samples to expose hot paths, not benchmark-grade confidence
# intervals.  The previous 3997 Hz / 30-sample settings created multi-gigabyte perf scripts
# for long-running cases.  Keep captures bounded so every case completes and agents can
# consume the artifacts without exhausting RAM during post-processing.
set -l standard_frequency 999
set -l slow_frequency 199
set -l sample_size 10
set -l slow_benches generics ef_inmemory

mkdir -p $trace_root

git rev-parse HEAD > "$trace_root/git-commit.txt"
git status --porcelain=v1 > "$trace_root/git-status.txt"
rustc -Vv > "$trace_root/rustc-version.txt"
dotnet --info > "$trace_root/dotnet-info.txt"
uname -a > "$trace_root/uname.txt"

printf 'benchmark\tfrequency_hz\tsample_size\tstarted_at\tfinished_at\tresult\texit_code\ttrace_dir\tquality_file\tworkflow_log\n' \
    > "$trace_root/runs.tsv"

for bench in $benches
    set -l run_dir "$trace_root/$bench"
    set -l started_at (date -u +%Y-%m-%dT%H:%M:%SZ)
    set -l frequency $standard_frequency
    if contains -- $bench $slow_benches
        set frequency $slow_frequency
    end

    mkdir -p $run_dir
    echo "=== tracing $bench (frequency=$frequency Hz, sample_size=$sample_size) ==="

    ./scripts/profile_perf.sh bench \
        --name $bench \
        --backend perf \
        --frequency $frequency \
        --call-graph fp \
        --sample-size $sample_size \
        --output-dir $run_dir \
        2>&1 | tee "$run_dir/workflow.log"
    set -l exit_code $pipestatus[1]

    set -l finished_at (date -u +%Y-%m-%dT%H:%M:%SZ)
    set -l result ok
    if test $exit_code -ne 0
        set result failed
        set failures 1
    end

    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        $bench $frequency $sample_size $started_at $finished_at $result $exit_code \
        $run_dir "$run_dir/quality.txt" "$run_dir/workflow.log" \
        >> "$trace_root/runs.tsv"
end

echo "Trace corpus: $trace_root"
echo "Run index: $trace_root/runs.tsv"

if test $failures -ne 0
    exit 1
end
