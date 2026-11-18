# Library test discovery inventory

Baseline `8431c1ab`. This is a source discovery aid, not evidence of review or execution. It lists test attributes, doctest fences, fuzz entrypoints, and nearby test function names; macro-generated cases, helpers, gating and non-Rust sources need separate inspection. Reproduce with `git grep -n -E "#\[(test|ignore|should_panic)|compile_fail|fuzz_target!|loom::model" 8431c1ab -- crates` and exclude dotnet-vm, dotnet-cli and dotnet-benchmarks.

## `crates/dotnet-assemblies/src/drop_tests.rs`

  - `6` #[test] → test_assembly_loader_drop_keeps_descriptors_valid
  - `53` #[test] → test_type_description_keeps_arena_alive
  - `96` #[test] → test_corlib_well_known_cache_and_dynamic_fallback
  - `141` #[test] → task_types_resolve_to_corelib_without_support_replacements
  - `177` #[test] → concrete_types_match_corelib_definitions_to_support_stubs
  - `208` #[test] → test_corlib_wkt_resolution_failures_are_retryable
  - `234` #[test] → test_exception_dispatch_state_prefers_slash_then_falls_back_to_plus
  - `284` #[test] → test_memory_extensions_slow_path_signature_uses_read_only_span

## `crates/dotnet-assemblies/src/host.rs`

  - `603` #[test] → parses_fixture_runtimeconfig
  - `631` #[test] → parses_fixture_deps_and_derives_no_nuget_probing_paths
  - `649` #[test] → parses_newtonsoft_deps_and_derives_package_probing_paths
  - `746` #[test] → resolves_framework_from_fixture_runtimeconfig_with_override_base
  - `767` #[test] → resolves_framework_from_fixture_runtimeconfig_using_dotnet_root_override
  - `796` #[test] → select_framework_version_applies_all_roll_forward_policies_for_10_0_0_request
  - `823` #[test] → disable_policy_requires_exact_match
  - `858` #[test] → classifies_managed_pe_with_cli_header
  - `866` #[test] → classifies_pe_without_cli_header_as_nativeaot
  - `874` #[test] → classifies_elf_as_nativeaot
  - `881` #[test] → classifies_bundle_signature_as_single_file
  - `895` #[test] → short_or_unknown_input_defaults_to_managed

## `crates/dotnet-assemblies/src/loader.rs`

  - `633` #[test] → register_probing_path_adds_lazy_external_entry
  - `659` #[test] → register_probing_path_keeps_existing_external_registration
  - `682` #[test] → add_scan_root_registers_dll_entries
  - `709` #[test] → new_from_host_uses_fixture_runtimeconfig_and_app_scan_root
  - `734` #[test] → new_from_host_registers_native_dirs_from_deps_assets

## `crates/dotnet-assemblies/src/support_contract_tests.rs`

  - `25` #[test] → test_registry_slot_count
  - `34` #[test] → generated_slot_descriptors_preserve_stable_ids_and_exact_shapes
  - `53` #[test] → generated_runtime_slot_enum_is_in_embedded_support_assembly
  - `68` #[test] → handle_override_is_limited_to_the_support_descriptor
  - `95` #[test] → loaded_support_metadata_has_an_independent_literal_slot_census
  - `189` #[test] → every_generated_id_and_typed_accessor_resolves_its_prescribed_owner
  - `308` #[test] → runtime_type_handle_object_ref_and_usize_views_round_trip
  - `358` #[test] → separate_span_layout_accessors_preserve_their_offsets

## `crates/dotnet-assemblies/src/support_contract_validation_tests.rs`

  - `125` #[test] → generated_runtime_slot_ids_are_required
  - `143` #[test] → unknown_id_is_a_contextual_contract_violation
  - `157` #[test] → missing_id_reports_its_exact_expected_slot
  - `168` #[test] → duplicate_annotations_report_the_id_and_both_locations
  - `185` #[test] → misplaced_id_reports_actual_and_expected_locations
  - `206` #[test] → exact_signature_mismatch_reports_expected_and_found_shape
  - `223` #[test] → wrong_staticness_reports_actual_and_expected_staticness
  - `242` #[test] → runtime_slot_field_without_used_implicitly_is_a_contract_violation
  - `316` #[test] → stub_schema_requires_exact_instance_string_in_place_of_field
  - `325` #[test] → stub_schema_requires_exactly_one_non_null_in_place_of_named_argument
  - `335` #[test] → stub_schema_rejects_duplicate_targets_without_mutating_registered_maps

## `crates/dotnet-assemblies/src/validation_tests.rs`

  - `9` #[test] → test_valid_metadata
  - `15` #[test] → test_invalid_module_name
  - `22` #[test] → test_empty_typedef_table
  - `29` #[test] → test_self_inheritance
  - `38` #[test] → test_empty_method_name

## `crates/dotnet-assemblies/src/version_tests.rs`

  - `11` #[test] → test_version_parsing
  - `22` #[test] → test_version_equality
  - `45` #[test] → test_version_binding_basic
  - `97` #[test] → test_binding_redirect

## `crates/dotnet-build-tools/src/lib.rs`

  - `192` #[test] → derives_profile_dir_for_default_target_layout
  - `202` #[test] → derives_profile_dir_for_custom_target_dir_layout
  - `212` #[test] → derives_profile_dir_for_non_default_profile_layout
  - `222` #[test] → derives_profile_dir_for_explicit_target_layout
  - `232` #[test] → falls_back_to_out_dir_when_build_ancestor_is_missing
  - `238` #[test] → fixture_output_base_default_layout
  - `246` #[test] → fixture_output_base_with_target_dir_profile_and_triple
  - `258` #[test] → shared_msbuild_inputs_walk_up_to_repo_root

## `crates/dotnet-build-tools/src/support_slots.rs`

  - `532` #[test] → parses_committed_twenty_one_slot_contract
  - `551` #[test] → parses_exact_signature_shapes_and_explicit_views
  - `593` #[test] → rejects_malformed_grammar
  - `609` #[test] → rejects_duplicate_ids_semantic_names_fields_and_primary_accessors
  - `638` #[test] → rejects_zero_non_dense_and_missing_accessors
  - `660` #[test] → rejects_unsupported_signature_storage_and_views

