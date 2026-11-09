#!/usr/bin/env python3
"""Exercise the actual cross-arena matrix scripts without running Miri."""

import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = (ROOT / '.github/workflows/ci.yml').read_text()
JOB = re.search(r'^  miri-cross-arena:\n(.*?)(?=^  [\w-]+:\n)', WORKFLOW, re.M | re.S).group(1)
BASE = 'executor::executor_teardown_tests::'
MT_TEARDOWN = {
    'retired_owner_keeps_registration_until_shutdown',
    'retired_shutdown_waits_for_inflight_collection_and_lease',
    'worker_panic_drains_lease_before_shutdown',
    'retired_self_held_lease_leaks_storage_at_shutdown',
    'worker_start_failure_retires_and_shutdown_reaps',
    'executor_normal_teardown_active_lease',
    'executor_extracted_teardown_active_lease',
    'executor_unwind_teardown_active_lease',
    'executor_nested_creation_fails_closed',
    'executor_self_held_lease_leaks_instead_of_deadlocking',
    'executor_external_remover_does_not_release_owner',
    'extracted_executor_tls_ownership',
    'ownership::forgotten_executor_tls_exit_retains_arena',
    'ownership::repeated_extraction_reserves_until_owner_drop',
    'ownership::constructor_rollback_releases_reservation',
    'ownership::executor_teardown_panic_retains_arena',
    'ownership::owner_first_guard_does_not_touch_successor',
    'ownership::self_held_drain_panic_and_no_diagnostic',
    'ownership::drain_warning_panic_retains_arena',
}
GUARD = 'test result: ok. 1 passed; 0 failed; 0 ignored;'

LIBRARY_BOTH = {
    ('dotnet-value', 'object_tests::tests::publication_pending_survival'),
    ('dotnet-value', 'pointer::tests::arena_identity_origin_aba'),
}
LIBRARY_ST = LIBRARY_BOTH | {
    ('dotnet-value', 'object_tests::tests::codec_local_invalid_control'),
    ('dotnet-value', 'object::tests::codec_short_storage'),
}
LIBRARY_MT = LIBRARY_BOTH | {
    ('dotnet-value', 'object_tests::tests::codec_id40000'),
    ('dotnet-value', 'object_tests::tests::codec_registered_boundaries'),
    ('dotnet-utils', 'gc::cross_arena::tests::arena_lifetime_transport'),
    ('dotnet-utils', 'gc::cross_arena::tests::arena_lifetime_transport_permissive_control'),
} | {('dotnet-utils', 'gc::cross_arena::tests::arena_identity_registry_' + suffix)
     for suffix in ('duplicate', 'fast_bit_transaction', 'generation_exhaustion',
                    'identity_token_reuse', 'drain_reuse_control', 'checked_counter_control')}


BRIDGE = {'effects::tests::' + name for name in (
    'nested_entry_rejected', 'panic_resets_entry_and_installation',
    'condition_replies_require_matching_effect_and_generation',
    'initializer_replies_require_matching_effect_and_generation',
    'continuation_polls_share_one_turn_grant_budget',
    'rooted_frame_home_descriptor_preserves_aliases_and_rejects_retired_home',
    'rooted_heap_pointer_descriptor_keeps_owner_alive')}
NATIVE = {'executor::native_substring_tests::' + name for name in (
    'native_activation_real_context_and_collection',
    'reflection_retry_completed_collection_does_not_retry',
    'condition_wait_tests::monitor_heap_byref_survives_stop_before_dispatch',
    'condition_wait_tests::monitor_byref_dispatch_writes_frame_home',
    'condition_wait_tests::monitor_timed_byref_alias_writes_once_for_timeout_and_acquisition',
    'condition_wait_tests::condition_wait_executor_runs_pending_finalizer_before_resuming',
    'condition_wait_tests::finalizer_wait_from_finalizer_returns_immediately',
    'finalizer_segment_resolves_parked_homes_and_preserves_serials',
    'finalizer_preserves_filter_and_suspended_native_context',
    'native_activation_nested_return_is_copied',
    'native_capture_filters_and_child_cleanup_precede_outer_search',
    'legacy_initializer_frame_retirement_fails_claim',
    'native_initializer_failure_is_published_with_original_inner',
    'native_error_retires_initializer_frame_before_future_drop',
    'static_store_operand_released_after_abandonment',
    'native_activation_calls_suspending_intrinsic',
    'native_activation_rejects_reply_from_wrong_root_set',
    'native_activation_forgotten_access_is_terminal',
    'native_activation_unregistered_pending_is_terminal',
    'native_activation_rejects_each_changed_caller_component',
    'native_activation_poll_panic_unwinds_and_releases_roots')}
