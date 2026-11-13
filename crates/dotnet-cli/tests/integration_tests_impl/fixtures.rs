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

#[cfg(not(feature = "fuzzing"))]
#[test]
fn multicast_selection_precedes_operand_commitment() {
    use dotnet_utils::sync::{Arc as SyncArc, AtomicUsize, Ordering};
    use tracing::{
        Event, Subscriber,
        field::{Field, Visit},
    };
    use tracing_subscriber::{
        layer::{Context, Layer},
        prelude::*,
    };

    #[derive(Default)]
    struct Counts {
        asynchronous: AtomicUsize,
        argument: AtomicUsize,
        result: AtomicUsize,
    }
    struct Capture(SyncArc<Counts>);
    #[derive(Default)]
    struct Fields {
        path: Option<String>,
        reason: Option<String>,
    }
    impl Visit for Fields {
        fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
        fn record_str(&mut self, field: &Field, value: &str) {
            match field.name() {
                "multicast_path" => self.path = Some(value.to_owned()),
                "reason" => self.reason = Some(value.to_owned()),
                _ => {}
            }
        }
    }
    impl<S: Subscriber> Layer<S> for Capture {
        fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
            let mut fields = Fields::default();
            event.record(&mut fields);
            match (fields.path.as_deref(), fields.reason.as_deref()) {
                (Some("async"), _) => {
                    self.0.asynchronous.fetch_add(1, Ordering::Relaxed);
                }
                (Some("fallback"), Some("argument")) => {
                    self.0.argument.fetch_add(1, Ordering::Relaxed);
                }
                (Some("fallback"), Some("return")) => {
                    self.0.result.fetch_add(1, Ordering::Relaxed);
                }
                _ => {}
            }
        }
    }
    for (fixture, expected) in [
        ("delegate_multicast_42.cs", (1, 0, 0)),
        ("delegate_multicast_many_42.cs", (1, 0, 0)),
        ("delegate_multicast_refs_42.cs", (2, 0, 0)),
        ("delegate_multicast_finalizers_42.cs", (2, 0, 0)),
        ("delegate_multicast_filters_42.cs", (4, 0, 0)),
        ("delegate_multicast_static_ref_filters_42.cs", (2, 0, 0)),
        ("delegate_multicast_cctor_42.cs", (1, 0, 0)),
        ("delegate_multicast_ref_cctor_42.cs", (1, 0, 0)),
        ("delegate_multicast_scalar_42.cs", (1, 0, 0)),
        ("delegate_multicast_reflection_42.cs", (10, 0, 0)),
        ("delegate_multicast_static_ref_42.cs", (1, 0, 0)),
        ("delegate_multicast_static_ref_cctor_42.cs", (1, 0, 0)),
        ("delegate_multicast_aggregate_42.cs", (1, 0, 0)),
        ("delegate_multicast_values_42.cs", (5, 0, 0)),
        ("delegate_multicast_ref_struct_42.cs", (0, 2, 1)),
        ("delegate_multicast_byref_return_42.cs", (0, 0, 1)),
        ("delegate_multicast_typed_reference_42.cs", (0, 1, 0)),
        ("delegate_multicast_unmanaged_ref_42.cs", (2, 0, 0)),
        ("delegate_multicast_gc_collect_42.cs", (3, 0, 0)),
        #[cfg(feature = "multithreading")]
        ("delegate_multicast_contended_42.cs", (1, 0, 0)),
    ] {
        let harness = TestHarness::get();
        let path = format!("tests/fixtures/delegates/{fixture}");
        let dll = harness.ensure_dll(Path::new(&path));
        let counts = SyncArc::new(Counts::default());
        let subscriber = tracing_subscriber::registry().with(Capture(SyncArc::clone(&counts)));
        let result =
            crate::integration_tests_impl::harness::with_counting_subscriber(subscriber, || {
                harness.run_with_timeout(&dll, std::time::Duration::from_secs(60))
            });
        assert_eq!(result.exit_code, 42, "{fixture}: {:?}", result.stderr);
        assert_eq!(
            (
                counts.asynchronous.load(Ordering::Relaxed),
                counts.argument.load(Ordering::Relaxed),
                counts.result.load(Ordering::Relaxed)
            ),
            expected,
            "{fixture}"
        );
    }
}

