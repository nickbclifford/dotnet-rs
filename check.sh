#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: ./check.sh [--fast|--full]

Default mode is --fast. You can also set CHECK_FULL=1 to enable --full.
EOF
}

is_truthy() {
    case "${1,,}" in
        1|true|yes|on) return 0 ;;
        *) return 1 ;;
    esac
}

FULL_MODE=0
if is_truthy "${CHECK_FULL:-0}"; then
    FULL_MODE=1
fi

for arg in "$@"; do
    case "$arg" in
        --fast)
            FULL_MODE=0
            ;;
        --full)
            FULL_MODE=1
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown argument: $arg" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if [ "$FULL_MODE" -eq 1 ]; then
    echo "Running full checks..."
    readarray -t FEATURES_COMBINATIONS < <(
        cargo run --quiet -p xtask -- matrix test-features --format lines
    )
else
    echo "Running fast checks..."
    FEATURES_COMBINATIONS=(
        ""
        "multithreading"
        "multithreading,validation-all"
    )
fi

if [ "$FULL_MODE" -eq 1 ]; then
    echo "Running build-script regression probes..."
    bash scripts/check_build_script_regressions.sh
fi

echo "Building .NET fixtures once..."
cargo run --quiet -p xtask -- fixtures build
export DOTNET_USE_PREBUILT_FIXTURES=1

echo "Running checks for feature combinations..."
for features in "${FEATURES_COMBINATIONS[@]}"; do
    if [ -z "$features" ]; then
        echo "=== Combination: no features ==="
        cargo clippy --all-targets --no-default-features -- -D warnings
        cargo nextest run --no-default-features
    else
        echo "=== Combination: $features ==="
        cargo clippy --all-targets --no-default-features --features "$features" -- -D warnings
        cargo nextest run --no-default-features --features "$features"
    fi
done

if [ "$FULL_MODE" -eq 1 ]; then
    # Optional feature runs are compile/regression guards for diagnostics and
    # alternate dispatch implementations that are not covered by the matrix above.
    echo "Running optional feature compilation guards..."
    cargo nextest run --features bench-instrumentation
    cargo nextest run --features heap-diagnostics
    cargo nextest run -p dotnet-vm --features deadlock-diagnostics
fi

echo "All checks passed!"
