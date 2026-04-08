#!/usr/bin/env python3
from __future__ import annotations

import argparse
import copy
import json
import math
import shutil
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

WORKLOADS = ["json", "arithmetic", "gc", "dispatch", "generics"]


@dataclass(frozen=True)
class RunConfig:
    name: str
    cargo_args: list[str]


DEFAULT_CONFIG = RunConfig(name="default", cargo_args=[])
NO_DEFAULT_CONFIG = RunConfig(name="no_default", cargo_args=["--no-default-features"])
INSTRUMENTED_CONFIG = RunConfig(
    name="instrumented_default", cargo_args=["--features", "bench-instrumentation"]
)


def run_command(cmd: list[str], cwd: Path) -> None:
    print("[capture-baseline] running:", " ".join(cmd))
    subprocess.run(cmd, cwd=cwd, check=True)


def load_json(path: Path) -> Any:
    with path.open("r", encoding="utf-8") as f:
        return json.load(f)


def percentile(values: list[float], p: float) -> float:
    if not values:
        raise ValueError("cannot compute percentile from empty values")
    if len(values) == 1:
        return values[0]

    rank = (len(values) - 1) * p
    low = math.floor(rank)
    high = math.ceil(rank)
    if low == high:
        return values[low]
    weight = rank - low
    return values[low] * (1.0 - weight) + values[high] * weight


def parse_criterion_metrics(target_dir: Path, workload: str) -> dict[str, float]:
    est_path = target_dir / "criterion" / workload / "new" / "estimates.json"
    sample_path = target_dir / "criterion" / workload / "new" / "sample.json"

    estimates = load_json(est_path)
    samples = load_json(sample_path)
    times = sorted(float(t) for t in samples["times"])

    return {
        "median_ns": float(estimates["median"]["point_estimate"]),
        "p95_ns": percentile(times, 0.95),
        "mean_ns": float(estimates["mean"]["point_estimate"]),
    }


def copy_criterion_artifacts(
    target_dir: Path, archive_root: Path, config_name: str, workload: str
) -> None:
    src = target_dir / "criterion" / workload
    dst = archive_root / config_name / workload
    if dst.exists():
        shutil.rmtree(dst)
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(src, dst)


def load_metrics_snapshot(target_dir: Path, workload: str) -> dict[str, Any]:
    metrics_path = target_dir / "release" / "dotnet-bench-metrics" / f"{workload}.json"
    return load_json(metrics_path)


def capture_runtime_config(
    repo_root: Path,
    target_dir: Path,
    archive_root: Path,
    config: RunConfig,
    workloads: list[str],
    sample_size: int,
    measurement_time: float,
    warm_up_time: float,
) -> dict[str, dict[str, float]]:
    captured: dict[str, dict[str, float]] = {}
    for workload in workloads:
        cmd = [
            "cargo",
            "bench",
            "-p",
            "dotnet-benchmarks",
            "--bench",
            "end_to_end",
            *config.cargo_args,
            "--",
            workload,
            "--sample-size",
            str(sample_size),
            "--measurement-time",
            str(measurement_time),
            "--warm-up-time",
            str(warm_up_time),
        ]
        run_command(cmd, repo_root)
        copy_criterion_artifacts(target_dir, archive_root, config.name, workload)
        captured[workload] = parse_criterion_metrics(target_dir, workload)
    return captured


def capture_instrumented(
    repo_root: Path,
    target_dir: Path,
    archive_root: Path,
    workloads: list[str],
    sample_size: int,
    measurement_time: float,
    warm_up_time: float,
) -> tuple[dict[str, dict[str, float]], dict[str, dict[str, Any]]]:
    timing: dict[str, dict[str, float]] = {}
    counters: dict[str, dict[str, Any]] = {}

    for workload in workloads:
        cmd = [
            "cargo",
            "bench",
            "-p",
            "dotnet-benchmarks",
            "--bench",
            "end_to_end",
            *INSTRUMENTED_CONFIG.cargo_args,
            "--",
            workload,
            "--sample-size",
            str(sample_size),
            "--measurement-time",
            str(measurement_time),
            "--warm-up-time",
            str(warm_up_time),
        ]
        run_command(cmd, repo_root)
        copy_criterion_artifacts(target_dir, archive_root, INSTRUMENTED_CONFIG.name, workload)
        timing[workload] = parse_criterion_metrics(target_dir, workload)
        counters[workload] = load_metrics_snapshot(target_dir, workload)

    return timing, counters


def format_delta(new: float, old: float) -> str:
    if old == 0:
        return "n/a"
    pct = (new - old) / old * 100.0
    return f"{pct:+.2f}%"


