# Pointer / provenance model reviews (2026-10-26/27)

Three research-only reviews of dotnet-rs's pointer model at commit `b49f5df8`. They ask how close the VM can get to
respecting Rust provenance (Miri, Tree Borrows) while staying faithful to ECMA-335. No implementation work has started.

| Folder | Pass | Style |
|---|---|---|
| [`pass-1/`](pass-1/REVIEW.md) | First | Library-level Miri probes (`probes/`), strict-provenance runs of value/memory/utils crates |
| [`pass-2/`](pass-2/REVIEW.md) | Second | IL-level probes through the real VM (`provenance_probe_tests.rs`), permissive TB |
| [`pass-3/`](pass-3/REVIEW.md) | Third | Independent IL probes p00–p20, standalone Miri lab, native FFI repro, validated fix patch, **ranked migration plan with gating tests** |

For implementation work, start with the [unified project plan](../plans/01-pointer-provenance.md).
It reconciles all three reviews, assigns milestone dependencies, and maps findings to validation gates.
The individual reviews remain historical evidence; pass 3 contains the follow-up experiments on native ints
and cached addresses.

## Notes

- These are snapshots at `b49f5df8`. Line numbers and file paths inside them refer to that commit.
- Logs are Miri/native output kept as evidence; each REVIEW.md names the log for each claim. Pass 1's REVIEW.md and
  its probe invocations still mention `/tmp/dotnet-provenance-review/…`; the equivalent files are now in `pass-1/`.
- Miri setup: `cargo +nightly-2026-05-27 miri`, `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation
  -Zmiri-ignore-leaks"`. A dotnet-vm Miri build takes ~10 min and each IL probe ~4 min; run failing probes one at a
  time because Miri stops at the first UB.
- IL probe modules (`pass-2/provenance_probe_tests.rs`, `pass-3/probes/*.rs`) are not wired into the build. To run
  them, copy into `crates/dotnet-vm/src/` (or `crates/dotnet-value/src/` for `addr_probe_tests.rs`) and add
  `#[cfg(test)] mod <name>;` to that crate's `lib.rs`.
- The standalone crates (`pass-1/probes`, `pass-3/lab`, `pass-3/ffi`) each declare an empty `[workspace]`, so they
  build independently of the repo workspace (`cargo` run from inside the folder). `target/` is git-ignored here.
- `pass-3/fix-variants/*.patch` apply with `patch -p1` to a pristine `b49f5df8` tree.
- Scratch checkouts (pass 2's `src/`, `provenance-3*`, build `target/` dirs; ~10 GB) were not copied into the repo; they may
  still exist under `/tmp` until it is cleared. Regenerate a scratch copy with `git archive b49f5df8`.
