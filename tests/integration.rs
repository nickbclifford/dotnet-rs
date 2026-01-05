use dotnet_rs::{
    assemblies,
    types::{members::MethodDescription, TypeDescription},
    utils::static_res_from_file,
    vm,
};
use dotnetdll::prelude::*;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub struct TestHarness {
    pub assemblies: &'static assemblies::AssemblyLoader,
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
        let assemblies = assemblies::AssemblyLoader::new(assemblies_path);
        let assemblies = Box::leak(Box::new(assemblies));
        Self { assemblies }
    }

    fn find_dotnet_app_path() -> PathBuf {
        let base = std::env::var("DOTNET_ROOT")
            .map(|p| PathBuf::from(p).join("shared/Microsoft.NETCore.App"))
            .unwrap_or_else(|_| PathBuf::from("/usr/share/dotnet/shared/Microsoft.NETCore.App"));

        let base = if !base.exists() {
            let alt_base = PathBuf::from("/usr/lib/dotnet/shared/Microsoft.NETCore.App");
            if alt_base.exists() {
                alt_base
            } else {
                base
            }
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

    pub fn build(&self, fixture_path: &Path) -> PathBuf {
        let file_name = fixture_path.file_stem().unwrap().to_str().unwrap();
        let output_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("dotnet-fixtures")
            .join(file_name);

        let absolute_file = std::fs::canonicalize(fixture_path).unwrap();
        let dll_path = output_dir.join("SingleFile.dll");

        if dll_path.exists() {
            let source_mtime = std::fs::metadata(fixture_path).unwrap().modified().unwrap();
            let dll_mtime = std::fs::metadata(&dll_path).unwrap().modified().unwrap();
            if source_mtime <= dll_mtime {
                return dll_path;
            }
        }

        let status = Command::new("dotnet")
            .args([
                "build",
                "tests/SingleFile.csproj",
                &format!("-p:TestFile={}", absolute_file.display()),
                "-o",
                output_dir.to_str().unwrap(),
                &format!(
                    "-p:IntermediateOutputPath={}/",
                    output_dir.join("obj").display()
                ),
            ])
            .status()
            .expect("failed to run dotnet build");
        assert!(
            status.success(),
            "dotnet build failed for {:?}",
            fixture_path
        );

        dll_path
    }

    pub fn run(&self, dll_path: &Path) -> u8 {
        let dll_path_str = dll_path.to_str().unwrap().to_string();
        let resolution = static_res_from_file(&dll_path_str);

        let entry_method = match resolution.entry_point {
            Some(EntryPoint::Method(m)) => m,
            _ => panic!("Expected method entry point in {:?}", dll_path),
        };

        let arena = Box::new(vm::GCArena::new(|gc| {
            let global = std::sync::Arc::new(vm::GlobalState::new(gc, self.assemblies));
            vm::CallStack::new(gc, global)
        }));
        let mut executor = vm::Executor::new(Box::leak(arena));

        let entrypoint = MethodDescription {
            parent: TypeDescription::new(
                resolution,
                &resolution.definition()[entry_method.parent_type()],
            ),
            method: &resolution.definition()[entry_method],
        };
        executor.entrypoint(entrypoint);

        match executor.run() {
            vm::ExecutorResult::Exited(i) => i,
            vm::ExecutorResult::Threw => {
                panic!("VM threw an exception while running {:?}", dll_path)
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
            let dll_path = harness.build(Path::new($path));
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

/// This test is intended for debugging purposes and is not part of the regular CI.
/// It runs a "Hello, World!" program using the full .NET SDK libraries.
/// To run this test, use:
/// `cargo test hello_world -- --ignored --nocapture`
#[test]
#[ignore]
fn hello_world() {
    let harness = TestHarness::get();
    let fixture_path = Path::new("tests/debug_fixtures/hello_world_0.cs");
    let dll_path = harness.build(fixture_path);
    let exit_code = harness.run(&dll_path);
    assert_eq!(exit_code, 0);
}
