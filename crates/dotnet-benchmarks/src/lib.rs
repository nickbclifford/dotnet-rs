#![allow(clippy::arc_with_non_send_sync)]
use dotnet_types::{TypeDescription, members::MethodDescription};
use dotnet_vm::{self as vm, ExecutorResult, state, sync::Arc};
use dotnetdll::prelude::{EntryPoint, MethodMemberIndex};
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Copy, Debug)]
pub struct BenchmarkCase {
    pub name: &'static str,
    pub source: &'static str,
    pub expected_exit_code: u8,
}

pub const JSON_BENCHMARK: BenchmarkCase = BenchmarkCase {
    name: "json",
    source: "fixtures/json/JsonBenchmark_0.cs",
    expected_exit_code: 0,
};

pub const ARITHMETIC_BENCHMARK: BenchmarkCase = BenchmarkCase {
    name: "arithmetic",
    source: "fixtures/arithmetic/TightLoop_0.cs",
    expected_exit_code: 0,
};

pub const GC_BENCHMARK: BenchmarkCase = BenchmarkCase {
    name: "gc",
    source: "fixtures/gc/AllocationPressure_0.cs",
    expected_exit_code: 0,
};

pub const DISPATCH_BENCHMARK: BenchmarkCase = BenchmarkCase {
    name: "dispatch",
    source: "fixtures/dispatch/VirtualDispatchStress_0.cs",
    expected_exit_code: 0,
};

pub const GENERICS_BENCHMARK: BenchmarkCase = BenchmarkCase {
    name: "generics",
    source: "fixtures/generics/GenericsStress_0.cs",
    expected_exit_code: 0,
};

pub const BENCHMARK_CASES: [BenchmarkCase; 5] = [
    JSON_BENCHMARK,
    ARITHMETIC_BENCHMARK,
    GC_BENCHMARK,
    DISPATCH_BENCHMARK,
    GENERICS_BENCHMARK,
];

pub struct BenchHarness {
    loader: Arc<dotnet_assemblies::AssemblyLoader>,
}

#[derive(Debug, Clone)]
pub struct BenchRunResult {
    pub exit_code: u8,
    pub metrics: vm::RuntimeMetricsSnapshot,
}

impl Default for BenchHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl BenchHarness {
    pub fn new() -> Self {
        let assemblies_path = dotnet_assemblies::find_dotnet_app_path()
            .expect("could not find .NET shared path")
            .to_str()
            .expect("failed to convert path to UTF-8")
            .to_string();

        let loader = dotnet_assemblies::AssemblyLoader::new(assemblies_path)
            .expect("failed to create AssemblyLoader");

        Self {
            loader: Arc::new(loader),
        }
    }

    pub fn ensure_fixture_dll(&self, case: BenchmarkCase) -> PathBuf {
        let source_path = fixture_source_path(case.source);
        let fixture_stem = source_path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("fixture file stem is not valid UTF-8");
        let category = source_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .expect("fixture category directory is invalid");

        let output_dir = fixture_output_base().join(category).join(fixture_stem);
        let dll_path = output_dir.join("SingleFile.dll");

        if dll_is_fresh(&source_path, &dll_path) {
            return dll_path;
        }

        fs::create_dir_all(&output_dir).expect("failed to create fixture output directory");

        let project = single_file_project_path();
        let status = Command::new("dotnet")
            .args([
                "build",
                project.to_str().expect("invalid SingleFile.csproj path"),
                "-p:AllowUnsafeBlocks=true",
                &format!("-p:TestFile={}", source_path.display()),
                "-o",
                output_dir.to_str().expect("invalid output directory"),
                &format!(
                    "-p:BaseIntermediateOutputPath={}/",
                    output_dir.join("obj").display()
                ),
                "-v:q",
                "--nologo",
            ])
            .status()
            .expect("failed to run dotnet build for benchmark fixture");

        assert!(
            status.success(),
            "dotnet build failed for benchmark fixture {}",
            source_path.display()
        );

        dll_path
    }

