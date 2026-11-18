# VM inline test inventory

Baseline `8431c1ab`. Every listed test body and setup was inspected; adjacent feature, target, debug and Loom gates remain part of each case. Names are source symbols, not a test-count requirement.

## `crates/dotnet-vm/src/gc/coordinator.rs`

- `arena_lifetime_fixed_point_command_retains_identity`
- `test_coordinator_registration`
- `test_allocation_pressure_trigger`
- `test_arena_gc_pressure_snapshot_reports_per_arena_metrics`
- `test_collect_all_arenas_no_deadlock`
- `test_collect_all_arenas_unregister_race`
- `test_collect_all_arenas_requires_registered_initiator`
- `test_command_finished_requires_pending_command`
- `stress_concurrent_register_unregister_with_collection_cycles`
- `stress_collect_all_arenas_with_concurrent_unregisters`
- `stress_lock_order_gc_cycle_guard_with_live_safepoints`
- `test_collection_session_cleanup_on_panic`
- `test_command_completion_guard_sends_signal_on_panic`
- `test_session_cleanup_precedes_stw_resume_on_unwind`

## `crates/dotnet-vm/src/threading/basic.rs`

- `registration_waits_until_stopped_world_resumes`
- `arena_lifetime_harvest_rejects_stale_registration`
- `managed_id_exhaustion`
- `stw_guard_drop_resets_gc_flags`
- `stw_guard_holds_gc_coordination_lock_until_drop`
- `registered_initiator_waits_out_registration_without_parking`
- `queued_initiator_reasserts_stop_request_after_previous_resume`
- `request_stop_the_world_rejects_active_thread_safe_write_guard`
- `lock_order_request_stop_the_world_rejects_inverted_top_level_order`
- `resume_threads_is_idempotent`
- `stw_panic_guard_armed_resumes_threads_on_drop`
- `stw_panic_guard_disarmed_does_not_resume_on_drop`
- `stw_guard_gc_flag_cleared_on_drop`
- `test_managed_thread_get_state_valid_and_invalid`
- `stress_register_unregister_interleaved_with_stw`
- `stress_unregister_during_stw_wait_does_not_deadlock`

## `crates/dotnet-vm/src/threading/canonical.rs`

- `canonical_homing_cancel_before_service`
- `canonical_homing_cancel_after_publication`
- `canonical_homing_simultaneous_requests_share_one_materialization`
- `canonical_homing_shutdown`
- `canonical_homing_failure_is_terminal_for_the_key`
- `canonical_homing_owner_never_queues`

## `crates/dotnet-vm/src/threading/local.rs`

- `managed_ids_are_monotonic_and_do_not_wrap`
- `managed_ids_are_unique_under_loom`
- `records_are_parent_local_and_start_unstarted`

## `crates/dotnet-vm/src/threading/pool.rs`

- `reservations_are_capped_and_idle_workers_are_reused`
- `requests_between_probe_registration_and_park_are_not_lost`

## `crates/dotnet-vm/src/threading/events.rs`

- `event_grants_deadlines_and_cancellation_are_serialized`

## `crates/dotnet-vm/src/threading/workers.rs`

- `completion_observation_waits_for_owner_cleanup`
- `completion_wait_wakes_on_publication_and_after_publication`
- `completed_outcomes_remain_consumable_after_retirement`
- `abort_prevents_retired_worker_from_starting_finalizers`
- `finalizing_preserves_managed_completion`
- `shutdown_requires_retirement_and_records_failed_drain`
- `quiescence_requires_no_running_or_finalizing_worker_and_idle_finalizers`
- `foreground_exit_cancels_only_after_foreground_and_finalizers_are_idle`
- `idle_finalizer_thread_does_not_cancel_other_entrypoints`
- `shutdown_joins_worker_started_after_signal`
- `worker_registry_publishes_and_reaps_success_and_panic`

## `crates/dotnet-vm/src/statics.rs`

- `wait_graph_edge_guard_removes_edge_on_scope_exit`
- `wait_graph_edge_guard_removes_edge_during_unwind`
- `owned_wait_registration_removes_transferred_edge_on_unwind`
- `wait_graph_preserves_nested_edges_and_detects_cycles`
- `initialization_transition_table`
- `initializer_claim_drop_and_explicit_completion`

## `crates/dotnet-vm/src/state.rs`

- `first_fatal_exception_requests_abort_and_is_preserved`
- `finalizer_publication_counts_arenas_once`
- `cloned_signal_requests_abort_on_shared_flag`
- `construction_reports_every_registry_cache_once`

## `crates/dotnet-vm/src/executor.rs`

- `entrypoint_return_exits_while_finalizers_pending`

## `crates/dotnet-vm/src/intrinsics/mod.rs`

- `blocking_intrinsics_have_no_sync_fallback`
- `async_only_intrinsic_signatures_are_transportable`
- `numeric_handlers_check_declared_type_slots`

## `crates/dotnet-vm/src/intrinsics/metadata.rs`

- `marvin_seed_uses_intrinsic_only_in_fuzzing`
- `unregistered_intrinsic_runs_body_unless_it_only_recurses`
- `span_copy_methods_are_not_intercepted`
- `array_data_reference_overloads_are_intercepted`
- `math_and_mathf_bodiless_methods_have_intrinsic_handlers`
- `test_intrinsic_kind_categories`
- `test_metadata_constructors`
- `test_intrinsic_classification_categories`
- `test_intrinsic_metadata_reason_field`
- `test_intrinsic_metadata_handler_storage`

## `crates/dotnet-vm/src/intrinsics/gc.rs`

- `gc_handle_slots_reserve_tag_bits_and_reject_invalid_values`

## `crates/dotnet-vm/src/stack/synchronized.rs`

- `unfinished_acquisition_holds_nothing`
- `completed_acquisition_holds_the_monitor`

## `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs`

- `owned_origin_typed_access_checks_complete_range`
- `inline_typed_access_rejects_out_of_bounds_and_overflow`

## Helpers and compile assertions

`src/macros.rs::{make_test_method,make_test_assembly}`, `src/test_utils.rs` via the other VM partition, and `state.rs` AbortSignal Send/Sync assertion. Worker test wrappers share the production lifecycle implementation. File/module gates and standalone Loom consumers are described in vm.md.