## `crates/dotnet-intrinsics-core/src/math.rs`

  - `1230` #[test] → integer_representations_truncate_to_byte
  - `1297` #[test] → min_max_decode_each_operand_in_stack_order
  - `1350` #[test] → non_integer_reports_type_mismatch

## `crates/dotnet-intrinsics-core/src/simd.rs`

  - `82` #[test] → hardware_acceleration_is_disabled_until_full_intrinsics_support_lands
  - `88` #[test] → vector256_probe_implies_vector128_probe

## `crates/dotnet-intrinsics-reflection/src/dynamic_method.rs`

  - `1472` #[test] → dynamic_assembly_ids_do_not_wrap

## `crates/dotnet-intrinsics-reflection/src/methods.rs`

  - `1392` #[test] → operand_window_rejects_managed_pointer_origins_without_consuming_values

## `crates/dotnet-intrinsics-span/src/equality.rs`

  - `456` #[test] → run_chunked_compare_finishes_in_one_call
  - `469` #[test] → cross_arena_span_read_rejects_torn_down_owner

## `crates/dotnet-intrinsics-string/src/operations.rs`

  - `649` #[test] → equality_roots_both_strings_and_resumes_after_collection
  - `745` #[test] → run_chunked_copy_completes_in_one_call

## `crates/dotnet-intrinsics-string/src/search.rs`

  - `333` #[test] → whitespace_keeps_source_and_progress_across_collections
  - `403` #[test] → repeated_constructor_keeps_progress_across_yields
  - `454` #[test] → async_body_preserves_source_and_progress_across_collections
  - `565` #[test] → validates_ranges_and_parameter_order
  - `578` #[test] → copies_bounded_chunks_without_altering_utf16

## `crates/dotnet-intrinsics-string/src/simd.rs`

  - `268` #[test] → probe_sequence_equal_matches_scalar_behavior
  - `282` #[test] → probe_index_of_matches_scalar_behavior
  - `296` #[test] → probe_ascii_whitespace_matches_scalar_behavior
  - `310` #[test] → probe_ascii_whitespace_handles_unicode_fallback

## `crates/dotnet-intrinsics-threading/src/monitor.rs`

  - `379` #[test] → monitor_argument_validation_orders_flag_null_and_timeout

## `crates/dotnet-macros-core/src/lib.rs`

  - `730` #[test] → optional_async_options_preserve_stacked_equality
  - `754` #[test] → intrinsic_argument_syntax
  - `776` #[test] → test_parse_simple_static
  - `787` #[test] → test_parse_instance_void
  - `797` #[test] → test_parse_qualified_params
  - `805` #[test] → test_parse_nested_type
  - `813` #[test] → test_parse_nested_type_parameter
  - `820` #[test] → test_parse_field_nested_type
  - `828` #[test] → test_parse_ref_params
  - `836` #[test] → test_expand_trait_aliases_multi_decl
  - `889` #[test] → test_expand_trait_alias_no_blanket_impl_generates_trait_only
  - `916` #[test] → test_expand_trait_alias_without_generics
  - `928` #[test] → test_parse_trait_alias_generics
  - `939` #[test] → test_struct_mapping_rejects_duplicate_binding_names
  - `952` #[test] → test_struct_mapping_rejects_duplicate_field_names

## `crates/dotnet-macros/src/lib.rs`

  - `345` #[test] → async_intrinsic_expands_once_for_stacked_annotations
  - `385` #[test] → async_intrinsic_rejects_mismatched_selectors
  - `427` #[test] → async_intrinsic_body_only_and_selector_only
  - `444` #[test] → async_intrinsic_requires_consistent_fallbacks
  - `480` #[test] → async_intrinsic_without_selector_and_sync_entry_shape
  - `507` #[test] → async_intrinsic_rejects_wrong_parameter_count
  - `526` #[test] → async_intrinsic_rejects_wrong_parameter_type
  - `547` #[test] → async_intrinsic_rejects_wrong_return_type
  - `575` #[test] → async_intrinsic_accepts_qualified_types_and_any_parameter_name
  - `598` #[test] → test_parse_instruction_mapping
  - `643` #[test] → test_instruction_mapping_to_match_arm
  - `689` #[test] → intrinsic_filter_checks_only_primitive_parameter_shapes
  - `707` #[test] → intrinsic_filter_is_true_only_when_all_parameters_are_non_primitive_shapes
  - `723` #[test] → intrinsic_filter_checks_every_normalized_primitive_keyword

## `crates/dotnet-metrics/src/lib.rs`

  - `1125` #[test] → cache_statistics_reads_sizes_by_cache_kind
  - `1193` #[test] → record_cache_updates_cache_statistics_for_selected_kinds
  - `1212` #[test] → record_cache_indexes_every_declared_kind
  - `1223` #[test] → cache_kind_indices_and_keys_are_stable
  - `1255` #[test] → global_kind_order_preserves_legacy_cache_stats_order
  - `1276` #[test] → generic_cache_instrumentation_uses_indexed_counters
  - `1316` #[test] → active_metrics_guard_restores_nested_thread_local_scope
  - `1348` #[test] → active_metrics_guard_skips_an_out_of_order_dropped_scope_and_cleans_up_tls
  - `1376` #[test] → leaked_active_metrics_guard_cannot_dangle
  - `1396` #[test] → allocation_pressure_instrumentation_reports_every_source
  - `1431` #[test] → snapshot_includes_always_on_gc_pause_percentiles
  - `1449` #[test] → snapshot_includes_per_arena_gc_pressure_metrics

## `crates/dotnet-pinvoke/src/call.rs`

  - `1360` #[test] → arena_identity_native_escape_preflight
  - `1408` #[test] → by_ref_write_back_propagates_managed_write_failures
  - `1425` #[test] → by_ref_write_back_raw_destination_copies_bytes
  - `1442` #[test] → typed_reference_input_abi_is_a_two_pointer_temporary
  - `1464` #[test] → typed_reference_pinvoke_return_is_rejected
  - `1472` #[test] → value_type_return_buffer_honors_alignment_and_zero_init
  - `1480` #[test] → value_type_return_copy_truncates_to_destination_size
  - `1488` #[test] → typed_return_abi_mismatch_is_rejected

