use dotnet_build_tools::{
    collect_files_with_extension, find_repo_root, fixture_cache_hash, fixture_output_base,
    hash_value_u64, shared_msbuild_input_candidates,
};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const DEFAULT_TARGET_DIR: &str = "target";
const DEFAULT_PROFILE: &str = "debug";
const SHARED_FEATURE_MATRIX: &[&str] = &[
    "",
    "multithreading",
    "simd",
    "multithreading,simd",
    "generic-constraint-validation",
    "memory-validation",
    "metadata-validation",
    "multithreading,memory-validation",
    "multithreading,validation-all",
    "fuzzing",
];
const CLIPPY_FEATURE_MATRIX: &[&str] = SHARED_FEATURE_MATRIX;
const TEST_FEATURE_MATRIX: &[&str] = SHARED_FEATURE_MATRIX;

#[derive(Debug, Clone, Copy)]
enum MatrixOutputFormat {
    Lines,
    Json,
}

#[derive(Debug, Clone)]
struct FixturesOptions {
    output_dir: Option<PathBuf>,
    target_dir: PathBuf,
    profile: String,
    target: Option<String>,
}

fn main() {
    if let Err(error) = run(env::args().skip(1).collect()) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    match args.as_slice() {
        [] => {
            print_help();
            Ok(())
        }
        [arg] if arg == "-h" || arg == "--help" => {
            print_help();
            Ok(())
        }
        [group, action, rest @ ..] if group == "fixtures" && action == "build" => {
            fixtures_build(rest)
        }
        [group, action, rest @ ..] if group == "fixtures" && action == "output-dir" => {
            fixtures_output_dir(rest)
        }
        [group, action, rest @ ..] if group == "fixtures" && action == "cache-key" => {
            fixtures_cache_key(rest)
        }
        [group, action, rest @ ..] if group == "matrix" && action == "clippy-features" => {
            matrix_features(rest, CLIPPY_FEATURE_MATRIX)
        }
        [group, action, rest @ ..] if group == "matrix" && action == "test-features" => {
            matrix_features(rest, TEST_FEATURE_MATRIX)
        }
        _ => Err(format!(
            "unknown command: `{}`\n\n{}",
            args.join(" "),
            help_text()
        )),
    }
}

fn fixtures_build(args: &[String]) -> Result<(), String> {
    if has_help(args) {
        print_help();
        return Ok(());
    }

    let repo_root = repo_root()?;
    let options = parse_fixtures_options(args)?;
    let resolved_output_dir = resolve_fixture_output_dir(&repo_root, &options)?;
    fs::create_dir_all(&resolved_output_dir)
        .map_err(|error| format!("failed to create output dir: {error}"))?;

    let output_abs = resolved_output_dir
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize output dir: {error}"))?;

    let msbuild_obj = output_abs.join("msbuild-obj");
    let msbuild_bin = output_abs.join("msbuild-bin");

    println!("Building .NET fixtures...");
    println!(
        "  Output dir: {}",
        display_repo_relative_path(&repo_root, &output_abs)
    );

    run_dotnet(
        &repo_root,
        [
            "restore".to_string(),
            "crates/dotnet-cli/tests/SingleFile.csproj".to_string(),
            format!("-p:BaseIntermediateOutputPath={}/", msbuild_obj.display()),
            format!("-p:BaseOutputPath={}/", msbuild_bin.display()),
            "--nologo".to_string(),
            "-v:q".to_string(),
        ],
    )?;

    run_dotnet(
        &repo_root,
        [
            "build".to_string(),
            "crates/dotnet-cli/tests/BatchFixtures.csproj".to_string(),
            format!("-p:FixtureOutputBase={}/", output_abs.display()),
            format!("-p:BaseIntermediateOutputPath={}/", msbuild_obj.display()),
            format!("-p:BaseOutputPath={}/", msbuild_bin.display()),
            "-m".to_string(),
            "-v:q".to_string(),
            "--nologo".to_string(),
            "-clp:ErrorsOnly".to_string(),
            "--no-restore".to_string(),
        ],
    )?;

    let fixture_hash = compute_fixture_content_hash(&repo_root)?;
    let hash_file = output_abs.join(".fixtures_hash");
    fs::write(&hash_file, fixture_hash.to_string())
        .map_err(|error| format!("failed to write fixture hash file: {error}"))?;

    println!("Fixtures built successfully.");
    println!("To use these fixtures in tests, run:");
    println!("  DOTNET_USE_PREBUILT_FIXTURES=1 cargo test");
    Ok(())
}

