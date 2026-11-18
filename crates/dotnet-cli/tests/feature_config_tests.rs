//! Configuration-specific tests for feature flags
//! Tests that verify the correct behavior of different feature configurations

use dotnet_vm::state;
use std::sync::Arc;

// ============================================================================
// Single-threaded Configuration Tests (no features)
// ============================================================================

#[test]
#[cfg(not(feature = "multithreading"))]
fn test_single_threaded_stub_thread_manager() {
    // In single-threaded mode, thread manager should exist but be a stub
    let loader = create_test_loader();
    #[allow(
        clippy::arc_with_non_send_sync,
        reason = "the no-MT test keeps shared state on its sole executor"
    )]
    let _shared = Arc::new(state::SharedGlobalState::new(loader));

    // Thread manager should provide a consistent thread ID (always 1)
    assert_eq!(
        dotnet_utils::sync::get_current_thread_id(),
        dotnet_utils::ArenaId::new(1)
    );
}

// ============================================================================
// Binary Size Tests (informational)
// ============================================================================

#[test]
fn test_basic_functionality_exists() {
    // This test should pass in all configurations
    let loader = create_test_loader();
    #[allow(
        clippy::arc_with_non_send_sync,
        reason = "the no-MT test keeps shared state on its sole executor"
    )]
    let shared = Arc::new(state::SharedGlobalState::new(loader));

    // Basic shared state should always be available
    assert!(!shared.loader.get_root().is_empty());
}

// ============================================================================
// Helper Functions
// ============================================================================

#[allow(
    clippy::arc_with_non_send_sync,
    reason = "the AssemblyLoader fixture is confined to this thread-local test cache"
)]
fn create_test_loader() -> Arc<dotnet_assemblies::AssemblyLoader> {
    thread_local! {
        static LOADER: Arc<dotnet_assemblies::AssemblyLoader> = {
            let assemblies_path = dotnet_assemblies::find_dotnet_app_path()
                .expect("could not find .NET shared path")
                .to_str()
                .unwrap()
                .to_string();
            let loader = dotnet_assemblies::AssemblyLoader::new(assemblies_path).expect("failed to create AssemblyLoader");
            Arc::new(loader)
        };
    }

    LOADER.with(|l| l.clone())
}
