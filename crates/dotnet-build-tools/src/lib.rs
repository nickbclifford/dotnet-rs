//! # dotnet-build-tools
//!
//! Shared helpers for deterministic build-script input scanning and caching.
//! Used by `dotnet-cli/build.rs` and `dotnet-assemblies/build.rs` to avoid
//! re-running MSBuild/dotnet restore when fixture inputs have not changed.
//!
//! ## Key Functions
//!
//! - [`cargo_profile_target_dir_from_out_dir`]: derives the Cargo target-profile
//!   directory from `OUT_DIR`.
//! - Fixture hash and input discovery utilities used by fixture caching tools.

use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
};

const SHARED_MSBUILD_INPUT_FILE_NAMES: &[&str] = &[
    "Directory.Build.props",
    "Directory.Build.targets",
    "Directory.Packages.props",
    "global.json",
    "nuget.config",
    "NuGet.config",
];

/// Returns `<target>/<profile>` from Cargo `OUT_DIR`.
///
/// Typical `OUT_DIR` shape:
/// `<target>/<profile>/build/<pkg>-<hash>/out`.
pub fn cargo_profile_target_dir_from_out_dir(out_dir: &Path) -> PathBuf {
    for ancestor in out_dir.ancestors() {
        if ancestor.file_name().is_some_and(|name| name == "build")
            && let Some(profile_dir) = ancestor.parent()
        {
            return profile_dir.to_path_buf();
        }
    }

    // Fallback for unexpected layouts: keep outputs under OUT_DIR.
    out_dir.to_path_buf()
}

/// Recursively collects files with the provided extension (without dot).
/// Entries are traversed in sorted order for deterministic outputs.
pub fn collect_files_with_extension(root: &Path, extension: &str) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_files_with_extension_inner(root, extension, &mut files)?;
    Ok(files)
}

fn collect_files_with_extension_inner(
    root: &Path,
    extension: &str,
    out: &mut Vec<PathBuf>,
) -> io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(root)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_files_with_extension_inner(&path, extension, out)?;
        } else if path.extension().and_then(|part| part.to_str()) == Some(extension) {
            out.push(path);
        }
    }

    Ok(())
}