fn fixtures_output_dir(args: &[String]) -> Result<(), String> {
    if has_help(args) {
        print_help();
        return Ok(());
    }

    let repo_root = repo_root()?;
    let options = parse_fixtures_options(args)?;
    let output_dir = resolve_fixture_output_dir(&repo_root, &options)?;
    println!("{}", display_repo_relative_path(&repo_root, &output_dir));
    Ok(())
}

fn fixtures_cache_key(args: &[String]) -> Result<(), String> {
    if has_help(args) {
        print_help();
        return Ok(());
    }

    let repo_root = repo_root()?;
    let options = parse_fixtures_options(args)?;
    let content_hash = compute_fixture_content_hash(&repo_root)?;
    let output_path = resolve_fixture_output_dir(&repo_root, &options)?;
    let key_input = format!(
        "{}:{}",
        content_hash,
        display_repo_relative_path(&repo_root, &output_path)
    );
    let key = hash_value_u64(&key_input);
    println!("{key:016x}");
    Ok(())
}

fn matrix_features(args: &[String], features: &[&str]) -> Result<(), String> {
    if has_help(args) {
        print_help();
        return Ok(());
    }
    let format = parse_matrix_output_format(args)?;
    print_features(features, format);
    Ok(())
}

fn parse_matrix_output_format(args: &[String]) -> Result<MatrixOutputFormat, String> {
    let mut idx = 0usize;
    let mut format = MatrixOutputFormat::Lines;
    while idx < args.len() {
        match args[idx].as_str() {
            "--format" => {
                let Some(value) = args.get(idx + 1) else {
                    return Err("--format requires one of: lines, json".to_string());
                };
                format = match value.as_str() {
                    "lines" => MatrixOutputFormat::Lines,
                    "json" => MatrixOutputFormat::Json,
                    _ => return Err(format!("unsupported --format value: `{value}`")),
                };
                idx += 2;
            }
            other => return Err(format!("unknown matrix option: `{other}`")),
        }
    }
    Ok(format)
}

fn print_features(features: &[&str], format: MatrixOutputFormat) {
    match format {
        MatrixOutputFormat::Lines => {
            for feature in features {
                println!("{feature}");
            }
        }
        MatrixOutputFormat::Json => {
            let quoted = features
                .iter()
                .map(|feature| format!("\"{}\"", json_escape(feature)))
                .collect::<Vec<_>>()
                .join(",");
            println!("[{quoted}]");
        }
    }
}

fn json_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn run_dotnet<I>(repo_root: &Path, args: I) -> Result<(), String>
where
    I: IntoIterator<Item = String>,
{
    let status = Command::new("dotnet")
        .args(args)
        .current_dir(repo_root)
        .status()
        .map_err(|error| format!("failed to run dotnet command: {error}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("dotnet command failed with status: {status}"))
    }
}

fn repo_root() -> Result<PathBuf, String> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "failed to determine repository root from xtask manifest".to_string())?;
    Ok(find_repo_root(&root))
}

fn has_help(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "-h" || arg == "--help")
}

