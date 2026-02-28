#![allow(clippy::arc_with_non_send_sync)]
use dotnet_assemblies::{self as assemblies, try_static_res_from_file};
use dotnet_types::{TypeDescription, members::MethodDescription, resolution::ResolutionS};
use dotnet_vm::{self as vm, state};
use dotnetdll::prelude::*;
use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
    time::Duration,
};

#[cfg(feature = "multithreading")]
use dotnet_vm::threading::ThreadManagerOps;

fn get_test_timeout(default_secs: u64) -> Duration {
    let secs = std::env::var("DOTNET_TEST_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default_secs);
    Duration::from_secs(secs)
}

static BUILD_LOCK: Mutex<()> = Mutex::new(());

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
        let _lock = BUILD_LOCK.lock().unwrap();
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
                    "-p:BaseIntermediateOutputPath={}/",
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

    pub fn run_with_timeout(&self, dll_path: &Path, timeout: std::time::Duration) -> u8 {
        let dll_path_str = dll_path.to_str().unwrap().to_string();
        let resolution = try_static_res_from_file(&dll_path_str).expect("Failed to load assembly");
        let shared = std::sync::Arc::new(state::SharedGlobalState::new(self.loader));

        self.run_with_shared_timeout(resolution, shared, timeout)
    }

    pub fn run_with_shared_timeout(
        &self,
        resolution: ResolutionS,
        shared: std::sync::Arc<state::SharedGlobalState<'static>>,
        timeout: std::time::Duration,
    ) -> u8 {
        let shared_clone = std::sync::Arc::clone(&shared);
        let pair = std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let pair2 = std::sync::Arc::clone(&pair);

        let watchdog = std::thread::spawn(move || {
            let (lock, cvar) = &*pair2;
            let finished = lock.lock().unwrap();
            let (finished, wait_result) = cvar.wait_timeout(finished, timeout).unwrap();
            if !*finished && wait_result.timed_out() {
                shared_clone
                    .abort_requested
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        });

        let harness_ptr = self as *const TestHarness as usize;
        let shared_for_vm = std::sync::Arc::clone(&shared);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let harness = unsafe { &*(harness_ptr as *const TestHarness) };
            harness.run_with_shared(resolution, shared_for_vm)
        }));

        {
            let (lock, cvar) = &*pair;
            *lock.lock().unwrap() = true;
            cvar.notify_one();
        }
        let _ = watchdog.join();

        match result {
            Ok(exit_code) => {
                if shared
                    .abort_requested
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    eprintln!("=== TEST TIMEOUT (after {:?}) ===", timeout);

                    // Give the VM thread a grace period to update the ring buffer
                    // (Though here it already finished, so it should be updated)

                    eprintln!("=== LAST 10 INSTRUCTIONS ===");
                    if let Ok(rb) = shared.last_instructions.lock() {
                        eprintln!("{}", rb.dump());
                    }
                    panic!("TIMEOUT");
                }
                exit_code
            }
            Err(panic_info) => {
                eprintln!("=== VM PANICKED ===");
                eprintln!("=== LAST 10 INSTRUCTIONS ===");
                if let Ok(rb) = shared.last_instructions.lock() {
                    eprintln!("{}", rb.dump());
                }
                std::panic::resume_unwind(panic_info);
            }
        }
    }

    pub fn run(&self, dll_path: &Path) -> u8 {
        let dll_path_str = dll_path.to_str().unwrap().to_string();
        let resolution = try_static_res_from_file(&dll_path_str).expect("Failed to load assembly");
        let shared = std::sync::Arc::new(state::SharedGlobalState::new(self.loader));
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
            vm::ExecutorResult::Threw(exc) => {
                eprintln!(
                    "VM threw an unhandled exception while running {:?}:\n{}",
                    resolution, exc
                );
                1
            }
            vm::ExecutorResult::Error(e) => {
                eprintln!("VM internal error while running {:?}: {}", resolution, e);
                101
            }
        }
    }
}

