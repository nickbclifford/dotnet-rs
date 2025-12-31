use dotnet_rs::utils::static_res_from_file;
use dotnet_rs::value::{MethodDescription, TypeDescription};
use dotnet_rs::{resolve, vm};
use dotnetdll::prelude::*;
use std::path::{Path, PathBuf};
use std::process::Command;

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

fn build_test(file: &Path, output_dir: &Path) {
    let absolute_file = std::fs::canonicalize(file).unwrap();
    let status = Command::new("dotnet")
        .args([
            "build",
            "tests/SingleFile.csproj",
            &format!("-p:TestFile={}", absolute_file.display()),
            "-o",
            output_dir.to_str().unwrap(),
        ])
        .status()
        .expect("failed to run dotnet build");
    assert!(status.success(), "dotnet build failed for {:?}", file);
}

fn run_test_dll(dll_path: &Path) -> u8 {
    let dll_path_str = dll_path.to_str().unwrap().to_string();
    let resolution = static_res_from_file(&dll_path_str);

    let entry_method = match resolution.entry_point {
        Some(EntryPoint::Method(m)) => m,
        _ => panic!("Expected method entry point in {:?}", dll_path),
    };

    let assemblies_path = find_dotnet_app_path().to_str().unwrap().to_string();
    let assemblies = resolve::Assemblies::new(assemblies_path);
    let assemblies = Box::leak(Box::new(assemblies));

    let arena = Box::new(vm::GCArena::new(|gc| vm::CallStack::new(gc, assemblies)));
    let mut executor = vm::Executor::new(Box::leak(arena));

    let entrypoint = MethodDescription {
        parent: TypeDescription {
            resolution,
            definition: &resolution.0[entry_method.parent_type()],
        },
        method: &resolution.0[entry_method],
    };
    executor.entrypoint(entrypoint);

    match executor.run() {
        vm::ExecutorResult::Exited(i) => i,
        vm::ExecutorResult::Threw => panic!("VM threw an exception while running {:?}", dll_path),
    }
}

#[test]
fn run_fixtures() {
    let fixtures_dir = Path::new("tests/fixtures");
    let bin_dir = Path::new("tests/bin");

    for entry in std::fs::read_dir(fixtures_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().map(|s| s == "cs").unwrap_or(false) {
            let file_name = path.file_stem().unwrap().to_str().unwrap();
            let expected_exit_code: u8 = file_name
                .split('_')
                .next_back()
                .unwrap()
                .parse()
                .expect("fixture file name must end with _<exit_code>.cs");

            let output_dir = bin_dir.join(file_name);
            build_test(&path, &output_dir);

            let dll_path = output_dir.join("SingleFile.dll");
            let exit_code = run_test_dll(&dll_path);
            assert_eq!(
                exit_code, expected_exit_code,
                "Test {:?} failed: expected exit code {}, got {}",
                file_name, expected_exit_code, exit_code
            );
        }
    }
}
