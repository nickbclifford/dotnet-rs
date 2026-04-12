#!/usr/bin/env bash
set -e
FEATURES_COMBINATIONS=(
    ""
    "multithreading"
    "generic-constraint-validation"
    "memory-validation"
    "metadata-validation"
    "multithreading,memory-validation"
    "multithreading,validation-all"
    "fuzzing"
)
LOCK_ORDER_TESTS=(
    "gc::coordinator::tests::stress_lock_order_gc_cycle_guard_with_live_safepoints"
    "threading::basic::tests::lock_order_request_stop_the_world_rejects_inverted_top_level_order"
)
echo "Running checks for all feature combinations..."
for features in "${FEATURES_COMBINATIONS[@]}"; do
    if [ -z "$features" ]; then
        echo "=== Combination: No features ==="
        cargo clippy --all-targets --no-default-features -- -D warnings
        # Use single test thread for non-multithreading build to avoid SIGSEGV in RefCell locks
        DOTNET_TEST_TIMEOUT_SECS=180 timeout 1200 cargo test --no-default-features -- --nocapture --test-threads=1
    else
        echo "=== Combination: $features ==="
        cargo clippy --all-targets --no-default-features --features "$features" -- -D warnings
        if [[ "$features" == *"multithreading"* ]]; then
            DOTNET_TEST_TIMEOUT_SECS=180 timeout 1200 cargo test --no-default-features --features "$features" -- --nocapture
            echo "=== Lock-order harness: $features ==="
            for TEST in "${LOCK_ORDER_TESTS[@]}"; do
                DOTNET_TEST_TIMEOUT_SECS=120 timeout 300 \
                    cargo test --no-default-features --features "$features" \
                    -p dotnet-vm "$TEST" -- --test-threads=1 --nocapture
            done
        else
            DOTNET_TEST_TIMEOUT_SECS=180 timeout 1200 cargo test --no-default-features --features "$features" -- --nocapture --test-threads=1
        fi
    fi
done

echo "Running experimental feature smoke tests..."
cargo test --features bench-instrumentation -- --nocapture
cargo test --features heap-diagnostics -- --nocapture
cargo test --features segmented-eval-stack-prototype -- --nocapture
cargo test -p dotnet-vm --features instruction-dispatch-jump-table -- --nocapture
cargo test -p dotnet-vm --features "instruction-dispatch-jump-table dispatch-super-instruction-prototype" -- --nocapture

echo "Running hang-probe integration tests..."
HANG_PROBE_TESTS=(
    "integration_tests_impl::fixtures::test_allocation_pressure"
    "integration_tests_impl::fixtures::test_gc_coordinator"
    "integration_tests_impl::fixtures::test_multiple_arenas"
    "integration_tests_impl::fixtures::test_stw_stress"
)
EXIT=0
for TEST in "${HANG_PROBE_TESTS[@]}"; do
    echo "  hang-probe: $TEST"
    DOTNET_TEST_TIMEOUT_SECS=60 timeout 300 \
        cargo test --no-default-features --features multithreading \
        -p dotnet-cli --test integration_tests "$TEST" \
        -- --test-threads=1 --nocapture || EXIT=$?
done
if [ "$EXIT" -ne 0 ]; then
    echo "One or more hang-probe integration tests failed." >&2
    exit "$EXIT"
fi

echo "All checks passed!"