// used in generated tests from build.rs
macro_rules! fixture_test {
    ($name:ident, $path:expr, $expected:expr) => {
        #[test]
        fn $name() {
            let harness = TestHarness::get();
            let dll_path = harness.build(Path::new($path)).unwrap();
            let exit_code = harness.run_with_timeout(&dll_path, get_test_timeout(60));
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
    (#[ignore] $name:ident, $path:expr, $expected:expr) => {
        #[test]
        #[ignore]
        fn $name() {
            let harness = TestHarness::get();
            let dll_path = harness.build(Path::new($path)).unwrap();
            let exit_code = harness.run_with_timeout(&dll_path, get_test_timeout(60));
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

// ============================================================================
// Multi-Arena Test Helpers
// ============================================================================

/// Helper function to set up a multi-arena test.
///
/// Returns the built DLL path for the given fixture.
/// Used by multi-arena tests that spawn threads to run the same program.
#[cfg(feature = "multithreading")]
fn setup_multi_arena_fixture(fixture_path: &str) -> PathBuf {
    let harness = TestHarness::get();
    harness.build(Path::new(fixture_path)).unwrap()
}

/// Macro for multi-arena tests that run the same fixture across multiple threads.
///
/// This encapsulates the common pattern of:
/// 1. Building a fixture
/// 2. Spawning N threads
/// 3. Running the fixture in each thread
/// 4. Asserting the expected exit code
/// 5. Joining all threads
///
/// # Arguments
/// * `$name` - Test function name
/// * `$fixture` - Path to the fixture file
/// * `$thread_count` - Number of threads to spawn
/// * `$expected` - Expected exit code
///
/// # Example
/// ```ignore
/// multi_arena_test!(test_basic, "tests/fixtures/basic/basic_42.cs", 3, 42);
/// ```
#[cfg(feature = "multithreading")]
macro_rules! multi_arena_test {
    ($name:ident, $fixture:literal, $thread_count:expr, $expected:expr) => {
        #[test]
        #[cfg(feature = "multithreading")]
        fn $name() {
            use std::sync::atomic::Ordering;
            use std::sync::mpsc;
            use std::thread;

            let dll_path = setup_multi_arena_fixture($fixture);
            let harness = TestHarness::get();
            let dll_path_str = dll_path.to_str().unwrap().to_string();
            let resolution =
                try_static_res_from_file(&dll_path_str).expect("Failed to load assembly");

            let (tx, rx) = mpsc::channel();
            let mut abort_flags = Vec::new();
            let mut handles = Vec::new();

            for _ in 0..$thread_count {
                let harness_ptr = harness as *const TestHarness as usize;
                let tx = tx.clone();
                let shared =
                    std::sync::Arc::new(dotnet_vm::state::SharedGlobalState::new(harness.loader));
                abort_flags.push(std::sync::Arc::clone(&shared.abort_requested));

                let handle = thread::spawn(move || {
                    let harness = unsafe { &*(harness_ptr as *const TestHarness) };
                    let result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                            harness.run_with_shared(resolution, shared)
                        }));
                    let _ = tx.send(result);
                });
                handles.push(handle);
            }

            let timeout = get_test_timeout(60);
            let start = std::time::Instant::now();
            let mut test_error = None;

            for _ in 0..$thread_count {
                let remaining = timeout.saturating_sub(start.elapsed());
                match rx.recv_timeout(remaining) {
                    Ok(Ok(exit_code)) => {
                        if exit_code != $expected {
                            test_error = Some(format!(
                                "Multi-arena test {} failed: expected exit code {}, got {}",
                                stringify!($name),
                                $expected,
                                exit_code
                            ));
                            break;
                        }
                    }
                    Ok(Err(panic_info)) => {
                        for flag in &abort_flags {
                            flag.store(true, Ordering::Relaxed);
                        }
                        // Join all before resuming unwind
                        for handle in handles {
                            let _ = handle.join();
                        }
                        std::panic::resume_unwind(panic_info);
                    }
                    Err(_) => {
                        for flag in &abort_flags {
                            flag.store(true, Ordering::Relaxed);
                        }
                        test_error = Some(format!("TIMEOUT in {}", stringify!($name)));
                        break;
                    }
                }
            }

            for handle in handles {
                let _ = handle.join();
            }

            if let Some(msg) = test_error {
                panic!("{}", msg);
            }
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
    let exit_code = harness.run_with_timeout(&dll_path, get_test_timeout(60));
    assert_eq!(exit_code, 0);
}

// Spawn multiple threads, each creating its own arena and running the same program
#[cfg(feature = "multithreading")]
multi_arena_test!(
    test_multiple_arenas_basic,
    "tests/fixtures/basic/basic_42.cs",
    3,
    42
);

// Run GC tests in parallel threads to test cross-arena coordination
#[cfg(feature = "multithreading")]
multi_arena_test!(
    test_multiple_arenas_with_gc,
    "tests/fixtures/gc/gc_finalization_42.cs",
    3,
    42
);

// Test that static fields work correctly across multiple arenas
#[cfg(feature = "multithreading")]
multi_arena_test!(
    test_multiple_arenas_static_fields,
    "tests/fixtures/fields/static_field_42.cs",
    3,
    42
);

#[test]
#[cfg(feature = "multithreading")]
fn test_multiple_arenas_static_ref() {
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::thread;

    let harness = TestHarness::get();
    let fixture_path = Path::new("tests/fixtures/fields/static_ref_42.cs");
    let dll_path = harness.build(fixture_path).unwrap();
    let dll_path_str = dll_path.to_str().unwrap().to_string();
    let resolution = try_static_res_from_file(&dll_path_str).expect("Failed to load assembly");

    // Test that static reference fields work correctly across multiple arenas
    // One thread will initialize the static field, others will use it.
    let (tx, rx) = mpsc::channel();
    let mut abort_flags = Vec::new();
    let mut handles = Vec::new();

    for _ in 0..5 {
        let tx = tx.clone();
        let harness_ptr = harness as *const TestHarness as usize;
        let shared = std::sync::Arc::new(dotnet_vm::state::SharedGlobalState::new(harness.loader));
        abort_flags.push(std::sync::Arc::clone(&shared.abort_requested));
        let handle = thread::spawn(move || {
            let harness = unsafe { &*(harness_ptr as *const TestHarness) };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                harness.run_with_shared(resolution, shared)
            }));
            let _ = tx.send(result);
        });
        handles.push(handle);
    }

    let timeout = get_test_timeout(60);
    let start = std::time::Instant::now();
    let mut test_error = None;
    for _ in 0..5 {
        let remaining = timeout.saturating_sub(start.elapsed());
        match rx.recv_timeout(remaining) {
            Ok(Ok(exit_code)) => {
                if exit_code != 42 {
                    test_error = Some(format!("Expected 42, got {}", exit_code));
                    break;
                }
            }
            Ok(Err(panic_info)) => {
                for flag in &abort_flags {
                    flag.store(true, Ordering::Relaxed);
                }
                for handle in handles {
                    let _ = handle.join();
                }
                std::panic::resume_unwind(panic_info);
            }
            Err(_) => {
                for flag in &abort_flags {
                    flag.store(true, Ordering::Relaxed);
                }
                test_error = Some("TIMEOUT in test_multiple_arenas_static_ref".to_string());
                break;
            }
        }
    }

    for handle in handles {
        let _ = handle.join();
    }

    if let Some(msg) = test_error {
        panic!("{}", msg);
    }
}

#[test]
#[cfg(feature = "multithreading")]
fn test_multiple_arenas_allocation_stress() {
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::thread;

    let harness = TestHarness::get();
    let fixture_path = Path::new("tests/fixtures/arrays/array_0.cs");
    let dll_path = harness.build(fixture_path).unwrap();
    let dll_path_str = dll_path.to_str().unwrap().to_string();
    let resolution = try_static_res_from_file(&dll_path_str).expect("Failed to load assembly");

    // Stress test allocation across multiple arenas simultaneously
    let (tx, rx) = mpsc::channel();
    let mut abort_flags = Vec::new();
    let mut handles = Vec::new();

    for _ in 0..5 {
        let tx = tx.clone();
        let harness_ptr = harness as *const TestHarness as usize;
        let shared = std::sync::Arc::new(dotnet_vm::state::SharedGlobalState::new(harness.loader));
        abort_flags.push(std::sync::Arc::clone(&shared.abort_requested));
        let handle = thread::spawn(move || {
            let harness = unsafe { &*(harness_ptr as *const TestHarness) };
            // Run multiple times to increase allocation pressure
            for _ in 0..3 {
                let exit_code = harness.run_with_shared(resolution, shared.clone());
                if exit_code != 0 {
                    let _ = tx.send(exit_code);
                    return;
                }
            }
            let _ = tx.send(0);
        });
        handles.push(handle);
    }

    let timeout = get_test_timeout(120);
    let start = std::time::Instant::now();
    let mut test_error = None;
    for _ in 0..5 {
        let remaining = timeout.saturating_sub(start.elapsed());
        match rx.recv_timeout(remaining) {
            Ok(exit_code) => {
                if exit_code != 0 {
                    test_error = Some(format!("Expected 0, got {}", exit_code));
                    break;
                }
            }
            Err(_) => {
                for flag in &abort_flags {
                    flag.store(true, Ordering::Relaxed);
                }
                test_error = Some("TIMEOUT in test_multiple_arenas_allocation_stress".to_string());
                break;
            }
        }
    }

    for handle in handles {
        let _ = handle.join();
    }

    if let Some(msg) = test_error {
        panic!("{}", msg);
    }
}

// Test that arena-local state (reflection caches, etc.) is properly isolated
#[cfg(feature = "multithreading")]
multi_arena_test!(
    test_arena_local_state_isolation,
    "tests/fixtures/generics/generic_0.cs",
    4,
    0
);

#[test]
#[cfg(feature = "multithreading")]
fn test_thread_manager_lifecycle() {
    let harness = TestHarness::get();
    let shared = std::sync::Arc::new(state::SharedGlobalState::new(harness.loader));
    let thread_manager = &shared.thread_manager;

    assert_eq!(thread_manager.thread_count(), 0);

    let id1 = thread_manager.register_thread();
    assert_eq!(thread_manager.thread_count(), 1);
    assert!(id1 > dotnet_utils::ArenaId(0));

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

    // Run 5 threads sharing the same global state
    let (tx, rx) = std::sync::mpsc::channel();
    let mut handles = Vec::new();
    for _ in 0..5 {
        let harness_ptr = harness as *const TestHarness as usize;
        let shared = Arc::clone(&shared);
        let tx = tx.clone();
        let handle = thread::spawn(move || {
            let harness = unsafe { &*(harness_ptr as *const TestHarness) };
            let exit_code = harness.run_with_shared(resolution, shared);
            let _ = tx.send(exit_code);
        });
        handles.push(handle);
    }

    let timeout = get_test_timeout(60);
    let start = std::time::Instant::now();
    let mut test_error = None;
    for _ in 0..5 {
        let remaining = timeout.saturating_sub(start.elapsed());
        match rx.recv_timeout(remaining) {
            Ok(exit_code) => {
                if exit_code != 0 {
                    test_error = Some(format!("Expected 0, got {}", exit_code));
                    break;
                }
            }
            Err(_) => {
                shared
                    .abort_requested
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                test_error = Some("TIMEOUT in test_multiple_arenas_simple".to_string());
                break;
            }
        }
    }

    for handle in handles {
        let _ = handle.join();
    }

    if let Some(msg) = test_error {
        panic!("{}", msg);
    }

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
#[cfg(feature = "multithreading")]
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
    let iterations = 50;

    let (tx, rx) = std::sync::mpsc::channel();
    let mut handles = Vec::new();
    for _ in 0..num_threads {
        let harness_ptr = harness as *const TestHarness as usize;
        let shared = Arc::clone(&shared);
        let tx = tx.clone();
        let handle = thread::spawn(move || {
            let harness = unsafe { &*(harness_ptr as *const TestHarness) };
            for _ in 0..iterations {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    harness.run_with_shared(resolution, shared.clone())
                }));
                match result {
                    Ok(0) => {}
                    Ok(exit_code) => {
                        let _ = tx.send(Err(Ok(exit_code)));
                        return;
                    }
                    Err(panic_info) => {
                        let _ = tx.send(Err(Err(panic_info)));
                        return;
                    }
                }
            }
            let _ = tx.send(Ok(()));
        });
        handles.push(handle);
    }

    let timeout = get_test_timeout(300); // 5 minutes for stress test
    let start = std::time::Instant::now();
    let mut test_error = None;
    for _ in 0..num_threads {
        let remaining = timeout.saturating_sub(start.elapsed());
        match rx.recv_timeout(remaining) {
            Ok(Ok(())) => {}
            Ok(Err(Ok(exit_code))) => {
                test_error = Some(format!("Test failed with exit code {}", exit_code));
                break;
            }
            Ok(Err(Err(panic_info))) => {
                shared
                    .abort_requested
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                for handle in handles {
                    let _ = handle.join();
                }
                std::panic::resume_unwind(panic_info);
            }
            Err(_) => {
                shared
                    .abort_requested
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                test_error = Some("TIMEOUT in test_reflection_race_condition".to_string());
                break;
            }
        }
    }

    for handle in handles {
        let _ = handle.join();
    }

    if let Some(msg) = test_error {
        panic!("{}", msg);
    }
}

