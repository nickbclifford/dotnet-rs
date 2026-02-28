use std::{fs::File, io::Write, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=tests/fixtures");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let destination = Path::new(&out_dir).join("tests.rs");
    let mut f = File::create(&destination).unwrap();

    let fixtures_dir = Path::new("tests/fixtures");
    let mut fixtures = Vec::new();
    find_fixtures(fixtures_dir, &mut fixtures);
    fixtures.sort_by_key(|p| p.to_string_lossy().to_string());

    for path in fixtures {
        let file_name = path.file_stem().unwrap().to_str().unwrap();
        let expected_exit_code: u8 = file_name
            .split('_')
            .next_back()
            .unwrap()
            .parse()
            .expect("fixture file name must end with _<exit_code>.cs");

        let mut ignore_prefix = "".to_string();
        if file_name == "bench_loop_42" {
            ignore_prefix = "#[ignore] ".to_string();
        }

        writeln!(
            f,
            "fixture_test!({}{}, {:?}, {});",
            ignore_prefix,
            file_name,
            path.to_str().unwrap(),
            expected_exit_code
        )
        .unwrap();
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn find_fixtures(dir: &Path, fixtures: &mut Vec<std::path::PathBuf>) {
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
