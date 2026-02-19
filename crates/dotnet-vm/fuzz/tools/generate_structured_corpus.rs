#!/usr/bin/env cargo
//! Tool to generate structured seed corpus for the CIL bytecode fuzzer.
//!
//! Usage:
//!   cargo run --bin generate_structured_corpus --manifest-path crates/dotnet-vm/fuzz/Cargo.toml
//!
//! This creates hand-crafted FuzzProgram instances that exercise interesting code paths.
use std::{fs, io::Write, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The corpus directory is relative to the fuzz workspace root
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .unwrap_or_else(|_| ".".to_string());
    let fuzz_dir = std::path::Path::new(&manifest_dir).parent().ok_or("Cannot find parent directory")?;
    let corpus_dir = fuzz_dir.join("corpus/fuzz_executor");

    // Create corpus directory if it doesn't exist
    fs::create_dir_all(&corpus_dir)?;

    println!("Generating structured seeds in {:?}", corpus_dir);

    // Generate various interesting seed patterns
    generate_arithmetic_seeds(&corpus_dir)?;
    generate_memory_seeds(&corpus_dir)?;
    generate_control_flow_seeds(&corpus_dir)?;
    generate_stack_seeds(&corpus_dir)?;

    println!("\n✓ Generated structured seeds");
    Ok(())
}


fn generate_arithmetic_seeds(corpus_dir: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    // Simple arithmetic: push two values, add, return
    // In raw bytes this would be: ldc.i4 42, ldc.i4 1, add, ret
    let seed = vec![
        0x20, 0x2A, 0x00, 0x00, 0x00,  // ldc.i4 42
        0x16,                           // ldc.i4.0 (we'll use shortcuts)
        0x58,                           // add
        0x2A,                           // ret
    ];
    write_seed(corpus_dir, "arithmetic_add", &seed)?;

    // Division by zero test
    let seed = vec![
        0x20, 0x0A, 0x00, 0x00, 0x00,  // ldc.i4 10
        0x16,                           // ldc.i4.0
        0x5B,                           // div
        0x2A,                           // ret
    ];
    write_seed(corpus_dir, "arithmetic_div_zero", &seed)?;

    // Overflow test
    let seed = vec![
        0x21, 0xFF, 0xFF, 0xFF, 0x7F,  // ldc.i4 0x7FFFFFFF (Int32.MaxValue)
        0x17,                           // ldc.i4.1
        0x58,                           // add
        0x2A,                           // ret
    ];
    write_seed(corpus_dir, "arithmetic_overflow", &seed)?;

    Ok(())
}

fn generate_memory_seeds(corpus_dir: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    // localloc test: allocate 100 bytes on stack
    let seed = vec![
        0x20, 0x64, 0x00, 0x00, 0x00,  // ldc.i4 100
        0xFE, 0x0F,                     // localloc
        0x26,                           // pop
        0x2A,                           // ret
    ];
    write_seed(corpus_dir, "memory_localloc", &seed)?;

    // Null pointer dereference test
    let seed = vec![
        0x14,                           // ldnull
        0x4A,                           // ldind.i4 (should throw NullReferenceException)
        0x2A,                           // ret
    ];
    write_seed(corpus_dir, "memory_null_deref", &seed)?;

    // cpblk with null pointers
    let seed = vec![
        0x14,                           // ldnull (dest)
        0x14,                           // ldnull (src)
        0x20, 0x0A, 0x00, 0x00, 0x00,  // ldc.i4 10 (size)
        0xFE, 0x17,                     // cpblk
        0x2A,                           // ret
    ];
    write_seed(corpus_dir, "memory_cpblk_null", &seed)?;

    Ok(())
}

fn generate_control_flow_seeds(corpus_dir: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    // Simple branch
    let seed = vec![
        0x17,                           // ldc.i4.1
        0x2C, 0x02,                     // brtrue 2 (skip next instruction)
        0x26,                           // pop (skipped)
        0x2A,                           // ret
    ];
    write_seed(corpus_dir, "control_branch", &seed)?;

    // Infinite loop (will hit instruction budget)
    let seed = vec![
        0x2B, 0xFE,                     // br -2 (jump to self)
        0x2A,                           // ret (unreachable)
    ];
    write_seed(corpus_dir, "control_infinite_loop", &seed)?;

    Ok(())
}

fn generate_stack_seeds(corpus_dir: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    // Deep stack: push many values
    let mut seed = Vec::new();
    for _ in 0..50 {
        seed.push(0x16);  // ldc.i4.0
    }
    seed.push(0x2A);  // ret
    write_seed(corpus_dir, "stack_deep", &seed)?;

    // Stack underflow
    let seed = vec![
        0x26,  // pop (with empty stack)
        0x2A,  // ret
    ];
    write_seed(corpus_dir, "stack_underflow", &seed)?;

    // dup test
    let seed = vec![
        0x20, 0x2A, 0x00, 0x00, 0x00,  // ldc.i4 42
        0x25,                           // dup
        0x58,                           // add (42 + 42 = 84)
        0x2A,                           // ret
    ];
    write_seed(corpus_dir, "stack_dup", &seed)?;

    Ok(())
}

fn write_seed(corpus_dir: &PathBuf, name: &str, data: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let path = corpus_dir.join(format!("seed_{}", name));
    let mut file = fs::File::create(&path)?;
    file.write_all(data)?;
    println!("  Generated: {} ({} bytes)", name, data.len());
    Ok(())
}