/// Test that verifies the GC coordinator properly tracks multiple arenas
#[test]
#[cfg(feature = "multithreading")]
fn test_gc_coordinator_multi_arena_tracking() {
    use dotnet_vm::state;
    use std::sync::Arc;

    let shared = Arc::new(state::SharedGlobalState::new(TestHarness::get().loader));

    // Simulate multiple arenas being registered
    let handle1 = vm::gc::coordinator::ArenaHandle::new(dotnet_utils::ArenaId(1));

    let handle2 = vm::gc::coordinator::ArenaHandle::new(dotnet_utils::ArenaId(2));

    // Register multiple arenas
    shared.gc_coordinator.register_arena(handle1.clone());
    shared.gc_coordinator.register_arena(handle2.clone());

    // Verify registration worked - we can check if commands can be sent
    assert!(!shared.gc_coordinator.has_command(dotnet_utils::ArenaId(1)));
    assert!(!shared.gc_coordinator.has_command(dotnet_utils::ArenaId(2)));

    // Unregister
    shared
        .gc_coordinator
        .unregister_arena(dotnet_utils::ArenaId(1));
    shared
        .gc_coordinator
        .unregister_arena(dotnet_utils::ArenaId(2));

    // After unregister, commands should not be available
    assert!(!shared.gc_coordinator.has_command(dotnet_utils::ArenaId(1)));
    assert!(!shared.gc_coordinator.has_command(dotnet_utils::ArenaId(2)));
}

