#[allow(unused_imports)]
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::integration_tests_impl::harness::TestHarness;
use dotnet_vm::state;

/// Helper function to set up a multi-arena test.
///
/// Returns the built DLL path for the given fixture.
/// Used by multi-arena tests that spawn threads to run the same program.
#[cfg(feature = "multithreading")]
pub fn setup_multi_arena_fixture(fixture_path: &str) -> PathBuf {
    let harness = TestHarness::get();
    harness.ensure_dll(Path::new(fixture_path))
}

#[test]
fn test_cache_observability() {
    let harness = TestHarness::get();
    let dll_path = harness
        .build(Path::new("tests/debug_fixtures/hello_world_0.cs"))
        .unwrap();
    let _ = harness.run(&dll_path);

    // Verify cache stats are tracking
    assert!(harness.loader.type_cache_size() > 0);
    assert!(harness.loader.method_cache_size() > 0);
}

#[test]
fn delegate_dispatch_classification_is_cached_by_resolved_method() {
    let harness = TestHarness::get();
    let dll_path = harness.ensure_dll(Path::new(
        "tests/fixtures/delegates/delegate_multicast_buffer_reuse_42.cs",
    ));
    let resolution = harness
        .loader
        .load_resolution_from_file(&dll_path)
        .expect("delegate fixture must load");
    #[allow(
        clippy::arc_with_non_send_sync,
        reason = "the no-MT integration test keeps shared state on its sole executor"
    )]
    let shared = Arc::new(state::SharedGlobalState::new(Arc::clone(&harness.loader)));

    let result = harness.run_with_shared(resolution, Arc::clone(&shared));
    assert_eq!(
        result.exit_code, 42,
        "delegate fixture failed: {:?}",
        result.stderr
    );

    let cache = shared.get_cache_stats().delegate_dispatch;
    assert!(
        cache.misses > 0,
        "delegate dispatch cache was never populated"
    );
    assert!(cache.hits > 0, "repeated delegate invokes missed the cache");
}

#[test]
fn static_constrained_dispatch_caches_only_exact_metadata() {
    let harness = TestHarness::get();
    let dll_path = harness.ensure_dll(Path::new(
        "tests/fixtures/interfaces/static_constrained_generic_default_42.cs",
    ));
    let resolution = harness
        .loader
        .load_resolution_from_file(&dll_path)
        .expect("static constrained fixture must load");
    #[allow(
        clippy::arc_with_non_send_sync,
        reason = "the no-MT integration test keeps shared state on its sole executor"
    )]
    let shared = Arc::new(state::SharedGlobalState::new(Arc::clone(&harness.loader)));

    let result = harness.run_with_shared(resolution, Arc::clone(&shared));
    assert_eq!(
        result.exit_code, 42,
        "static constrained fixture failed: {:?}",
        result.stderr
    );

    let cache = shared.get_cache_stats().static_constrained;
    let cache_enabled =
        std::env::var("DOTNET_STATIC_CONSTRAINED_CACHE").map_or(true, |value| value != "0");
    if cache_enabled {
        assert!(
            cache.misses > 0,
            "static constrained cache was never populated"
        );
        assert!(
            cache.hits > 0,
            "repeated static constrained calls missed the cache"
        );
    } else {
        assert_eq!(cache.hits, 0, "disabled cache recorded a hit");
        assert_eq!(cache.misses, 0, "disabled cache recorded a miss");
    }
}

#[test]
#[cfg(not(feature = "fuzzing"))]
fn hello_world() {
    let harness = TestHarness::get();
    let dll_path = harness
        .build(Path::new("tests/debug_fixtures/hello_world_0.cs"))
        .unwrap();
    let (exit_code, stdout) = harness.run_cli(&dll_path);

    assert_eq!(exit_code, 0);
    assert_eq!(stdout.trim(), "Hello, World!");
}

#[test]
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
fn managed_thread_lifecycle() {
    let harness = TestHarness::get();
    let dll_path = harness
        .build(Path::new(
            "tests/debug_fixtures/managed_thread_lifecycle_42.cs",
        ))
        .expect("managed Thread lifecycle fixture must build");
    let (exit_code, stdout) = harness.run_cli(&dll_path);

    assert_eq!(exit_code, 42, "managed Thread lifecycle fixture failed");
    assert!(stdout.trim().is_empty(), "unexpected stdout: {stdout:?}");
}

#[test]
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
fn pinvoke_last_error_isolation() {
    let harness = TestHarness::get();
    let dll_path = harness.ensure_dll(Path::new(
        "tests/fixtures/pinvoke/pinvoke_last_error_isolation_42.cs",
    ));
    let (exit_code, stdout) = harness.run_cli(&dll_path);

    assert_eq!(
        exit_code, 42,
        "P/Invoke last-error isolation fixture failed"
    );
    assert!(stdout.trim().is_empty(), "unexpected stdout: {stdout:?}");
}

multi_arena_test!(
    test_multiple_arenas_static_ref,
    "tests/fixtures/fields/static_ref_42.cs",
    3,
    42
);

multi_arena_test!(
    test_multiple_arenas_allocation_stress,
    "tests/fixtures/gc/cache_test_0.cs",
    4,
    0
);

multi_arena_test!(
    test_multiple_arenas_simple,
    "tests/fixtures/basic/basic_42.cs",
    3,
    42
);

multi_arena_test!(
    test_reflection_race_condition,
    "tests/fixtures/reflection/reflection_stress_0.cs",
    5,
    0
);

multi_arena_test!(
    test_gc_coordinator_multi_arena_tracking,
    "tests/fixtures/gc/cache_test_0.cs",
    2,
    0
);

multi_arena_test!(
    test_volatile_sharing,
    "tests/fixtures/threading/volatile_sharing_42.cs",
    2,
    42
);

multi_arena_test!(
    test_cross_arena_reference_tracking,
    "tests/fixtures/fields/static_ref_42.cs",
    2,
    42
);

multi_arena_test!(
    test_allocation_pressure_triggers_collection,
    "tests/fixtures/gc/cache_test_0.cs",
    2,
    0
);

multi_arena_test!(test_stw_stress, "tests/fixtures/gc/cache_test_0.cs", 6, 0);

multi_arena_test!(
    test_statics_circular_init_mt,
    "tests/fixtures/statics/circular_init_mt_42.cs",
    2,
    42
);

include!(concat!(env!("OUT_DIR"), "/tests.rs"));