def print_delta_report(
    previous: dict[str, Any] | None,
    current: dict[str, Any],
    workloads: list[str],
) -> None:
    if previous is None:
        print("[capture-baseline] no previous baseline found; skipping delta report")
        return

    print("[capture-baseline] delta report vs stored baseline:")
    for config_name in [DEFAULT_CONFIG.name, NO_DEFAULT_CONFIG.name, INSTRUMENTED_CONFIG.name]:
        prev_cfg = previous.get("results", {}).get(config_name, {})
        cur_cfg = current.get("results", {}).get(config_name, {})
        print(f"  {config_name}:")
        for workload in workloads:
            prev_metrics = prev_cfg.get(workload)
            cur_metrics = cur_cfg.get(workload)
            if not prev_metrics or not cur_metrics:
                print(f"    {workload}: missing prior/current data")
                continue
            median_delta = format_delta(cur_metrics["median_ns"], prev_metrics["median_ns"])
            p95_delta = format_delta(cur_metrics["p95_ns"], prev_metrics["p95_ns"])
            print(
                f"    {workload}: median {median_delta}, p95 {p95_delta}"
            )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Capture and persist dotnet-rs Phase 0 baselines")
    parser.add_argument(
        "--workloads",
        nargs="+",
        default=WORKLOADS,
        choices=WORKLOADS,
        help="Subset of workloads to run (default: all)",
    )
    parser.add_argument(
        "--skip-no-default",
        action="store_true",
        help="Skip no-default-features benchmark runs",
    )
    parser.add_argument(
        "--skip-instrumented",
        action="store_true",
        help="Skip instrumented benchmark runs",
    )
    parser.add_argument(
        "--sample-size",
        type=int,
        default=10,
        help="Criterion sample size (default: 10)",
    )
    parser.add_argument(
        "--measurement-time",
        type=float,
        default=5.0,
        help="Criterion measurement time in seconds (default: 5)",
    )
    parser.add_argument(
        "--warm-up-time",
        type=float,
        default=1.0,
        help="Criterion warm-up time in seconds (default: 1)",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    repo_root = Path(__file__).resolve().parents[3]
    target_dir = repo_root / "target"

    baseline_root = repo_root / "crates" / "dotnet-benchmarks" / "baselines" / "phase0"
    baseline_file = baseline_root / "baseline.json"
    archive_root = baseline_root / "criterion"

    previous = load_json(baseline_file) if baseline_file.exists() else None

    previous_results = previous.get("results", {}) if previous else {}
    previous_metrics = previous.get("instrumentation", {}) if previous else {}

    results: dict[str, Any] = copy.deepcopy(previous_results)
    metrics: dict[str, Any] = copy.deepcopy(previous_metrics)

    default_result = capture_runtime_config(
        repo_root,
        target_dir,
        archive_root,
        DEFAULT_CONFIG,
        args.workloads,
        args.sample_size,
        args.measurement_time,
        args.warm_up_time,
    )
    results.setdefault(DEFAULT_CONFIG.name, {}).update(default_result)

    if not args.skip_no_default:
        no_default_result = capture_runtime_config(
            repo_root,
            target_dir,
            archive_root,
            NO_DEFAULT_CONFIG,
            args.workloads,
            args.sample_size,
            args.measurement_time,
            args.warm_up_time,
        )
        results.setdefault(NO_DEFAULT_CONFIG.name, {}).update(no_default_result)

    if not args.skip_instrumented:
        timing, counters = capture_instrumented(
            repo_root,
            target_dir,
            archive_root,
            args.workloads,
            args.sample_size,
            args.measurement_time,
            args.warm_up_time,
        )
        results.setdefault(INSTRUMENTED_CONFIG.name, {}).update(timing)
        metrics.setdefault(INSTRUMENTED_CONFIG.name, {}).update(counters)

    persisted_workloads = set(args.workloads)
    if previous:
        persisted_workloads.update(previous.get("workloads", []))

    payload = {
        "schema_version": 1,
        "captured_at_utc": datetime.now(timezone.utc).isoformat(),
        "workloads": sorted(persisted_workloads),
        "results": results,
        "instrumentation": metrics,
    }

    baseline_root.mkdir(parents=True, exist_ok=True)
    with baseline_file.open("w", encoding="utf-8") as f:
        json.dump(payload, f, indent=2, sort_keys=True)
        f.write("\n")

    print(f"[capture-baseline] wrote baseline: {baseline_file}")
    print_delta_report(previous, payload, args.workloads)
    return 0


if __name__ == "__main__":
    sys.exit(main())
