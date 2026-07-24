use std::path::{Path, PathBuf};

pub(crate) fn fixture_probe_dir() -> PathBuf {
    if let Some(base) = std::env::var_os("DOTNET_FIXTURES_BASE") {
        let path = PathBuf::from(base).join("basic").join("basic_42");
        if path.exists() {
            return path;
        }
    }

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    if let Some(repo_root) = manifest_dir.parent().and_then(Path::parent) {
        let path = repo_root
            .join("target")
            .join("debug")
            .join("dotnet-fixtures")
            .join("basic")
            .join("basic_42");
        if path.exists() {
            return path;
        }
    }

    PathBuf::from("/tmp/fixture-probe")
}

pub(crate) fn fixture_probe_path(file_name: &str) -> PathBuf {
    fixture_probe_dir().join(file_name)
}
