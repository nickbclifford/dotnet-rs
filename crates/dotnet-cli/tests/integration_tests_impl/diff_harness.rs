macro_rules! diff_test {
    ($name:ident, $fixture_path:expr) => {
        #[cfg(not(feature = "fuzzing"))]
        #[test]
        fn $name() {
            use crate::integration_tests_impl::harness::TestHarness;
            use std::{path::Path, process::Command};

            let harness = TestHarness::get();
            let dll_path = harness.ensure_dll(Path::new($fixture_path));

            let dotnet_output = Command::new("dotnet")
                .arg(&dll_path)
                .output()
                .expect("failed to run fixture with dotnet");
            let dotnet_exit_code = dotnet_output.status.code().unwrap_or(255) as u8;
            let dotnet_stdout = String::from_utf8_lossy(&dotnet_output.stdout).to_string();

            let (dotnet_rs_exit_code, dotnet_rs_stdout) = harness.run_cli(&dll_path);

            assert_eq!(
                dotnet_exit_code, 42,
                "sanity check failed: dotnet should return fixture exit code 42"
            );
            assert_eq!(
                dotnet_rs_exit_code, dotnet_exit_code,
                "dotnet-rs exit code diverged from dotnet"
            );
            assert_eq!(
                dotnet_rs_stdout, dotnet_stdout,
                "dotnet-rs stdout diverged from dotnet"
            );
        }
    };
}

macro_rules! diff_test_exit_code {
    ($name:ident, $fixture_path:expr, $expected_exit_code:expr) => {
        #[cfg(not(feature = "fuzzing"))]
        #[test]
        fn $name() {
            use crate::integration_tests_impl::harness::TestHarness;
            use std::{path::Path, process::Command};

            let harness = TestHarness::get();
            let dll_path = harness.ensure_dll(Path::new($fixture_path));

            let dotnet_output = Command::new("dotnet")
                .arg(&dll_path)
                .output()
                .expect("failed to run fixture with dotnet");
            let dotnet_exit_code = dotnet_output.status.code().unwrap_or(255) as u8;

            let (dotnet_rs_exit_code, _) = harness.run_cli(&dll_path);
            let expected_exit_code: u8 = $expected_exit_code;

            assert_eq!(
                dotnet_exit_code, expected_exit_code,
                "sanity check failed: dotnet should return fixture exit code {}",
                expected_exit_code
            );
            assert_eq!(
                dotnet_rs_exit_code, dotnet_exit_code,
                "dotnet-rs exit code diverged from dotnet"
            );
        }
    };
}

// Rejected candidates: structs/interlocked_misaligned_1 (intentional ECMA alignment
// divergence);
// exceptions/unhandled_exception_254 and stack_trace_*_254 (the in-process fixture oracle is
// reserved code 254, while the unchanged subprocess behavior is real .NET exit 134 versus
// dotnet-rs CLI exit 1 for unhandled exceptions); exceptions/intrinsic_trace_42 (rejected from
// full stdout comparison for its GC.Collect mismatch, but covered by the qualified exit-code-only
// differential test);
// structs/explicit_layout_0 (real .NET exits 134); unsafe/managed_ptr_size_0 (fat pointer
// representation); and threading/threading_*_0 (exit-code-only comparison has no signal).
// The original expression-compilation case is retained while the canonical set expands coverage.
diff_test!(
    statics_cctor_filter_matches_dotnet,
    "tests/fixtures/statics/cctor_filter_42.cs"
);
diff_test!(
    statics_cctor_field_operations_matches_dotnet,
    "tests/fixtures/statics/cctor_field_operations_42.cs"
);
#[cfg(feature = "multithreading")]
diff_test!(
    statics_cctor_wait_success_matches_dotnet,
    "tests/fixtures/statics/cctor_wait_success_42.cs"
);
#[cfg(feature = "multithreading")]
diff_test!(
    statics_cctor_wait_failure_matches_dotnet,
    "tests/fixtures/statics/cctor_wait_failure_42.cs"
);
diff_test!(
    reflection_invoke_reference_byref_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_reference_byref_42.cs"
);
diff_test!(
    reflection_invoke_fallback_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_fallback_42.cs"
);
diff_test!(
    reflection_invoke_constructors_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_constructors_42.cs"
);
diff_test!(
    reflection_invoke_results_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_results_42.cs"
);
diff_test!(
    reflection_invoke_intrinsic_value_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_intrinsic_value_42.cs"
);
diff_test!(
    reflection_invoke_byref_gc_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_byref_gc_42.cs"
);
diff_test!(
    fields_enum_frame_homes_42_matches_dotnet,
    "tests/fixtures/fields/enum_frame_homes_42.cs"
);
diff_test!(
    delegates_multicast_static_ref_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_static_ref_42.cs"
);
diff_test!(
    delegates_multicast_many_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_many_42.cs"
);
diff_test!(
    delegates_multicast_scalar_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_scalar_42.cs"
);
diff_test!(
    delegates_multicast_aggregate_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_aggregate_42.cs"
);
diff_test!(
    delegates_multicast_nested_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_nested_42.cs"
);
diff_test!(
    delegates_multicast_reflection_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_reflection_42.cs"
);
diff_test!(
    delegates_multicast_finalizers_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_finalizers_42.cs"
);
diff_test!(
    delegates_multicast_filters_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_filters_42.cs"
);
diff_test!(
    delegates_multicast_static_ref_filters_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_static_ref_filters_42.cs"
);
diff_test!(
    delegates_multicast_refs_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_refs_42.cs"
);
diff_test!(
    delegates_multicast_ref_cctor_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_ref_cctor_42.cs"
);
diff_test!(
    delegates_multicast_cctor_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_cctor_42.cs"
);
diff_test!(
    delegates_multicast_static_ref_cctor_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_static_ref_cctor_42.cs"
);
diff_test!(
    strings_substring_chunks_42_matches_dotnet,
    "tests/fixtures/strings/substring_chunks_42.cs"
);
diff_test!(
    strings_substring_fallback_42_matches_dotnet,
    "tests/fixtures/strings/substring_fallback_42.cs"
);
diff_test!(
    expressions_expression_compile_42_matches_dotnet,
    "tests/fixtures/expressions/expression_compile_42.cs"
);
diff_test!(
    arithmetic_arithmetic_operations_42_matches_dotnet,
    "tests/fixtures/arithmetic/arithmetic_operations_42.cs"
);
diff_test!(
    exceptions_exception_batch_42_matches_dotnet,
    "tests/fixtures/exceptions/exception_batch_42.cs"
);
diff_test!(
    exceptions_nested_exceptions_42_matches_dotnet,
    "tests/fixtures/exceptions/nested_exceptions_42.cs"
);
diff_test!(
    memory_nullable_boxing_42_matches_dotnet,
    "tests/fixtures/memory/nullable_boxing_42.cs"
);
diff_test!(
    threading_interlocked_42_matches_dotnet,
    "tests/fixtures/threading/interlocked_42.cs"
);
diff_test!(
    threading_volatile_intrinsics_42_matches_dotnet,
    "tests/fixtures/threading/volatile_intrinsics_42.cs"
);
diff_test_exit_code!(
    exceptions_intrinsic_trace_42_matches_dotnet_exit_code,
    "tests/fixtures/exceptions/intrinsic_trace_42.cs",
    42
);

