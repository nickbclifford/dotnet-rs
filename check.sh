#!/usr/bin/env bash
set -e

FEATURES_COMBINATIONS=(
    ""
    "multithreading"
)

echo "Running checks for all feature combinations..."

for features in "${FEATURES_COMBINATIONS[@]}"; do
    if [ -z "$features" ]; then
        echo "=== Combination: No features ==="
        cargo clippy --all-targets --no-default-features -- -D warnings
        # Use single test thread for non-multithreading build to avoid SIGSEGV in RefCell locks
        cargo test --no-default-features -- --nocapture --test-threads=1
    else
        echo "=== Combination: $features ==="
        cargo clippy --all-targets --no-default-features --features "$features" -- -D warnings
        cargo test --no-default-features --features "$features" -- --nocapture
    fi
done

echo "All checks passed!"
