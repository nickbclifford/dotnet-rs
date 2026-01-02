use std::path::Path;
use std::process::Command;

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
    watch_dir(Path::new("src/support"));
    println!("cargo:rerun-if-changed=tests/fixtures");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let destination = Path::new(&out_dir).join("tests.rs");
    let mut f = std::fs::File::create(&destination).unwrap();
    use std::io::Write;

    let fixtures_dir = Path::new("tests/fixtures");
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
            
            writeln!(f, "fixture_test!({}, {:?}, {});", file_name, path.to_str().unwrap(), expected_exit_code).unwrap();
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    let status = Command::new("dotnet")
        .args(["build", "src/support/support.csproj", "-c", "Debug"])
        .status()
        .expect("failed to run dotnet build");

    if !status.success() {
        panic!("dotnet build failed for support library");
    }
}