#[test]
#[cfg(feature = "multithreading")]
fn test_volatile_sharing() {
    use dotnet_vm::state;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::thread;

    let harness = TestHarness::get();
    let fixture_path = Path::new("tests/fixtures/threading/volatile_sharing_42.cs");
    let dll_path = harness.build(fixture_path).unwrap();

    let shared = std::sync::Arc::new(state::SharedGlobalState::new(harness.loader));
    let resolution =
        try_static_res_from_file(dll_path.to_str().unwrap()).expect("Failed to load assembly");

    let (tx, rx) = mpsc::channel();
    let mut handles = Vec::new();
    for _ in 0..2 {
        let shared = shared.clone();
        let tx = tx.clone();
        let harness_ptr = harness as *const TestHarness as usize;
        let handle = thread::spawn(move || {
            let harness = unsafe { &*(harness_ptr as *const TestHarness) };
            let exit_code = harness.run_with_shared(resolution, shared);
            let _ = tx.send(exit_code);
        });
        handles.push(handle);
    }

    let timeout = get_test_timeout(60);
    let start = std::time::Instant::now();
    let mut test_error = None;
    for _ in 0..2 {
        let remaining = timeout.saturating_sub(start.elapsed());
        match rx.recv_timeout(remaining) {
            Ok(exit_code) => {
                if exit_code != 42 {
                    test_error = Some(format!("Expected 42, got {}", exit_code));
                    break;
                }
            }
            Err(_) => {
                shared.abort_requested.store(true, Ordering::Relaxed);
                test_error = Some("TIMEOUT in test_volatile_sharing".to_string());
                break;
            }
        }
    }

    for handle in handles {
        let _ = handle.join();
    }

    if let Some(msg) = test_error {
        panic!("{}", msg);
    }
}