diff_test!(
    nested_filter_catches_matches_dotnet,
    "tests/fixtures/exceptions/nested_filter_catches_42.cs"
);

diff_test!(
    nested_filter_throws_matches_dotnet,
    "tests/fixtures/exceptions/nested_filter_throws_42.cs"
);

diff_test!(
    nested_filter_depth_matches_dotnet,
    "tests/fixtures/exceptions/nested_filter_depth_42.cs"
);

diff_test!(
    nested_filter_byref_matches_dotnet,
    "tests/fixtures/exceptions/nested_filter_byref_42.cs"
);

diff_test!(
    nested_filter_rethrow_matches_dotnet,
    "tests/fixtures/exceptions/nested_filter_rethrow_42.cs"
);

diff_test_exit_code!(
    exceptions_exception_filter_7_matches_dotnet,
    "tests/fixtures/exceptions/exception_filter_7.cs",
    7
);

#[cfg(feature = "multithreading")]
diff_test!(
    threading_monitor_contended_lock_42_matches_dotnet,
    "tests/fixtures/threading/monitor_contended_lock_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    threading_monitor_fresh_object_race_42_matches_dotnet,
    "tests/fixtures/threading/monitor_fresh_object_race_42.cs"
);

diff_test!(
    threading_monitor_byref_origins_42_matches_dotnet,
    "tests/fixtures/threading/monitor_byref_origins_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    threading_foreground_worker_42_matches_dotnet,
    "tests/fixtures/threading/foreground_worker_42.cs"
);

diff_test!(
    weak_reference_liveness_matches_dotnet,
    "tests/fixtures/gc/weak_reference_liveness_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    retired_arena_cycles_matches_dotnet,
    "tests/fixtures/gc/retired_arena_cycles_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    retired_finalizer_matches_dotnet,
    "tests/debug_fixtures/retired_finalizer_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    retired_finalizer_monitor_matches_dotnet,
    "tests/debug_fixtures/retired_finalizer_monitor_42.cs"
);
diff_test!(
    finalizer_collect_matches_dotnet,
    "tests/fixtures/gc/finalizer_collect_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    retired_finalizer_collect_matches_dotnet,
    "tests/debug_fixtures/retired_finalizer_collect_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    global_finalizer_wait_matches_dotnet,
    "tests/debug_fixtures/global_finalizer_wait_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    live_finalizer_wait_matches_dotnet,
    "tests/debug_fixtures/live_finalizer_wait_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    foreground_finalizer_wait_matches_dotnet,
    "tests/debug_fixtures/foreground_finalizer_wait_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    thread_finalization_matches_dotnet,
    "tests/debug_fixtures/thread_finalization_42.cs"
);