/// Hashes `path` and file bytes when present. Missing files are treated as absent input.
pub fn hash_if_present<H: Hasher>(hasher: &mut H, path: &Path) -> io::Result<bool> {
    match fs::read(path) {
        Ok(content) => {
            path.to_string_lossy().hash(hasher);
            content.hash(hasher);
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub fn hash_value_u64<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// Returns true when Cargo is invoking rustc through clippy-driver wrappers.
pub fn is_clippy_invocation() -> bool {
    fn env_is_clippy_driver(var: &str) -> bool {
        std::env::var_os(var).is_some_and(|value| {
            Path::new(&value)
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.to_ascii_lowercase().starts_with("clippy-driver"))
        })
    }

    env_is_clippy_driver("RUSTC_WORKSPACE_WRAPPER")
        || env_is_clippy_driver("RUSTC_WRAPPER")
        || std::env::var_os("CLIPPY_ARGS").is_some()
}

/// Returns true when dotnet build work should be skipped for build scripts.
pub fn should_skip_dotnet_build() -> bool {
    is_clippy_invocation() || std::env::var("DOTNET_SKIP_BUILD").is_ok_and(|v| v == "1")
}

pub fn find_repo_root(start: &Path) -> PathBuf {
    for ancestor in start.ancestors() {
        if ancestor.join(".git").exists() {
            return ancestor.to_path_buf();
        }
    }
    start.to_path_buf()
}

/// Lists shared MSBuild/NuGet config files from `start_dir` up to `repo_root`.
/// Missing files are intentionally included in the returned list so callers can
/// still emit rerun directives and pick up newly introduced files.
pub fn shared_msbuild_input_candidates(start_dir: &Path, repo_root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    for ancestor in start_dir.ancestors() {
        if !ancestor.starts_with(repo_root) {
            continue;
        }

        for file_name in SHARED_MSBUILD_INPUT_FILE_NAMES {
            candidates.push(ancestor.join(file_name));
        }

        if ancestor == repo_root {
            break;
        }
    }

    candidates
}

/// Computes fixture output directory as:
/// `<target-dir>/<target?>/<profile>/dotnet-fixtures`.
pub fn fixture_output_base(target_dir: &Path, profile: &str, target: Option<&str>) -> PathBuf {
    let mut output = target_dir.to_path_buf();
    if let Some(target_triple) = target {
        output.push(target_triple);
    }
    output.push(profile);
    output.push("dotnet-fixtures");
    output
}

/// Computes fixture cache hash from csproj + fixture + shared MSBuild inputs.
pub fn fixture_cache_hash(
    fixtures: &[PathBuf],
    tests_dir: &Path,
    shared_msbuild_inputs: &[PathBuf],
) -> io::Result<u64> {
    let mut hasher = DefaultHasher::new();

    for csproj in ["SingleFile.csproj", "BatchFixtures.csproj"] {
        let path = tests_dir.join(csproj);
        hash_if_present(&mut hasher, &path)?;
    }

    for fixture in fixtures {
        hash_if_present(&mut hasher, fixture)?;
    }

    for input in shared_msbuild_inputs {
        hash_if_present(&mut hasher, input)?;
    }

    Ok(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::{
        cargo_profile_target_dir_from_out_dir, fixture_output_base, shared_msbuild_input_candidates,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn derives_profile_dir_for_default_target_layout() {
        assert_eq!(
            cargo_profile_target_dir_from_out_dir(Path::new(
                "target/debug/build/dotnet-cli-abc123/out"
            )),
            PathBuf::from("target/debug")
        );
    }

    #[test]
    fn derives_profile_dir_for_custom_target_dir_layout() {
        assert_eq!(
            cargo_profile_target_dir_from_out_dir(Path::new(
                "/tmp/dnr-targetdir/debug/build/dotnet-cli-abc123/out"
            )),
            PathBuf::from("/tmp/dnr-targetdir/debug")
        );
    }

    #[test]
    fn derives_profile_dir_for_non_default_profile_layout() {
        assert_eq!(
            cargo_profile_target_dir_from_out_dir(Path::new(
                "target/bench-fat/build/dotnet-cli-abc123/out"
            )),
            PathBuf::from("target/bench-fat")
        );
    }

    #[test]
    fn derives_profile_dir_for_explicit_target_layout() {
        assert_eq!(
            cargo_profile_target_dir_from_out_dir(Path::new(
                "target/x86_64-unknown-linux-gnu/debug/build/dotnet-cli-abc123/out"
            )),
            PathBuf::from("target/x86_64-unknown-linux-gnu/debug")
        );
    }

    #[test]
    fn falls_back_to_out_dir_when_build_ancestor_is_missing() {
        let out_dir = Path::new("target/debug/dotnet-cli-abc123/out");
        assert_eq!(cargo_profile_target_dir_from_out_dir(out_dir), out_dir);
    }

    #[test]
    fn fixture_output_base_default_layout() {
        assert_eq!(
            fixture_output_base(Path::new("target"), "debug", None),
            PathBuf::from("target/debug/dotnet-fixtures")
        );
    }

    #[test]
    fn fixture_output_base_with_target_dir_profile_and_triple() {
        assert_eq!(
            fixture_output_base(
                Path::new("/tmp/custom-target"),
                "bench-fat",
                Some("x86_64-unknown-linux-gnu")
            ),
            PathBuf::from("/tmp/custom-target/x86_64-unknown-linux-gnu/bench-fat/dotnet-fixtures")
        );
    }

    #[test]
    fn shared_msbuild_inputs_walk_up_to_repo_root() {
        let repo_root = Path::new("/repo");
        let start_dir = Path::new("/repo/crates/dotnet-cli/tests");
        let candidates = shared_msbuild_input_candidates(start_dir, repo_root);

        assert!(candidates.contains(&PathBuf::from(
            "/repo/crates/dotnet-cli/tests/Directory.Build.props"
        )));
        assert!(candidates.contains(&PathBuf::from(
            "/repo/crates/dotnet-cli/Directory.Build.props"
        )));
        assert!(candidates.contains(&PathBuf::from("/repo/crates/Directory.Build.props")));
        assert!(candidates.contains(&PathBuf::from("/repo/Directory.Build.props")));
        assert!(candidates.contains(&PathBuf::from("/repo/NuGet.config")));
    }
}