#[test]
fn reflection_path_selection() {
    use dotnet_utils::sync::{Arc as SyncArc, AtomicUsize, Ordering};
    use tracing::{
        Event, Subscriber,
        field::{Field, Visit},
    };
    use tracing_subscriber::{
        layer::{Context, Layer},
        prelude::*,
    };

    #[derive(Default)]
    struct Counts {
        asynchronous: AtomicUsize,
        fallback: AtomicUsize,
        retry: AtomicUsize,
    }
    struct Capture(SyncArc<Counts>);
    #[derive(Default)]
    struct Fields {
        path: Option<String>,
    }
    impl Visit for Fields {
        fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "reflection_invoke_path" {
                self.path = Some(value.to_owned());
            }
        }
    }
    impl<S: Subscriber> Layer<S> for Capture {
        fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
            let mut fields = Fields::default();
            event.record(&mut fields);
            match fields.path.as_deref() {
                Some("async") => {
                    self.0.asynchronous.fetch_add(1, Ordering::Relaxed);
                }
                Some("fallback") => {
                    self.0.fallback.fetch_add(1, Ordering::Relaxed);
                }
                Some("retry") => {
                    self.0.retry.fetch_add(1, Ordering::Relaxed);
                }
                _ => {}
            }
        }
    }
    let harness = TestHarness::get();
    for (path, expected) in [
        ("tests/fixtures/reflection/invoke_byref_gc_42.cs", (1, 0, 0)),
        (
            "tests/fixtures/reflection/invoke_reference_byref_42.cs",
            (1, 0, 0),
        ),
        (
            "tests/fixtures/reflection/invoke_byref_exception_42.cs",
            (1, 0, 0),
        ),
        (
            "tests/fixtures/reflection/invoke_intrinsic_value_42.cs",
            (1, 0, 0),
        ),
        ("tests/fixtures/reflection/invoke_results_42.cs", (3, 0, 0)),
        // These fixtures print on the successful path, which requires P/Invoke.
        #[cfg(not(feature = "fuzzing"))]
        ("tests/fixtures/reflection/invoke_collect_42.cs", (3, 0, 0)),
        (
            "tests/fixtures/reflection/invoke_constructors_42.cs",
            (7, 0, 0),
        ),
        ("tests/fixtures/reflection/invoke_fallback_42.cs", (5, 0, 0)),
        #[cfg(not(feature = "fuzzing"))]
        ("tests/fixtures/reflection/invoke_values_42.cs", (6, 0, 0)),
        #[cfg(not(feature = "fuzzing"))]
        (
            "tests/fixtures/reflection/invoke_byref_like_42.cs",
            (27, 0, 0),
        ),
        (
            "tests/fixtures/reflection/invoke_target_validation_42.cs",
            (18, 0, 0),
        ),
        (
            "tests/fixtures/reflection/invoke_argument_count_42.cs",
            (16, 0, 0),
        ),
        (
            "tests/fixtures/reflection/invoke_argument_types_42.cs",
            (26, 0, 0),
        ),
        (
            "tests/fixtures/reflection/invoke_byref_writethrough_42.cs",
            (1, 0, 0),
        ),
    ] {
        let dll = harness.ensure_dll(Path::new(path));
        let count = SyncArc::new(Counts::default());
        let subscriber =
            tracing_subscriber::registry().with(Capture(SyncArc::clone(&count)).with_filter(
                tracing_subscriber::filter::filter_fn(|metadata| {
                    metadata
                        .target()
                        .starts_with("dotnet_intrinsics_reflection::")
                        || metadata.target() == "dotnet_vm::stack::reflection_ops_impl"
                }),
            ));
        assert_eq!(
            crate::integration_tests_impl::harness::with_counting_subscriber(subscriber, || {
                harness.run(&dll)
            }),
            42,
            "{path}"
        );
        assert_eq!(
            (
                count.asynchronous.load(Ordering::Relaxed),
                count.fallback.load(Ordering::Relaxed),
                count.retry.load(Ordering::Relaxed)
            ),
            expected,
            "{path}"
        );
    }
}

