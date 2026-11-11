use dotnet_build_tools::{
    cargo_profile_target_dir_from_out_dir, collect_files_with_extension, find_repo_root,
    fixture_cache_hash, shared_msbuild_input_candidates, should_skip_dotnet_build,
};
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

/// Returns true when we should use prebuilt fixtures from `target/<profile>/dotnet-fixtures`.
fn use_prebuilt_fixtures() -> bool {
    std::env::var("DOTNET_USE_PREBUILT_FIXTURES").is_ok_and(|v| v == "1")
}

fn main() {
    println!("cargo:rerun-if-changed=tests/SingleFile.csproj");
    println!("cargo:rerun-if-changed=tests/BatchFixtures.csproj");
    // Ensure adding/removing fixtures triggers regeneration of `tests.rs`.
    println!("cargo:rerun-if-changed=tests/fixtures");
    println!("cargo:rerun-if-env-changed=DOTNET_SKIP_BUILD");
    println!("cargo:rerun-if-env-changed=DOTNET_USE_PREBUILT_FIXTURES");
    println!("cargo:rerun-if-env-changed=DOTNET_FIXTURES_BASE");
    println!("cargo:rerun-if-env-changed=DOTNET_TEST_FILTER");
    println!("cargo:rerun-if-env-changed=RUSTC_WORKSPACE_WRAPPER");
    println!("cargo:rerun-if-env-changed=RUSTC_WRAPPER");
    println!("cargo:rerun-if-env-changed=CLIPPY_ARGS");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let out_dir_path = PathBuf::from(&out_dir);
    let destination = out_dir_path.join("tests.rs");
    let mut f = File::create(&destination).unwrap();

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tests_dir = manifest_dir.join("tests");
    let fixtures_dir = tests_dir.join("fixtures");
    let repo_root = find_repo_root(manifest_dir);
    let shared_msbuild_inputs = shared_msbuild_input_candidates(&tests_dir, &repo_root);
    for input in &shared_msbuild_inputs {
        // Emit these even when missing so introducing one later retriggers build.rs.
        println!("cargo:rerun-if-changed={}", input.display());
    }

    let output_base = std::env::var("DOTNET_FIXTURES_BASE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let cargo_profile_target_dir = cargo_profile_target_dir_from_out_dir(&out_dir_path);
            cargo_profile_target_dir.join("dotnet-fixtures")
        });

    println!(
        "cargo:rustc-env=DOTNET_FIXTURES_BASE={}",
        output_base.display()
    );

    let mut fixtures = collect_files_with_extension(&fixtures_dir, "cs").unwrap_or_else(|error| {
        panic!(
            "Failed to recursively scan fixture files under `{}`: {error}",
            fixtures_dir.display()
        )
    });
    fixtures.sort_by_key(|p| p.to_string_lossy().to_string());

    if let Ok(filter) = std::env::var("DOTNET_TEST_FILTER")
        && !filter.is_empty()
    {
        fixtures.retain(|p| p.to_string_lossy().contains(&filter));
    }

    let hash = fixture_cache_hash(&fixtures, &tests_dir, &shared_msbuild_inputs)
        .unwrap_or_else(|error| panic!("Failed to compute fixtures hash: {error}"));
    let hash_file = output_base.join(".fixtures_hash");

    if use_prebuilt_fixtures() {
        // Validate prebuilt fixtures exist and directory is not empty.
        if !output_base.exists() {
            println!(
                "cargo:warning=DOTNET_USE_PREBUILT_FIXTURES=1 but {} does not exist",
                output_base.display()
            );
            panic!("Prebuilt fixtures directory missing");
        }

        let is_empty = std::fs::read_dir(&output_base)
            .map(|mut d| d.next().is_none())
            .unwrap_or(true);

        if is_empty {
            println!(
                "cargo:warning=DOTNET_USE_PREBUILT_FIXTURES=1 but {} is empty",
                output_base.display()
            );
            panic!("Prebuilt fixtures directory empty");
        }

        let prebuilt_hash = std::fs::read_to_string(&hash_file)
            .ok()
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .unwrap_or_else(|| {
                println!(
                    "cargo:warning=DOTNET_USE_PREBUILT_FIXTURES=1 but {} is missing/invalid",
                    hash_file.display()
                );
                panic!(
                    "Stale/incomplete artifact: missing or invalid prebuilt hash file. Run scripts/build_fixtures.sh to update."
                );
            });

        if prebuilt_hash != hash {
            println!(
                "cargo:warning=Prebuilt fixtures hash mismatch at {} (expected {}, found {})",
                hash_file.display(),
                hash,
                prebuilt_hash
            );
            panic!(
                "Stale/incomplete artifact: prebuilt fixtures hash mismatch. Run scripts/build_fixtures.sh to update."
            );
        }
    } else {
        let previous_hash: Option<u64> = std::fs::read_to_string(&hash_file)
            .ok()
            .and_then(|s| s.trim().parse().ok());

        let needs_dotnet_build = previous_hash != Some(hash);

        if !should_skip_dotnet_build() && needs_dotnet_build {
            // Ensure output_base exists before we save the hash later
            std::fs::create_dir_all(&output_base).unwrap();

            let msbuild_obj = output_base.join("msbuild-obj");
            let msbuild_bin = output_base.join("msbuild-bin");

            // Restore packages once for SingleFile.csproj.
            let restore_status = Command::new("dotnet")
                .args(["restore", "SingleFile.csproj", "-v:q", "--nologo"])
                .arg(format!(
                    "-p:BaseIntermediateOutputPath={}/",
                    msbuild_obj.display()
                ))
                .arg(format!("-p:BaseOutputPath={}/", msbuild_bin.display()))
                .current_dir(&tests_dir)
                .status()
                .expect("Failed to run dotnet restore");

            assert!(restore_status.success(), "dotnet restore failed");

            // Batch compile all fixtures (restore already done).
            let status = Command::new("dotnet")
                .args([
                    "build",
                    "BatchFixtures.csproj",
                    "-m",              // parallel MSBuild nodes
                    "-v:q",            // quiet verbosity
                    "--nologo",        // suppress banner
                    "-clp:ErrorsOnly", // only show errors
                    "--no-restore",
                ])
                .arg(format!("-p:FixtureOutputBase={}/", output_base.display()))
                .arg(format!(
                    "-p:BaseIntermediateOutputPath={}/",
                    msbuild_obj.display()
                ))
                .arg(format!("-p:BaseOutputPath={}/", msbuild_bin.display()))
                .current_dir(&tests_dir)
                .status()
                .expect("Failed to run dotnet build. Is the .NET SDK installed?");

            assert!(
                status.success(),
                "dotnet build BatchFixtures.csproj failed. Check that the .NET 10 SDK is installed."
            );

            // Save hash on success
            std::fs::write(&hash_file, hash.to_string()).unwrap();
        }
    }

    let mut missing_prebuilt_dlls = Vec::new();
    for path in fixtures {
        let file_name = path.file_stem().unwrap().to_str().unwrap();
        let expected_exit_code: u8 = file_name
            .split('_')
            .next_back()
            .unwrap()
            .parse()
            .expect("fixture file name must end with _<exit_code>.cs");

        let relative_path = path.strip_prefix(&fixtures_dir).unwrap();
        let test_name = relative_path
            .with_extension("")
            .to_str()
            .unwrap()
            .replace("/", "_")
            .replace("\\", "_");

        let dll_path = output_base
            .join(relative_path.parent().unwrap())
            .join(file_name)
            .join("SingleFile.dll");

        let mut ignore_prefix = "".to_string();
        let is_multithreading_enabled = std::env::var("CARGO_FEATURE_MULTITHREADING").is_ok();
        let is_generic_constraint_validation_enabled =
            std::env::var("CARGO_FEATURE_GENERIC_CONSTRAINT_VALIDATION").is_ok();
        let is_fuzzing_enabled = std::env::var("CARGO_FEATURE_FUZZING").is_ok();
        let is_pinvoke_fixture = relative_path
            .components()
            .next()
            .and_then(|c| c.as_os_str().to_str())
            .is_some_and(|c| c == "pinvoke");
        let uses_console_in_fuzzing = is_fuzzing_enabled
            && (test_name.starts_with("delegates_delegate_multicast_")
                || test_name.starts_with("exceptions_nested_filter_")
                || test_name == "statics_cctor_filter_42"
                || matches!(
                    test_name.as_str(),
                    "strings_substring_fallback_42"
                        | "strings_substring_chunks_42"
                        | "gc_weak_reference_liveness_42"
                ))
            && std::fs::read_to_string(&path)
                .expect("fixture source must be readable")
                .contains("Console.");

        if !dll_path.exists() {
            if use_prebuilt_fixtures() {
                missing_prebuilt_dlls.push(dll_path.display().to_string());
            }
            ignore_prefix = "#[ignore] ".to_string();
        } else if is_fuzzing_enabled
            && (is_pinvoke_fixture
                || uses_console_in_fuzzing
                || test_name == "memory_nullable_boxing_42")
        {
            // Fuzzing mode hardens native interop paths by denying P/Invoke, so
            // fixtures that rely on libc/libm entry points (and console code paths
            // that transitively use them, e.g. Console.WriteLine -> SystemNative_SetErrNo)
            // are expected to fail.
            ignore_prefix = "#[ignore] ".to_string();
        } else if (file_name == "monitor_try_enter_timeout_42"
            || file_name == "monitor_is_entered_42"
            || file_name == "synchronized_method_42"
            || file_name == "canonical_type_objects_42"
            || file_name == "canonical_member_objects_42"
            || file_name == "monitor_contended_lock_42"
            || file_name == "delegate_monitor_enter_42"
            || file_name == "monitor_static_flags_42"
            || file_name == "monitor_fresh_object_race_42"
            || file_name == "foreground_worker_42"
            || file_name == "thread_sleep_gc_42"
            || file_name == "thread_background_42"
            || file_name == "monitor_wait_threads_42"
            || file_name == "event_workers_42"
            || file_name == "async_pool_run_result_42"
            || file_name == "async_pool_once_42"
            || file_name == "async_pool_continue_with_42"
            || file_name == "async_pool_yield_main_42"
            || file_name == "async_pool_when_all_42"
            || file_name == "async_pool_fault_42"
            || file_name == "async_pool_asynclocal_task_run_42"
            || file_name == "async_pool_asynclocal_yield_42"
            || file_name == "threading_pool_queue_42"
            || file_name == "threading_pool_asynclocal_isolation_42"
            || file_name == "threading_pool_asynclocal_safe_42"
            || file_name == "threading_pool_asynclocal_unsafe_42"
            || file_name == "threading_pool_current_task_42"
            || file_name == "threading_thread_static_42"
            || file_name == "threading_pool_mres_42"
            || file_name == "threading_pool_counts_gc_42"
            || file_name == "async_blocking_worker_tcs_42"
            || file_name == "async_incomplete_wait_timeout_42"
            || file_name == "async_task_delay_cancel_after_42"
            || file_name == "async_task_delay_elapsed_42"
            || file_name == "async_task_when_any_timeout_42"
            || file_name == "threading_timer_callback_pool_thread_42"
            || file_name == "threading_timer_idle_background_exit_42"
            || file_name == "thread_identity_start_42"
            || file_name == "thread_cross_start_42"
            || file_name == "thread_retired_creator_42"
            || file_name == "retired_arena_cycles_42"
            || file_name == "circular_init_mt_42"
            || file_name == "cctor_wait_success_42"
            || file_name == "cctor_wait_failure_42")
            && (!is_multithreading_enabled || (is_fuzzing_enabled && matches!(file_name, "foreground_worker_42" | "thread_sleep_gc_42" | "thread_background_42" | "monitor_wait_threads_42" | "thread_identity_start_42" | "thread_cross_start_42" | "thread_retired_creator_42" | "retired_arena_cycles_42")))
            // generic_constraints_fail_0 deliberately tests constraint-violation rejection;
            // that logic only exists under the generic-constraint-validation feature.
            || (file_name == "generic_constraints_fail_0"
                && !is_generic_constraint_validation_enabled)
        {
            ignore_prefix = "#[ignore] ".to_string();
        }

        let multi_arena_threads = match file_name {
            // This fixture is inherently multi-threaded: threads are assigned roles via
            // Interlocked.Increment, so a single-executor fixture_test! would hang forever
            // (Thread 0 busy-waits for Thread 1 which never runs). Use 3 executors.
            "monitor_try_enter_timeout_42" => Some(3),
            // This fixture intentionally deadlocks two type initializers unless both roles
            // run concurrently. Use 2 executors to ensure each role gets a worker thread.
            "circular_init_mt_42" => Some(2),
            _ => None,
        };

        // These fixtures use custom registrations to control thread ownership or feature gating.
        // Keep them in the fixture list for prebuilt validation.
        let is_manually_registered = matches!(
            file_name,
            "pinvoke_last_error_isolation_42"
                | "invoke_byref_foreign_42"
                | "invoke_argument_race_42"
                | "async_task_delay_elapsed_42"
        );

        let source_path = path.strip_prefix(manifest_dir).unwrap().to_str().unwrap();

        if is_manually_registered {
            // The custom test below owns this fixture's registration.
        } else if test_name == "gc_finalizer_after_entrypoint_0" {
            writeln!(
                f,
                "finalizer_fixture_test!({}{}, {:?});",
                ignore_prefix, test_name, source_path
            )
            .unwrap();
        } else if let Some(thread_count) = multi_arena_threads.filter(|_| is_multithreading_enabled)
        {
            writeln!(
                f,
                "#[cfg(feature = \"multithreading\")] multi_arena_test!({}, {:?}, {}, {}, {:?});",
                test_name,
                dll_path.to_str().unwrap(),
                thread_count,
                expected_exit_code,
                source_path
            )
            .unwrap();
        } else {
            writeln!(
                f,
                "fixture_test!({}{}, {:?}, {}, Some({:?}));",
                ignore_prefix,
                test_name,
                dll_path.to_str().unwrap(),
                expected_exit_code,
                source_path
            )
            .unwrap();
        }
        println!("cargo:rerun-if-changed={}", path.display());
    }

    if !missing_prebuilt_dlls.is_empty() {
        for dll in &missing_prebuilt_dlls {
            println!("cargo:warning=Missing prebuilt DLL: {}", dll);
        }
        panic!(
            "Stale/incomplete artifact: {} DLLs missing. Run scripts/build_fixtures.sh to update.",
            missing_prebuilt_dlls.len()
        );
    }
}
