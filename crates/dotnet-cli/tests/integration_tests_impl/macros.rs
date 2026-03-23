// used in generated tests from build.rs
macro_rules! fixture_test {
    ($(#[$attr:meta])* $name:ident, $dll_path:expr, $expected:expr) => {
        fixture_test!($(#[$attr])* $name, $dll_path, $expected, None);
    };
    ($(#[$attr:meta])* $name:ident, $dll_path:expr, $expected:expr, $source_path:expr) => {
        #[test]
        $(#[$attr])*
        fn $name() {
            use crate::integration_tests_impl::harness::{TestHarness, get_test_timeout};
            let harness = TestHarness::get();
            let source_path: Option<&str> = $source_path;
            let dll_path = if let Some(source) = source_path {
                harness.ensure_dll(std::path::Path::new(source))
            } else {
                std::path::PathBuf::from($dll_path)
            };
            let result = harness.run_with_timeout(&dll_path, get_test_timeout(60));

            assert_eq!(
                result.exit_code,
                $expected,
                "Test {} failed: expected exit code {}, got {}. Stderr: {:?}",
                stringify!($name),
                $expected,
                result.exit_code,
                result.stderr
            );
        }
    };
}

/// Macro for multi-arena tests that run the same fixture across multiple threads.
#[cfg(feature = "multithreading")]
macro_rules! multi_arena_test {
    ($name:ident, $fixture:expr, $thread_count:expr, $expected:expr) => {
        multi_arena_test!($name, $fixture, $thread_count, $expected, $fixture);
    };
    ($name:ident, $fixture:expr, $thread_count:expr, $expected:expr, $source_path:expr) => {
        #[test]
        #[cfg(feature = "multithreading")]
        fn $name() {
            use crate::integration_tests_impl::fixtures::setup_multi_arena_fixture;
            use crate::integration_tests_impl::harness::{TestHarness, get_test_timeout};

            let dll_path = setup_multi_arena_fixture($source_path);
            let harness = TestHarness::get();
            harness.run_multi_arena_fixture(
                &dll_path,
                $thread_count,
                $expected,
                get_test_timeout(60),
                stringify!($name),
            );
        }
    };
}
