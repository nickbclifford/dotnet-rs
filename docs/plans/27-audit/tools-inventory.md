# Tools and benchmarks discovery inventory

Baseline: `8431c1ab`. This enumerates reviewed source cases, not a suite-size requirement.
Source inspection covers the listed functions including subtests and synthetic inputs.

## `scripts/tests/test_ci_aggregates.py`

- `AggregateTests.test_fast_core_rejects_every_unsuccessful_result`
- `AggregateTests.test_successful_selected_and_unselected_modes`
- `AggregateTests.test_fast_or_selector_failure_is_not_a_deep_pass`
- `AggregateTests.test_selected_lanes_cannot_fail_cancel_or_disappear`
- `AggregateTests.test_batch_failure_stays_failed_after_attribution`
- `AggregateTests.test_missing_selection_is_required_and_singleton_failure_fails`
- `AggregateTests.test_failed_selector_cannot_publish_partial_outputs`

## `scripts/tests/test_ci_cross_arena_manifest.py`

- `CrossArenaManifestTests.test_matrix_teardown_is_exact_and_mt_only`
- `CrossArenaManifestTests.test_native_rows_compile_the_selected_crate`
- `CrossArenaManifestTests.test_each_row_executes_guarded_exact_selectors`
- `CrossArenaManifestTests.test_strict_core_is_isolated_and_guarded`

## `scripts/tests/test_ci_diagnostics.py`

- `DiagnosticsTests.test_success_streams_metadata_and_required_summary`
- `DiagnosticsTests.test_failure_classes_and_original_status`
- `DiagnosticsTests.test_child_signal_is_not_assumed_to_be_timeout`
- `DiagnosticsTests.test_missing_summary_is_nonzero_and_not_test_success`
- `DiagnosticsTests.test_timeout_is_evidenced_and_kills_child_group`
- `DiagnosticsTests.test_timeout_kills_grandchild_in_its_own_process_group`
- `DiagnosticsTests.test_term_and_int_save_partial_status_and_nonzero`
- `DiagnosticsTests.test_finalize_missing_status_without_inventing_success`
- `DiagnosticsTests.test_path_safety_collision_invalid_timeout_and_spawn_failure`
- `DiagnosticsTests.test_job_deadline_caps_timeout_and_skips_spent_budget`
- `DiagnosticsTests.test_environment_defaults_supply_directory_and_identity`
- `DiagnosticsTests.test_summarize_finalizes_interrupted_phases_and_never_fails`

## `scripts/tests/test_ci_select.py`

- `SelectionTests.test_docs_only`
- `SelectionTests.test_value_src_everything`
- `SelectionTests.test_value_closure_crates_everything`
- `SelectionTests.test_value_fuzz_dir_everything`
- `SelectionTests.test_vm_only`
- `SelectionTests.test_vm_fuzz_dir_vm_tier`
- `SelectionTests.test_intrinsics_string_like_vm`
- `SelectionTests.test_assemblies_support_sources_are_vm_tier`
- `SelectionTests.test_cli_and_benchmarks_nothing`
- `SelectionTests.test_root_build_config_everything`
- `SelectionTests.test_ci_yml_and_deep_scripts_everything`
- `SelectionTests.test_unrelated_script_and_workflow_nothing`
- `SelectionTests.test_unknown_paths_everything_and_named`
- `SelectionTests.test_mixed_docs_and_vm`
- `SelectionTests.test_mixed_vm_and_value_is_everything`
- `SelectionTests.test_reasons_are_single_line_and_name_paths`
- `SelectionTests.test_md_pulled_in_by_include_str_is_code`
- `SelectionTests.test_repo_has_no_unexpected_md_includes`
- `FailSafeTests.test_full_flag`
- `FailSafeTests.test_empty_paths_is_full`
- `FailSafeTests.test_zero_base_is_full`
- `FailSafeTests.test_bad_base_is_full`
- `FailSafeTests.test_git_failure_is_full`
- `FailSafeTests.test_selector_exception_is_full`
- `FailSafeTests.test_real_git_ranges`
- `OutputTests.test_fuzz_matrix_entries_identical_to_original`
- `OutputTests.test_github_output_and_summary_files`
- `OutputTests.test_ci_yml_fallback_matches_full_selection`
- `OutputTests.test_ci_yml_fallback_pointer_mode_is_singleton`
- `OutputTests.test_pointer_mode_by_selection`
- `ClosureDriftTests.test_vm_closure`
- `ClosureDriftTests.test_value_closure_from_fuzz_workspace`
- `ClosureDriftTests.test_vm_fuzz_workspace_closure_is_vm_closure`
- `ClosureDriftTests.test_value_closure_is_inside_vm_closure`
- `ClosureDriftTests.test_every_workspace_crate_is_classified`
- `ClosureDriftTests.test_non_deep_crates_are_not_dependencies_of_closure`

