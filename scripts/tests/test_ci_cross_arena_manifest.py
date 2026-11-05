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


def matrix_rows():
    rows = re.findall(r'^          - \{ ([^\n]+) \}$', JOB, re.M)
    return [dict(part.strip().split(': ', 1) for part in row.split(', ')) for row in rows]


def script(name):
    match = re.search(r'^      - name: ' + re.escape(name) +
                      r'\n(?:[^\n]*\n)*?^        run: \|\n((?:^          .*\n|^[ \t]*\n)+)', JOB, re.M)
    if not match:
        raise AssertionError('missing script ' + name)
    return textwrap.dedent(match.group(1))


def execute(row):
    source = script('Execute guarded singletons')
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
        self.assertEqual(len(rows), 21)
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

    def test_each_row_executes_guarded_exact_selectors(self):
        seen = {'no-features': set(), 'multithreading': set()}
        groups = set()
        for row in matrix_rows():
            with self.subTest(row=row):
                library = row['group'] == 'library'
                self.assertEqual((row['job_timeout'], row['invocation_timeout']),
                                 ('30', '900') if library else ('45', '1800'))
                calls = execute(row)
                self.assertEqual(len(calls), (12 if row['configuration'] == 'multithreading' else 4) if library else 1)
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
                    self.assertIn(crate, {'dotnet-value', 'dotnet-utils'} if library else {'dotnet-vm'})
                    pair = (crate, cargo[-4])
                    self.assertNotIn(pair, seen[row['configuration']])
                    seen[row['configuration']].add(pair)
                    if row['group'] == 'vm-teardown':
                        self.assertEqual(cargo[-4], row['selector'])
                if row['group'] == 'vm-stress':
                    self.assertEqual(calls[0][-4], BASE + 'arena_lifetime_teardown_stress')
                if row['group'] == 'vm-collection':
                    suffix = ('arena_lifetime_vm_collection_fixed_point_payload_and_reclamation'
                              if row['configuration'] == 'multithreading' else 'arena_lifetime_vm_collection')
                    self.assertEqual(calls[0][-4], 'executor::executor_collection_tests::' + suffix)
                if library:
                    actual = {(c[c.index('-p') + 1], c[-4]) for c in calls}
                    self.assertEqual(actual, LIBRARY_MT if row['configuration'] == 'multithreading' else LIBRARY_ST)
                if row['group'] != 'vm-teardown':
                    groups.add((row['configuration'], row['group']))
        self.assertEqual(groups, {(config, group) for config in ('no-features', 'multithreading')
                                  for group in ('library', 'vm-collection', 'vm-stress')})

        self.assertEqual(len(seen['no-features']), 7)
        self.assertEqual(len(seen['multithreading']), 28)

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
