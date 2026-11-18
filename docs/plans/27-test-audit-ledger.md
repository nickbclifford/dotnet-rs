# Test audit ledger

Shared record for [27a](27a-test-audit.md) (audit) and [27b](27b-test-consolidation.md) (cleanup).
The per-partition audit reviews and case inventories were removed after the cleanup; they are in
git.

## Partitions and dispositions

| Partition | Locations | Findings | Cleanup record |
|---|---|---|---|
| VM | `crates/dotnet-vm/**`, including its fuzz workspace and models | VM-1–9, VI-1–5, VP-1–4 | [VM](27-audit/cleanup-vm.md) |
| CLI and managed fixtures | `crates/dotnet-cli/**` | CLI-1–23 | [CLI](27-audit/cleanup-cli.md) |
| Libraries | Other crates except benchmarks, including value fuzzing and memory Loom models | LIB-1–25 | [libraries](27-audit/cleanup-libraries.md) |
| Tools and benchmarks | `scripts/**`, `xtask/**`, workflow test wiring, `crates/dotnet-benchmarks/**`, root check entrypoints | TOOLS-1–7 | [tools and benchmarks](27-audit/cleanup-tools.md) |

The keep dispositions record useful internal, safety and guest contracts; they are
source-inspection conclusions, not claims that every plausible mutation was tried.

## Verification

The combined cleanup was verified with:

- `./check.sh --fast` (no features, multithreading, multithreading with `validation-all`, including
  clippy with warnings denied and the managed fixtures);
- `python3 -m unittest discover -s scripts/tests -p 'test_*.py'`, including the registered-growth,
  missing-selection, wrong-selector and ignored/failing-execution controls;
- `cargo test -p xtask`;
- `bash scripts/check_doc_drift.sh` and its injected negative controls;
- the native-effects and `FieldType` compile-fail checkers in both thread configurations;
- the Loom STW and shared-table/finalizer models;
- the affected Miri selections (pointer arithmetic, fuzz-harness groups, advisory call-depth
  singletons) and provenance lints;
- `ASAN_OPTIONS=detect_leaks=0 python3 scripts/replay_fuzz_corpus.py ...` for the changed corpora.

## Limits

- The full Miri matrix, live fuzzing and the destructive clean-build probes were not rerun beyond
  the affected lanes.
- Stable rustdoc does not check `compile_fail` error numbers (nightly-only); the diagnostic checker
  covers LIB-15 instead.
- Local fuzz replays ran with LeakSanitizer disabled because the sandbox's ptrace restriction
  prevents it; CI is unaffected.
