#[cfg(not(feature = "fuzzing"))]
#[test]
fn expressions_expression_compile_42_matches_dotnet() {
    use crate::integration_tests_impl::harness::TestHarness;
    use std::{path::Path, process::Command};

    let harness = TestHarness::get();
    let dll_path = harness.ensure_dll(Path::new(
        "tests/fixtures/expressions/expression_compile_42.cs",
    ));

    let dotnet_output = Command::new("dotnet")
        .arg(&dll_path)
        .output()
        .expect("failed to run fixture with dotnet");
    let dotnet_exit_code = dotnet_output.status.code().unwrap_or(255) as u8;
    let dotnet_stdout = String::from_utf8_lossy(&dotnet_output.stdout).to_string();

    let (dotnet_rs_exit_code, dotnet_rs_stdout) = harness.run_cli(&dll_path);

    assert_eq!(
        dotnet_exit_code, 42,
        "sanity check failed: dotnet should return fixture exit code 42"
    );
    assert_eq!(
        dotnet_rs_exit_code, dotnet_exit_code,
        "dotnet-rs exit code diverged from dotnet"
    );
    assert_eq!(
        dotnet_rs_stdout, dotnet_stdout,
        "dotnet-rs stdout diverged from dotnet"
    );
}