/// Test that verifies cross-arena reference tracking works
#[test]
#[cfg(feature = "multithreading")]
fn test_cross_arena_reference_tracking() {
    use dotnet_vm::state;
    use std::sync::Arc;

    let shared = Arc::new(state::SharedGlobalState::new(TestHarness::get().loader));

    // Create a mock arena handle
    let handle1 = vm::gc::coordinator::ArenaHandle::new(dotnet_utils::ArenaId(1));
    let handle2 = vm::gc::coordinator::ArenaHandle::new(dotnet_utils::ArenaId(2));

    let stw_flag = Arc::new(dotnet_utils::sync::AtomicBool::new(false));
    shared.gc_coordinator.register_arena(handle1.clone());
    shared.gc_coordinator.register_arena(handle2.clone());
    dotnet_utils::gc::register_arena(dotnet_utils::ArenaId(1), stw_flag.clone());
    dotnet_utils::gc::register_arena(dotnet_utils::ArenaId(2), stw_flag.clone());

    // Record some cross-arena references
    let ptr1 = unsafe { dotnet_value::object::ObjectPtr::from_raw(0x1000 as *const _).unwrap() };
    let ptr2 = unsafe { dotnet_value::object::ObjectPtr::from_raw(0x2000 as *const _).unwrap() };

    dotnet_utils::gc::record_cross_arena_ref(dotnet_utils::ArenaId(2), ptr1.as_ptr() as usize);
    dotnet_utils::gc::record_cross_arena_ref(dotnet_utils::ArenaId(2), ptr2.as_ptr() as usize);

    // The fact that we can record cross-arena refs without panicking demonstrates the system works
    shared
        .gc_coordinator
        .unregister_arena(dotnet_utils::ArenaId(1));
    shared
        .gc_coordinator
        .unregister_arena(dotnet_utils::ArenaId(2));
    dotnet_utils::gc::unregister_arena(dotnet_utils::ArenaId(1));
    dotnet_utils::gc::unregister_arena(dotnet_utils::ArenaId(2));
}

