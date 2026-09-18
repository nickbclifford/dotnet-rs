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
// divergence); exceptions/exception_filter_5 (VM correctness mismatch);
// exceptions/unhandled_exception_254 and stack_trace_*_254 (the in-process fixture oracle is
// reserved code 254, while the unchanged subprocess behavior is real .NET exit 134 versus
// dotnet-rs CLI exit 1 for unhandled exceptions); exceptions/intrinsic_trace_42 (rejected from
// full stdout comparison for its GC.Collect mismatch, but covered by the qualified exit-code-only
// differential test);
// structs/explicit_layout_0 (real .NET exits 134); unsafe/managed_ptr_size_0 (fat pointer
// representation); and threading/threading_*_0 (exit-code-only comparison has no signal).
// The original expression-compilation case is retained while the canonical set expands coverage.
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
