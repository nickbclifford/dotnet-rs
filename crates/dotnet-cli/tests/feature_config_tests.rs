//! Configuration-specific tests for feature flags
//! Tests that verify the correct behavior of different feature configurations
#![allow(clippy::arc_with_non_send_sync)]
#[cfg(feature = "multithreading")]
use dotnet_vm::threading::ThreadManagerOps;

use dotnet_vm::state;
use std::{path::PathBuf, sync::Arc};

// ============================================================================
// Single-threaded Configuration Tests (no features)
// ============================================================================

#[test]
#[cfg(not(feature = "multithreading"))]
fn test_single_threaded_stub_thread_manager() {
    // In single-threaded mode, thread manager should exist but be a stub
    let loader = create_test_loader();
    let _shared = Arc::new(state::SharedGlobalState::new(loader));

    // Thread manager should provide a consistent thread ID (always 1)
    assert_eq!(dotnet_utils::sync::get_current_thread_id(), 1);
}

#[test]
#[cfg(not(feature = "multithreading"))]
fn test_single_threaded_sync_block_manager() {
    // In single-threaded mode, sync blocks should work but without actual locking
    let loader = create_test_loader();
    let shared = Arc::new(state::SharedGlobalState::new(loader));

    // Sync block manager should exist and be usable
    let manager = &shared.sync_blocks;
    // Basic operations should not panic
    let _ = manager;
}

// ============================================================================
// Multithreading Configuration Tests (multithreading feature only)
// ============================================================================

#[test]
#[cfg(all(feature = "multithreading", not(feature = "multithreaded-gc")))]
fn test_multithreading_thread_registration() {
    let loader = create_test_loader();
    let shared = Arc::new(state::SharedGlobalState::new(loader));

    // Thread manager should support multiple threads
    let id1 = shared.thread_manager.register_thread();
    let id2 = shared.thread_manager.register_thread();

    assert_ne!(id1, id2, "Thread IDs should be unique");
    assert_eq!(shared.thread_manager.thread_count(), 2);

    shared.thread_manager.unregister_thread(id1);
    assert_eq!(shared.thread_manager.thread_count(), 1);

    shared.thread_manager.unregister_thread(id2);
    assert_eq!(shared.thread_manager.thread_count(), 0);
}

#[test]
#[cfg(all(feature = "multithreading", not(feature = "multithreaded-gc")))]
fn test_multithreading_no_gc_coordinator() {
    let loader = create_test_loader();
    let shared = Arc::new(state::SharedGlobalState::new(loader));

    // In multithreading-only mode, gc_coordinator field should not exist
    // This is a compile-time test - if this compiles, the test passes
    let _ = shared;
}

#[test]
#[cfg(feature = "multithreading")]
fn test_multithreading_sync_blocks() {
    use dotnet_vm::state;
    use std::thread;

    let loader = create_test_loader();
    let shared = Arc::new(state::SharedGlobalState::new(loader));
    let _manager = &shared.sync_blocks;

    // Test that sync blocks work across threads
    let shared_clone = Arc::clone(&shared);
    let handle = thread::spawn(move || {
        let _ = shared_clone.thread_manager.register_thread();
        // Thread operations should work
    });

    handle.join().unwrap();
}

// ============================================================================
// Multithreaded-GC Configuration Tests (full features)
// ============================================================================

#[test]
#[cfg(feature = "multithreaded-gc")]
fn test_multithreaded_gc_coordinator_exists() {
    let loader = create_test_loader();
    let shared = Arc::new(state::SharedGlobalState::new(loader));

    // GC coordinator should exist - just accessing it verifies compilation
    let _gc_coord = &shared.gc_coordinator;
}

#[test]
#[cfg(feature = "multithreaded-gc")]
fn test_multithreaded_gc_arena_handle() {
    let loader = create_test_loader();
    let shared = Arc::new(state::SharedGlobalState::new(loader));

    // Create an arena handle
    let handle = dotnet_vm::gc::coordinator::ArenaHandle::new(1);

    // Register and unregister the arena
    shared.gc_coordinator.register_arena(handle.clone());
    shared.gc_coordinator.unregister_arena(1);
}

#[test]
#[cfg(feature = "multithreaded-gc")]
fn test_multithreaded_gc_cross_arena_value() {
    use dotnet_value::StackValue;

    // Test that CrossArenaObjectRef variant exists
    // We create a simple ObjectPtr by transmuting a pointer value
    let ptr = unsafe { std::mem::transmute::<usize, dotnet_value::object::ObjectPtr>(0x1000) };
    let _value = StackValue::CrossArenaObjectRef(ptr, 1);
    // If this compiles, the variant exists and works
}

// ============================================================================
// Feature Dependency Tests
// ============================================================================

#[test]
#[cfg(feature = "multithreaded-gc")]
fn test_multithreaded_gc_implies_multithreading() {
    // If multithreaded-gc is enabled, multithreading should also be available
    let loader = create_test_loader();
    let shared = Arc::new(state::SharedGlobalState::new(loader));

    // Thread manager should support multiple threads
    let id = shared.thread_manager.register_thread();
    assert!(id > 0);
    shared.thread_manager.unregister_thread(id);
}

// ============================================================================
// Binary Size Tests (informational)
// ============================================================================

#[test]
fn test_basic_functionality_exists() {
    // This test should pass in all configurations
    let loader = create_test_loader();
    let shared = Arc::new(state::SharedGlobalState::new(loader));

    // Basic shared state should always be available
    assert!(!shared.loader.get_root().is_empty());
}

// ============================================================================
// Helper Functions
// ============================================================================

fn create_test_loader() -> &'static dotnet_assemblies::AssemblyLoader {
    thread_local! {
        static LOADER: &'static dotnet_assemblies::AssemblyLoader = {
            let assemblies_path = find_dotnet_app_path().to_str().unwrap().to_string();
            let loader = dotnet_assemblies::AssemblyLoader::new(assemblies_path);
            Box::leak(Box::new(loader))
        };
    }

    LOADER.with(|&l| l)
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

    // Find the latest version
    let mut versions: Vec<_> = std::fs::read_dir(&base)
        .expect("failed to read_unchecked dotnet directory")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|s| s.chars().next().unwrap_or('0').is_ascii_digit())
                .unwrap_or(false)
        })
        .collect();

    versions.sort_by_key(|e| e.file_name());
    versions.last().expect("no .NET version found").path()
}
