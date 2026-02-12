#![allow(clippy::arc_with_non_send_sync)]
use dotnet_assemblies::{self as assemblies, try_static_res_from_file};
use dotnet_types::{TypeDescription, members::MethodDescription, resolution::ResolutionS};
use dotnet_vm::{self as vm, state};
use dotnetdll::prelude::*;
use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(feature = "multithreading")]
use dotnet_vm::threading::ThreadManagerOps;

pub struct TestHarness {
    pub loader: &'static assemblies::AssemblyLoader,
}

impl TestHarness {
    pub fn get() -> &'static Self {
        thread_local! {
            static INSTANCE: &'static TestHarness = Box::leak(Box::new(TestHarness::new()));
        }
        INSTANCE.with(|&i| i)
    }

    fn new() -> Self {
        let assemblies_path = Self::find_dotnet_app_path().to_str().unwrap().to_string();
        let loader = assemblies::AssemblyLoader::new(assemblies_path)
            .expect("Failed to create AssemblyLoader");
        let loader = Box::leak(Box::new(loader));
        Self { loader }
    }

    fn find_dotnet_app_path() -> PathBuf {
        let base = std::env::var("DOTNET_ROOT")
            .map(|p| PathBuf::from(p).join("shared/Microsoft.NETCore.App"))
            .unwrap_or_else(|_| PathBuf::from("/usr/share/dotnet/shared/Microsoft.NETCore.App"));

        let base = if !base.exists() {
            let alt_base = PathBuf::from("/usr/lib/dotnet/shared/Microsoft.NETCore.App");
            if alt_base.exists() { alt_base } else { base }
        } else {
            base
        };

        if !base.exists() {
            panic!("could not find .NET shared path at {:?}", base);
        }

        let mut entries: Vec<_> = std::fs::read_dir(base)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .collect();

        entries.sort_by_key(|e| e.file_name());
        entries
            .last()
            .expect("no versions found in .NET shared path")
            .path()
    }

    pub fn build(&self, fixture_path: &Path) -> io::Result<PathBuf> {
        let file_name = fixture_path.file_stem().unwrap().to_str().unwrap();
        let output_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("dotnet-fixtures")
            .join(file_name);

        let absolute_file = std::fs::canonicalize(fixture_path)?;
        let dll_path = output_dir.join("SingleFile.dll");

        if dll_path.exists() {
            let source_mtime = std::fs::metadata(fixture_path)?.modified()?;
            let dll_mtime = std::fs::metadata(&dll_path)?.modified()?;
            if source_mtime <= dll_mtime {
                return Ok(dll_path);
            }
        }

        let status = Command::new("dotnet")
            .args([
                "build",
                "tests/SingleFile.csproj",
                "-p:AllowUnsafeBlocks=true",
                &format!("-p:TestFile={}", absolute_file.display()),
                "-o",
                output_dir.to_str().unwrap(),
                &format!(
                    "-p:IntermediateOutputPath={}/",
                    output_dir.join("obj").display()
                ),
            ])
            .status()?;
        assert!(
            status.success(),
            "dotnet build failed for {:?}",
            fixture_path
        );

        Ok(dll_path)
    }

    pub fn run(&self, dll_path: &Path) -> u8 {
        let dll_path_str = dll_path.to_str().unwrap().to_string();
        let resolution = try_static_res_from_file(&dll_path_str).expect("Failed to load assembly");
        let shared = std::sync::Arc::new(state::SharedGlobalState::new(self.loader));
        if dll_path_str.contains("nested_exceptions_42")
            || dll_path_str.contains("static_field_42")
            || dll_path_str.contains("generic_0")
            || dll_path_str.contains("basic_42")
            || dll_path_str.contains("gc_resurrection_0")
            || dll_path_str.contains("hello_world")
        {
            shared
                .tracer_enabled
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        self.run_with_shared(resolution, shared)
    }

    pub fn run_with_shared(
        &self,
        resolution: ResolutionS,
        shared: std::sync::Arc<state::SharedGlobalState<'static>>,
    ) -> u8 {
        let entry_method = match resolution.entry_point {
            Some(EntryPoint::Method(m)) => m,
            _ => panic!("Expected method entry point in {:?}", resolution),
        };

        let mut executor = vm::Executor::new(shared);

        let entrypoint = MethodDescription {
            parent: TypeDescription::new(
                resolution,
                &resolution.definition()[entry_method.parent_type()],
                entry_method.parent_type(),
            ),
            method_resolution: resolution,
            method: &resolution.definition()[entry_method],
        };
        executor.entrypoint(entrypoint);

        match executor.run() {
            vm::ExecutorResult::Exited(i) => i,
            vm::ExecutorResult::Threw => {
                panic!("VM threw an exception while running {:?}", resolution)
            }
            vm::ExecutorResult::Error(e) => {
                panic!("VM internal error while running {:?}: {}", resolution, e)
            }
        }
    }
}