## `crates/dotnet-pinvoke/src/loader.rs`

  - `262` #[test] → find_library_path_uses_additional_search_dirs
  - `284` #[test] → find_library_path_prefers_primary_root_over_additional_dirs
  - `309` #[test] → system_native_remains_loaded_after_cache_drop

## `crates/dotnet-pinvoke/src/sandbox.rs`

  - `53` #[test] → indirect_calls_require_explicit_sandbox_permission

## `crates/dotnet-runtime-memory/src/access.rs`

  - `1641` #[test] → owner_retention_transient_managed_ptr_store_is_rejected
  - `1723` #[test] → owner_atomic_load_rejects_offset_past_allocation
  - `1792` #[test] → check_bounds_in_range_ok
  - `1800` #[test] → bytes_ptr_access_preserves_live_pointer_provenance
  - `1828` #[test] → atomic_ptr_access_preserves_live_pointer_provenance_and_contracts
  - `1943` #[test] → check_bounds_null_base_skips_check
  - `1949` #[test] → check_bounds_out_of_range_err
  - `1964` #[test] → check_bounds_offset_overflow_err
  - `1981` #[test] → atomic_value_access_uses_unaligned_fallback
  - `2003` #[test] → field_layout_with_object_ref_gc_desc_requires_heap_ref_barrier
  - `2018` #[test] → field_layout_without_gc_desc_or_managed_ptrs_needs_no_heap_ref_barrier
  - `2033` #[test] → compare_exchange_error_variants
  - `2057` #[test] → check_bounds_error_promoted_to_compare_exchange_error
  - `2115` #[test] → write_barrier_panic_flush_guard_drains_on_panic
  - `2143` #[test] → write_barrier_panic_flush_guard_does_not_flush_on_normal_drop
  - `2194` #[test] → cross_arena_heap_storage_access_supports_non_static_gc_lifetime
  - `2310` #[test] → arena_identity_memory_owner_aba

## `crates/dotnet-runtime-memory/src/write_barrier.rs`

  - `263` #[test] → invalid_recorder_skips_cross_arena_managed_ptrs

## `crates/dotnet-runtime-memory/tests/loom_shared_tables.rs`

  - `43` let mut builder = loom::model::Builder::new();
  - `220` #[test] → shared_handles_publish_and_clear_by_owner_partition
  - `280` #[test] → finalizer_queue_roots_claimed_and_pending_objects_while_worker_is_parked

## `crates/dotnet-simd/src/lib.rs`

  - `188` #[test] → sequence_equal_reports_expected_results
  - `196` #[test] → copy_nonoverlapping_copies_all_bytes
  - `204` #[test] → fill_and_clear_update_buffer_contents
  - `213` #[test] → raw_copy_fill_and_clear_update_buffer_contents
  - `230` #[test] → vector_width_is_zero_when_simd_is_unavailable

## `crates/dotnet-tracer/src/lib.rs`

  - `830` #[test] → method_filter_selects_entries_and_exits_without_filtering_interop
  - `860` #[test] → backpressure_drops_events_when_channel_is_full
  - `879` #[test] → flush_waits_for_queued_entries_and_reports_pending_drops
  - `909` #[test] → tracer_channel_throughput_accounting_under_backpressure
  - `943` #[test] → enabled_emit_skips_closure_when_tracer_is_disabled
  - `960` #[test] → runtime_metrics_dump_uses_supplied_cache_stats
  - `987` #[test] → interop_span_emits_balanced_exit_on_drop

## `crates/dotnet-types/src/comparer.rs`

  - `1303` #[test] → test_types_equal_primitive
  - `1317` #[test] → test_param_types_equal
  - `1331` #[test] → signatures_variance_keeps_return_covariance_and_parameter_contravariance
  - `1364` #[test] → signatures_variance_rejects_typed_reference_return_type
  - `1379` #[test] → signatures_variance_keeps_byref_parameters_exact
  - `1399` #[test] → types_equal_preserves_same_placeholder_identity_before_substitution
  - `1419` #[test] → types_equal_returns_false_on_resolution_failure_after_generic_substitution
  - `1443` #[test] → concrete_equals_method_type_panics_on_resolution_failure
  - `1463` #[test] → concrete_types_equal_keeps_equal_error_result_behavior
  - `1486` #[test] → property_getter_lookup_panics_on_ptr_eq_unwrap
  - `1590` #[test] → types_equal_matches_same_name_across_resolutions

## `crates/dotnet-types/src/error.rs`

  - `395` #[test] → test_error_conversions
  - `413` #[test] → test_assembly_load_io_preserves_source

## `crates/dotnet-types/src/generics.rs`

  - `951` #[test] → test_validate_constraints_no_type_constraints_returns_ok
  - `981` #[test] → test_validate_constraints_zero_arity_mismatch_is_ignored
  - `1008` #[test] → test_validate_constraints_non_zero_arity_mismatch_errors
  - `1044` #[test] → test_direct_type_generic_self_reference_does_not_recurse
  - `1083` #[test] → test_cyclic_generic_constraint_causes_unbounded_recursion_regression
  - `1128` #[test] → test_cyclic_generic_constraint_detection_terminates_quickly
  - `1166` #[test] → test_non_cyclic_constraint_violations_still_error
  - `1239` #[test] → type_arg_returns_requested_slot
  - `1248` #[test] → method_arg_returns_requested_slot
  - `1263` #[test] → setters_recompute_the_memoized_hash
  - `1280` #[test] → bounded_accessors_return_generic_index_out_of_bounds
  - `1346` #[test] → debug_formats_nested_types_against_their_own_resolution
  - `1365` #[test] → value_kind_does_not_affect_identity_or_hashing

## `crates/dotnet-types/src/lib.rs`

  - `243` #[test] → test_null_exists
  - `249` #[test] → test_send_sync_traits

## `crates/dotnet-types/src/members.rs`

  - `331` #[test] → type_descriptions_with_different_indices_are_not_equal
  - `343` #[test] → method_descriptions_with_different_parents_are_not_equal
  - `372` #[test] → method_descriptions_with_different_parents_produce_different_hashes
  - `459` #[test] → generic_slots_reject_surplus_and_missing_arguments

