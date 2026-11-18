//! Generate corpus files that decode to named `FuzzProgram` values.
//!
//! Usage:
//!   cargo run -p corpus-tools --bin generate_structured_corpus
use arbitrary::{Arbitrary, Unstructured};
use dotnet_vm::fuzzing::SemanticFuzzSeed;
use std::{fs, io, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string()));
    let fuzz_dir = manifest_dir.parent().ok_or("Cannot find fuzz workspace")?;
    let corpus_dir = fuzz_dir.join("corpus/fuzz_executor");
    fs::create_dir_all(&corpus_dir)?;

    for seed in SemanticFuzzSeed::ALL {
        let bytes = seed.encode();
        let mut input = Unstructured::new(&bytes);
        let decoded = dotnet_vm::fuzzing::FuzzProgram::arbitrary(&mut input)?;
        if decoded != seed.program() {
            return Err(format!("semantic seed {seed:?} decoded to an unexpected program").into());
        }
        write_seed(&corpus_dir.join(seed.file_name()), &bytes)?;
    }
    Ok(())
}

fn write_seed(path: &std::path::Path, bytes: &[u8]) -> io::Result<()> {
    if path.exists() {
        let existing = fs::read(path)?;
        if existing == bytes {
            return Ok(());
        }
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "refusing to replace existing corpus input {}",
                path.display()
            ),
        ));
    }
    fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotnet_vm::fuzzing::FuzzProgram;

    #[test]
    fn generated_seed_bytes_decode_to_the_named_program() {
        for seed in SemanticFuzzSeed::ALL {
            let bytes = seed.encode();
            let mut input = Unstructured::new(&bytes);
            let decoded = FuzzProgram::arbitrary(&mut input).expect("valid semantic seed");
            assert_eq!(decoded, seed.program(), "{seed:?}");
        }
    }
}
