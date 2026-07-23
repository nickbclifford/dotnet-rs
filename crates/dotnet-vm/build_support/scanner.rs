use dotnet_build_tools::collect_files_with_extension;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct SourceScanRoot {
    pub directory: PathBuf,
    pub module_prefix: String,
}

pub fn instruction_source_roots() -> Vec<SourceScanRoot> {
    vec![SourceScanRoot {
        directory: PathBuf::from("src/instructions"),
        module_prefix: "crate::instructions".to_string(),
    }]
}

pub fn intrinsic_source_roots() -> Vec<SourceScanRoot> {
    vec![
        SourceScanRoot {
            directory: PathBuf::from("src/intrinsics"),
            module_prefix: "crate::intrinsics".to_string(),
        },
        SourceScanRoot {
            directory: PathBuf::from("../dotnet-intrinsics-core/src"),
            module_prefix: "dotnet_intrinsics_core".to_string(),
        },
        SourceScanRoot {
            directory: PathBuf::from("../dotnet-intrinsics-delegates/src"),
            module_prefix: "dotnet_intrinsics_delegates".to_string(),
        },
        SourceScanRoot {
            directory: PathBuf::from("../dotnet-intrinsics-span/src"),
            module_prefix: "dotnet_intrinsics_span".to_string(),
        },
        SourceScanRoot {
            directory: PathBuf::from("../dotnet-intrinsics-string/src"),
            module_prefix: "dotnet_intrinsics_string".to_string(),
        },
        SourceScanRoot {
            directory: PathBuf::from("../dotnet-intrinsics-threading/src"),
            module_prefix: "dotnet_intrinsics_threading".to_string(),
        },
        SourceScanRoot {
            directory: PathBuf::from("../dotnet-intrinsics-reflection/src"),
            module_prefix: "dotnet_intrinsics_reflection".to_string(),
        },
        SourceScanRoot {
            directory: PathBuf::from("../dotnet-intrinsics-unsafe/src"),
            module_prefix: "dotnet_intrinsics_unsafe".to_string(),
        },
    ]
}

pub fn scan_rs_files(root: &SourceScanRoot, mut process_file: impl FnMut(&Path)) -> usize {
    let root_path = &root.directory;
    if !root_path.exists() {
        panic!(
            "Configured source root `{}` does not exist",
            root_path.display()
        );
    }
    if !root_path.is_dir() {
        panic!(
            "Configured source root `{}` is not a directory",
            root_path.display()
        );
    }

    let rs_files = collect_files_with_extension(root_path, "rs").unwrap_or_else(|error| {
        panic!(
            "Failed while walking source root `{}`: {error}",
            root_path.display()
        )
    });

    for path in &rs_files {
        process_file(path);
    }

    rs_files.len()
}

pub fn assert_handler_discovery(
    handler_kind: &str,
    rs_files_scanned: usize,
    handlers_discovered: usize,
    roots: &[SourceScanRoot],
    expected_attribute_hint: &str,
) {
    if rs_files_scanned == 0 {
        panic!(
            "No Rust source files were found while scanning {handler_kind} roots: {}",
            format_roots_for_error(roots)
        );
    }
    if handlers_discovered == 0 {
        panic!(
            "No {handler_kind} handlers were discovered while scanning roots: {}. \
Expected to find at least one {expected_attribute_hint} annotation.",
            format_roots_for_error(roots)
        );
    }
}

pub fn emit_rerun_directives(
    instruction_roots: &[SourceScanRoot],
    intrinsic_roots: &[SourceScanRoot],
) {
    for root in instruction_roots.iter().chain(intrinsic_roots) {
        println!("cargo:rerun-if-changed={}", root.directory.display());
    }
}

fn format_roots_for_error(roots: &[SourceScanRoot]) -> String {
    roots
        .iter()
        .map(|root| format!("`{}`", root.directory.display()))
        .collect::<Vec<_>>()
        .join(", ")
}