## `scripts/tests/test_diff_run_vm_only.py`

- `VmOnlyGuardTest.test_vm_only_source_is_rejected_before_runtime_execution`

## `scripts/tests/test_miri_advisory.py`

- `AdvisoryTests.test_manifest_matrix_exactly_once`
- `AdvisoryTests.test_workflow_discovers_each_configuration_once_without_suppressing_shards`
- `AdvisoryTests.test_manifest_rejects_missing_extra_duplicate_and_invalid`
- `AdvisoryTests.test_listing_union_and_empty_ancillary`
- `AdvisoryTests.test_exact_result_with_diagnostics_and_failure_modes`
- `AdvisoryTests.test_fake_cargo_original_filters_and_exact_execution`

## `scripts/tests/test_pointer_provenance.py`

- `PointerScriptTests.test_catalog_matches_source_and_shards`
- `PointerScriptTests.test_singleton_partition_is_one_control_per_shard`
- `PointerScriptTests.test_guard_good_and_stale_or_ignored`
- `PointerScriptTests.test_caller_log_directory_retains_both_streams_and_failure`
- `PointerScriptTests.test_controls_count_and_guard`
- `PointerScriptTests.test_each_control_shard_uses_exact_names_and_pinned_flags`
- `PointerScriptTests.test_reject_unknown_and_native_before_cargo`
- `PointerScriptTests.test_diagnostic_continues_and_distinguishes_failures`
- `PointerScriptTests.test_diagnostic_recognizes_inline_warning_success`
- `PointerScriptTests.test_ci_shards_match_catalog`
- `PointerScriptTests.test_reject_invalid_control_table_before_cargo`
- `PointerScriptTests.test_simulated_promotion_and_stale_table`
- `PointerScriptTests.test_batches_cover_each_control_exactly_once`
- `PointerScriptTests.test_batch_tests_are_outside_the_probe_discovery_namespace`
- `PointerScriptTests.test_verify_batches_rejects_drift`
- `PointerScriptTests.test_batch_command_runs_one_exact_batch_with_pinned_flags`
- `PointerScriptTests.test_batch_command_rejects_bad_arguments_before_cargo`
- `PointerScriptTests.test_batch_failures_fail_and_name_the_control`
- `PointerScriptTests.test_batch_log_is_retained_with_the_guard_message`
- `PointerScriptTests.test_ci_batch_job_is_wired_to_the_same_catalog`
- `PointerScriptTests.test_all_dry_runs_do_not_invoke_cargo`

## `scripts/tests/test_replay_fuzz_corpus.py`

- `LogTests.test_valid_and_extra_callbacks`
- `LogTests.test_fail_closed_logs`
- `CorpusTests.test_tracked_manifest_and_rejections`
- `CorpusTests.test_tracked_symlink`
- `CorpusTests.test_child_exit_and_log_persistence`

## `scripts/tests/test_resolve_perf_syms.py`

- `ResolvePerfSymbolsTests.test_reports_threads_shallow_stacks_and_foreign_processes`
- `ResolvePerfSymbolsTests.test_keeps_the_perf_script_shape_when_symbols_stay_unresolved`
- `ResolvePerfSymbolsTests.test_preserves_file_input_shape`

## `xtask/src/main.rs`

- `resolves_default_output_dir_using_convention`
- `resolves_target_dir_profile_and_triple`
- `preserves_absolute_output_dir_override`
- `rejects_mixed_override_and_layout_flags`
- `parser_reads_layout_flags`
- `displays_relative_path_under_repo_root`
- `matrix_format_defaults_to_lines`
- `matrix_format_accepts_json`
- `matrix_format_rejects_unknown_value`
- `slot_scanner_uses_accessor_calls_not_comments_or_test_modules`
- `slot_scanner_ignores_non_slot_receivers`
- `slot_verifier_rejects_unknown_accessor_calls`

## `scripts/tests/async_dispatch_probe.rs`

- `async_probe_dispatch_modes`
- `async_probe_il_routes`

## `crates/dotnet-benchmarks/src/lib.rs`

- `repeated_runs_retain_managed_static_state`

## Shell and compiler probes

- `scripts/check_build_script_regressions.sh`: `assert_clippy_skips_dotnet`, `assert_skip_toggle_invalidates_support_build`, `assert_vm_directory_rerun_invalidation`, `assert_async_intrinsic_wiring` (including all scanner conflict/option/visibility cases).
- `scripts/check_vm_effects_compile_fail.py`: positive facade compilation and every extracted effects compile-fail block; Rust bodies reviewed in the libraries partition.
- `scripts/tests/check_doc_drift_negative.sh`: missing/unknown SAFETY and INVARIANT citations; parent drift script scans registry and documentation links.
- `scripts/check_provenance_lints.sh`: compiler lints across workspace/standalone fuzz configurations.