#[cfg(not(feature = "fuzzing"))]
#[test]
fn reflection_byref_return_is_rejected_before_target_runs() {
    let harness = TestHarness::get();
    let dll = harness.ensure_dll(Path::new(
        "tests/fixtures/reflection/invoke_byref_return_255.cs",
    ));
    let (status, stdout) = harness.run_cli(&dll);
    assert_ne!(status, 42);
    assert!(!stdout.contains("target ran"), "{stdout}");
    let resolution = harness.loader.load_resolution_from_file(&dll).unwrap();
    #[allow(
        clippy::arc_with_non_send_sync,
        reason = "the no-MT integration test keeps shared state on its sole executor"
    )]
    let shared = Arc::new(state::SharedGlobalState::new(Arc::clone(&harness.loader)));
    let result = harness.run_with_shared(resolution, shared);
    assert_eq!(result.exit_code, 255);
    assert!(
        result
            .stderr
            .unwrap_or_default()
            .contains("reflection invocation with a byref return")
    );
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
fn finalizer_runs_after_entrypoint_completes() {
    let harness = TestHarness::get();
    let dll_path = harness.ensure_dll(Path::new(
        "tests/fixtures/gc/finalizer_after_entrypoint_0.cs",
    ));
    harness.assert_finalizer_lifecycle(&dll_path);
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
fn retired_arena_static_survives_join_and_collection() {
    let harness = TestHarness::get();
    let dll_path = harness
        .build(Path::new("tests/debug_fixtures/retired_arena_static_42.cs"))
        .expect("retired arena fixture must build");
    let result = harness.run_with_timeout(&dll_path, std::time::Duration::from_secs(60));
    assert_eq!(result.exit_code, 42, "fixture failed: {:?}", result.stderr);
    assert_eq!(result.stderr, None);
}

/// Runs a byref-like P/Invoke fixture and requires the specific pre-marshal
/// rejection, not an arbitrary VM error with the same exit code.
#[cfg(not(feature = "fuzzing"))]
fn assert_byref_like_pinvoke_rejected(fixture: &str, what: &str) {
    let harness = TestHarness::get();
    let dll_path = harness.ensure_dll(Path::new(fixture));
    let resolution = harness
        .loader
        .load_resolution_from_file(&dll_path)
        .expect("byref-like P/Invoke fixture must load");
    #[allow(
        clippy::arc_with_non_send_sync,
        reason = "the no-MT integration test keeps shared state on its sole executor"
    )]
    let shared = Arc::new(state::SharedGlobalState::new(Arc::clone(&harness.loader)));
    let result = harness.run_with_shared(resolution, shared);
    assert_eq!(
        result.exit_code, 255,
        "byref-like P/Invoke {what} was not rejected: {:?}",
        result.stderr
    );
    let stderr = result.stderr.unwrap_or_default();
    assert!(
        stderr.contains(&format!("P/Invoke {what} is a byref-like value type")),
        "unexpected rejection diagnostic: {stderr}"
    );
}

/// A byref-like aggregate
/// passed by value is rejected before marshaling, in every configuration.
#[test]
#[cfg(not(feature = "fuzzing"))]
fn pinvoke_byreflike_argument_rejected() {
    assert_byref_like_pinvoke_rejected(
        "tests/fixtures/pinvoke/pinvoke_byreflike_argument_255.cs",
        "argument 0",
    );
}