fn parse_fixtures_options(args: &[String]) -> Result<FixturesOptions, String> {
    let mut output_dir = None;
    let mut target_dir = PathBuf::from(DEFAULT_TARGET_DIR);
    let mut profile = DEFAULT_PROFILE.to_string();
    let mut target = None;
    let mut idx = 0usize;

    while idx < args.len() {
        match args[idx].as_str() {
            "--output-dir" => {
                let Some(value) = args.get(idx + 1) else {
                    return Err("--output-dir requires a path".to_string());
                };
                output_dir = Some(PathBuf::from(value));
                idx += 2;
            }
            "--target-dir" => {
                let Some(value) = args.get(idx + 1) else {
                    return Err("--target-dir requires a path".to_string());
                };
                target_dir = PathBuf::from(value);
                idx += 2;
            }
            "--profile" => {
                let Some(value) = args.get(idx + 1) else {
                    return Err("--profile requires a value".to_string());
                };
                profile = value.clone();
                idx += 2;
            }
            "--target" => {
                let Some(value) = args.get(idx + 1) else {
                    return Err("--target requires a target triple".to_string());
                };
                target = Some(value.clone());
                idx += 2;
            }
            other => return Err(format!("unknown fixtures option: `{other}`")),
        }
    }

    Ok(FixturesOptions {
        output_dir,
        target_dir,
        profile,
        target,
    })
}

fn resolve_fixture_output_dir(
    repo_root: &Path,
    options: &FixturesOptions,
) -> Result<PathBuf, String> {
    if let Some(output_dir) = &options.output_dir {
        if options.target.is_some()
            || options.target_dir != Path::new(DEFAULT_TARGET_DIR)
            || options.profile != DEFAULT_PROFILE
        {
            return Err(
                "--output-dir cannot be combined with --target-dir, --profile, or --target"
                    .to_string(),
            );
        }
        return Ok(resolve_repo_relative_path(repo_root, output_dir));
    }

    let target_dir = resolve_repo_relative_path(repo_root, &options.target_dir);
    Ok(fixture_output_base(
        &target_dir,
        &options.profile,
        options.target.as_deref(),
    ))
}

fn resolve_repo_relative_path(repo_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    }
}

fn display_repo_relative_path(repo_root: &Path, path: &Path) -> String {
    path.strip_prefix(repo_root).map_or_else(
        |_| path.to_string_lossy().to_string(),
        |relative| relative.to_string_lossy().to_string(),
    )
}

fn compute_fixture_content_hash(repo_root: &Path) -> Result<u64, String> {
    let tests_dir = repo_root.join("crates/dotnet-cli/tests");
    let fixtures_dir = tests_dir.join("fixtures");
    let shared_inputs = shared_msbuild_input_candidates(&tests_dir, repo_root);
    let mut fixtures = collect_files_with_extension(&fixtures_dir, "cs")
        .map_err(|error| format!("failed to collect fixture files: {error}"))?;
    fixtures.sort_by_key(|path| path.to_string_lossy().to_string());

    fixture_cache_hash(&fixtures, &tests_dir, &shared_inputs)
        .map_err(|error| format!("failed to compute fixture cache hash: {error}"))
}

fn help_text() -> &'static str {
    "Usage:
  cargo run -p xtask -- fixtures build [options]
  cargo run -p xtask -- fixtures output-dir [options]
  cargo run -p xtask -- fixtures cache-key [options]
  cargo run -p xtask -- matrix clippy-features [--format lines|json]
  cargo run -p xtask -- matrix test-features [--format lines|json]

Commands:
  fixtures build       Build .NET test fixtures used by dotnet-cli integration tests.
  fixtures output-dir  Print resolved fixture output directory.
  fixtures cache-key   Print fixture cache key from build.rs-aligned inputs.
  matrix clippy-features  Print clippy feature matrix.
  matrix test-features    Print test feature matrix.

Options:
  --output-dir <path>  Explicit fixture output dir (cannot be combined with layout flags)
  --target-dir <path>  Cargo target dir for convention-based output path (default: target)
  --profile <name>     Cargo profile for convention-based output path (default: debug)
  --target <triple>    Cargo target triple for convention-based output path
  --format <format>    For matrix commands: `lines` (default) or `json`"
}

