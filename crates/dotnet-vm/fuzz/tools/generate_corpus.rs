#!/usr/bin/env cargo
//! Tool to generate a seed corpus for the CIL bytecode fuzzer from existing test fixtures.
//!
//! Usage:
//!   cargo run --manifest-path crates/dotnet-vm/fuzz/tools/Cargo.toml
//!
//! This extracts method bodies from compiled test fixtures and saves them as binary seeds.
use dotnetdll::prelude::*;
use std::{
    fs, io::Write, path::{Path, PathBuf},
    process::Command,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let workspace_root = find_workspace_root()?;
    let fixtures_dir = workspace_root.join("crates/dotnet-cli/tests/fixtures");
    let corpus_dir = workspace_root.join("crates/dotnet-vm/fuzz/corpus/fuzz_executor");

    // Create corpus directory if it doesn't exist
    fs::create_dir_all(&corpus_dir)?;

    println!("Scanning fixtures in {:?}", fixtures_dir);
    let mut seed_count = 0;

    for entry in walkdir::WalkDir::new(&fixtures_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "cs"))
    {
        let cs_path = entry.path();
        println!("\nProcessing: {}", cs_path.display());

        match build_fixture(cs_path, &workspace_root) {
            Ok(dll_path) => {
                match extract_method_bodies(&dll_path, &corpus_dir, &mut seed_count) {
                    Ok(extracted) => println!("  Extracted {} method bodies", extracted),
                    Err(e) => eprintln!("  Warning: Failed to extract: {}", e),
                }
            }
            Err(e) => eprintln!("  Warning: Failed to build: {}", e),
        }
    }

    println!("\n✓ Generated {} seeds in {:?}", seed_count, corpus_dir);
    Ok(())
}

fn find_workspace_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut current = std::env::current_dir()?;
    loop {
        if current.join("crates/dotnet-cli/tests/fixtures").exists() {
            return Ok(current);
        }
        if current.join("Cargo.toml").exists() {
            let cargo_toml = fs::read_to_string(current.join("Cargo.toml"))?;
            if cargo_toml.contains("[workspace]")
                && current.join("crates/dotnet-vm").exists()
                && current.join("crates/dotnet-value").exists()
            {
                return Ok(current);
            }
        }
        if !current.pop() {
            return Err("Could not find repository workspace root".into());
        }
    }
}

fn build_fixture(cs_path: &Path, workspace_root: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let file_name = cs_path.file_stem().unwrap().to_str().unwrap();
    let output_dir = workspace_root
        .join("target")
        .join("dotnet-fixtures")
        .join(file_name);

    let dll_path = output_dir.join("SingleFile.dll");

    // Check if already built and up-to-date
    if dll_path.exists() {
        let source_mtime = fs::metadata(cs_path)?.modified()?;
        let dll_mtime = fs::metadata(&dll_path)?.modified()?;
        if source_mtime <= dll_mtime {
            return Ok(dll_path);
        }
    }

    let absolute_file = fs::canonicalize(cs_path)?;

    let status = Command::new("dotnet")
        .args([
            "build",
            "tests/SingleFile.csproj",
            "-p:AllowUnsafeBlocks=true",
            &format!("-p:TestFile={}", absolute_file.display()),
            "-o",
            output_dir.to_str().unwrap(),
            &format!("-p:IntermediateOutputPath={}/", output_dir.join("obj").display()),
        ])
        .current_dir(workspace_root.join("crates/dotnet-cli"))
        .status()?;

    if !status.success() {
        return Err(format!("dotnet build failed for {:?}", cs_path).into());
    }

    Ok(dll_path)
}

fn extract_method_bodies(
    dll_path: &Path,
    corpus_dir: &Path,
    seed_count: &mut usize,
) -> Result<usize, Box<dyn std::error::Error>> {
    // Read the DLL file
    let mut file = std::fs::File::open(dll_path)?;
    let mut buf = vec![];
    std::io::Read::read_to_end(&mut file, &mut buf)?;

    // Parse with dotnetdll
    let resolution = Resolution::parse(&buf, ReadOptions::default())?;
    let mut extracted = 0;

    // Iterate through type/method definitions in the resolved model.
    for (type_idx, type_def) in resolution.type_definitions.iter().enumerate() {
        for (method_idx, method) in type_def.methods.iter().enumerate() {
            let Some(body) = &method.body else {
                continue;
            };
            if body.instructions.is_empty() {
                continue;
            }

            let il_bytes = encode_seed_bytes(&body.instructions);
            if il_bytes.is_empty() {
                continue;
            }

            let seed_file = corpus_dir.join(format!(
                "seed_{}_{:03}_{:04}_{}",
                dll_path.file_stem().unwrap().to_str().unwrap(),
                type_idx,
                method_idx,
                sanitize_filename(&method.name)
            ));

            // Write deterministic binary seed bytes for this method.
            let mut file = fs::File::create(&seed_file)?;
            file.write_all(&il_bytes)?;

            extracted += 1;
            *seed_count += 1;
        }
    }

    Ok(extracted)
}

fn encode_seed_bytes(instructions: &[Instruction]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(instructions.len() * 6);
    for instruction in instructions {
        let repr = format!("{instruction:?}");
        bytes.extend_from_slice(repr.as_bytes());
        bytes.push(0x0a);
        // Keep generated seeds in a practical fuzz-friendly range.
        if bytes.len() >= 4096 {
            bytes.truncate(4096);
            break;
        }
    }
    bytes
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => c,
            _ => '_',
        })
        .take(50)
        .collect()
}