## `crates/dotnet-types/src/resolution.rs`

  - `166` #[test] → test_descriptor_lifetime_safety
  - `206` #[test] → test_multiple_descriptors_one_arena

## `crates/dotnet-types/src/wkt.rs`

  - `308` #[test] → names_round_trip_and_discriminants_are_contiguous
  - `325` #[test] → span1_is_appended_without_renumbering_existing_variants
  - `332` #[test] → from_name_accepts_dispatch_state_alias_and_rejects_unknown_names
  - `369` #[test] → type_resolver_default_method_uses_the_canonical_name

## `crates/dotnet-utils/src/atomic.rs`

  - `377` #[test] → preserves_pointer_provenance_through_atomic_operations
  - `424` #[test] → unaligned_pointer_access_is_limited_to_synchronized_load_and_store
  - `804` #[test] → test_sizes
  - `848` #[test] → typed_atomic_operations_cover_every_supported_width
  - `927` #[test] → test_orderings
  - `945` #[test] → test_invalid_load_ordering
  - `947` #[should_panic(expected = "Invalid load ordering")]
  - `957` #[test] → test_invalid_store_ordering
  - `959` #[should_panic(expected = "Invalid store ordering")]
  - `969` #[test] → test_mixed_access_validation
  - `983` #[test] → test_validate_alignment_rejects_misaligned_pointer
  - `985` #[should_panic(expected = "Alignment violation")]
  - `993` #[test] → test_load_field_misaligned_falls_back_in_mt
  - `1008` #[test] → test_store_field_large_size_falls_back_in_mt
  - `1021` #[test] → test_load_field_misaligned_falls_back_non_mt
  - `1034` #[test] → test_store_field_unsupported_size_falls_back_non_mt

## `crates/dotnet-utils/src/gc/arena.rs`

  - `182` #[test] → record_allocation_tracks_peak_and_collection_trigger_count

## `crates/dotnet-utils/src/gc/cross_arena.rs`

  - `777` #[test] → lease_on_unregistered_arena_returns_none
  - `788` #[test] → lease_on_registered_arena_returns_some
  - `802` #[test] → lease_drop_does_not_unregister_arena
  - `819` #[test] → lease_after_unregister_returns_none
  - `830` #[test] → multiple_leases_track_borrow_count
  - `858` #[test] → unregister_waits_for_outstanding_lease
  - `911` #[test] → lease_reflects_stw_flag
  - `933` #[test] → lease_works_for_low_arena_id
  - `946` #[test] → is_valid_cross_arena_ref_reflects_registration_state
  - `959` #[test] → each_registration_has_a_unique_increasing_generation
  - `982` #[test] → lease_generation_matches_arena_state_generation
  - `1003` #[test] → lease_is_valid_while_arena_alive
  - `1019` #[test] → reregistered_arena_has_different_generation_than_old_lease
  - `1042` #[test] → record_cross_arena_ref_stores_generation
  - `1072` #[test] → record_cross_arena_ref_rejects_stale_identity_after_reregistration
  - `1097` #[test] → record_cross_arena_ref_outside_tracing_returns_validity
  - `1118` #[test] → stress_arena_lifecycle_concurrency
  - `1223` #[test] → stress_stw_flag_transitions_with_lease_acquisition
  - `1374` #[test] → record_and_harvest_race_with_reregistration
  - `1423` #[test] → arena_identity_registry_duplicate
  - `1444` #[test] → arena_identity_registry_fast_bit_transaction
  - `1479` #[test] → arena_identity_registry_drain_reuse_control
  - `1554` #[test] → arena_identity_registry_generation_exhaustion
  - `1627` #[test] → arena_identity_registry_identity_token_reuse
  - `1664` #[test] → arena_identity_registry_checked_counter_control
  - `1690` #[test] → arena_lifetime_transport_permissive_control
  - `1725` #[test] → arena_lifetime_transport

## `crates/dotnet-utils/src/gc/mod.rs`

  - `357` #[test] → arena_lifetime_opaque_transport

## `crates/dotnet-utils/src/gc/thread_safe_lock.rs`

  - `389` #[test] → test_basic_borrow
  - `396` #[test] → test_multiple_readers
  - `405` #[test] → test_exclusive_writer
  - `422` #[test] → test_try_borrow
  - `437` #[test] → write_guards_track_safepoint_exclusion_across_mapping
  - `459` #[test] → borrow_mut_barriers_new_gc_children_of_marked_parent

## `crates/dotnet-utils/src/raw_block.rs`

  - `129` #[test] → requested_alignment_is_honoured_for_payload_sizes
  - `145` #[test] → newly_allocated_payload_is_zeroed
  - `158` #[test] → payload_layout_accounting_and_pointer_bounds_are_distinct
  - `176` #[test] → earlier_blocks_remain_usable_after_allocating_sixty_four
  - `218` #[test] → invalid_alignments_and_layout_overflows_do_not_change_live_count
  - `249` #[test] → live_count_returns_to_baseline_after_drop
  - `259` #[test] → live_count_returns_to_baseline_after_caught_panic

