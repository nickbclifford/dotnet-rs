use criterion::{Criterion, criterion_group, criterion_main};
use dotnet_benchmarks::{BenchHarness, BenchmarkCase, BenchmarkFixture};

fn benchmarks(c: &mut Criterion) {
    for (name, source) in [
        ("substring_empty_small", "fixtures/substring/Small_0.cs"),
        ("substring_large", "fixtures/substring/Large_0.cs"),
    ] {
        let case = BenchmarkCase {
            name,
            fixture: BenchmarkFixture::SingleFile { source },
            expected_exit_code: 0,
        };
        let harness = BenchHarness::new();
        let mut prepared = None;
        c.bench_function(name, |b| {
            let fixture = prepared.get_or_insert_with(|| harness.prepare_case(case));
            b.iter(|| assert_eq!(harness.run_prepared(fixture), 0));
        });
    }
}

criterion_group!(benches, benchmarks);
criterion_main!(benches);
