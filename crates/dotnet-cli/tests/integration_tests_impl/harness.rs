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
        let output_dir = Self::fixtures_base_dir()
            .join("adhoc")
            .join(file_name);

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

    #[allow(dead_code)]
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

    pub fn run_with_timeout(&self, dll_path: &Path, timeout: Duration) -> u8 {
        let resolution = self.loader.load_resolution_from_file(dll_path).unwrap();
        let shared = Arc::new(state::SharedGlobalState::new(Arc::clone(&self.loader)));
        self.run_with_shared_timeout(resolution, shared, timeout)
    }

    pub fn run_with_shared_timeout(
        &self,
        resolution: ResolutionS,
        shared: Arc<state::SharedGlobalState>,
        timeout: Duration,
    ) -> u8 {
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
            let method = MethodDescription::new(
                TypeDescription::new(
                    resolution,
                    &resolution.definition()[entry_method.parent_type()],
                    entry_method.parent_type(),
                ),
                vm::GenericLookup::default(),
                resolution,
                &resolution.definition()[entry_method],
            );
            executor.entrypoint(method);

            let result = match executor.run() {
                vm::ExecutorResult::Exited(code) => code,
                vm::ExecutorResult::Threw(e) => {
                    eprintln!("Execution threw: {:?}", e);
                    1
                }
                vm::ExecutorResult::Error(e) => {
                    eprintln!("Execution error: {:?}", e);
                    255
                }
            };

            let _ = tx.send(result);
        });

        match rx.recv_timeout(timeout) {
            Ok(code) => {
                handle.join().expect("Executor thread panicked");
                code
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
        self.run_with_shared(resolution, shared)
    }

    pub fn run_with_shared(
        &self,
        resolution: ResolutionS,
        shared: Arc<state::SharedGlobalState>,
    ) -> u8 {
        let mut executor = vm::Executor::new(shared.clone());
        let entry_method = match resolution.entry_point {
            Some(EntryPoint::Method(m)) => m,
            Some(EntryPoint::File(f)) => todo!("find entry point in file {}", resolution[f].name),
            None => panic!("expected input module to have an entry point, received one without"),
        };
        let method = MethodDescription::new(
            TypeDescription::new(
                resolution,
                &resolution.definition()[entry_method.parent_type()],
                entry_method.parent_type(),
            ),
            vm::GenericLookup::default(),
            resolution,
            &resolution.definition()[entry_method],
        );
        executor.entrypoint(method);

        match executor.run() {
            vm::ExecutorResult::Exited(code) => code,
            vm::ExecutorResult::Threw(e) => {
                eprintln!("Execution threw: {:?}", e);
                1
            }
            vm::ExecutorResult::Error(e) => {
                eprintln!("Execution error: {:?}", e);
                255
            }
        }
    }

    /// Runs the CLI binary as a subprocess and captures its output.
    pub fn run_cli(&self, dll_path: &Path) -> (u8, String) {
        let assemblies_path = Self::find_dotnet_app_path();
        let mut command = Command::new("cargo");
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

        command.args([
            "--",
            "-a",
            assemblies_path.to_str().unwrap(),
            dll_path.to_str().unwrap(),
        ]);

        let output = command.output().expect("failed to run cargo run");

        let exit_code = output.status.code().unwrap_or(255) as u8;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        (exit_code, stdout)
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
