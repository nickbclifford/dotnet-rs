//! Cold-start end-to-end benchmark.
//!
//! `end_to_end` reuses a single `AssemblyLoader` across iterations, so the `System.Private.CoreLib`
//! parse + type/method resolution land once in warmup and are never re-measured — it reports warm
//! steady-state execution. This benchmark instead builds a *fresh* loader every iteration
//! ([`cold_run`]), so each sample includes the full cold cost: parse corlib + framework deps,
//! resolve types/methods, and execute. It is the measurement that settles whether **lazy** (parse
//! cheap, decode method bodies on-demand single-threaded on the hot path) or **eager** (decode all
//! bodies up front via dotnetdll's rayon-parallel decoder) wins for a real workload.
//!
//! Two fixtures bracket the workload space:
//! - `arithmetic`: trivial — touches almost no corlib methods (lazy should dominate).
//! - `json`: execution-heavy — exercises a large slice of corlib/System.Text.Json.

use criterion::{Criterion, criterion_group, criterion_main};
use dotnet_benchmarks::{
    ARITHMETIC_BENCHMARK, BenchHarness, BenchmarkCase, JSON_BENCHMARK, LOAD_DOMINATED_BENCHMARK,
    assemblies_path, cold_run,
};
use dotnetdll::prelude::ReadOptions;

fn lazy_opts() -> ReadOptions {
    dotnet_assemblies::default_read_options()
}

fn eager_opts() -> ReadOptions {
    ReadOptions::default()
}

fn run_case(c: &mut Criterion, case: BenchmarkCase) {
    // Compile the fixture once (slow `dotnet build`); this is setup, not measured.
    let harness = BenchHarness::new();
    let dll = harness.ensure_fixture_dll(case);
    let assemblies = assemblies_path();

    let mut group = c.benchmark_group(format!("cold_start/{}", case.name));
    // Each iteration re-parses the 12 MB corlib, so keep sample counts bounded.
    group.sample_size(20);

    group.bench_function("lazy", |b| {
        let opts = lazy_opts();
        b.iter(|| {
            let code = cold_run(&assemblies, &dll, opts);
            assert_eq!(code, case.expected_exit_code, "unexpected exit code (lazy)");
        });
    });

    group.bench_function("eager", |b| {
        let opts = eager_opts();
        b.iter(|| {
            let code = cold_run(&assemblies, &dll, opts);
            assert_eq!(code, case.expected_exit_code, "unexpected exit code (eager)");
        });
    });

    group.finish();
}

fn cold_start(c: &mut Criterion) {
    run_case(c, ARITHMETIC_BENCHMARK);
    run_case(c, JSON_BENCHMARK);
    // Load-dominated: framework working set parse with trivial execution.
    // Primary regression guard for rayon thread-pool tuning — cold−warm ≈ pure load cost.
    run_case(c, LOAD_DOMINATED_BENCHMARK);
}

criterion_group! {
    name = cold_start_group;
    config = Criterion::default().configure_from_args();
    targets = cold_start
}
criterion_main!(cold_start_group);
