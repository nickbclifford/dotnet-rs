// used in generated tests from build.rs
macro_rules! fixture_test {
    ($name:ident, $dll_path:expr, $expected:expr) => {
        #[test]
        fn $name() {
            use crate::integration_tests_impl::harness::{TestHarness, get_test_timeout};
            let harness = TestHarness::get();
            let dll_path = std::path::Path::new($dll_path);
            let exit_code = harness.run_with_timeout(dll_path, get_test_timeout(60));

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
    (#[ignore] $name:ident, $dll_path:expr, $expected:expr) => {
        #[test]
        #[ignore]
        fn $name() {
            use crate::integration_tests_impl::harness::{TestHarness, get_test_timeout};
            let harness = TestHarness::get();
            let dll_path = std::path::Path::new($dll_path);
            let exit_code = harness.run_with_timeout(dll_path, get_test_timeout(60));

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

/// Macro for multi-arena tests that run the same fixture across multiple threads.
#[cfg(feature = "multithreading")]
macro_rules! multi_arena_test {
    ($name:ident, $fixture:literal, $thread_count:expr, $expected:expr) => {
        #[test]
        #[cfg(feature = "multithreading")]
        fn $name() {
            use crate::integration_tests_impl::fixtures::setup_multi_arena_fixture;
            use crate::integration_tests_impl::harness::{TestHarness, get_test_timeout};
            use std::sync::atomic::Ordering;
            use std::sync::mpsc;
            use std::thread;

            let dll_path = setup_multi_arena_fixture($fixture);
            let harness = TestHarness::get();
            let resolution = harness
                .loader
                .load_resolution_from_file(&dll_path)
                .expect("Failed to load assembly");

            let (tx, rx) = mpsc::channel();
            let mut abort_flags = Vec::new();
            let mut handles = Vec::new();
            let release_gate =
                std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));

            let shared = std::sync::Arc::new(dotnet_vm::state::SharedGlobalState::new(
                harness.loader.clone(),
            ));
            for _ in 0..$thread_count {
                let harness = harness.clone();
                let tx = tx.clone();
                let shared = std::sync::Arc::clone(&shared);
                let release_gate = std::sync::Arc::clone(&release_gate);
                let resolution = resolution.clone();
                abort_flags.push(std::sync::Arc::clone(&shared.abort_requested));

                let handle = thread::spawn(move || {
                    let tx_ok = tx.clone();
                    let tx_err = tx;
                    let result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                            harness.run_with_shared_signal_then_wait(
                                resolution.clone(),
                                shared,
                                &release_gate,
                                move |exit_code| {
                                    let _ = tx_ok.send(Ok(exit_code));
                                },
                            );
                        }));
                    if let Err(panic_info) = result {
                        let _ = tx_err.send(Err(panic_info));
                    }
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
                        {
                            let (lock, condvar) = &*release_gate;
                            let mut released = lock.lock().expect("release gate lock poisoned");
                            *released = true;
                            condvar.notify_all();
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

            if let Some(err) = test_error {
                for flag in &abort_flags {
                    flag.store(true, Ordering::Relaxed);
                }
                {
                    let (lock, condvar) = &*release_gate;
                    let mut released = lock.lock().expect("release gate lock poisoned");
                    *released = true;
                    condvar.notify_all();
                }
                for handle in handles {
                    let _ = handle.join();
                }
                panic!("{}", err);
            }

            {
                let (lock, condvar) = &*release_gate;
                let mut released = lock.lock().expect("release gate lock poisoned");
                *released = true;
                condvar.notify_all();
            }
            for handle in handles {
                let _ = handle.join();
            }
        }
    };
}
