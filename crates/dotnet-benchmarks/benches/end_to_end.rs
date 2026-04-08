use criterion::{Criterion, criterion_group, criterion_main};
#[cfg(feature = "bench-instrumentation")]
use dotnet_benchmarks::write_bench_metrics_snapshot;
use dotnet_benchmarks::{
    ARITHMETIC_BENCHMARK, BenchHarness, DISPATCH_BENCHMARK, GC_BENCHMARK, GENERICS_BENCHMARK,
    JSON_BENCHMARK,
};

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
    run_case(c, DISPATCH_BENCHMARK);
    run_case(c, GENERICS_BENCHMARK);
}

criterion_group! {
    name = end_to_end;
    config = Criterion::default().configure_from_args();
    targets = benchmarks
}
criterion_main!(end_to_end);
