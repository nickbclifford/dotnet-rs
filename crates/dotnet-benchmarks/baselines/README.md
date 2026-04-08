# Benchmark Baselines

Canonical tracked baseline data lives in:

- `crates/dotnet-benchmarks/baselines/phase0/baseline.json`

Do not rely on archived Criterion directory snapshots for review/automation. Those paths are ignored and treated as local scratch artifacts:

- `crates/dotnet-benchmarks/baselines/phase0/criterion/`

To refresh baseline data:

```bash
python3 crates/dotnet-benchmarks/scripts/capture_baseline.py
```

For a quick delta check against the stored baseline (single workload):

```bash
python3 crates/dotnet-benchmarks/scripts/capture_baseline.py --workloads json
```