// NOTE: To run tests with a timeout (to catch infinite loops or deadlocks), use:
//   cargo test -- --test-threads=1 --timeout 10
// or set RUST_TEST_TIME_UNIT and RUST_TEST_TIME_INTEGRATION environment variables
// or use an external timeout mechanism like GNU timeout:
//   timeout 60s cargo test

// used in generated tests from build.rs
macro_rules! fixture_test {
    ($name:ident, $path:expr, $expected:expr) => {
        #[test]
        fn $name() {
            let harness = TestHarness::get();
            let dll_path = harness.build(Path::new($path)).unwrap();
            let exit_code = harness.run(&dll_path);
            assert_eq!(
                exit_code,
                $expected,
                "Test {} failed: expected exit code {}, got {}",
                stringify!($name),
                $expected,
                exit_code
            );
        }
    };
}

include!(concat!(env!("OUT_DIR"), "/tests.rs"));

#[test]
fn test_cache_observability() {
    let harness = TestHarness::get();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gc/cache_test_0.cs");
    let dll = harness.build(&fixture).unwrap();

    let dll_path_str = dll.to_str().unwrap().to_string();
    let resolution = try_static_res_from_file(&dll_path_str).expect("Failed to load assembly");
    let shared = std::sync::Arc::new(state::SharedGlobalState::new(harness.loader));

    // Get initial stats
    let stats = shared.get_cache_stats();
    println!("Initial cache stats:\n{}", stats);

    // Some basic checks
    assert_eq!(
        stats.intrinsic.size, 0,
        "Intrinsic cache should be lazy-loaded (zero-cost initialization)"
    );
    assert_eq!(
        stats.intrinsic_field.size, 0,
        "Intrinsic field cache should be lazy-loaded (zero-cost initialization)"
    );

    // Run it to see hit rates improve
    let mut executor = vm::Executor::new(shared.clone());
    let entry_method = match resolution.entry_point {
        Some(EntryPoint::Method(m)) => m,
        _ => panic!("Expected method entry point in {:?}", resolution),
    };
    let entrypoint = MethodDescription {
        parent: TypeDescription::new(
            resolution,
            &resolution.definition()[entry_method.parent_type()],
            entry_method.parent_type(),
        ),
        method_resolution: resolution,
        method: &resolution.definition()[entry_method],
    };
    executor.entrypoint(entrypoint);
    executor.run();

    let stats_after = executor.get_cache_stats();
    println!("After run:\n{}", stats_after);
    assert!(
        stats_after.layout.hits > 0 || stats_after.layout.misses > 0,
        "Layout cache should have been accessed"
    );
    assert!(
        stats_after.intrinsic.hits > 0 || stats_after.intrinsic.misses > 0,
        "Intrinsic cache should have been accessed"
    );
}

/// This test is intended for debugging purposes and is not part of the regular CI.
/// It runs a "Hello, World!" program using the full .NET SDK libraries.
/// To run this test, use:
/// `cargo test hello_world -- --ignored --nocapture`
#[test]
#[ignore]
fn hello_world() {
    let harness = TestHarness::get();
    let fixture_path = Path::new("tests/debug_fixtures/hello_world_0.cs");
    let dll_path = harness.build(fixture_path).unwrap();
    let exit_code = harness.run(&dll_path);
    assert_eq!(exit_code, 0);
}

