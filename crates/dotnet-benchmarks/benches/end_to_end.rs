use criterion::{Criterion, criterion_group, criterion_main};
use dotnet_benchmarks::{
    ALLOC_THROUGHPUT_BENCHMARK, ARITHMETIC_BENCHMARK, BenchHarness, DISPATCH_BENCHMARK,
    GC_BENCHMARK, GC_CROSS_ARENA_BENCHMARK, GENERICS_BENCHMARK, JSON_BENCHMARK, MEMORY_BENCHMARK,
    REFLECTION_BENCHMARK, SPAN_BENCHMARK, SPAN_EQUALITY_BENCHMARK, STACK_BENCHMARK,
    STRING_BENCHMARK, UNSAFE_BUFFER_BENCHMARK,
};

#[cfg(feature = "bench-instrumentation")]
use criterion::{
    Throughput,
    measurement::{Measurement, ValueFormatter},
};
#[cfg(feature = "bench-instrumentation")]
use dotnet_benchmarks::write_bench_metrics_snapshot;

#[cfg(feature = "bench-instrumentation")]
const GC_PAUSE_P99_BOUND_ENV: &str = "DOTNET_GC_PAUSE_P99_MAX_US";
#[cfg(feature = "bench-instrumentation")]
const DEFAULT_GC_PAUSE_P99_BOUND_US: u64 = 250_000;

fn run_case(c: &mut Criterion, case: dotnet_benchmarks::BenchmarkCase) {
    let harness = BenchHarness::new();
    let mut cached_dll = None;
    #[cfg(feature = "bench-instrumentation")]
    let mut latest_run = None;

    c.bench_function(case.name, |b| {
        let dll = cached_dll
            .get_or_insert_with(|| harness.ensure_fixture_dll(case))
            .clone();
        b.iter(|| {
            #[cfg(feature = "bench-instrumentation")]
            let run = harness.run_dll_with_metrics(&dll);
            #[cfg(not(feature = "bench-instrumentation"))]
            let exit_code = harness.run_dll(&dll);
            #[cfg(feature = "bench-instrumentation")]
            let exit_code = run.exit_code;
            assert_eq!(
                exit_code, case.expected_exit_code,
                "benchmark fixture {} returned unexpected exit code",
                case.name
            );
            #[cfg(feature = "bench-instrumentation")]
            {
                latest_run = Some(run);
            }
        });
    });

    #[cfg(feature = "bench-instrumentation")]
    if let Some(run) = latest_run.as_ref()
        && let Err(err) = write_bench_metrics_snapshot(case.name, run)
    {
        panic!(
            "failed to write metrics snapshot for {}: {}",
            case.name, err
        );
    }
}

fn benchmarks(c: &mut Criterion) {
    run_case(c, JSON_BENCHMARK);
    run_case(c, ARITHMETIC_BENCHMARK);
    run_case(c, GC_BENCHMARK);
    run_case(c, ALLOC_THROUGHPUT_BENCHMARK);
    run_case(c, GC_CROSS_ARENA_BENCHMARK);
    run_case(c, DISPATCH_BENCHMARK);
    run_case(c, GENERICS_BENCHMARK);
    run_case(c, STACK_BENCHMARK);
    run_case(c, SPAN_BENCHMARK);
    run_case(c, SPAN_EQUALITY_BENCHMARK);
    run_case(c, MEMORY_BENCHMARK);
    run_case(c, UNSAFE_BUFFER_BENCHMARK);
    run_case(c, STRING_BENCHMARK);
    run_case(c, REFLECTION_BENCHMARK);
}

#[cfg(feature = "bench-instrumentation")]
struct GcPauseP99Formatter;

#[cfg(feature = "bench-instrumentation")]
impl ValueFormatter for GcPauseP99Formatter {
    fn scale_values(&self, _typical_value: f64, _values: &mut [f64]) -> &'static str {
        "gc_pause_p99_us"
    }

    fn scale_throughputs(
        &self,
        _typical_value: f64,
        _throughput: &Throughput,
        _values: &mut [f64],
    ) -> &'static str {
        "gc_pause_p99_us"
    }

    fn scale_for_machines(&self, _values: &mut [f64]) -> &'static str {
        "gc_pause_p99_us"
    }
}

#[cfg(feature = "bench-instrumentation")]
struct GcPauseP99Measurement;

