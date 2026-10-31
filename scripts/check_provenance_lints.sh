#!/usr/bin/env bash
# Deny `fuzzy_provenance_casts` and `lossy_provenance_casts` in first-party code.
#
# Both lints are nightly-only (feature `strict_provenance_lints`), so this runs on the
# pinned Miri/fuzz nightly and injects the feature with `-Zcrate-attr` instead of
# editing every crate root. Pointer-to-integer conversions must use `.addr()` (pure
# compare/hash/alignment/diagnostics) or `.expose_provenance()` (the integer may be
# turned back into a pointer); integer-to-pointer conversions must use
# `with_exposed_provenance*` or `without_provenance*`. See
# docs/plans/01-pointer-provenance.md (M0, provenance-lint migration).
#
# Usage: scripts/check_provenance_lints.sh [extra cargo check args]
set -euo pipefail

NIGHTLY="${PROVENANCE_LINT_TOOLCHAIN:-nightly-2026-05-27}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export RUSTFLAGS="${RUSTFLAGS:-} -Zcrate-attr=feature(strict_provenance_lints) -Dfuzzy_provenance_casts -Dlossy_provenance_casts"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target/provenance-lints}"
export DOTNET_SKIP_BUILD="${DOTNET_SKIP_BUILD:-1}"

# `--no-default-features` covers cfg(not(feature = ...)) paths; `--all-features`
# covers every feature-gated path (multithreading, fuzzing, memory-validation, ...).
for features in --no-default-features "" --all-features; do
    echo "== provenance lints: workspace ${features:-(default features)}"
    # shellcheck disable=SC2086
    cargo "+$NIGHTLY" check --manifest-path "$ROOT/Cargo.toml" --workspace --all-targets $features "$@"
done

for fuzz in crates/dotnet-value/fuzz crates/dotnet-vm/fuzz; do
    echo "== provenance lints: $fuzz"
    cargo "+$NIGHTLY" check --manifest-path "$ROOT/$fuzz/Cargo.toml" --all-targets "$@"
done