/// A byref-like aggregate
/// return type is rejected before native code runs, in every configuration.
#[test]
#[cfg(not(feature = "fuzzing"))]
fn pinvoke_byreflike_return_rejected() {
    assert_byref_like_pinvoke_rejected(
        "tests/fixtures/pinvoke/pinvoke_byreflike_return_255.cs",
        "return type",
    );
}

#[test]
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
fn reflection_byref_foreign_owner() {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let harness = TestHarness::get();
    let dll = harness.ensure_dll(Path::new(
        "tests/fixtures/reflection/invoke_byref_foreign_42.cs",
    ));
    let runtime = dotnet_assemblies::find_dotnet_app_path().expect(".NET runtime");
    let mut child = Command::new(env!("CARGO_BIN_EXE_dotnet-rs"))
        .arg("-a")
        .arg(runtime)
        .arg(dll)
        .stdout(Stdio::null())
        .spawn()
        .expect("start VM");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                assert_eq!(status.code(), Some(42));
                break;
            }
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("failed to wait for VM: {error}");
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("foreign-owner fixture timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
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

#[test]
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
fn retired_finalizer_observes_abort() {
    let harness = TestHarness::get();
    let dll = harness.ensure_dll(Path::new("tests/debug_fixtures/retired_finalizer_abort.cs"));
    let resolution = harness.loader.load_resolution_from_file(&dll).unwrap();
    let shared = Arc::new(state::SharedGlobalState::new(Arc::clone(&harness.loader)));
    let signal = shared.abort_signal();
    let timer = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        signal.request_abort();
    });
    let result = harness.run_with_shared_observed(resolution, shared, |shared, program, _| {
        let storage = shared
            .statics
            .get(program.clone(), &dotnet_vm::GenericLookup::default());
        let bytes = storage.storage.get_field_atomic(
            program,
            "Started",
            dotnet_utils::sync::Ordering::Acquire,
        );
        assert_eq!(
            i32::from_ne_bytes(bytes.try_into().unwrap()),
            1,
            "finalizer never started"
        );
    });
    timer.join().unwrap();
    assert_eq!(
        result.exit_code, 255,
        "finalizer did not wait for abort: {result:?}"
    );
    assert!(format!("{:?}", result.stderr).contains("Aborted"));
}

#[test]
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
fn retired_finalizer_shutdown_observes_abort() {
    let harness = TestHarness::get();
    let dll = harness.ensure_dll(Path::new(
        "tests/debug_fixtures/retired_finalizer_shutdown.cs",
    ));
    let resolution = harness.loader.load_resolution_from_file(&dll).unwrap();
    let shared = Arc::new(state::SharedGlobalState::new(Arc::clone(&harness.loader)));
    let signal = shared.abort_signal();
    let fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let timer_fired = Arc::clone(&fired);
    let timer = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        timer_fired.store(true, std::sync::atomic::Ordering::Release);
        signal.request_abort();
    });
    let result = harness.run_with_shared_observed(resolution, shared, |shared, program, _| {
        let storage = shared
            .statics
            .get(program.clone(), &dotnet_vm::GenericLookup::default());
        let bytes = storage.storage.get_field_atomic(
            program,
            "Started",
            dotnet_utils::sync::Ordering::Acquire,
        );
        assert_eq!(
            i32::from_ne_bytes(bytes.try_into().unwrap()),
            1,
            "finalizer never started"
        );
    });
    assert!(
        fired.load(std::sync::atomic::Ordering::Acquire),
        "shutdown returned before finalizer abort"
    );
    timer.join().unwrap();
    assert_eq!(
        result.exit_code, 42,
        "Main must return before worker shutdown: {result:?}"
    );
}