FOREIGN = {'executor::native_substring_tests::' + name for name in (
    'condition_wait_tests::finalizer_wait_rechecks_after_another_owner_finishes',
    'condition_wait_tests::retired_owner_remains_registered_while_other_arena_has_queued_finalizer',
    'legacy_initializer_host_error_releases_waiter',
    'worker_teardown_cleans_native_before_retiring_caller',
    'foreign_source_survives_coordinated_collections_between_chunks',
    'native_initializer_failure_survives_owner_retirement_and_collection',
    'native_initializer_wait_rechecks_cycle_after_real_collection',
    'native_initializer_wait_replies_before_collection_when_owner_is_busy',
    'condition_wait_tests::monitor_heap_byref_survives_collection_and_writes_owner',
    'condition_wait_tests::monitor_byref_contended_dispatch_survives_collection',
    'condition_wait_tests::condition_wait_deadline_survives_real_collection',
    'condition_wait_tests::condition_wait_foreign_monitor_owner_survives_collection',
    'native_activation_foreign_child_return_survives_collection',
    'native_foreign_byref_owner_survives_collection_and_child_call',
    'native_cross_arena_byref_owner_survives_collection_and_child_call')}
COLLECTION_MT = {'executor::executor_collection_tests::' + name for name in (
    'arena_lifetime_vm_collection_fixed_point_payload_and_reclamation',
    'arena_lifetime_vm_collection',
    'arena_lifetime_vm_collection_unrooted_negative_control')}


def matrix_rows():
    rows = re.findall(r'^          - \{ ([^\n]+) \}$', JOB, re.M)
    return [dict(part.strip().split(': ', 1) for part in row.split(', ')) for row in rows]


def script(name):
    match = re.search(r'^      - name: ' + re.escape(name) +
                      r'\n(?:[^\n]*\n)*?^        run: \|\n((?:^          .*\n|^[ \t]*\n)+)', JOB, re.M)
    if not match:
        raise AssertionError('missing script ' + name)
    return textwrap.dedent(match.group(1))


def execute(row, name='Execute guarded singletons'):
    source = script(name)
    for key, value in row.items():
        source = source.replace('${{ matrix.' + key + ' }}', value)
    with tempfile.TemporaryDirectory() as tmp:
        fake = Path(tmp) / 'python3'
        fake.write_text('#!/usr/bin/env python3\nimport json, os, sys\n'
                        'with open(os.environ["RECORD"], "a") as f: '
                        'f.write(json.dumps(sys.argv[1:]) + "\\n")\n')
        # Use an absolute interpreter so the fake python3 does not call itself.
        fake.write_text(fake.read_text().replace('/usr/bin/env python3', '/usr/bin/python3'))
        fake.chmod(0o755)
        log = Path(tmp) / 'calls.jsonl'
        env = dict(os.environ, PATH=tmp + os.pathsep + os.environ['PATH'], RECORD=str(log))
        result = subprocess.run(['bash', '-eo', 'pipefail', '-c', source], env=env,
                                capture_output=True, text=True)
        if result.returncode:
            raise AssertionError(result.stdout + result.stderr)
        return [json.loads(line) for line in log.read_text().splitlines()]