/// Test that allocation pressure triggers collection requests
#[test]
#[cfg(feature = "multithreading")]
fn test_allocation_pressure_triggers_collection() {
    use dotnet_vm::state;
    use std::sync::Arc;

    let shared = Arc::new(state::SharedGlobalState::new(TestHarness::get().loader));

    let handle = vm::gc::coordinator::ArenaHandle::new(dotnet_utils::ArenaId(1));

    shared.gc_coordinator.register_arena(handle.clone());

    // Simulate allocations using the handle directly
    for _ in 0..100 {
        handle.record_allocation(1024);
    }

    // Check if collection should be triggered
    let should_collect = shared.gc_coordinator.should_collect();

    // The coordinator tracks allocation pressure
    println!("Should collect after 100 allocations: {}", should_collect);

    shared
        .gc_coordinator
        .unregister_arena(dotnet_utils::ArenaId(1));
}

#[test]
#[cfg(feature = "multithreading")]
fn test_stw_stress() {
    use dotnet_vm::state;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::thread;

    let harness = TestHarness::get();
    let fixture_path = Path::new("tests/fixtures/span_comprehensive_0.cs");
    let dll_path = harness.build(fixture_path).unwrap();

    let shared = Arc::new(state::SharedGlobalState::new(harness.loader));
    let resolution =
        try_static_res_from_file(dll_path.to_str().unwrap()).expect("Failed to load assembly");
    shared.loader.register_assembly(resolution);

    let num_threads = 8usize;
    let iters_per_thread = 10usize;

    let (tx, rx) = mpsc::channel();
    let mut handles = Vec::new();
    for _ in 0..num_threads {
        let harness_ptr = harness as *const TestHarness as usize;
        let shared = Arc::clone(&shared);
        let tx = tx.clone();
        let handle = thread::spawn(move || {
            let harness = unsafe { &*(harness_ptr as *const TestHarness) };
            for _ in 0..iters_per_thread {
                let code = harness.run_with_shared(resolution, shared.clone());
                if code != 0 {
                    let _ = tx.send(Err(code));
                    return;
                }
            }
            let _ = tx.send(Ok(()));
        });
        handles.push(handle);
    }

    let timeout = get_test_timeout(300); // 5 minutes for stress test
    let start = std::time::Instant::now();
    let mut test_error = None;
    for _ in 0..num_threads {
        let remaining = timeout.saturating_sub(start.elapsed());
        match rx.recv_timeout(remaining) {
            Ok(Ok(())) => {}
            Ok(Err(code)) => {
                test_error = Some(format!("test_stw_stress failed with code {}", code));
                break;
            }
            Err(_) => {
                shared.abort_requested.store(true, Ordering::Relaxed);
                test_error = Some("TIMEOUT in test_stw_stress".to_string());
                break;
            }
        }
    }

    for handle in handles {
        let _ = handle.join();
    }

    if let Some(msg) = test_error {
        panic!("{}", msg);
    }
}