## Benchmark functions and fixtures

All source in these files was inspected. Microbenchmarks establish only the explicitly asserted smoke/setup contracts; timing loops are not comprehensive correctness tests.

- `crates/dotnet-benchmarks/benches/cold_start.rs`
- `crates/dotnet-benchmarks/benches/end_to_end.rs`
- `crates/dotnet-benchmarks/benches/managed_ptr_serde.rs`
- `crates/dotnet-benchmarks/benches/metadata_load.rs`
- `crates/dotnet-benchmarks/benches/runtime_primitives.rs`
- `crates/dotnet-benchmarks/benches/runtime_type.rs`
- `crates/dotnet-benchmarks/benches/static_fields.rs`
- `crates/dotnet-benchmarks/benches/substring.rs`
- `crates/dotnet-benchmarks/fixtures/arithmetic/TightLoop_0.cs`
- `crates/dotnet-benchmarks/fixtures/async_await/AsyncAwaitBenchmark_0.cs`
- `crates/dotnet-benchmarks/fixtures/dispatch/VirtualDispatchStress_0.cs`
- `crates/dotnet-benchmarks/fixtures/ef_inmemory/Program.cs`
- `crates/dotnet-benchmarks/fixtures/exception_messages/ExceptionMessages_0.cs`
- `crates/dotnet-benchmarks/fixtures/exception_messages/RepeatedStateProbe_0.cs`
- `crates/dotnet-benchmarks/fixtures/exception_messages/ResourceColdLoad_0.cs`
- `crates/dotnet-benchmarks/fixtures/exception_messages/ResourceWarmLookup_0.cs`
- `crates/dotnet-benchmarks/fixtures/gc/AllocationPressure_0.cs`
- `crates/dotnet-benchmarks/fixtures/gc/AllocationThroughput_0.cs`
- `crates/dotnet-benchmarks/fixtures/gc_cross_arena/CrossArenaGcPressure_0.cs`
- `crates/dotnet-benchmarks/fixtures/gc_handles/ConditionalWeakTableOps_0.cs`
- `crates/dotnet-benchmarks/fixtures/gc_handles/GcHandleOps_0.cs`
- `crates/dotnet-benchmarks/fixtures/gc_handles/WeakReferenceGet_0.cs`
- `crates/dotnet-benchmarks/fixtures/generics/GenericsStress_0.cs`
- `crates/dotnet-benchmarks/fixtures/heap_access/HeapAccessOps_0.cs`
- `crates/dotnet-benchmarks/fixtures/json/JsonBenchmark_0.cs`
- `crates/dotnet-benchmarks/fixtures/json_dom/JsonDomBenchmark_0.cs`
- `crates/dotnet-benchmarks/fixtures/linq_pipeline/LinqPipelineBenchmark_0.cs`
- `crates/dotnet-benchmarks/fixtures/load_dominated/LoadDominated_0.cs`
- `crates/dotnet-benchmarks/fixtures/memory/MemoryBulkOps_0.cs`
- `crates/dotnet-benchmarks/fixtures/reflection/ReflectionEnumeration_0.cs`
- `crates/dotnet-benchmarks/fixtures/reflection/RuntimeType_0.cs`
- `crates/dotnet-benchmarks/fixtures/span/SpanEquality_0.cs`
- `crates/dotnet-benchmarks/fixtures/span_equality/SpanEqualityFocused_0.cs`
- `crates/dotnet-benchmarks/fixtures/stack/StackOpsStress_0.cs`
- `crates/dotnet-benchmarks/fixtures/static_fields/StaticFields_0.cs`
- `crates/dotnet-benchmarks/fixtures/string/StringOpsSimd_0.cs`
- `crates/dotnet-benchmarks/fixtures/substring/Large_0.cs`
- `crates/dotnet-benchmarks/fixtures/substring/Small_0.cs`
- `crates/dotnet-benchmarks/fixtures/unsafe_buffer/UnsafeBufferOps_0.cs`
- `crates/dotnet-benchmarks/src/lib.rs`

## Runner interpretation

Read `.github/workflows/{ci,miri,fuzz,valgrind,self-hosted-probe}.yml` at the test-selection/result boundaries, `.config/nextest.toml`, root `check.sh`, and runner functions used by the cases above. `scripts/{profile_perf,bench_pgo,build_fixtures}.sh` supply profiling/build entrypoints, not additional unit cases. There are no tracked root `tests/**` sources. Dependency code and generated/bin/obj/target artifacts are outside review scope.