#[cfg(feature = "bench-instrumentation")]
const GC_PAUSE_P99_FORMATTER: GcPauseP99Formatter = GcPauseP99Formatter;

#[cfg(feature = "bench-instrumentation")]
impl Measurement for GcPauseP99Measurement {
    type Intermediate = ();
    type Value = u64;

    fn start(&self) -> Self::Intermediate {}

    fn end(&self, _intermediate: Self::Intermediate) -> Self::Value {
        0
    }

    fn add(&self, left: &Self::Value, right: &Self::Value) -> Self::Value {
        left.saturating_add(*right)
    }

    fn zero(&self) -> Self::Value {
        0
    }

    fn to_f64(&self, value: &Self::Value) -> f64 {
        *value as f64
    }

    fn formatter(&self) -> &dyn ValueFormatter {
        &GC_PAUSE_P99_FORMATTER
    }
}

#[cfg(feature = "bench-instrumentation")]
fn gc_pause_p99_bound_us() -> u64 {
    std::env::var(GC_PAUSE_P99_BOUND_ENV)
        .map(|raw| {
            raw.parse::<u64>().unwrap_or_else(|err| {
                panic!("failed to parse {GC_PAUSE_P99_BOUND_ENV}={raw:?} as u64: {err}")
            })
        })
        .unwrap_or(DEFAULT_GC_PAUSE_P99_BOUND_US)
}

#[cfg(feature = "bench-instrumentation")]
fn run_gc_pause_case(
    c: &mut Criterion<GcPauseP99Measurement>,
    case: dotnet_benchmarks::BenchmarkCase,
    gc_pause_p99_bound_us: u64,
) {
    let harness = BenchHarness::new();
    let mut cached_dll = None;
    let mut latest_run = None;

    c.bench_function(case.name, |b| {
        let dll = cached_dll
            .get_or_insert_with(|| harness.ensure_fixture_dll(case))
            .clone();

        b.iter_custom(|iters| {
            let mut gc_pause_p99_total_us = 0_u64;
            for _ in 0..iters {
                let run = harness.run_dll_with_metrics(&dll);
                assert_eq!(
                    run.exit_code, case.expected_exit_code,
                    "GC pause benchmark fixture {} returned unexpected exit code",
                    case.name
                );

                let gc_pause_p99_us = run.metrics.bench.gc_pause_p99_us;
                assert!(
                    gc_pause_p99_us < gc_pause_p99_bound_us,
                    "GC pause p99 benchmark {} exceeded bound: {} us >= {} us (configure via {})",
                    case.name,
                    gc_pause_p99_us,
                    gc_pause_p99_bound_us,
                    GC_PAUSE_P99_BOUND_ENV,
                );

                gc_pause_p99_total_us = gc_pause_p99_total_us.saturating_add(gc_pause_p99_us);
                latest_run = Some(run);
            }
            gc_pause_p99_total_us
        });
    });

    if let Some(run) = latest_run.as_ref()
        && let Err(err) = write_bench_metrics_snapshot(&format!("{}_gc_pause_p99", case.name), run)
    {
        panic!(
            "failed to write GC pause metrics snapshot for {}: {}",
            case.name, err
        );
    }
}

#[cfg(feature = "bench-instrumentation")]
fn gc_pause_benchmarks(c: &mut Criterion<GcPauseP99Measurement>) {
    let gc_pause_p99_bound_us = gc_pause_p99_bound_us();
    run_gc_pause_case(c, GC_BENCHMARK, gc_pause_p99_bound_us);
    run_gc_pause_case(c, GC_CROSS_ARENA_BENCHMARK, gc_pause_p99_bound_us);
}

#[cfg(feature = "bench-instrumentation")]
fn gc_pause_criterion() -> Criterion<GcPauseP99Measurement> {
    Criterion::default()
        .with_measurement(GcPauseP99Measurement)
        .configure_from_args()
}

criterion_group! {
    name = end_to_end;
    config = Criterion::default().configure_from_args();
    targets = benchmarks
}

#[cfg(feature = "bench-instrumentation")]
criterion_group! {
    name = gc_pause_p99;
    config = gc_pause_criterion();
    targets = gc_pause_benchmarks
}

#[cfg(feature = "bench-instrumentation")]
criterion_main!(end_to_end, gc_pause_p99);

#[cfg(not(feature = "bench-instrumentation"))]
criterion_main!(end_to_end);
