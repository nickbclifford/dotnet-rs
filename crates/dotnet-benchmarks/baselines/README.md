# Benchmark Baselines

Canonical tracked baseline data lives in:

- `crates/dotnet-benchmarks/baselines/phase0/baseline.json`

Phase 6 profile comparison artifacts live in:

- `crates/dotnet-benchmarks/baselines/phase6/bench-thin.json`
- `crates/dotnet-benchmarks/baselines/phase6/bench-fat.json`
- `crates/dotnet-benchmarks/baselines/phase6/PROFILE_COMPARISON.md`

Do not rely on archived Criterion directory snapshots for review/automation. Those paths are local scratch artifacts:

- `crates/dotnet-benchmarks/baselines/phase0/criterion/`

To refresh phase0 baseline data:

```bash
python3 crates/dotnet-benchmarks/scripts/capture_baseline.py
```

To refresh phase6 profile-comparison data:

```bash
cargo bench --profile bench-thin -p dotnet-benchmarks --bench end_to_end -- --sample-size 10
cargo bench --profile bench-fat -p dotnet-benchmarks --bench end_to_end -- --sample-size 10
```

Then update the tracked JSON snapshots/report in `crates/dotnet-benchmarks/baselines/phase6/`.