fn print_help() {
    println!("{}", help_text());
}

#[cfg(test)]
mod tests {
    use super::{
        FixturesOptions, MatrixOutputFormat, display_repo_relative_path, parse_fixtures_options,
        parse_matrix_output_format, resolve_fixture_output_dir,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn resolves_default_output_dir_using_convention() {
        let root = Path::new("/repo");
        let options = FixturesOptions {
            output_dir: None,
            target_dir: PathBuf::from("target"),
            profile: "debug".to_string(),
            target: None,
        };
        assert_eq!(
            resolve_fixture_output_dir(root, &options).unwrap(),
            PathBuf::from("/repo/target/debug/dotnet-fixtures")
        );
    }

    #[test]
    fn resolves_target_dir_profile_and_triple() {
        let root = Path::new("/repo");
        let options = FixturesOptions {
            output_dir: None,
            target_dir: PathBuf::from("/tmp/dnr-target"),
            profile: "bench-fat".to_string(),
            target: Some("x86_64-unknown-linux-gnu".to_string()),
        };
        assert_eq!(
            resolve_fixture_output_dir(root, &options).unwrap(),
            PathBuf::from("/tmp/dnr-target/x86_64-unknown-linux-gnu/bench-fat/dotnet-fixtures")
        );
    }

    #[test]
    fn preserves_absolute_output_dir_override() {
        let root = Path::new("/repo");
        let options = FixturesOptions {
            output_dir: Some(PathBuf::from("/tmp/fixtures")),
            target_dir: PathBuf::from("target"),
            profile: "debug".to_string(),
            target: None,
        };
        assert_eq!(
            resolve_fixture_output_dir(root, &options).unwrap(),
            PathBuf::from("/tmp/fixtures")
        );
    }

    #[test]
    fn rejects_mixed_override_and_layout_flags() {
        let root = Path::new("/repo");
        let options = FixturesOptions {
            output_dir: Some(PathBuf::from("/tmp/fixtures")),
            target_dir: PathBuf::from("/tmp/target"),
            profile: "debug".to_string(),
            target: None,
        };
        assert!(resolve_fixture_output_dir(root, &options).is_err());
    }

    #[test]
    fn parser_reads_layout_flags() {
        let args = vec![
            "--target-dir".to_string(),
            "/tmp/target".to_string(),
            "--profile".to_string(),
            "bench-fat".to_string(),
            "--target".to_string(),
            "x86_64-unknown-linux-gnu".to_string(),
        ];
        let options = parse_fixtures_options(&args).unwrap();
        assert_eq!(options.target_dir, PathBuf::from("/tmp/target"));
        assert_eq!(options.profile, "bench-fat");
        assert_eq!(options.target, Some("x86_64-unknown-linux-gnu".to_string()));
    }

    #[test]
    fn displays_relative_path_under_repo_root() {
        let root = Path::new("/repo");
        let path = Path::new("/repo/target/debug/dotnet-fixtures");
        assert_eq!(
            display_repo_relative_path(root, path),
            "target/debug/dotnet-fixtures"
        );
    }

    #[test]
    fn matrix_format_defaults_to_lines() {
        assert!(matches!(
            parse_matrix_output_format(&[]).unwrap(),
            MatrixOutputFormat::Lines
        ));
    }

    #[test]
    fn matrix_format_accepts_json() {
        let args = vec!["--format".to_string(), "json".to_string()];
        assert!(matches!(
            parse_matrix_output_format(&args).unwrap(),
            MatrixOutputFormat::Json
        ));
    }

    #[test]
    fn matrix_format_rejects_unknown_value() {
        let args = vec!["--format".to_string(), "yaml".to_string()];
        assert!(parse_matrix_output_format(&args).is_err());
    }
}
