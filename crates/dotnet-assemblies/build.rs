use dotnet_build_tools::{
    find_repo_root, shared_msbuild_input_candidates, should_skip_dotnet_build,
};
use std::{path::Path, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=src/support/support.csproj");
    println!("cargo:rerun-if-changed=src/support");
    println!("cargo:rerun-if-env-changed=DOTNET_SKIP_BUILD");
    println!("cargo:rerun-if-env-changed=RUSTC_WORKSPACE_WRAPPER");
    println!("cargo:rerun-if-env-changed=RUSTC_WRAPPER");
    println!("cargo:rerun-if-env-changed=CLIPPY_ARGS");

    fn watch_dir(dir: &Path) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap().to_str().unwrap();
                if name != "bin" && name != "obj" {
                    watch_dir(&path);
                }
            } else {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let support_dir = manifest_dir.join("src/support");
    let repo_root = find_repo_root(manifest_dir);
    for input in shared_msbuild_input_candidates(&support_dir, &repo_root) {
        // Emit these even when missing so introducing one later retriggers build.rs.
        println!("cargo:rerun-if-changed={}", input.display());
    }

    if support_dir.exists() {
        watch_dir(&support_dir);
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