#[test]
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
fn harness_timeout_aborts_and_reaps_retired_finalizer() {
    use std::time::Duration;

    let harness = TestHarness::get();
    let dll = harness.ensure_dll(Path::new("tests/debug_fixtures/retired_finalizer_abort.cs"));
    let resolution = harness.loader.load_resolution_from_file(&dll).unwrap();
    let shared = Arc::new(state::SharedGlobalState::new(Arc::clone(&harness.loader)));
    let observed = std::cell::Cell::new(false);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        harness.run_with_shared_timeout_observed(
            resolution,
            shared.clone(),
            Duration::from_secs(2),
            |shared, program, result| {
                let storage = shared
                    .statics
                    .get(program.clone(), &dotnet_vm::GenericLookup::default());
                let started = storage.storage.get_field_atomic(
                    program,
                    "Started",
                    dotnet_utils::sync::Ordering::Acquire,
                );
                observed.set(
                    i32::from_ne_bytes(started.try_into().unwrap()) == 1
                        && result.exit_code == 255
                        && format!("{:?}", result.stderr).contains("Aborted"),
                );
            },
        );
    }));
    shared.shutdown_workers();
    assert!(result.is_err(), "harness did not report its timeout");
    assert!(observed.get(), "abort must interrupt the running finalizer");
}

#[test]
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
fn shutdown_waits_for_active_retired_finalizer() {
    use dotnet_types::{TypeDescription, members::MethodDescription};
    use dotnetdll::prelude::{EntryPoint, MethodMemberIndex};
    use std::{sync::mpsc, time::Duration};

    let harness = TestHarness::get();
    let dll = harness.ensure_dll(Path::new(
        "tests/debug_fixtures/retired_finalizer_shutdown.cs",
    ));
    let resolution = harness.loader.load_resolution_from_file(&dll).unwrap();
    let EntryPoint::Method(entry) = resolution.entry_point.unwrap() else {
        panic!("expected method entry point");
    };
    let program = TypeDescription::new(resolution.clone(), entry.parent_type());
    let index = program
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, &resolution.definition()[entry]))
        .unwrap();
    let method = MethodDescription::new(
        program.clone(),
        dotnet_vm::GenericLookup::default(),
        resolution,
        MethodMemberIndex::Method(index),
    );
    let shared = Arc::new(state::SharedGlobalState::new(Arc::clone(&harness.loader)));
    let mut executor = dotnet_vm::Executor::new(shared.clone());
    executor.entrypoint(method).unwrap();
    let result = executor.run_entrypoint();
    let storage = shared
        .statics
        .get(program.clone(), &dotnet_vm::GenericLookup::default());
    let started =
        storage
            .storage
            .get_field_atomic(program, "Started", dotnet_utils::sync::Ordering::Acquire);

    let (entered_tx, entered_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let shutdown_state = shared.clone();
    let shutdown = std::thread::spawn(move || {
        entered_tx.send(()).unwrap();
        shutdown_state.shutdown_workers();
        done_tx.send(()).unwrap();
    });
    let entered = entered_rx.recv_timeout(Duration::from_secs(30)).is_ok();
    let completed_before_abort = done_rx.recv_timeout(Duration::from_millis(200)).is_ok();
    shared.abort_signal().request_abort();
    let completed_after_abort =
        completed_before_abort || done_rx.recv_timeout(Duration::from_secs(30)).is_ok();
    shutdown.join().unwrap();
    drop(executor);
    shared.shutdown_workers();

    assert!(matches!(result, dotnet_vm::ExecutorResult::Exited(42)));
    assert_eq!(i32::from_ne_bytes(started.try_into().unwrap()), 1);
    assert!(entered && !completed_before_abort && completed_after_abort);
}

#[test]
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
fn retired_finalizer_keeps_entry_arena_until_completion() {
    let harness = TestHarness::get();
    let dll = harness.ensure_dll(Path::new(
        "tests/debug_fixtures/retired_finalizer_entry_owner.cs",
    ));
    let resolution = harness.loader.load_resolution_from_file(&dll).unwrap();
    let shared = Arc::new(state::SharedGlobalState::new(Arc::clone(&harness.loader)));
    let result = harness.run_with_shared_observed(resolution, shared, |shared, program, _| {
        let storage = shared
            .statics
            .get(program.clone(), &dotnet_vm::GenericLookup::default());
        let bytes = storage.storage.get_field_atomic(
            program,
            "Done",
            dotnet_utils::sync::Ordering::Acquire,
        );
        assert_eq!(
            i32::from_ne_bytes(bytes.try_into().unwrap()),
            1,
            "finalizer did not finish"
        );
    });
    assert_eq!(result.exit_code, 42);
}

