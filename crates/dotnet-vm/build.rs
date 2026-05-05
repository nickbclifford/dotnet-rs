#[path = "build_support/mod.rs"]
mod build_support;

use build_support::{
    codegen::{generate_instruction_table, generate_intrinsic_phf},
    parser::{
        assert_unique_instruction_variants, process_instruction_file, process_intrinsic_file,
    },
    scanner::{
        assert_handler_discovery, emit_rerun_directives, instruction_source_roots,
        intrinsic_source_roots, scan_rs_files,
    },
};
use std::env;

fn main() {
    let out_dir = env::var_os("OUT_DIR").unwrap();

    let instruction_roots = instruction_source_roots();
    let mut instruction_entries = Vec::new();
    let mut instruction_rs_files_scanned = 0usize;
    for root in &instruction_roots {
        instruction_rs_files_scanned += scan_rs_files(root, |path| {
            process_instruction_file(path, root, &mut instruction_entries)
        });
    }
    assert_handler_discovery(
        "instruction",
        instruction_rs_files_scanned,
        instruction_entries.len(),
        &instruction_roots,
        "#[dotnet_instruction(...)]",
    );
    assert_unique_instruction_variants(&instruction_entries);
    instruction_entries.sort_by(|left, right| left.variant_name.cmp(&right.variant_name));
    generate_instruction_table(&out_dir, &instruction_entries);

    let intrinsic_roots = intrinsic_source_roots();
    let mut intrinsic_entries = Vec::new();
    let mut intrinsic_rs_files_scanned = 0usize;
    for root in &intrinsic_roots {
        intrinsic_rs_files_scanned += scan_rs_files(root, |path| {
            process_intrinsic_file(path, root, &mut intrinsic_entries)
        });
    }
    assert_handler_discovery(
        "intrinsic",
        intrinsic_rs_files_scanned,
        intrinsic_entries.len(),
        &intrinsic_roots,
        "#[dotnet_intrinsic(...)] / #[dotnet_intrinsic_field(...)]",
    );
    generate_intrinsic_phf(&out_dir, &intrinsic_entries);

    emit_rerun_directives(&instruction_roots, &intrinsic_roots);
}