    pub fn run_case(&self, case: BenchmarkCase) -> u8 {
        let dll_path = self.ensure_fixture_dll(case);
        self.run_dll(&dll_path)
    }

    pub fn run_case_with_metrics(&self, case: BenchmarkCase) -> BenchRunResult {
        let dll_path = self.ensure_fixture_dll(case);
        self.run_dll_with_metrics(&dll_path)
    }

    pub fn run_dll(&self, dll_path: &Path) -> u8 {
        self.run_dll_with_metrics(dll_path).exit_code
    }

    pub fn run_dll_with_metrics(&self, dll_path: &Path) -> BenchRunResult {
        let resolution = self
            .loader
            .load_resolution_from_file(dll_path)
            .expect("failed to load benchmark fixture assembly");

        let shared = Arc::new(state::SharedGlobalState::new(Arc::clone(&self.loader)));
        let mut executor = vm::Executor::new(shared);

        let entry_method = match resolution.entry_point {
            Some(EntryPoint::Method(m)) => m,
            Some(EntryPoint::File(f)) => {
                panic!(
                    "expected entry-point method, found file: {}",
                    resolution[f].name
                )
            }
            None => panic!("expected benchmark fixture to contain an entry point"),
        };

        let td = TypeDescription::new(resolution.clone(), entry_method.parent_type());
        let method_index = td
            .definition()
            .methods
            .iter()
            .position(|m| std::ptr::eq(m, &resolution.definition()[entry_method]))
            .expect("failed to locate entry method index");

        let entrypoint = MethodDescription::new(
            td,
            vm::GenericLookup::default(),
            resolution,
            MethodMemberIndex::Method(method_index),
        );
        executor.entrypoint(entrypoint);

        let exit_code = match executor.run() {
            ExecutorResult::Exited(code) => code,
            ExecutorResult::Threw(exc) => {
                panic!("benchmark fixture threw managed exception: {exc}")
            }
            ExecutorResult::Error(err) => panic!("benchmark fixture failed with VM error: {err}"),
        };

        BenchRunResult {
            exit_code,
            metrics: executor.get_runtime_metrics_snapshot(),
        }
    }
}

pub fn case_by_name(name: &str) -> Option<BenchmarkCase> {
    BENCHMARK_CASES.into_iter().find(|c| c.name == name)
}

pub fn fixture_output_base() -> PathBuf {
    cargo_profile_dir().join("dotnet-bench-fixtures")
}

pub fn bench_metrics_output_base() -> PathBuf {
    cargo_profile_dir().join("dotnet-bench-metrics")
}

pub fn write_bench_metrics_snapshot(case_name: &str, run: &BenchRunResult) -> io::Result<PathBuf> {
    let base = bench_metrics_output_base();
    fs::create_dir_all(&base)?;
    let path = base.join(format!("{case_name}.json"));
    let payload = serde_json::to_vec_pretty(&run.metrics).expect("failed to serialize metrics");
    fs::write(&path, payload)?;
    Ok(path)
}

fn cargo_profile_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("failed to determine benchmark executable path");

    let deps_dir = exe
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == "deps"))
        .expect("failed to locate cargo profile deps directory");

    deps_dir
        .parent()
        .expect("failed to locate cargo profile directory")
        .to_path_buf()
}

fn fixture_source_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn single_file_project_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../dotnet-cli/tests/SingleFile.csproj")
}

fn dll_is_fresh(source: &Path, dll: &Path) -> bool {
    if !dll.exists() {
        return false;
    }

    let source_modified = source
        .metadata()
        .and_then(|m| m.modified())
        .expect("failed to read fixture source metadata");
    let dll_modified = dll
        .metadata()
        .and_then(|m| m.modified())
        .expect("failed to read fixture DLL metadata");

    source_modified <= dll_modified
}