#[cfg(all(not(feature = "multithreading"), not(feature = "fuzzing")))]
#[test]
fn async_task_delay_requires_multithreading() {
    let harness = TestHarness::get();
    let dll = harness.ensure_dll(Path::new(
        "tests/fixtures/async/async_task_delay_elapsed_42.cs",
    ));
    let resolution = harness.loader.load_resolution_from_file(&dll).unwrap();
    #[allow(
        clippy::arc_with_non_send_sync,
        reason = "single-threaded executor owns the state"
    )]
    let shared = Arc::new(state::SharedGlobalState::new(Arc::clone(&harness.loader)));
    let result = harness.run_with_shared(resolution, shared);

    assert_eq!(result.exit_code, 254, "{result:?}");
    let stderr = result.stderr.unwrap_or_default();
    assert!(
        stderr.contains("Managed Thread(ThreadStart) requires the multithreading feature."),
        "Task.Delay did not fail at managed timer-thread startup: {stderr}"
    );
    assert!(
        !stderr.contains("CreateEventEx") && !stderr.contains("WaitHandle_WaitOneCore"),
        "Task.Delay failed through a native event path: {stderr}"
    );
}

#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
#[test]
fn async_task_delay_elapsed_lower_bound() {
    let harness = TestHarness::get();
    let dll = harness.ensure_dll(Path::new(
        "tests/fixtures/async/async_task_delay_elapsed_42.cs",
    ));
    let result = harness.run_with_timeout(&dll, std::time::Duration::from_secs(60));
    assert_eq!(result.exit_code, 42, "{result:?}");
    assert_eq!(
        result.stderr, None,
        "Task.Delay threw instead of completing"
    );
}

#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
#[test]
fn threading_timer_background_threads_do_not_hold_process_open() {
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };

    let harness = TestHarness::get();
    let dll = harness.ensure_dll(Path::new(
        "tests/fixtures/threading/threading_timer_idle_background_exit_42.cs",
    ));
    let mut command = harness.cli_command(&dll);
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start dotnet-rs timer lifecycle fixture");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if child
            .try_wait()
            .expect("failed to poll timer fixture")
            .is_some()
        {
            let output = child
                .wait_with_output()
                .expect("failed to collect timer fixture output");
            assert_eq!(
                output.status.code(),
                Some(42),
                "timer fixture did not exit after its background timer and pool became idle: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.stdout.is_empty());
            return;
        }
        if Instant::now() >= deadline {
            child.kill().expect("failed to stop stuck timer fixture");
            let output = child
                .wait_with_output()
                .expect("failed to collect stuck timer fixture output");
            panic!(
                "timer/pool background workers held the process open: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(not(feature = "multithreading"))]
#[test]
fn async_queued_task_result_reports_single_threaded_deadlock() {
    let harness = TestHarness::get();
    let dll = harness.ensure_dll(Path::new(
        "tests/fixtures/async/async_pool_run_result_42.cs",
    ));
    let resolution = harness.loader.load_resolution_from_file(&dll).unwrap();
    #[allow(
        clippy::arc_with_non_send_sync,
        reason = "single-threaded executor owns the state"
    )]
    let shared = Arc::new(state::SharedGlobalState::new(Arc::clone(&harness.loader)));
    let result = harness.run_with_shared(resolution, shared);
    assert_eq!(result.exit_code, 255, "{result:?}");
    assert!(
        result
            .stderr
            .as_deref()
            .unwrap_or_default()
            .contains("Monitor.Wait would deadlock in single-threaded mode"),
        "{result:?}"
    );
}
