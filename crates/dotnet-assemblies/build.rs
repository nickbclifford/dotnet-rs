use std::{path::Path, process::Command};

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