## `crates/dotnet-utils/src/sync.rs`

  - `598` #[test] → compat_mutex_lock_unlock
  - `608` #[test] → compat_rwlock_read_write
  - `616` #[test] → compat_rwlock_try_variants_succeed_when_unlocked
  - `623` #[test] → compat_rwlock_try_write_fails_while_borrowed
  - `634` #[test] → compat_mutex_double_lock_panics
  - `635` #[should_panic]
  - `643` #[test] → compat_rwlock_write_while_reading_panics
  - `644` #[should_panic]
  - `651` #[test] → compat_rwlock_read_while_writing_panics
  - `652` #[should_panic]
  - `660` #[test] → compat_condvar_notifications_do_not_panic
  - `667` #[test] → compat_condvar_wait_panics
  - `668` #[should_panic(expected = "compat::Condvar::wait cannot be used")]
  - `693` #[test] → mutex_lock_and_try_lock_follow_facade_semantics
  - `695` loom::model(|| {
  - `727` #[test] → mapped_rwlock_guards_preserve_the_source_lock
  - `729` loom::model(|| {
  - `746` #[test] → condition_wait_and_notify_use_a_mutable_facade_guard
  - `748` loom::model(|| {
  - `766` #[test] → ordered_locks_compose_acquisition_tokens
  - `768` loom::model(|| {

## `crates/dotnet-utils/src/sync/lock_order.rs`

  - `435` #[test] → level_names_match_canonical_lock_order_table
  - `469` #[test] → lock_level_edges_cover_runtime_chains
  - `481` #[test] → ordered_mutex_supports_nested_acquisition_tokens
  - `496` #[test] → ordered_mutex_exposes_raw_guard_for_condvar_interop
  - `506` #[test] → ordered_rwlock_supports_read_and_write

## `crates/dotnet-value/fuzz/fuzz_targets/fuzz_managed_ptr_offset.rs`

  - `32` fuzz_target!(|input: OffsetInput| {

## `crates/dotnet-value/fuzz/fuzz_targets/fuzz_managed_ptr_roundtrip.rs`

  - `131` fuzz_target!(|input: RoundtripInput| {

## `crates/dotnet-value/fuzz/fuzz_targets/fuzz_raw_memory_access.rs`

  - `6` fuzz_target!(|data: (u16, u8, u64, u8)| {

## `crates/dotnet-value/src/atomic_tests.rs`

  - `19` #[test] → test_atomic_load_store
  - `87` #[test] → test_atomic_object_load_store
  - `141` #[test] → test_misaligned_atomic_load_is_rejected
  - `143` #[should_panic(expected = "Alignment violation")]
  - `155` #[test] → test_misaligned_atomic_store_is_rejected
  - `157` #[should_panic(expected = "Alignment violation")]
  - `178` #[test] → test_concurrent_atomic_load_store

## `crates/dotnet-value/src/cts_cli_conversion.rs`

  - `383` #[test] → pointer_slots_preserve_native_bits_and_null
  - `396` #[test] → convert_i64_rejects_non_int64_input
  - `408` #[test] → enum_underlying_widening_preserves_sign_and_zero_extension
  - `429` #[test] → load_widening_small_integers_preserves_sign_and_zero_extension
  - `453` #[test] → widening_preserves_pointer_and_reference_payloads

## `crates/dotnet-value/src/layout.rs`

  - `581` /// '''compile_fail,E0133
  - `586` /// '''compile_fail
  - `670` #[test] → test_gc_desc_bitmap_set_and_merge
  - `687` #[test] → test_gc_desc_unaligned_offsets_are_sorted_and_unique
  - `698` #[test] → test_field_layout_manager_recursion

## `crates/dotnet-value/src/object/mod.rs`

  - `1055` #[test] → sync_block_index_encoding_is_compact
  - `1061` #[test] → object_inner_sync_block_index_roundtrip
  - `1111` #[test] → codec_address_limits
  - `1125` #[test] → codec_short_storage
  - `1143` #[test] → test_read_unchecked_cross_arena_invalid
  - `1176` #[test] → arena_lifetime_opaque_gc_transport
  - `1214` #[test] → test_read_unchecked_cross_arena_valid
  - `1255` #[test] → test_write_encodes_owner_tag_even_on_owner_thread
  - `1316` #[test] → test_object_ptr_as_heap_storage_corrupt_magic_panics
  - `1351` #[test] → test_object_ptr_as_heap_storage_cross_arena_id_panics

## `crates/dotnet-value/src/object/types.rs`

  - `601` #[test] → object_ref_elements_reads_pointer_sized_stride_with_gc_branding
  - `615` #[test] → write_object_ref_elements_mut_uses_pointer_stride_and_preserves_tail
  - `633` #[test] → tracing_a_primitive_vector_skips_element_storage
  - `651` #[test] → tracing_a_value_type_vector_keeps_embedded_object_refs_alive

## `crates/dotnet-value/src/object_tests.rs`

  - `63` #[test] → codec_id40000
  - `79` #[test] → codec_registered_boundaries
  - `97` #[test] → codec_reject_large_owners
  - `129` #[test] → codec_unavailable_owner
  - `159` #[test] → codec_local_invalid_control
  - `231` #[test] → publication_old_path_diagnostic
  - `237` #[test] → publication_root_barrier_control
  - `243` #[test] → publication_pending_survival
  - `267` #[test] → heap_storage_payload_extents_and_raw_bases
  - `330` #[test] → test_read_branded_null
  - `343` #[test] → test_read_valid_object
  - `358` #[test] → test_storage_pointer_roundtrip
  - `377` #[test] → test_non_object_tag_decodes_to_null

## `crates/dotnet-value/src/pointer/tests.rs`

  - `38` #[test] → cts_pointer_write_rejects_invalid_slot_size
  - `110` #[test] → owner_retention_transient_native_int_conversion_is_rejected
  - `181` #[test] → owner_retention_cli_to_cts_rejects_transient_integer_escape
  - `249` #[test] → owner_retention_retained_ptr_keeps_transient_owner_alive
  - `268` #[test] → owner_retention_retained_ptr_heap_origin_reads_through_owner
  - `292` #[test] → retained_checked_range_bounds_heap_payload
  - `343` #[test] → retained_checked_range_resolves_static_block
  - `395` #[test] → owner_retention_retained_ptr_rejects_non_pointer_values
  - `419` #[test] → owner_retention_retained_ptr_null
  - `447` #[test] → owner_retention_normalize_retains_transient_owner
  - `461` #[test] → owner_retention_normalize_transient_access_after_consume
  - `473` #[test] → owner_retention_from_info_full_retains_transient_owner
  - `494` #[test] → owner_retention_from_info_full_transient_access_after_info_consumed
  - `514` #[test] → owner_retention_transient_managed_ptr_clone_aliases_owner_storage
  - `542` #[test] → heap_access_one_byte_heap_read
  - `574` #[test] → heap_access_heap_pointer_survives_typed_write
  - `658` #[test] → heap_read_windows_are_exact_for_array_object_and_boxed_payloads
  - `747` #[test] → heap_with_data_rejects_short_range
  - `748` #[should_panic(expected = "ManagedPtr::with_data: heap range overflow or out of bounds")]
  - `776` #[test] → cross_arena_heap_read_uses_exact_payload_range
  - `813` #[test] → heap_address_observation_wraps_without_granting_read_access
  - `904` #[test] → heap_access_safe_oob_address_observation
  - `936` #[test] → test_managed_ptr_offset_oob
  - `949` #[test] → test_managed_ptr_offset_valid
  - `965` #[test] → test_managed_ptr_metadata_rejects_unknown_static_subtag
  - `1039` #[test] → signed_frame_home_serialization_and_bounds
  - `1078` #[test] → frame_home_pointer_round_trips_serial_and_offset
  - `1108` #[test] → frame_home_pointer_rejects_returned_frame_and_offset_mismatch
  - `1144` #[test] → frame_home_access_checks_liveness_and_range
  - `1178` #[test] → test_heap_decode_cache_hit_miss_and_integrity
  - `1248` #[test] → test_managed_ptr_serialization_roundtrip
  - `1407` #[test] → test_gc_alignment
  - `1420` #[test] → test_managed_ptr_serialization_bugs_reproduction
  - `1461` #[test] → test_static_registry_deduplication
  - `1505` #[test] → thread_static_pointer_serialization_preserves_owner
  - `1548` #[test] → test_managed_ptr_unmanaged_roundtrip_miri
  - `1571` #[test] → ptr_arith_managed_ptr_checked_offset_reports_errors
  - `1708` #[test] → ptr_arith_unmanaged_ptr_null_contract
  - `1777` #[test] → ptr_arith_unmanaged_offsets_wrap_and_round_trip_with_deref
  - `1809` #[test] → ptr_arith_unmanaged_arithmetic_wraps_to_explicit_null
  - `1842` #[test] → ptr_arith_min_signed_operands_do_not_overflow
  - `1876` #[test] → ptr_arith_pointer_comparisons_are_value_bearing
  - `2028` #[test] → ptr_arith_managed_ptr_far_oob_compare_and_difference
  - `2135` #[test] → ptr_arith_managed_ptr_arithmetic_boundaries_report_explicit_errors
  - `2287` #[test] → owner_retention_cross_arena_access_after_owner_teardown_is_rejected
  - `2302` #[test] → owner_retention_cross_arena_lease_holds_teardown_until_access_completes
  - `2335` #[test] → owner_retention_retained_ptr_cross_arena_lease_blocks_teardown
  - `2399` #[test] → owner_retention_fill_fast_path_observed_pointer_dangles_after_teardown
  - `2400` #[ignore = "documents the unprotected observe-then-write pattern the Fill fast path must not use; known-dangling, run under Miri with --ignored only"]
  - `2420` #[test] → owner_retention_cross_arena_validation_accepts_in_flight_lease
  - `2518` #[test] → arena_identity_origin_aba
  - `2532` #[test] → arena_identity_in_memory_origin_aba
  - `2541` #[test] → arena_identity_origin_aba_registry_control
  - `2550` #[test] → arena_identity_token_wire_identity
  - `2609` #[test] → arena_identity_origin_aba
  - `2698` #[test] → arena_lifetime_transport_permissive_control
  - `2704` #[test] → arena_lifetime_transport_strict_core
  - `2711` #[test] → retained_frame_home_range_checks_full_access
  - `2742` #[test] → retired_stack_encoding_is_rejected

## `crates/dotnet-value/src/stack_value.rs`

  - `1142` #[test] → test_stack_value_arithmetic
  - `1177` #[test] → managed_pointer_numeric_operations_use_cached_non_heap_address
  - `1200` #[test] → signed_managed_pointer_arithmetic_comparison_and_difference
  - `1232` #[test] → typed_reference_slot_copy_retains_type_lifetime_and_pointer_origin
  - `1256` #[test] → uninitialized_typed_reference_has_a_distinct_stack_kind

## `crates/dotnet-value/src/storage.rs`

  - `537` #[test] → get_field_local_rejects_misaligned_storage
  - `538` #[should_panic(expected = "Alignment violation: FieldStorage::get_field_local")]
  - `544` #[test] → get_field_mut_local_rejects_misaligned_storage
  - `545` #[should_panic(expected = "Alignment violation: FieldStorage::get_field_mut_local")]
  - `551` #[test] → get_field_atomic_rejects_misaligned_storage
  - `552` #[should_panic(expected = "Alignment violation: FieldStorage::get_field_atomic")]
  - `558` #[test] → set_field_atomic_rejects_misaligned_storage
  - `559` #[should_panic(expected = "Alignment violation: FieldStorage::set_field_atomic")]
  - `607` #[test] → object_size_while_storage_read_guard_is_held
  - `630` #[test] → nested_and_array_reference_slots_require_exact_start
  - `652` #[test] → typed_reference_access_requires_matching_slot

## `crates/dotnet-value/src/string.rs`

  - `294` #[test] → strings_keep_a_nul_terminator_outside_their_length

## `crates/dotnet-value/src/test_helpers.rs`

  - `105` #[test] → test_context_restores_thread_id_during_unwind

## `crates/dotnet-value/src/validation_tests.rs`

  - `9` #[test] → test_arena_mismatch_panics
  - `34` #[test] → test_dangling_arena_panics
  - `71` #[test] → test_uncoordinated_stw_access_panics

## `crates/dotnet-value/tests/layout_sizes.rs`

  - `142` #[test] → layout_sizes_match_expected

## `crates/dotnet-value/tests/stack_value_cross_arena_retired.rs`

  - `16` // interpreted. cfg-excluded rather than '#[ignore]'d so no ignored test is reported.
  - `69` #[test] → scanner_distinguishes_origin_from_retired_stack_variant
  - `97` #[test] → stack_value_cross_arena_object_ref_stays_retired

## `crates/dotnet-vm-data/src/frame_alloc.rs`

  - `161` #[test] → signed_and_request_limits
  - `181` #[test] → frame_budget_and_failure_leave_state_unchanged
  - `209` #[test] → external_home_blocks_share_localloc_budget_and_rollback
  - `239` #[test] → thread_budget_is_shared_and_restored_on_drop
  - `272` #[test] → zero_size_blocks_are_distinct_and_charged
  - `290` #[test] → allocation_order_and_markers_survive_growth
  - `311` #[test] → release_all_is_idempotent_and_reusable
  - `332` #[test] → overflow_and_invalid_layout_leave_state_unchanged
  - `360` #[test] → frame_blocks_survive_suspension_and_restoration

## `crates/dotnet-vm-data/src/frame_homes.rs`

  - `302` #[test] → temporary_homes_survive_growth_and_move_and_retire_on_release
  - `332` #[test] → temporary_and_primary_lookup_are_independent

## `crates/dotnet-vm-data/src/stack.rs`

  - `388` #[test] → nested_suspension_restores_each_operand_segment
  - `403` #[test] → empty_suspension_has_its_own_depth_and_discards_cleanly
  - `452` #[test] → owner_retention_pop_retained_ptr_keeps_transient_owner_alive
  - `465` #[test] → owner_retention_pop_address_matches_stack_value_pointer
  - `473` #[test] → owner_retention_pointer_pops_reject_non_pointers
  - `490` #[test] → owner_retention_pop_ptr_retains_transient_owner
  - `498` #[test] → owner_retention_pop_ptr_access_after_pop
  - `511` #[test] → owner_retention_peek_clone_drop_keeps_transient_address_valid
  - `539` #[test] → owner_retention_write_back_origin_clone_retains_transient_owner
  - `564` #[test] → owner_retention_typed_ref_clone_retains_transient_owner
  - `600` #[test] → copy_slots_into_copies_requested_slice

## `crates/dotnet-vm-data/tests/layout_sizes.rs`

  - `84` #[test] → layout_sizes_match_expected

## `crates/dotnet-vm-ops/src/effects.rs`

  - `1562` /// '''compile_fail,E0277
  - `1570` /// '''compile_fail,E0277
  - `1878` /// '''compile_fail
  - `1889` /// '''compile_fail
  - `1898` /// '''compile_fail
  - `1909` /// '''compile_fail
  - `1918` /// '''compile_fail
  - `1930` /// '''compile_fail
  - `2363` #[test] → abandoned_call_cannot_yield_or_access_vm
  - `2416` #[test] → control_transition_table
  - `2580` #[test] → storage_scopes_are_rejected_at_poll_boundaries
  - `2626` #[test] → default_native_host_runs_fallback_for_non_inline_mode
  - `2665` #[test] → condition_replies_require_matching_effect_and_generation
  - `2703` #[test] → event_replies_require_matching_effect_and_generation
  - `2732` #[test] → sleep_replies_require_matching_effect_and_generation
  - `2769` #[test] → initializer_replies_require_matching_effect_and_generation
  - `2820` #[test] → nested_entry_rejected
  - `2841` #[test] → panic_resets_entry_and_installation
  - `2870` #[test] → grant_budget_yields_before_grant_33
  - `2901` #[test] → continuation_polls_share_one_turn_grant_budget
  - `2949` #[test] → stop_and_explicit_yield_return_to_executor
  - `2994` #[test] → pending_without_effect_is_rejected
  - `3008` #[test] → ready_with_registered_effect_is_rejected
  - `3029` #[test] → forgotten_access_is_rejected
  - `3045` #[test] → substring_operands_and_root_survive_collection
  - `3103` #[test] → raw_gc_trace_bypasses_foreign_object_recording
  - `3155` #[test] → rooted_frame_home_descriptor_preserves_aliases_and_rejects_retired_home
  - `3318` #[test] → rooted_unmanaged_pointer_preserves_provenance
  - `3341` #[test] → rooted_nested_pointer_value_keeps_owner_alive
  - `3429` #[test] → rooted_heap_pointer_descriptor_keeps_owner_alive
  - `3475` #[test] → rooted_typed_reference_descriptor_keeps_owner_alive
  - `3530` #[test] → rooted_clone_retains_then_releases_and_mismatch_is_error

## `crates/dotnet-vm-ops/src/ops.rs`

  - `796` #[test] → owner_retention_pointer_pops_use_trait_pop_and_reject_non_pointers


## Static trait and documentation fence discovery

Reproduce with `rg -n "assert_(not_)?impl_(all|any)!|///[[:space:]]*\x60\x60\x60" crates`, excluding the three out-of-scope crates. These entries supplement, rather than prove completion of, the attribute list.

- `crates/dotnet-metrics/src/lib.rs:1113` static_assertions::assert_not_impl_all!(ActiveRuntimeMetricsGuard: Send);
- `crates/dotnet-metrics/src/lib.rs:1115` static_assertions::assert_not_impl_all!(ActiveRuntimeMetricsGuard: Sync);
- `crates/dotnet-types/src/resolution.rs:162` assert_impl_all!(MetadataArena: Send, Sync);
- `crates/dotnet-types/src/resolution.rs:163` assert_impl_all!(ResolutionS: Send, Sync);
- `crates/dotnet-types/src/resolution.rs:164` assert_impl_all!(Resolution<'static>: Send, Sync);
- `crates/dotnet-utils/src/gc/mod.rs:351` assert_impl_all!(OpaqueObjectPtr: Send, Sync, Copy);
- `crates/dotnet-utils/src/gc/mod.rs:352` assert_impl_all!(CrossArenaRef: Send, Sync, Copy);
- `crates/dotnet-utils/src/gc/thread_safe_lock.rs:364` assert_impl_all!(ThreadSafeLock<i32>: Send, Sync);
- `crates/dotnet-utils/src/gc/thread_safe_lock.rs:366` assert_impl_all!(ThreadSafeLock<Cell<u8>>: Send);
- `crates/dotnet-utils/src/gc/thread_safe_lock.rs:368` assert_not_impl_all!(ThreadSafeLock<Cell<u8>>: Sync);
- `crates/dotnet-utils/src/gc/thread_safe_lock.rs:370` assert_not_impl_all!(ThreadSafeLock<Rc<u8>>: Send);
- `crates/dotnet-utils/src/gc/thread_safe_lock.rs:372` assert_not_impl_all!(ThreadSafeLock<Rc<u8>>: Sync);
- `crates/dotnet-utils/src/gc/thread_safe_lock.rs:374` assert_not_impl_all!(ThreadSafeWriteGuard<'static, i32>: Send);
- `crates/dotnet-utils/src/gc/thread_safe_lock.rs:377` assert_impl_all!(ThreadSafeLock<i32>: Send);
- `crates/dotnet-utils/src/gc/thread_safe_lock.rs:379` assert_not_impl_all!(ThreadSafeLock<i32>: Sync);
- `crates/dotnet-utils/src/gc/thread_safe_lock.rs:381` assert_impl_all!(ThreadSafeLock<Cell<u8>>: Send);
- `crates/dotnet-utils/src/gc/thread_safe_lock.rs:383` assert_not_impl_all!(ThreadSafeLock<Cell<u8>>: Sync);
- `crates/dotnet-utils/src/gc/thread_safe_lock.rs:385` assert_not_impl_all!(ThreadSafeLock<Rc<u8>>: Send);
- `crates/dotnet-utils/src/gc/thread_safe_lock.rs:387` assert_not_impl_all!(ThreadSafeLock<Rc<u8>>: Sync);
- `crates/dotnet-utils/src/raw_block.rs:127` static_assertions::assert_not_impl_any!(RawBlock: Clone, Send, Sync);
- `crates/dotnet-utils/src/sync.rs:580` assert_not_impl_all!(Mutex<i32>: Sync);
- `crates/dotnet-utils/src/sync.rs:581` assert_not_impl_all!(RwLock<i32>: Sync);
- `crates/dotnet-utils/src/sync.rs:583` assert_not_impl_all!(super::compat::MutexGuard<'static, i32>: Sync);
- `crates/dotnet-utils/src/sync.rs:584` assert_not_impl_all!(super::compat::RwLockReadGuard<'static, i32>: Sync);
- `crates/dotnet-utils/src/sync.rs:585` assert_not_impl_all!(super::compat::RwLockWriteGuard<'static, i32>: Sync);
- `crates/dotnet-utils/src/sync.rs:589` assert_impl_all!(Mutex<i32>: Send);
- `crates/dotnet-utils/src/sync.rs:590` assert_impl_all!(RwLock<i32>: Send);
- `crates/dotnet-utils/src/sync.rs:592` assert_not_impl_all!(Mutex<*mut i32>: Send);
- `crates/dotnet-utils/src/sync.rs:593` assert_not_impl_all!(RwLock<*mut i32>: Send);
- `crates/dotnet-utils/src/sync.rs:594` assert_not_impl_all!(Mutex<*mut i32>: Sync);
- `crates/dotnet-utils/src/sync.rs:595` assert_not_impl_all!(RwLock<*mut i32>: Sync);
- `crates/dotnet-utils/src/sync/lock_order.rs:417` assert_not_impl_all!(levels::CollectionLock: AcquireAfter<levels::GcCoordination>);
- `crates/dotnet-utils/src/sync/lock_order.rs:418` assert_not_impl_all!(levels::GcCoordination: AcquireAfter<levels::ThreadRegistry>);
- `crates/dotnet-utils/src/sync/lock_order.rs:419` assert_not_impl_all!(levels::CollectionLock: AcquireAfter<levels::CoordinatorArenas>);
- `crates/dotnet-utils/src/sync/lock_order.rs:420` assert_not_impl_all!(levels::CollectionLock: AcquireAfter<levels::ArenaCurrentCommand>);
- `crates/dotnet-utils/src/sync/lock_order.rs:421` assert_not_impl_all!(levels::CoordinatorArenas: AcquireAfter<levels::ArenaCurrentCommand>);
- `crates/dotnet-utils/src/sync/lock_order.rs:422` assert_not_impl_all!(levels::StaticInitMutex: AcquireAfter<levels::StaticWaitGraph>);
- `crates/dotnet-utils/src/sync/lock_order.rs:423` assert_not_impl_all!(levels::SyncBlocks: AcquireAfter<levels::SyncNextIndex>);
- `crates/dotnet-utils/src/sync/lock_order.rs:425` assert_not_impl_all!(levels::SyncNextIndex: AcquireAfter<levels::Unlocked>);
- `crates/dotnet-value/src/layout.rs:581` /// '''compile_fail,E0133
- `crates/dotnet-value/src/layout.rs:584` /// '''
- `crates/dotnet-value/src/layout.rs:586` /// '''compile_fail
- `crates/dotnet-value/src/layout.rs:594` /// '''
- `crates/dotnet-value/src/object/mod.rs:228` // by 'assert_impl_all!' below). 'ObjectInner<'static>: Send + Sync' is
- `crates/dotnet-value/src/object/mod.rs:1074` assert_impl_all!(ObjectPtr: Send, Sync);
- `crates/dotnet-value/src/object/mod.rs:1076` assert_impl_all!(ObjectInner<'static>: Send, Sync);
- `crates/dotnet-value/src/object/mod.rs:1078` assert_impl_all!(ThreadSafeLock<ObjectInner<'static>>: Send, Sync);
- `crates/dotnet-value/src/object/mod.rs:1080` assert_not_impl_all!(ObjectPtr: Send);
- `crates/dotnet-value/src/object/mod.rs:1082` assert_not_impl_all!(ObjectPtr: Sync);
- `crates/dotnet-vm-data/src/lib.rs:74` static_assertions::assert_not_impl_any!(MethodState: Clone);
- `crates/dotnet-vm-ops/src/effects.rs:1562` /// '''compile_fail,E0277
- `crates/dotnet-vm-ops/src/effects.rs:1566` /// '''
- `crates/dotnet-vm-ops/src/effects.rs:1570` /// '''compile_fail,E0277
- `crates/dotnet-vm-ops/src/effects.rs:1574` /// '''
- `crates/dotnet-vm-ops/src/effects.rs:1878` /// '''compile_fail
- `crates/dotnet-vm-ops/src/effects.rs:1885` /// '''
- `crates/dotnet-vm-ops/src/effects.rs:1889` /// '''compile_fail
- `crates/dotnet-vm-ops/src/effects.rs:1896` /// '''
- `crates/dotnet-vm-ops/src/effects.rs:1898` /// '''compile_fail
- `crates/dotnet-vm-ops/src/effects.rs:1905` /// '''
- `crates/dotnet-vm-ops/src/effects.rs:1909` /// '''compile_fail
- `crates/dotnet-vm-ops/src/effects.rs:1916` /// '''
- `crates/dotnet-vm-ops/src/effects.rs:1918` /// '''compile_fail
- `crates/dotnet-vm-ops/src/effects.rs:1926` /// '''
- `crates/dotnet-vm-ops/src/effects.rs:1930` /// '''compile_fail
- `crates/dotnet-vm-ops/src/effects.rs:1936` /// '''