class CrossArenaManifestTests(unittest.TestCase):
    def test_matrix_teardown_is_exact_and_mt_only(self):
        rows = matrix_rows()
        self.assertEqual(len(rows), 97)
        self.assertEqual(len({(r['configuration'], r['group'], r['shard']) for r in rows}), len(rows))
        teardown = [r for r in rows if r['group'] == 'vm-teardown']
        st = [r for r in teardown if r['configuration'] == 'no-features']
        mt = [r for r in teardown if r['configuration'] == 'multithreading']
        self.assertEqual([r['selector'] for r in st], [BASE + 'executor_single_arena_cleanup_control'])
        self.assertEqual(len(mt), len(MT_TEARDOWN))
        self.assertEqual({r['selector'] for r in mt}, {BASE + n for n in MT_TEARDOWN})
        self.assertIn("if: needs.select-deep.outputs.cross_arena != 'false'", JOB)
        self.assertIn('needs: [fast-core, select-deep]', JOB)
        self.assertIn('fail-fast: false', JOB)
        self.assertIn('${{ matrix.configuration }}-${{ matrix.group }}-${{ matrix.shard }}', JOB)
        self.assertEqual(JOB.count('${{ matrix.configuration }}-${{ matrix.group }}-${{ matrix.shard }}'), 2)
        self.assertIn('MIRIFLAGS: "-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"', JOB)
        absence = JOB[JOB.index('      - name: Verify no-features registry absence'):
                      JOB.index('      - name: Execute guarded singletons')]
        self.assertIn("matrix.configuration == 'no-features' && matrix.group == 'library'", absence)
        self.assertIn("--require '0 tests, 0 benchmarks'", absence)
        self.assertIn('arena_lifetime_transport arena_identity_registry_', absence)

    def test_native_rows_compile_the_selected_crate(self):
        for row in matrix_rows():
            if row['group'] not in ('native-bridge', 'vm-native'):
                continue
            with self.subTest(row=row):
                calls = execute(row, 'Compile singleton crates')
                self.assertEqual(len(calls), 1)
                cargo = calls[0][calls[0].index('--') + 1:]
                self.assertEqual(cargo[cargo.index('-p') + 1],
                                 'dotnet-vm-ops' if row['group'] == 'native-bridge' else 'dotnet-vm')
                self.assertIn('--no-run', cargo)
                self.assertIn('--no-default-features', cargo)
                self.assertEqual('--features' in cargo, row['configuration'] == 'multithreading')

    def test_each_row_executes_guarded_exact_selectors(self):
        seen = {'no-features': set(), 'multithreading': set()}
        groups = set()
        for row in matrix_rows():
            with self.subTest(row=row):
                library = row['group'] == 'library'
                self.assertEqual((row['job_timeout'], row['invocation_timeout']),
                                 ('30', '900') if library else ('45', '1800'))
                calls = execute(row)
                if library:
                    expected_count = 12 if row['configuration'] == 'multithreading' else 4
                elif row['group'] == 'vm-collection' and row['configuration'] == 'multithreading':
                    expected_count = 3
                else:
                    expected_count = 1
                self.assertEqual(len(calls), expected_count)
                for args in calls:
                    split = args.index('--')
                    options, cargo = args[:split], args[split + 1:]
                    self.assertEqual(options[:3], ['scripts/ci_diagnostics.py', 'run', '--phase'])
                    self.assertEqual(options[3], 'execute')
                    self.assertEqual(options[options.index('--require') + 1], GUARD)
                    self.assertEqual(options[options.index('--timeout') + 1], row['invocation_timeout'])
                    self.assertEqual(options[options.index('--features') + 1], row['configuration'])
                    self.assertEqual(cargo[:3], ['cargo', '+nightly-2026-05-27', 'miri'])
                    self.assertIn('--lib', cargo)
                    self.assertIn('--no-default-features', cargo)
                    self.assertEqual('--features' in cargo, row['configuration'] == 'multithreading')
                    if row['configuration'] == 'multithreading':
                        self.assertEqual(cargo[cargo.index('--features') + 1], 'multithreading')
                    self.assertEqual(cargo[-3:], ['--', '--exact', '--test-threads=1'])
                    crate = cargo[cargo.index('-p') + 1]
                    if library:
                        expected_crates = {'dotnet-value', 'dotnet-utils'}
                    else:
                        expected_crates = {'dotnet-vm-ops' if row['group'] == 'native-bridge' else 'dotnet-vm'}
                    self.assertIn(crate, expected_crates)
                    pair = (crate, cargo[-4])
                    self.assertNotIn(pair, seen[row['configuration']])
                    seen[row['configuration']].add(pair)
                    if row['group'] in ('vm-teardown', 'native-bridge', 'vm-native'):
                        self.assertEqual(cargo[-4], row['selector'])
                if row['group'] == 'vm-stress':
                    self.assertEqual(calls[0][-4], BASE + 'arena_lifetime_teardown_stress')
                if row['group'] == 'vm-collection':
                    expected = (COLLECTION_MT if row['configuration'] == 'multithreading' else
                                {'executor::executor_collection_tests::arena_lifetime_vm_collection'})
                    self.assertEqual({c[-4] for c in calls}, expected)
                if library:
                    actual = {(c[c.index('-p') + 1], c[-4]) for c in calls}
                    self.assertEqual(actual, LIBRARY_MT if row['configuration'] == 'multithreading' else LIBRARY_ST)
                if row['group'] not in ('vm-teardown', 'native-bridge', 'vm-native'):
                    groups.add((row['configuration'], row['group']))
        self.assertEqual(groups, {(config, group) for config in ('no-features', 'multithreading')
                                  for group in ('library', 'vm-collection', 'vm-stress')})

        for config in seen:
            bridge = {selector for crate, selector in seen[config] if crate == 'dotnet-vm-ops'}
            native = {selector for crate, selector in seen[config]
                      if crate == 'dotnet-vm' and selector.startswith('executor::native_substring_tests::')}
            self.assertEqual(bridge, BRIDGE)
            self.assertEqual(native, NATIVE | (FOREIGN if config == 'multithreading' else set()))
        self.assertEqual(len(seen['no-features']), 35)
        self.assertEqual(len(seen['multithreading']), 78)

    def test_strict_core_is_isolated_and_guarded(self):
        strict = JOB[JOB.index('      - name: Isolated strict-provenance transport core'):
                     JOB.index('      - name: Summarize diagnostics')]
        self.assertIn("matrix.configuration == 'multithreading' && matrix.group == 'library'", strict)
        self.assertIn('-Zmiri-ignore-leaks -Zmiri-strict-provenance', strict)
        self.assertNotIn('-Zmiri-strict-provenance', JOB[:JOB.index('      - name: Isolated strict-provenance transport core')])
        self.assertIn("--timeout 900", strict)
        self.assertIn("--require '" + GUARD + "'", strict)
        self.assertIn('pointer::tests::arena_lifetime_transport_strict_core -- --exact --test-threads=1', strict)


if __name__ == '__main__':
    unittest.main()
