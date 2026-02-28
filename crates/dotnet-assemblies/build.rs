use std::{path::Path, process::Command};

/// Returns true when we should skip expensive dotnet build steps.
/// This is the case during `cargo clippy` (detected via CARGO_CFG_CLIPPY) or
/// when the user explicitly sets `DOTNET_SKIP_BUILD=1`.
fn should_skip_dotnet_build() -> bool {
    std::env::var("CARGO_CFG_CLIPPY").is_ok()
        || std::env::var("DOTNET_SKIP_BUILD").is_ok_and(|v| v == "1")
}

fn main() {
    println!("cargo:rerun-if-changed=src/support/support.csproj");

    fn watch_dir(dir: &Path) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap().to_str().unwrap();
                if name != "bin" && name != "obj" {
                    watch_dir(&path);
                }
            } else if path.extension().map(|s| s == "cs").unwrap_or(false) {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
    // Only watch if the directory exists (it might not be moved yet when I create this file, but it will be there when I run build)
    if Path::new("src/support").exists() {
        watch_dir(Path::new("src/support"));
    }

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let dll_path = Path::new(&out_dir).join("support.dll");

    if should_skip_dotnet_build() {
        // Create an empty stub so that `include_bytes!` in lib.rs can resolve the path.
        // The real DLL is only needed at runtime, not during check/clippy.
        if !dll_path.exists() {
            std::fs::write(&dll_path, b"").expect("failed to create stub support.dll");
        }
        return;
    }

    let status = Command::new("dotnet")
        .args([
            "build",
            "src/support/support.csproj",
            "-c",
            "Debug",
            "-o",
            &out_dir,
            &format!("-p:IntermediateOutputPath={}/support-obj/", out_dir),
        ])
        .status()
        .expect("failed to run dotnet build");

    if !status.success() {
        panic!("dotnet build failed for support library");
    }
}
