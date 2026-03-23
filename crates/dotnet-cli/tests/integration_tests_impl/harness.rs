use dotnet_assemblies as assemblies;
use dotnet_types::{TypeDescription, members::MethodDescription, resolution::ResolutionS};
use dotnet_vm::{self as vm, state};
use dotnetdll::prelude::*;
use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

pub fn get_test_timeout(default_secs: u64) -> Duration {
    let secs = std::env::var("DOTNET_TEST_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default_secs);
    Duration::from_secs(secs)
}

pub struct TestHarness {
    pub loader: Arc<assemblies::AssemblyLoader>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionResult {
    pub exit_code: u8,
    pub stderr: Option<String>,
}

impl TestHarness {
    fn fixtures_base_dir() -> PathBuf {
        PathBuf::from(env!("DOTNET_FIXTURES_BASE"))
    }

    pub fn get() -> Arc<Self> {
        thread_local! {
            static INSTANCE: Arc<TestHarness> = Arc::new(TestHarness::new());
        }
        INSTANCE.with(|i| i.clone())
    }

    fn new() -> Self {
        let assemblies_path = Self::find_dotnet_app_path().to_str().unwrap().to_string();
        let loader = assemblies::AssemblyLoader::new(assemblies_path)
            .expect("Failed to create AssemblyLoader");
        Self {
            loader: Arc::new(loader),
        }
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
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let output_dir = Self::fixtures_base_dir().join("adhoc").join(file_name);

        let fixture_path_abs = if fixture_path.is_absolute() {
            fixture_path.to_path_buf()
        } else {
            manifest_dir.join(fixture_path)
        };

        let absolute_file = std::fs::canonicalize(&fixture_path_abs)?;
        let dll_path = output_dir.join("SingleFile.dll");

        if dll_path.exists() {
            let source_mtime = std::fs::metadata(&fixture_path_abs)?.modified()?;
            let dll_mtime = std::fs::metadata(&dll_path)?.modified()?;
            if source_mtime <= dll_mtime {
                return Ok(dll_path);
            }
        }

        let status = Command::new("dotnet")
            .args([
                "build",
                manifest_dir
                    .join("tests/SingleFile.csproj")
                    .to_str()
                    .unwrap(),
                "-p:AllowUnsafeBlocks=true",
                &format!("-p:TestFile={}", absolute_file.display()),
                "-o",
                output_dir.to_str().unwrap(),
                &format!(
                    "-p:BaseIntermediateOutputPath={}/",
                    output_dir.join("obj").display()
                ),
                "-v:q",
                "--nologo",
            ])
            .status()?;
        assert!(
            status.success(),
            "dotnet build failed for {:?}",
            fixture_path
        );
        Ok(dll_path)
    }

    pub fn ensure_dll(&self, fixture_path: &Path) -> PathBuf {
        let prebuilt = self.prebuilt_dll_path(fixture_path);
        if prebuilt.exists() {
            return prebuilt;
        }

        eprintln!(
            "WARNING: Prebuilt DLL missing at {:?}. Lazy compiling {:?} using SingleFile.csproj...",
            prebuilt, fixture_path
        );
        self.build(fixture_path).expect("failed to lazy compile fixture")
    }

    pub fn prebuilt_dll_path(&self, fixture_path: &Path) -> PathBuf {
        let fixtures_base = Self::fixtures_base_dir();
        let file_name = fixture_path.file_stem().unwrap().to_str().unwrap();
        if let Ok(relative_path) = fixture_path.strip_prefix("tests/fixtures") {
            let parent = relative_path.parent().unwrap_or_else(|| Path::new(""));
            fixtures_base
                .join(parent)
                .join(file_name)
                .join("SingleFile.dll")
        } else {
            // Not a standard fixture path, just return something that won't exist
            fixtures_base
                .join("non-existent")
                .join(file_name)
                .join("SingleFile.dll")
        }
    }

    pub fn run_with_timeout(&self, dll_path: &Path, timeout: Duration) -> ExecutionResult {
        let resolution = self.loader.load_resolution_from_file(dll_path).unwrap();
        let shared = Arc::new(state::SharedGlobalState::new(Arc::clone(&self.loader)));
        self.run_with_shared_timeout(resolution, shared, timeout)
    }

    pub fn run_with_shared_timeout(
        &self,
        resolution: ResolutionS,
        shared: Arc<state::SharedGlobalState>,
        timeout: Duration,
    ) -> ExecutionResult {
        let (tx, rx) = std::sync::mpsc::channel();
        let shared_clone = Arc::clone(&shared);
        let handle = std::thread::spawn(move || {
            let mut executor = vm::Executor::new(shared_clone.clone());
            let entry_method = match resolution.entry_point {
                Some(EntryPoint::Method(m)) => m,
                Some(EntryPoint::File(f)) => {
                    todo!("find entry point in file {}", resolution[f].name)
                }
                None => {
                    panic!("expected input module to have an entry point, received one without")
                }
            };
            let td = TypeDescription::new(resolution.clone(), entry_method.parent_type());
            let method_index = td
                .definition()
                .methods
                .iter()
                .position(|m| std::ptr::eq(m, &resolution.definition()[entry_method]))
                .unwrap();

            let method = MethodDescription::new(
                td,
                vm::GenericLookup::default(),
                resolution,
                MethodMemberIndex::Method(method_index),
            );
            executor.entrypoint(method);

            let result = match executor.run() {
                vm::ExecutorResult::Exited(code) => ExecutionResult {
                    exit_code: code,
                    stderr: None,
                },
                vm::ExecutorResult::Threw(e) => {
                    let msg = format!("Execution threw: {:?}", e);
                    eprintln!("{}", msg);
                    ExecutionResult {
                        exit_code: 1,
                        stderr: Some(msg),
                    }
                }
                vm::ExecutorResult::Error(e) => {
                    let msg = format!("Execution error: {:?}", e);
                    eprintln!("{}", msg);
                    ExecutionResult {
                        exit_code: 255,
                        stderr: Some(msg),
                    }
                }
            };

            let _ = tx.send(result);
        });

        match rx.recv_timeout(timeout) {
            Ok(result) => {
                handle.join().expect("Executor thread panicked");
                result
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                shared.abort_requested.store(true, Ordering::Relaxed);
                let _ = handle.join();
                panic!("Execution timed out after {:?}", timeout);
            }
            Err(e) => {
                shared.abort_requested.store(true, Ordering::Relaxed);
                let _ = handle.join();
                panic!("Execution failed: {:?}", e);
            }
        }
    }

    pub fn run(&self, dll_path: &Path) -> u8 {
        let resolution = self.loader.load_resolution_from_file(dll_path).unwrap();
        let shared = Arc::new(state::SharedGlobalState::new(Arc::clone(&self.loader)));
        self.run_with_shared(resolution, shared).exit_code
    }

    pub fn run_with_shared(
        &self,
        resolution: ResolutionS,
        shared: Arc<state::SharedGlobalState>,
    ) -> ExecutionResult {
        self.run_with_shared_internal(resolution, shared, |_| {})
    }

    #[cfg(feature = "multithreading")]
    pub fn run_with_shared_signal_then_wait<F>(
        &self,
        resolution: ResolutionS,
        shared: Arc<state::SharedGlobalState>,
        release_gate: &Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
        on_result: F,
    ) where
        F: FnOnce(ExecutionResult),
    {
        self.run_with_shared_internal(resolution, shared, |result| {
            on_result(result);
            let (lock, condvar) = &**release_gate;
            let mut released = lock.lock().expect("release gate lock poisoned");
            while !*released {
                released = condvar
                    .wait(released)
                    .expect("release gate wait lock poisoned");
            }
        });
    }

    fn run_with_shared_internal<F>(
        &self,
        resolution: ResolutionS,
        shared: Arc<state::SharedGlobalState>,
        on_complete: F,
    ) -> ExecutionResult
    where
        F: FnOnce(ExecutionResult),
    {
        let mut executor = vm::Executor::new(shared.clone());
        let entry_method = match resolution.entry_point {
            Some(EntryPoint::Method(m)) => m,
            Some(EntryPoint::File(f)) => todo!("find entry point in file {}", resolution[f].name),
            None => panic!("expected input module to have an entry point, received one without"),
        };
        let td = TypeDescription::new(resolution.clone(), entry_method.parent_type());
        let method_index = td
            .definition()
            .methods
            .iter()
            .position(|m| std::ptr::eq(m, &resolution.definition()[entry_method]))
            .unwrap();

        let method = MethodDescription::new(
            td,
            vm::GenericLookup::default(),
            resolution,
            MethodMemberIndex::Method(method_index),
        );
        executor.entrypoint(method);

        let result = match executor.run() {
            vm::ExecutorResult::Exited(code) => ExecutionResult {
                exit_code: code,
                stderr: None,
            },
            vm::ExecutorResult::Threw(e) => {
                let msg = format!("Execution threw: {:?}", e);
                eprintln!("{}", msg);
                ExecutionResult {
                    exit_code: 1,
                    stderr: Some(msg),
                }
            }
            vm::ExecutorResult::Error(e) => {
                let msg = format!("Execution error: {:?}", e);
                eprintln!("{}", msg);
                ExecutionResult {
                    exit_code: 255,
                    stderr: Some(msg),
                }
            }
        };

        // In multithreading mode, extract the arena BEFORE dropping the executor.
        // Other threads may still be executing and accessing shared static references
        // that point into this arena's memory, so we must keep it alive until after
        // the on_complete callback (and any gate waits) finish.
        #[cfg(feature = "multithreading")]
        let arena_guard = executor.extract_arena();

        // Drop the executor BEFORE calling on_complete to avoid STW deadlocks.
        //
        // Executor::drop unregisters the arena from the GC coordinator first, then
        // unregisters the thread. If we kept the executor alive across on_complete, a
        // caller that blocks inside on_complete (e.g. run_with_shared_signal_then_wait
        // waiting on the release-gate condvar) would leave this thread still registered
        // with the GC coordinator. Any concurrent arena that triggers a collection would
        // then dispatch GCCommand::MarkAll to this thread's ArenaHandle; the blocked
        // thread never calls safe_point again, so wait_on_other_arenas hangs forever.
        //
        // The arena has already been extracted above (in multithreading mode), so
        // Executor::drop will find THREAD_ARENA empty and skip clearing it.
        drop(executor);

        on_complete(result.clone());

        // In multithreading mode, drop the arena AFTER on_complete (and any gate waits) finish.
        #[cfg(feature = "multithreading")]
        drop(arena_guard);

        result
    }

    /// Runs the CLI binary as a subprocess and captures its output.
    pub fn run_cli(&self, dll_path: &Path) -> (u8, String) {
        let assemblies_path = Self::find_dotnet_app_path();
        let output = if let Some(bin_path) = std::env::var_os("CARGO_BIN_EXE_dotnet-rs") {
            Command::new(bin_path)
                .args([
                    "-a",
                    assemblies_path.to_str().unwrap(),
                    dll_path.to_str().unwrap(),
                ])
                .output()
                .expect("failed to run dotnet-rs binary")
        } else {
            let cargo_bin = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
            let mut command = Command::new(cargo_bin);
            command.args([
                "run",
                "--quiet",
                "--bin",
                "dotnet-rs",
                "--no-default-features",
            ]);

            #[cfg(feature = "multithreading")]
            command.args(["--features", "multithreading"]);

            #[cfg(feature = "memory-validation")]
            command.args(["--features", "memory-validation"]);

            if let Ok(target) = std::env::var("CARGO_BUILD_TARGET") {
                command.args(["--target", target.as_str()]);
            }

            command.args([
                "--",
                "-a",
                assemblies_path.to_str().unwrap(),
                dll_path.to_str().unwrap(),
            ]);
            command.output().expect("failed to run cargo run")
        };

        let exit_code = output.status.code().unwrap_or(255) as u8;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        (exit_code, stdout)
    }

    /// Orchestrates a multi-arena test with the given fixture.
    #[cfg(feature = "multithreading")]
    pub fn run_multi_arena_fixture(
        self: &Arc<Self>,
        dll_path: &Path,
        thread_count: usize,
        expected_exit_code: u8,
        timeout: Duration,
        test_name: &str,
    ) {
        use std::sync::mpsc;
        use std::thread;

        let resolution = self
            .loader
            .load_resolution_from_file(dll_path)
            .expect("Failed to load assembly");

        let (tx, rx) = mpsc::channel();
        let mut abort_flags = Vec::new();
        let mut handles = Vec::new();
        let release_gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));

        let shared = Arc::new(dotnet_vm::state::SharedGlobalState::new(self.loader.clone()));
        for _ in 0..thread_count {
            let harness = self.clone();
            let tx = tx.clone();
            let shared = Arc::clone(&shared);
            let release_gate = Arc::clone(&release_gate);
            let resolution = resolution.clone();
            abort_flags.push(Arc::clone(&shared.abort_requested));

            let handle = thread::spawn(move || {
                let tx_ok = tx.clone();
                let tx_err = tx;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                    harness.run_with_shared_signal_then_wait(
                        resolution.clone(),
                        shared,
                        &release_gate,
                        move |result| {
                            let _ = tx_ok.send(Ok(result));
                        },
                    );
                }));
                if let Err(panic_info) = result {
                    let _ = tx_err.send(Err(panic_info));
                }
            });
            handles.push(handle);
        }

        let start = std::time::Instant::now();
        let mut test_error = None;

        for _ in 0..thread_count {
            let remaining = timeout.saturating_sub(start.elapsed());
            match rx.recv_timeout(remaining) {
                Ok(Ok(result)) => {
                    if result.exit_code != expected_exit_code {
                        test_error = Some(format!(
                            "Multi-arena test {} failed: expected exit code {}, got {}. Stderr: {:?}",
                            test_name, expected_exit_code, result.exit_code, result.stderr
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
                    test_error = Some(format!("TIMEOUT in {}", test_name));
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
}

#[test]
fn fixtures_base_is_not_crate_local() {
    let fixtures_base = TestHarness::fixtures_base_dir();
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    assert!(
        !fixtures_base.starts_with(&manifest_dir),
        "expected DOTNET_FIXTURES_BASE to be outside the crate directory; base={:?} manifest={:?}",
        fixtures_base,
        manifest_dir
    );
}
