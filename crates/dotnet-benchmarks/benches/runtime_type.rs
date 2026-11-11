use criterion::{Criterion, criterion_group, criterion_main};
use dotnet_benchmarks::{BenchHarness, BenchmarkCase, BenchmarkFixture};

fn benchmarks(c: &mut Criterion) {
    let case = BenchmarkCase {
        name: "runtime_type",
        fixture: BenchmarkFixture::SingleFile {
            source: "fixtures/reflection/RuntimeType_0.cs",
        },
        expected_exit_code: 0,
    };
    let harness = BenchHarness::new();
    let mut prepared = None;
    c.bench_function(case.name, |b| {
        let fixture = prepared.get_or_insert_with(|| harness.prepare_case(case));
        b.iter(|| assert_eq!(harness.run_prepared(fixture), case.expected_exit_code));
    });
}

criterion_group!(benches, benchmarks);
criterion_main!(benches);