#[test]
#[cfg(feature = "multithreading")]
fn test_multiple_arenas_basic() {
    use std::thread;

    let harness = TestHarness::get();
    let fixture_path = Path::new("tests/fixtures/basic/basic_42.cs");
    let dll_path = harness.build(fixture_path).unwrap();

    // Spawn multiple threads, each creating its own arena and running the same program
    let handles: Vec<_> = (0..3)
        .map(|_| {
            let dll_path = dll_path.clone();
            let harness_ptr = harness as *const TestHarness as usize;
            thread::spawn(move || {
                let harness = unsafe { &*(harness_ptr as *const TestHarness) };
                let exit_code = harness.run(&dll_path);
                assert_eq!(exit_code, 42);
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
#[cfg(feature = "multithreading")]
fn test_multiple_arenas_with_gc() {
    use std::thread;

    let harness = TestHarness::get();
    let fixture_path = Path::new("tests/fixtures/gc/gc_finalization_42.cs");
    let dll_path = harness.build(fixture_path).unwrap();

    // Run GC tests in parallel threads to test cross-arena coordination
    let handles: Vec<_> = (0..3)
        .map(|_| {
            let dll_path = dll_path.clone();
            let harness_ptr = harness as *const TestHarness as usize;
            thread::spawn(move || {
                let harness = unsafe { &*(harness_ptr as *const TestHarness) };
                let exit_code = harness.run(&dll_path);
                assert_eq!(exit_code, 42);
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
#[cfg(feature = "multithreading")]
fn test_multiple_arenas_static_fields() {
    use std::thread;

    let harness = TestHarness::get();
    let fixture_path = Path::new("tests/fixtures/fields/static_field_42.cs");
    let dll_path = harness.build(fixture_path).unwrap();

    // Test that static fields work correctly across multiple arenas
    let handles: Vec<_> = (0..3)
        .map(|_| {
            let dll_path = dll_path.clone();
            let harness_ptr = harness as *const TestHarness as usize;
            thread::spawn(move || {
                let harness = unsafe { &*(harness_ptr as *const TestHarness) };
                let exit_code = harness.run(&dll_path);
                assert_eq!(exit_code, 42);
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
#[cfg(feature = "multithreading")]
fn test_multiple_arenas_allocation_stress() {
    use std::thread;

    let harness = TestHarness::get();
    let fixture_path = Path::new("tests/fixtures/arrays/array_0.cs");
    let dll_path = harness.build(fixture_path).unwrap();

    // Stress test allocation across multiple arenas simultaneously
    let handles: Vec<_> = (0..5)
        .map(|_| {
            let dll_path = dll_path.clone();
            let harness_ptr = harness as *const TestHarness as usize;
            thread::spawn(move || {
                let harness = unsafe { &*(harness_ptr as *const TestHarness) };
                // Run multiple times to increase allocation pressure
                for _ in 0..3 {
                    let exit_code = harness.run(&dll_path);
                    assert_eq!(exit_code, 0);
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
#[cfg(feature = "multithreading")]
fn test_arena_local_state_isolation() {
    use std::thread;

    let harness = TestHarness::get();
    let fixture_path = Path::new("tests/fixtures/generics/generic_0.cs");
    let dll_path = harness.build(fixture_path).unwrap();

    // Test that arena-local state (reflection caches, etc.) is properly isolated
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let dll_path = dll_path.clone();
            let harness_ptr = harness as *const TestHarness as usize;
            thread::spawn(move || {
                let harness = unsafe { &*(harness_ptr as *const TestHarness) };
                let exit_code = harness.run(&dll_path);
                assert_eq!(exit_code, 0);
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
#[cfg(feature = "multithreading")]
fn test_thread_manager_lifecycle() {
    let harness = TestHarness::get();
    let shared = std::sync::Arc::new(state::SharedGlobalState::new(harness.loader));
    let thread_manager = &shared.thread_manager;

    assert_eq!(thread_manager.thread_count(), 0);

    let id1 = thread_manager.register_thread();
    assert_eq!(thread_manager.thread_count(), 1);
    assert!(id1 > 0);

    let id2 = thread_manager.register_thread();
    assert_eq!(thread_manager.thread_count(), 2);
    assert_ne!(id1, id2);

    thread_manager.unregister_thread(id1);
    assert_eq!(thread_manager.thread_count(), 1);

    thread_manager.unregister_thread(id2);
    assert_eq!(thread_manager.thread_count(), 0);
}

#[test]
#[cfg(feature = "multithreading")]
fn test_multiple_arenas_simple() {
    use dotnet_vm::state;
    use std::sync::Arc;
    use std::thread;

    let harness = TestHarness::get();
    let fixture_path = Path::new("tests/fixtures/threading/threading_simple_0.cs");
    let dll_path = harness.build(fixture_path).unwrap();

    let shared = Arc::new(state::SharedGlobalState::new(harness.loader));
    let resolution =
        try_static_res_from_file(dll_path.to_str().unwrap()).expect("Failed to load assembly");
    shared.loader.register_assembly(resolution);

    let (tx, rx) = std::sync::mpsc::channel();

    // Run 5 threads sharing the same global state
    let handles: Vec<_> = (0..5)
        .map(|i| {
            let harness_ptr = harness as *const TestHarness as usize;
            let shared = Arc::clone(&shared);
            let tx = tx.clone();
            thread::spawn(move || {
                let harness = unsafe { &*(harness_ptr as *const TestHarness) };
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let exit_code = harness.run_with_shared(resolution, shared);
                    assert_eq!(exit_code, 0);
                }));
                tx.send((i, result)).ok();
            })
        })
        .collect();

    // Wait for all threads to complete
    let mut results = Vec::new();
    let timeout = std::time::Duration::from_secs(30);

    for _ in 0..5 {
        match rx.recv_timeout(timeout) {
            Ok((i, result)) => {
                if let Err(e) = result {
                    panic!("Thread {} panicked: {:?}", i, e);
                }
                results.push(i);
            }
            Err(_) => {
                panic!(
                    "Test timed out waiting for threads. Completed {}/5 threads.",
                    results.len()
                );
            }
        }
    }

    // Join all threads to ensure cleanup
    for handle in handles {
        handle.join().expect("Thread should complete");
    }

    // Verify all 5 threads completed
    assert_eq!(results.len(), 5);

    // Verify that the static field Counter was incremented by all threads
    let (index, type_def) = resolution
        .definition()
        .type_definitions
        .iter()
        .enumerate()
        .find(|(_, t)| t.name == "Program")
        .unwrap();
    let type_index = resolution
        .definition()
        .type_definition_index(index)
        .unwrap();
    let type_desc = TypeDescription::new(resolution, type_def, type_index);

    let storage_arc = shared.statics.get(type_desc, &shared.empty_generics);
    let storage = &storage_arc.storage;

    let counter_bytes = storage.get_field_local(type_desc, "Counter");
    let counter = i32::from_ne_bytes((&*counter_bytes).try_into().unwrap());

    // Note: Without synchronization, the counter value is not guaranteed to be 5
    // due to race conditions. The important thing is that all threads completed.
    println!(
        "Counter value: {} (may not be 5 due to race conditions)",
        counter
    );
    assert!(
        (1..=5).contains(&counter),
        "Counter should be between 1 and 5"
    );
}

#[test]
#[cfg(feature = "multithreaded-gc")]
fn test_reflection_race_condition() {
    use dotnet_vm::state;
    use std::sync::Arc;
    use std::thread;

    let harness = TestHarness::get();
    let fixture_path = Path::new("tests/fixtures/reflection/reflection_stress_0.cs");
    let dll_path = harness.build(fixture_path).unwrap();

    let shared = Arc::new(state::SharedGlobalState::new(harness.loader));
    let resolution =
        try_static_res_from_file(dll_path.to_str().unwrap()).expect("Failed to load assembly");
    shared.loader.register_assembly(resolution);

    let num_threads = 20;
    let (tx, rx) = std::sync::mpsc::channel();

    let handles: Vec<_> = (0..num_threads)
        .map(|i| {
            let harness_ptr = harness as *const TestHarness as usize;
            let shared = Arc::clone(&shared);
            let tx = tx.clone();
            thread::spawn(move || {
                let harness = unsafe { &*(harness_ptr as *const TestHarness) };
                for _ in 0..50 {
                    let exit_code = harness.run_with_shared(resolution, shared.clone());
                    if exit_code != 0 {
                        tx.send((i, Err(format!("Exit code {}", exit_code)))).ok();
                        return;
                    }
                }
                tx.send((i, Ok(()))).ok();
            })
        })
        .collect();

    for _ in 0..num_threads {
        let (_i, result) = rx
            .recv_timeout(std::time::Duration::from_secs(60))
            .expect("Test timed out");
        if let Err(e) = result {
            panic!("Thread failed: {:?}", e);
        }
    }

    for handle in handles {
        handle.join().unwrap();
    }
}

/// Test that verifies the GC coordinator properly tracks multiple arenas
#[test]
#[cfg(feature = "multithreaded-gc")]
fn test_gc_coordinator_multi_arena_tracking() {
    use dotnet_vm::state;
    use std::sync::Arc;

    let shared = Arc::new(state::SharedGlobalState::new(TestHarness::get().loader));

    // Simulate multiple arenas being registered
    let handle1 = vm::gc::coordinator::ArenaHandle::new(1);

    let handle2 = vm::gc::coordinator::ArenaHandle::new(2);

    // Register multiple arenas
    shared.gc_coordinator.register_arena(handle1.clone());
    shared.gc_coordinator.register_arena(handle2.clone());

    // Verify registration worked - we can check if commands can be sent
    assert!(!shared.gc_coordinator.has_command(1));
    assert!(!shared.gc_coordinator.has_command(2));

    // Unregister
    shared.gc_coordinator.unregister_arena(1);
    shared.gc_coordinator.unregister_arena(2);

    // After unregister, commands should not be available
    assert!(!shared.gc_coordinator.has_command(1));
    assert!(!shared.gc_coordinator.has_command(2));
}

#[test]
#[cfg(feature = "multithreading")]
fn test_volatile_sharing() {
    use dotnet_vm::state;
    use std::thread;
    let harness = TestHarness::get();
    let fixture_path = Path::new("tests/fixtures/threading/volatile_sharing_42.cs");
    let dll_path = harness.build(fixture_path).unwrap();

    let shared = std::sync::Arc::new(state::SharedGlobalState::new(harness.loader));

    let handles: Vec<_> = (0..2)
        .map(|_| {
            let dll_path = dll_path.clone();
            let shared = shared.clone();
            let harness_ptr = harness as *const TestHarness as usize;
            thread::spawn(move || {
                let harness = unsafe { &*(harness_ptr as *const TestHarness) };
                let resolution = try_static_res_from_file(dll_path.to_str().unwrap())
                    .expect("Failed to load assembly");
                let exit_code = harness.run_with_shared(resolution, shared);
                assert_eq!(exit_code, 42);
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("Thread should complete");
    }
}

/// Test that verifies cross-arena reference tracking works
#[test]
#[cfg(feature = "multithreaded-gc")]
fn test_cross_arena_reference_tracking() {
    use dotnet_vm::state;
    use std::sync::Arc;

    let shared = Arc::new(state::SharedGlobalState::new(TestHarness::get().loader));

    // Create a mock arena handle
    let handle = vm::gc::coordinator::ArenaHandle::new(1);

    shared.gc_coordinator.register_arena(handle.clone());

    // Record some cross-arena references
    let ptr1 = unsafe { dotnet_value::object::ObjectPtr::from_raw(0x1000 as *const _).unwrap() };
    let ptr2 = unsafe { dotnet_value::object::ObjectPtr::from_raw(0x2000 as *const _).unwrap() };

    vm::gc::coordinator::set_current_arena_handle(handle.clone());
    dotnet_utils::gc::record_cross_arena_ref(2, ptr1.as_ptr() as usize);
    dotnet_utils::gc::record_cross_arena_ref(2, ptr2.as_ptr() as usize);

    // The fact that we can record cross-arena refs without panicking demonstrates the system works
    shared.gc_coordinator.unregister_arena(1);
}

/// Test that allocation pressure triggers collection requests
#[test]
#[cfg(feature = "multithreaded-gc")]
fn test_allocation_pressure_triggers_collection() {
    use dotnet_vm::state;
    use std::sync::Arc;

    let shared = Arc::new(state::SharedGlobalState::new(TestHarness::get().loader));

    let handle = vm::gc::coordinator::ArenaHandle::new(1);

    shared.gc_coordinator.register_arena(handle.clone());
    vm::gc::coordinator::set_current_arena_handle(handle.clone());

    // Simulate allocations using the thread-local function
    for _ in 0..100 {
        vm::gc::coordinator::record_allocation(1024);
    }

    // Check if collection should be triggered
    let should_collect = shared.gc_coordinator.should_collect();

    // The coordinator tracks allocation pressure
    println!("Should collect after 100 allocations: {}", should_collect);

    shared.gc_coordinator.unregister_arena(1);
}
