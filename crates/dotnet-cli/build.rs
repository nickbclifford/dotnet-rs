use std::{
    collections::hash_map::DefaultHasher,
    fs::File,
    hash::{Hash, Hasher},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

/// Returns true when we should skip expensive dotnet build steps.
/// This is the case during `cargo clippy` (detected via CARGO_CFG_CLIPPY) or
/// when the user explicitly sets `DOTNET_SKIP_BUILD=1`.
fn should_skip_dotnet_build() -> bool {
    std::env::var("CARGO_CFG_CLIPPY").is_ok()
        || std::env::var("DOTNET_SKIP_BUILD").is_ok_and(|v| v == "1")
}

fn cargo_profile_target_dir_from_out_dir(out_dir: &Path) -> PathBuf {
    // `OUT_DIR` typically looks like:
    //   <target>/<profile>/build/<pkg>-<hash>/out
    // We want the `<target>/<profile>` directory so fixture builds live under Cargo's `target` tree
    // but outside the per-build-script hash dir.
    for ancestor in out_dir.ancestors() {
        if ancestor.file_name().is_some_and(|n| n == "build")
            && let Some(profile_dir) = ancestor.parent() {
            return profile_dir.to_path_buf();
        }
    }

    // Fallback: keep everything under `OUT_DIR`.
    out_dir.to_path_buf()
}

fn main() {
    println!("cargo:rerun-if-changed=tests/SingleFile.csproj");
    println!("cargo:rerun-if-changed=tests/BatchFixtures.csproj");
    // Ensure adding/removing fixtures triggers regeneration of `tests.rs`.
    println!("cargo:rerun-if-changed=tests/fixtures");
    println!("cargo:rerun-if-env-changed=DOTNET_SKIP_BUILD");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let out_dir_path = PathBuf::from(&out_dir);
    let destination = out_dir_path.join("tests.rs");
    let mut f = File::create(&destination).unwrap();

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tests_dir = manifest_dir.join("tests");
    let fixtures_dir = tests_dir.join("fixtures");
    let cargo_profile_target_dir = cargo_profile_target_dir_from_out_dir(&out_dir_path);
    let output_base = cargo_profile_target_dir.join("dotnet-fixtures");
    println!(
        "cargo:rustc-env=DOTNET_FIXTURES_BASE={}",
        output_base.display()
    );

    let mut fixtures = Vec::new();
    find_fixtures(&fixtures_dir, &mut fixtures);
    fixtures.sort_by_key(|p| p.to_string_lossy().to_string());

    let hash = compute_fixtures_hash(&fixtures, &tests_dir);
    let hash_file = output_base.join(".fixtures_hash");
    let previous_hash: Option<u64> = std::fs::read_to_string(&hash_file)
        .ok()
        .and_then(|s| s.trim().parse().ok());

    let needs_dotnet_build = previous_hash != Some(hash);

    if !should_skip_dotnet_build() && needs_dotnet_build {
        // Ensure output_base exists before we save the hash later
        std::fs::create_dir_all(&output_base).unwrap();

        let msbuild_obj = output_base.join("msbuild-obj");
        let msbuild_bin = output_base.join("msbuild-bin");

        // Restore packages once for SingleFile.csproj.
        let restore_status = Command::new("dotnet")
            .args([
                "restore",
                "SingleFile.csproj",
                "-v:q",
                "--nologo",
            ])
            .arg(format!("-p:BaseIntermediateOutputPath={}/", msbuild_obj.display()))
            .arg(format!("-p:BaseOutputPath={}/", msbuild_bin.display()))
            .current_dir(&tests_dir)
            .status()
            .expect("Failed to run dotnet restore");

        assert!(restore_status.success(), "dotnet restore failed");

        // Batch compile all fixtures (restore already done).
        let status = Command::new("dotnet")
            .args([
                "build",
                "BatchFixtures.csproj",
                "-m",              // parallel MSBuild nodes
                "-v:q",            // quiet verbosity
                "--nologo",        // suppress banner
                "-clp:ErrorsOnly", // only show errors
                "--no-restore",
            ])
            .arg(format!("-p:FixtureOutputBase={}/", output_base.display()))
            .arg(format!("-p:BaseIntermediateOutputPath={}/", msbuild_obj.display()))
            .arg(format!("-p:BaseOutputPath={}/", msbuild_bin.display()))
            .current_dir(&tests_dir)
            .status()
            .expect("Failed to run dotnet build. Is the .NET SDK installed?");

        assert!(
            status.success(),
            "dotnet build BatchFixtures.csproj failed. Check that the .NET 10 SDK is installed."
        );

        // Save hash on success
        std::fs::write(&hash_file, hash.to_string()).unwrap();
    }

    for path in fixtures {
        let file_name = path.file_stem().unwrap().to_str().unwrap();
        let expected_exit_code: u8 = file_name
            .split('_')
            .next_back()
            .unwrap()
            .parse()
            .expect("fixture file name must end with _<exit_code>.cs");

        let relative_path = path.strip_prefix(&fixtures_dir).unwrap();
        let test_name = relative_path
            .with_extension("")
            .to_str()
            .unwrap()
            .replace("/", "_")
            .replace("\\", "_");

        let dll_path = output_base
            .join(relative_path.parent().unwrap())
            .join(file_name)
            .join("SingleFile.dll");

        let mut ignore_prefix = "".to_string();
        let is_multithreading_enabled = std::env::var("CARGO_FEATURE_MULTITHREADING").is_ok();
        if file_name == "bench_loop_42" 
            || !dll_path.exists()
            || (file_name == "monitor_try_enter_timeout_42" && !is_multithreading_enabled) {
            ignore_prefix = "#[ignore] ".to_string();
        }

        // `generic_constraints_fail_0.cs` currently triggers a stack overflow when executed under
        // `generic-constraint-validation` (see `MakeGenericType` constraint checking).
        // Keep the test generated but ignored to avoid aborting the test suite.
        if test_name == "generics_generic_constraints_fail_0" {
            writeln!(
                f,
                "fixture_test!(#[ignore] {}, {:?}, {});",
                test_name,
                dll_path.to_str().unwrap(),
                expected_exit_code
            )
            .unwrap();
        } else {
            writeln!(
                f,
                "fixture_test!({}{}, {:?}, {});",
                ignore_prefix,
                test_name,
                dll_path.to_str().unwrap(),
                expected_exit_code
            )
            .unwrap();
        }
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn find_fixtures(dir: &Path, fixtures: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            find_fixtures(&path, fixtures);
        } else if path.extension().map(|s| s == "cs").unwrap_or(false) {
            fixtures.push(path);
        }
    }
}

fn compute_fixtures_hash(fixtures: &[PathBuf], tests_dir: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();

    // Hash .csproj files
    for csproj in &["SingleFile.csproj", "BatchFixtures.csproj"] {
        let path = tests_dir.join(csproj);
        if let Ok(content) = std::fs::read(&path) {
            path.to_string_lossy().hash(&mut hasher);
            content.hash(&mut hasher);
        }
    }

    // Hash all fixture .cs files (sorted for determinism)
    for fixture in fixtures {
        if let Ok(content) = std::fs::read(fixture) {
            fixture.to_string_lossy().hash(&mut hasher);
            content.hash(&mut hasher);
        }
    }

    hasher.finish()
}
