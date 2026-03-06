#[allow(unused_imports)]
use std::path::{Path, PathBuf};

use crate::integration_tests_impl::harness::TestHarness;

/// Helper function to set up a multi-arena test.
///
/// Returns the built DLL path for the given fixture.
/// Used by multi-arena tests that spawn threads to run the same program.
#[cfg(feature = "multithreading")]
pub fn setup_multi_arena_fixture(fixture_path: &str) -> PathBuf {
    let harness = TestHarness::get();
    let path = Path::new(fixture_path);
    let dll_path = harness.prebuilt_dll_path(path);
    if dll_path.exists() {
        dll_path
    } else {
        harness.build(path).unwrap()
    }
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
fn hello_world() {
    let harness = TestHarness::get();
    let dll_path = harness
        .build(Path::new("tests/debug_fixtures/hello_world_0.cs"))
        .unwrap();
    let exit_code = harness.run(&dll_path);
    assert_eq!(exit_code, 0);
}

#[cfg(feature = "multithreading")]
multi_arena_test!(
    test_multiple_arenas_static_ref,
    "tests/fixtures/fields/static_ref_42.cs",
    3,
    42
);

#[cfg(feature = "multithreading")]
multi_arena_test!(
    test_multiple_arenas_allocation_stress,
    "tests/fixtures/gc/cache_test_0.cs",
    4,
    0
);

#[cfg(feature = "multithreading")]
#[test]
fn test_thread_manager_lifecycle() {
    use dotnet_vm::threading::ThreadManagerOps;
    let harness = TestHarness::get();
    let shared = std::sync::Arc::new(dotnet_vm::state::SharedGlobalState::new(
        harness.loader.clone(),
    ));

    let handle = std::thread::spawn(move || {
        let _id = shared.thread_manager.register_thread();
        // Do nothing
    });

    handle.join().unwrap();
}

#[cfg(feature = "multithreading")]
multi_arena_test!(
    test_multiple_arenas_simple,
    "tests/fixtures/basic/basic_42.cs",
    3,
    42
);

#[cfg(feature = "multithreading")]
multi_arena_test!(
    test_reflection_race_condition,
    "tests/fixtures/reflection/reflection_stress_0.cs",
    5,
    0
);

#[cfg(feature = "multithreading")]
multi_arena_test!(
    test_gc_coordinator_multi_arena_tracking,
    "tests/fixtures/gc/cache_test_0.cs",
    2,
    0
);

#[cfg(feature = "multithreading")]
multi_arena_test!(
    test_volatile_sharing,
    "tests/fixtures/threading/volatile_sharing_42.cs",
    2,
    42
);

#[cfg(feature = "multithreading")]
multi_arena_test!(
    test_cross_arena_reference_tracking,
    "tests/fixtures/fields/static_ref_42.cs",
    2,
    42
);

#[cfg(feature = "multithreading")]
multi_arena_test!(
    test_allocation_pressure_triggers_collection,
    "tests/fixtures/gc/cache_test_0.cs",
    2,
    0
);

#[cfg(feature = "multithreading")]
multi_arena_test!(test_stw_stress, "tests/fixtures/gc/cache_test_0.cs", 6, 0);

include!(concat!(env!("OUT_DIR"), "/tests.rs"));
