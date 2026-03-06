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

fn main() {
    println!("cargo:rerun-if-changed=tests/SingleFile.csproj");
    println!("cargo:rerun-if-changed=tests/BatchFixtures.csproj");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let destination = Path::new(&out_dir).join("tests.rs");
    let mut f = File::create(&destination).unwrap();

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tests_dir = manifest_dir.join("tests");
    let fixtures_dir = tests_dir.join("fixtures");
    let output_base = manifest_dir.join("target").join("dotnet-fixtures");

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

        // Phase 0: Restore packages once for SingleFile.csproj
        let restore_status = Command::new("dotnet")
            .args(["restore", "SingleFile.csproj", "-v:q", "--nologo"])
            .current_dir(&tests_dir)
            .status()
            .expect("Failed to run dotnet restore");

        assert!(restore_status.success(), "dotnet restore failed");

        // Phase 1: Batch compile all fixtures (restore already done)
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
        if file_name == "bench_loop_42" || !dll_path.exists() {
            ignore_prefix = "#[ignore] ".to_string();
        }

        writeln!(
            f,
            "fixture_test!({}{}, {:?}, {});",
            ignore_prefix,
            test_name,
            dll_path.to_str().unwrap(),
            expected_exit_code
        )
        .unwrap();
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
