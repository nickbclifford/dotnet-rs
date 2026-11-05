#!/usr/bin/env python3
"""Safe contract tests: every Cargo invocation is replaced by a fake executable."""
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/pointer_provenance.sh"
SOURCE = ROOT / "crates/dotnet-vm/src/pointer_provenance_tests.rs"
# Edit this mapping alongside the catalog and runner table when promoting a probe.
CONTROLS_BY_SHARD = {
    1: ["p00_smoke"],
    2: ["p02_ldloca_stind_i4_control"],
    3: ["p03_ldloca_convu_ldind_i4"],
    4: ["p05_single_localloc_roundtrip"],
    5: ["p07_localloc_one_past_end"],
    6: ["p11_ldelema_stind_heap_array_control"],
    7: ["p10_ldelema_convu_stind_heap_array"],
    8: ["p20_heap_convu_then_typed_write_then_raw_store"],
    9: ["p13_ldelema_managed_add_far_out_of_bounds_then_conv_u"],
    10: ["p14_initblk_heap_array_via_managed_ptr"],
    11: ["p16_cpblk_heap_arrays_via_managed_ptr"],
    12: ["p_null_through_pointer_local_address"],
    13: ["p06_localloc_minus_one_then_plus_one"],
    14: ["p08_localloc_overshoot"],
    15: ["p17_localloc_ptr_ceq_self"],
    16: ["p18_localloc_ptr_clt"],
    17: ["p21_localloc_signed_extreme_offsets_roundtrip"],
    18: ["p22_localloc_wrap_to_zero_roundtrip"],
    19: ["p23_localloc_pointer_loop_unsigned_branch_compare"],
    20: ["p24_unsafe_byte_offset_direction_and_deref"],
    21: ["p25_convi_and_stind_i_pointer_roundtrip_deref"],
    22: ["p26_native_int_boundary_store_load_roundtrip"],
    23: ["p27_stind_i_fresh_pointer_through_local_slot_dereferences"],
    24: ["p28_stind_i_fresh_pointer_through_heap_memory_dereferences"],
    25: ["p04_double_localloc_write_first"],
    26: ["p29_localloc_blocks_survive_growth_and_are_16_byte_aligned"],
    27: ["p30_localloc_block_survives_nested_callee_localloc"],
    28: ["p31_localloc_blocks_released_on_return_and_pooling"],
    29: ["p32_localloc_blocks_released_on_tail_call_replacement"],
    30: ["p33_localloc_blocks_released_on_exception_unwind"],
    31: ["p34_localloc_blocks_released_on_executor_teardown"],
    32: ["p35_ldflda_on_stack_value_type_then_ldind"],
    33: ["p36_static_address_survives_forced_collection"],
}
CONTROLS = [name for names in CONTROLS_BY_SHARD.values() for name in names]
DIAGNOSTICS = ['p01_ldloca_convu_stind_i4', 'p09_ldloca_convu_survives_callee_stack_growth',
               'p12_ldsflda_convu_stind_static', 'p15_initblk_stack_local_via_managed_ptr',
               'p19_ldelem_negative_index', 'p_wide_store_into_int_local']
# Edit alongside the runner table's batch column and the control_batch! invocations.
BATCHES = {
    1: [
        'p00_smoke',
        'p07_localloc_one_past_end',
        'p13_ldelema_managed_add_far_out_of_bounds_then_conv_u',
        'p06_localloc_minus_one_then_plus_one',
        'p21_localloc_signed_extreme_offsets_roundtrip',
        'p25_convi_and_stind_i_pointer_roundtrip_deref',
        'p04_double_localloc_write_first',
        'p32_localloc_blocks_released_on_tail_call_replacement',
        'p36_static_address_survives_forced_collection',
    ],
    2: [
        'p02_ldloca_stind_i4_control',
        'p11_ldelema_stind_heap_array_control',
        'p14_initblk_heap_array_via_managed_ptr',
        'p08_localloc_overshoot',
        'p22_localloc_wrap_to_zero_roundtrip',
        'p26_native_int_boundary_store_load_roundtrip',
        'p29_localloc_blocks_survive_growth_and_are_16_byte_aligned',
        'p33_localloc_blocks_released_on_exception_unwind',
    ],
    3: [
        'p03_ldloca_convu_ldind_i4',
        'p10_ldelema_convu_stind_heap_array',
        'p16_cpblk_heap_arrays_via_managed_ptr',
        'p17_localloc_ptr_ceq_self',
        'p23_localloc_pointer_loop_unsigned_branch_compare',
        'p27_stind_i_fresh_pointer_through_local_slot_dereferences',
        'p30_localloc_block_survives_nested_callee_localloc',
        'p34_localloc_blocks_released_on_executor_teardown',
    ],
    4: [
        'p05_single_localloc_roundtrip',
        'p20_heap_convu_then_typed_write_then_raw_store',
        'p_null_through_pointer_local_address',
        'p18_localloc_ptr_clt',
        'p24_unsafe_byte_offset_direction_and_deref',
        'p28_stind_i_fresh_pointer_through_heap_memory_dereferences',
        'p31_localloc_blocks_released_on_return_and_pooling',
        'p35_ldflda_on_stack_value_type_then_ldind',
    ],
}
BATCH_CONTROLS = [name for names in BATCHES.values() for name in names]
FAKE = r'''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
a = sys.argv[1:]
with open(os.environ['CALLS'], 'a') as f:
    f.write(json.dumps({'args': a, 'flags': os.environ.get('MIRIFLAGS')}) + '\n')
names = ['pointer_provenance_tests::' + n for n in json.loads(os.environ['CONTROLS'])]
mode = os.environ.get('MODE', '')
diagnostics = ['pointer_provenance_tests::' + n for n in json.loads(os.environ['DIAGNOSTICS'])]
if mode in ('new-enabled', 'new-ignored'):
    names.append('pointer_provenance_tests::p99_new_probe')
batches = {int(k): v for k, v in json.loads(os.environ['BATCHES']).items()}
batch_tests = ['pointer_provenance_tests::batches::batch_%d' % k for k in batches]
if '--list' in a:
    if mode == 'missing': names.pop(0)
    if mode == 'typo': names[0] += '_typo'
    if mode == 'duplicate': names.append(names[0])
    for n in names + diagnostics + batch_tests: print(n + ': test')
elif 'nextest' in a:
    cases = {n: {'kind': 'test', 'ignored': mode == 'ignored' and i == 0,
                 'filter-match': {'status': 'mismatch' if mode == 'filter-mismatch' and i == 0 else 'matches'}}
             for i, n in enumerate(names)}
    if mode == 'new-ignored':
        cases[names[-1]].update(ignored=True, **{'filter-match': {'status': 'mismatch', 'reason': 'ignored'}})
    cases.update({n: {'kind': 'test', 'ignored': True, 'filter-match': {'status': 'mismatch', 'reason': 'ignored'}}
                  for n in diagnostics})
    cases.update({n: {'kind': 'test', 'ignored': False, 'filter-match': {'status': 'matches'}} for n in batch_tests})
    print(json.dumps({'rust-suites': {'dotnet-vm': {'package-name': 'dotnet-vm', 'kind': 'lib',
          'status': 'skipped' if mode == 'not-listed' else 'listed', 'testcases': cases}}}))
else:
    name = a[a.index('--exact') + 1]
    print('fake stderr marker', file=sys.stderr)
    if '::batches::batch_' in name:
        assert '--nocapture' in a and '--ignored' not in a
        members = ['pointer_provenance_tests::' + n for n in batches[int(name.rsplit('_', 1)[1])]]
        if mode == 'batch-zero':
            print('test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out'); sys.exit(0)
        if mode == 'batch-ignored':
            print('test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 1 filtered out'); sys.exit(0)
        print('test ' + name + ' ... ', end='')
        done = members
        if mode == 'batch-missing-line': done = members[:2] + members[3:]
        if mode == 'batch-extra-line': done = members + ['pointer_provenance_tests::p01_ldloca_convu_stind_i4']
        if mode == 'batch-reordered': done = [members[1], members[0]] + members[2:]
        stop_at = 3 if mode in ('batch-fail-mid', 'batch-ub-mid', 'batch-timeout') else None
        for i, n in enumerate(done):
            print('batch-control start ' + n)
            if i == stop_at:
                if mode == 'batch-ub-mid':
                    print('error: Undefined Behavior: fake'); sys.exit(1)
                if mode == 'batch-timeout': sys.exit(124)
                print("thread panicked at fake assertion `left == right` failed")
                print('test ' + name + ' ... FAILED')
                print('test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out'); sys.exit(101)
            print('batch-control ok ' + n)
        print('ok')
        print('test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out')
        sys.exit(0)
    if mode == 'fail-first' and name.endswith('p01_ldloca_convu_stind_i4'):
        print('error: Undefined Behavior: fake'); sys.exit(1)
    if mode == 'setup-first' and name.endswith('p01_ldloca_convu_stind_i4'):
        print('error: could not compile fake'); sys.exit(101)
    if mode == 'panic-first' and name.endswith('p01_ldloca_convu_stind_i4'):
        print('thread panicked at fake assertion'); sys.exit(101)
    if mode == 'timeout-first' and name.endswith('p01_ldloca_convu_stind_i4'):
        sys.exit(124)
    if mode == 'zero':
        print('test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out'); sys.exit(0)
    if mode == 'ignored-run':
        print('test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out'); sys.exit(0)
    if mode == 'inline-warning':
        print('test ' + name + ' ... warning: integer-to-pointer cast')
        print('  warning details and backtrace')
        print('ok')
    else:
        print('test ' + name + ' ... ok')
    print('test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out')
'''


class PointerScriptTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.path = Path(self.tmp.name)
        cargo = self.path / 'cargo'
        cargo.write_text(FAKE)
        cargo.chmod(0o755)
        self.calls = self.path / 'calls'
        self.script = SCRIPT
        self.env = dict(os.environ, PATH=str(self.path) + os.pathsep + os.environ['PATH'],
                        CALLS=str(self.calls), CONTROLS=json.dumps(CONTROLS), BATCHES=json.dumps(BATCHES),
                        DIAGNOSTICS=json.dumps(['p01_ldloca_convu_stind_i4', 'p09_ldloca_convu_survives_callee_stack_growth',
                                                'p12_ldsflda_convu_stind_static', 'p15_initblk_stack_local_via_managed_ptr',
                                                'p19_ldelem_negative_index', 'p_wide_store_into_int_local']), TMPDIR=str(self.path))

    def invoke(self, *args, mode='', ok=True):
        env = dict(self.env, MODE=mode)
        result = subprocess.run(['bash', str(self.script), *args], cwd=ROOT, env=env,
                                text=True, capture_output=True, timeout=30)
        self.assertEqual(result.returncode == 0, ok, result.stdout + result.stderr)
        return result.stdout + result.stderr

    def recorded(self):
        return [json.loads(line) for line in self.calls.read_text().splitlines()] if self.calls.exists() else []

    def test_catalog_matches_source_and_shards(self):
        output = self.invoke('list')
        rows = [line.split('|') for line in output.splitlines()[1:]]
        text = SOURCE.read_text()
        source = re.findall(r'^fn (p(?:\d\d_|_)[a-z0-9_]+)\(', text, re.M)
        attributes = {
            name: attrs for attrs, name in re.findall(
                r'((?:^#\[[^\]]*\]\n)+)fn (p(?:\d\d_|_)[a-z0-9_]+)\(', text, re.M,
            )
        }
        self.assertEqual(set(attributes), set(source))
        ignored = {name for name, attrs in attributes.items()
                   if re.search(r'^#\[ignore(?:\s*=\s*"[^\n]+")?\]$', attrs, re.M)}
        self.assertEqual(ignored, set(source) - set(CONTROLS))
        # p13 and the conv.u8 far-OOB regression are conditionally ignored:
        # runner configurations omit this feature, while validation-all
        # intentionally triggers their debug bounds trap.
        all_attributes = {
            name: attrs for attrs, name in re.findall(
                r'((?:^#\[[^\]]*\]\n)+)fn ([a-z][a-z0-9_]+)\(', text, re.M,
            )
        }
        conditional = {name: attr for name, attrs in all_attributes.items()
                       for attr in re.findall(r'^#\[cfg_attr\(([^\]]*\bignore\b[^\]]*)\)\]$', attrs, re.M)}
        self.assertEqual(set(conditional), {
            'p13_ldelema_managed_add_far_out_of_bounds_then_conv_u',
            'conv_u8_after_far_oob_managed_add_does_not_panic',
        })
        self.assertRegex(
            conditional['p13_ldelema_managed_add_far_out_of_bounds_then_conv_u'],
            r'^\s*feature = "memory-validation",\s*ignore = "p13 deliberately forms an out-of-bounds managed address; memory-validation\'s debug bound trap rejects it by design \(F-TEST-003\)"\s*$',
        )
        self.assertRegex(
            conditional['conv_u8_after_far_oob_managed_add_does_not_panic'],
            r'^\s*feature = "memory-validation",\s*ignore = "conv.u8 regression deliberately forms an out-of-bounds managed address; memory-validation\'s debug bound trap rejects it by design \(F-TEST-003\)"\s*$',
        )
        self.assertEqual({r[0].split('::')[-1] for r in rows}, set(source))
        self.assertEqual(len(set(r[0] for r in rows)), len(rows))
        self.assertEqual({int(r[3]) for r in rows if r[1] == 'control'}, set(CONTROLS_BY_SHARD))
        for shard, names in CONTROLS_BY_SHARD.items():
            self.assertEqual([r[0].split('::')[-1] for r in rows if r[1] == 'control' and r[3] == str(shard)], names)
        self.assertEqual([r[0].split('::')[-1] for r in rows if r[2] == 'yes'],
                         ['p19_ldelem_negative_index'])
        self.assertEqual(self.recorded(), [])

    def test_singleton_partition_is_one_control_per_shard(self):
        self.assertTrue(all(len(names) == 1 for names in CONTROLS_BY_SHARD.values()))
        self.assertEqual(list(CONTROLS_BY_SHARD), list(range(1, 34)))
        self.assertEqual(len(set(CONTROLS)), 33)
        for config in ('no-features', 'multithreading'):
            for invalid in ('0', '34'):
                self.invoke('controls', '--configuration', config, '--shard', invalid, '--dry-run', ok=False)
        self.assertEqual(self.recorded(), [])

    def test_guard_good_and_stale_or_ignored(self):
        for config in ('no-features', 'multithreading'):
            self.assertIn('33 exact non-ignored controls', self.invoke('verify-controls', '--configuration', config))
            for mode in ('missing', 'typo', 'duplicate', 'ignored', 'filter-mismatch', 'not-listed', 'new-enabled'):
                self.assertIn('guard failed', self.invoke('verify-controls', '--configuration', config, mode=mode, ok=False))
        self.assertIn('33 exact non-ignored controls', self.invoke('verify-controls', '--configuration', 'no-features', mode='new-ignored'))
        for call in self.recorded():
            self.assertNotIn('--ignored', call['args'])

    def test_caller_log_directory_retains_both_streams_and_failure(self):
        logdir = self.path / 'durable logs'
        self.env['POINTER_PROVENANCE_LOG_DIR'] = str(logdir)
        args = ('controls', '--configuration', 'no-features', '--shard', '33')
        output = self.invoke(*args)
        log = (logdir / 'p36_static_address_survives_forced_collection.log').read_text()
        self.assertIn('fake stderr marker', log)
        self.assertIn('test result: ok. 1 passed;', log)
        self.assertIn('fake stderr marker', output)
        self.invoke(*args, mode='zero', ok=False)
        self.assertIn('0 passed', (logdir / 'p36_static_address_survives_forced_collection.log').read_text())

    def test_controls_count_and_guard(self):
        shard = next(iter(CONTROLS_BY_SHARD))
        names = CONTROLS_BY_SHARD[shard]
        base = ('controls', '--configuration', 'no-features', '--shard', str(shard))
        self.assertIn(f'total: {len(names)} passed, 0 failed', self.invoke(*base))
        calls = self.recorded()
        self.assertEqual(len(calls), 2 + len(names))
        self.assertTrue(all('--ignored' not in c['args'] for c in calls))
        self.assertEqual([c['args'][c['args'].index('--exact') + 1] for c in calls[2:]],
                         ['pointer_provenance_tests::' + c for c in names])
        for mode in ('zero', 'ignored-run'):
            self.assertIn('zero-ignored-or-unverified-count', self.invoke(*base, mode=mode, ok=False))
        before = len(self.recorded())
        self.invoke(*base, mode='ignored', ok=False)
        self.assertEqual(len(self.recorded()) - before, 2)  # no Miri after guard failure

    def test_each_control_shard_uses_exact_names_and_pinned_flags(self):
        for config in ('no-features', 'multithreading'):
            for shard, names in CONTROLS_BY_SHARD.items():
                self.calls.unlink(missing_ok=True)
                self.invoke('controls', '--configuration', config, '--shard', str(shard))
                calls = self.recorded()
                self.assertEqual(len(calls), 2 + len(names))
                self.assertTrue(all('--ignored' not in c['args'] for c in calls))
                self.assertEqual([c['args'][c['args'].index('--exact') + 1] for c in calls[2:]],
                                 ['pointer_provenance_tests::' + name for name in names])
                for call in calls[2:]:
                    self.assertIn('+nightly-2026-05-27', call['args'])
                    self.assertEqual(call['flags'], '-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks')
                    self.assertEqual('--features' in call['args'], config == 'multithreading')

    def test_reject_unknown_and_native_before_cargo(self):
        for args in [('--probe', 'p00_smok'), ('--probe', ''), ('--probe', 'p01_ldloca_convu_stind_i4', '--lane', 'native'),
                     ('--probe', 'p00_smoke'),
                     ('--all', '--lane', 'native'), ('--probe', 'pointer_provenance_tests::p01_ldloca_convu_stind_i4')]:
            self.invoke('diagnostic', '--configuration', 'no-features', *args, ok=False)
        for name in CONTROLS:
            self.invoke('diagnostic', '--configuration', 'no-features', '--probe', name, ok=False)
        self.assertEqual(self.recorded(), [])
        self.invoke('diagnostic', '--configuration', 'no-features', '--probe', 'p19_ldelem_negative_index', '--lane', 'native')
        call = self.recorded()[0]['args']
        self.assertIn('--ignored', call)
        self.assertNotIn('miri', call)

    def test_diagnostic_continues_and_distinguishes_failures(self):
        rows = [line.split('|') for line in self.invoke('list').splitlines()[1:]]
        diagnostics = {r[0].split('::')[-1] for r in rows if r[1] != 'control'}
        for mode, classification in [('fail-first', 'miri-ub'),
                                     ('setup-first', 'compile-setup-or-process-failure'),
                                     ('panic-first', 'assertion-or-panic'),
                                     ('timeout-first', 'timeout-or-killed')]:
            self.calls.unlink(missing_ok=True)
            output = self.invoke('diagnostic', '--configuration', 'multithreading', '--all', mode=mode, ok=False)
            self.assertIn(classification, output)
            self.assertIn(f'total: {len(diagnostics) - 1} passed, 1 failed', output)
            calls = self.recorded()
            self.assertEqual(len(calls), len(diagnostics))
            names = {c['args'][c['args'].index('--exact') + 1].split('::')[-1] for c in calls}
            self.assertEqual(names, diagnostics)
            self.assertTrue(names.isdisjoint(CONTROLS))
            for call in calls:
                self.assertIn('--ignored', call['args'])
                self.assertIn('--test-threads=1', call['args'])
                self.assertEqual(call['flags'], '-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks')
                self.assertIn('+nightly-2026-05-27', call['args'])
        output = self.invoke('diagnostic', '--configuration', 'no-features', '--probe', 'p01_ldloca_convu_stind_i4', mode='zero', ok=False)
        self.assertIn('zero-ignored-or-unverified-count', output)

    def test_diagnostic_recognizes_inline_warning_success(self):
        output = self.invoke('diagnostic', '--configuration', 'no-features',
                             '--probe', 'p01_ldloca_convu_stind_i4', mode='inline-warning')
        self.assertIn('total: 1 passed, 0 failed', output)
        calls = self.recorded()
        self.assertEqual(len(calls), 1)
        self.assertIn('--ignored', calls[0]['args'])
        self.assertEqual(calls[0]['args'][calls[0]['args'].index('--exact') + 1],
                         'pointer_provenance_tests::p01_ldloca_convu_stind_i4')

    def test_ci_shards_match_catalog(self):
        # Parse the specific inline matrix list without a PyYAML dependency. Fail
        # closed if its job or syntax changes instead of silently skipping it.
        workflow = (ROOT / '.github/workflows/ci.yml').read_text()
        job = re.search(r'^  miri-pointer-controls:\s*\n(?P<body>(?:^(?: {4,}.*|\s*)\n)*)', workflow, re.M)
        self.assertIsNotNone(job, 'missing miri-pointer-controls job')
        matrix = re.findall(r'^        shard: \[([0-9, ]+)\]\s*$', job.group('body'), re.M)
        self.assertEqual(len(matrix), 1, 'expected one inline shard list')
        ci_shards = [int(value.strip()) for value in matrix[0].split(',')]
        self.assertEqual(len(ci_shards), len(set(ci_shards)))
        rows = [line.split('|') for line in self.invoke('list').splitlines()[1:]]
        self.assertEqual(set(ci_shards), {int(row[3]) for row in rows if row[1] == 'control'})
        self.assertIn('key: miri-pointer-v2-nightly-2026-05-27-${{ matrix.configuration }}-${{ matrix.shard }}',
                      job.group('body'))

    def test_reject_invalid_control_table_before_cargo(self):
        original = SCRIPT.read_text()
        shard, names = next(iter(CONTROLS_BY_SHARD.items()))
        full = 'pointer_provenance_tests::' + names[0]
        table_row = f"'{full}|{shard}|1'"
        catalog_row = f"'{full}|control|no|{shard}'"
        mutations = (
            original.replace(table_row, ''),
            original.replace(table_row, table_row + '\n' + table_row),
            original.replace(catalog_row, catalog_row + '\n' + catalog_row),
            original.replace(catalog_row, f"'{full}|control|no|{max(CONTROLS_BY_SHARD) + 1}'"),
            re.sub(r'expected_controls=\(.*?\n\)', 'expected_controls=(\n)', original, count=1, flags=re.S),
        )
        self.script = self.path / 'runner.sh'
        for mutated in mutations:
            self.assertNotEqual(mutated, original)
            self.script.write_text(mutated)
            self.invoke('verify-controls', '--configuration', 'no-features', ok=False)
            self.assertEqual(self.recorded(), [])

    def test_simulated_promotion_and_stale_table(self):
        # Modify a temporary copy only: no real ignored probe is executed.
        original = SCRIPT.read_text()
        rows = [line.split('|') for line in self.invoke('list').splitlines()[1:]]
        candidate = next(r for r in rows if r[1] != 'control' and r[2] == 'no')
        full = candidate[0]
        name = full.split('::')[-1]
        row = "'" + '|'.join(candidate) + "'"
        table_end = ')\n\n# full name | classification'
        for shard in (next(iter(CONTROLS_BY_SHARD)), max(CONTROLS_BY_SHARD) + 1):
            with self.subTest(shard=shard):
                # A catalog-only promotion must fail before Cargo is invoked.
                catalog_only = original.replace(row, f"'{full}|control|no|{shard}'")
                self.script = self.path / 'runner.sh'
                self.script.write_text(catalog_only)
                self.calls.unlink(missing_ok=True)
                self.invoke('controls', '--configuration', 'no-features', '--shard', str(shard), ok=False)
                self.assertEqual(self.recorded(), [])
                # Even changing both catalogs cannot silently grow or pair the
                # reviewed 33 singleton controls; a promotion needs a review.
                self.script.write_text(catalog_only.replace(table_end, f"'{full}|{shard}|1'\n" + table_end))
                self.env['CONTROLS'] = json.dumps(CONTROLS + [name])
                self.invoke('controls', '--configuration', 'no-features', '--shard', str(shard), ok=False)
                self.assertEqual(self.recorded(), [])

    def test_batches_cover_each_control_exactly_once(self):
        self.assertEqual(sorted(BATCH_CONTROLS), sorted(CONTROLS))
        self.assertEqual(len(BATCH_CONTROLS), 33)
        self.assertEqual(list(BATCHES), [1, 2, 3, 4])
        for names in BATCHES.values():
            self.assertTrue(8 <= len(names) <= 9)
            self.assertTrue(set(names).isdisjoint(DIAGNOSTICS))
        # Execution order inside a batch is shard order (the order of the runner table).
        shard_of = {n: s for s, names in CONTROLS_BY_SHARD.items() for n in names}
        for names in BATCHES.values():
            self.assertEqual(names, sorted(names, key=shard_of.get))
        out = self.invoke('verify-batches')
        self.assertIn('33 reviewed controls in 4 batches', out)
        self.assertEqual(self.recorded(), [])  # static: no Cargo

    def test_batch_tests_are_outside_the_probe_discovery_namespace(self):
        text = SOURCE.read_text()
        self.assertEqual(re.findall(r'^    control_batch!\(batch_(\d+): \[', text, re.M), ['1', '2', '3', '4'])
        for number in BATCHES:
            self.assertNotRegex(f'pointer_provenance_tests::batches::batch_{number}', r'^pointer_provenance_tests::p')
        # The discovery guard still passes with batch tests present in the fake listing and metadata.
        self.assertIn('33 exact non-ignored controls', self.invoke('verify-controls', '--configuration', 'no-features'))

    def test_verify_batches_rejects_drift(self):
        original = SOURCE.read_text()
        first = BATCHES[1][0]
        second = BATCHES[1][1]
        mutations = {
            'dropped': original.replace(f'        {first},\n', '', 1),
            'duplicated': original.replace(f'        {first},\n', f'        {first},\n        {first},\n', 1),
            'moved': original.replace(f'        {first},\n', '', 1).replace(
                f'        {BATCHES[2][0]},\n', f'        {BATCHES[2][0]},\n        {first},\n', 1),
            'diagnostic': original.replace(f'        {first},\n', f'        {first},\n        p19_ldelem_negative_index,\n', 1),
            'unknown': original.replace(f'        {first},\n', f'        {first},\n        p99_new_probe,\n', 1),
            'reordered': original.replace(f'        {first},\n        {second},\n', f'        {second},\n        {first},\n', 1),
            'extra-batch': original.replace('    control_batch!(batch_4: [',
                                            '    control_batch!(batch_5: [\n        p00_smoke,\n    ]);\n\n    control_batch!(batch_4: ['),
            'no-module': original.replace('mod batches {', 'mod other_batches {', 1),
        }
        mutated_source = self.path / 'mutated.rs'
        for label, text in mutations.items():
            with self.subTest(label):
                self.assertNotEqual(text, original)
                mutated_source.write_text(text)
                self.env['POINTER_PROVENANCE_SOURCE'] = str(mutated_source)
                self.assertIn('batch drift guard failed', self.invoke('verify-batches', ok=False))
        # A table-side drift (batch column edited without the source) fails as well.
        self.env['POINTER_PROVENANCE_SOURCE'] = str(SOURCE)
        table = SCRIPT.read_text()
        row = f"pointer_provenance_tests::{first}|1|1'"
        self.assertIn(row, table)
        self.script = self.path / 'runner.sh'
        self.script.write_text(table.replace(row, f"pointer_provenance_tests::{first}|1|2'"))
        self.assertIn('batch drift guard failed', self.invoke('verify-batches', ok=False))
        self.assertEqual(self.recorded(), [])

    def test_batch_command_runs_one_exact_batch_with_pinned_flags(self):
        for config in ('no-features', 'multithreading'):
            for number in BATCHES:
                self.calls.unlink(missing_ok=True)
                out = self.invoke('batch', '--configuration', config, '--batch', str(number))
                self.assertIn('total: 1 passed, 0 failed', out)
                calls = self.recorded()
                self.assertEqual(len(calls), 3)  # list, nextest metadata, one Miri run
                run = calls[2]
                self.assertEqual(run['args'][run['args'].index('--exact') + 1],
                                 f'pointer_provenance_tests::batches::batch_{number}')
                self.assertEqual(run['args'].count('--exact'), 1)
                for flag in ('+nightly-2026-05-27', '--nocapture', '--test-threads=1', 'miri'):
                    self.assertIn(flag, run['args'])
                self.assertNotIn('--ignored', run['args'])
                self.assertEqual(run['flags'], '-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks')
                self.assertEqual('--features' in run['args'], config == 'multithreading')

    def test_batch_command_rejects_bad_arguments_before_cargo(self):
        for args in [('--batch', '0'), ('--batch', '5'), ('--batch', ''), ('--batch', '1', '--batch', '2'), ('--batch', 'x'),
                     ('--batch', '1', '--shard', '1'), ('--batch', '1', '--probe', 'p00_smoke'), ('--batch', '1', '--all'),
                     ('--batch', '1', '--lane', 'native'), ()]:
            self.invoke('batch', '--configuration', 'no-features', *args, ok=False)
        self.invoke('batch', '--batch', '1', ok=False)
        self.invoke('controls', '--configuration', 'no-features', '--shard', '1', '--batch', '1', ok=False)
        self.invoke('diagnostic', '--configuration', 'no-features', '--all', '--batch', '1', ok=False)
        self.invoke('verify-batches', '--configuration', 'no-features', ok=False)
        self.invoke('verify-batches', '--dry-run', ok=False)
        self.assertEqual(self.recorded(), [])

    def test_batch_failures_fail_and_name_the_control(self):
        names = ['pointer_provenance_tests::' + n for n in BATCHES[2]]
        cases = {
            'batch-fail-mid': ('assertion-or-panic', names[3]),
            'batch-ub-mid': ('miri-ub', names[3]),
            'batch-timeout': ('timeout-or-killed', names[3]),
            'batch-missing-line': ('zero-ignored-or-unverified-count', names[2]),
            'batch-extra-line': ('zero-ignored-or-unverified-count', 'p01_ldloca_convu_stind_i4'),
            'batch-reordered': ('zero-ignored-or-unverified-count', 'order differs'),
            'batch-zero': ('zero-ignored-or-unverified-count', "'1 passed; 0 failed; 0 ignored'"),
            'batch-ignored': ('zero-ignored-or-unverified-count', "'1 passed; 0 failed; 0 ignored'"),
        }
        for mode, (outcome, needle) in cases.items():
            with self.subTest(mode):
                output = self.invoke('batch', '--configuration', 'no-features', '--batch', '2', mode=mode, ok=False)
                self.assertIn(outcome, output)
                self.assertIn(needle, output)
                self.assertIn('total: 0 passed, 1 failed', output)
        # Mid-batch stops name the control in flight and show how many had completed.
        output = self.invoke('batch', '--configuration', 'no-features', '--batch', '2', mode='batch-fail-mid', ok=False)
        self.assertIn(f'control in flight when the batch stopped: {names[3]}', output)
        self.assertIn(f'completed 3 of {len(names)} controls', output)

    def test_batch_log_is_retained_with_the_guard_message(self):
        logdir = self.path / 'batch logs'
        self.env['POINTER_PROVENANCE_LOG_DIR'] = str(logdir)
        self.invoke('batch', '--configuration', 'no-features', '--batch', '1')
        self.assertIn('batch-control ok pointer_provenance_tests::p00_smoke', (logdir / 'batch_1.log').read_text())
        self.invoke('batch', '--configuration', 'no-features', '--batch', '1', mode='batch-fail-mid', ok=False)
        self.assertIn('batch guard failed', (logdir / 'batch_1.log').read_text())

    def test_ci_batch_job_is_wired_to_the_same_catalog(self):
        workflow = (ROOT / '.github/workflows/ci.yml').read_text()
        job = re.search(r'^  miri-pointer-batches:\s*\n(?P<body>(?:^(?: {4,}.*|\s*)\n)*)', workflow, re.M)
        self.assertIsNotNone(job, 'missing miri-pointer-batches job')
        matrix = re.findall(r'^        batch: \[([0-9, ]+)\]\s*$', job.group('body'), re.M)
        self.assertEqual(len(matrix), 1)
        self.assertEqual([int(v) for v in matrix[0].split(',')], list(BATCHES))
        self.assertIn('pointer_provenance.sh batch --configuration ${{ matrix.configuration }} --batch ${{ matrix.batch }}',
                      job.group('body'))
        self.assertIn('pointer_provenance.sh verify-controls', job.group('body'))
        self.assertIn('configuration: [no-features, multithreading]', job.group('body'))
        self.assertIn('bash scripts/pointer_provenance.sh verify-batches', workflow)

    def test_all_dry_runs_do_not_invoke_cargo(self):
        for config in ('no-features', 'multithreading'):
            for shard, names in CONTROLS_BY_SHARD.items():
                out = self.invoke('controls', '--configuration', config, '--shard', str(shard), '--dry-run')
                self.assertEqual(out.count('--exact'), len(names))
                self.assertNotIn('--ignored', out)
                self.assertIn('2700s', out)
                self.assertEqual('--features multithreading' in out, config == 'multithreading')
            for number in BATCHES:
                out = self.invoke('batch', '--configuration', config, '--batch', str(number), '--dry-run')
                self.assertEqual(out.count('--exact'), 1)
                self.assertIn(f'batches::batch_{number}', out)
                self.assertNotIn('--ignored', out)
                self.assertIn('3000s', out)  # batches get a larger per-process budget than singletons
            out = self.invoke('diagnostic', '--configuration', config, '--probe', 'p01_ldloca_convu_stind_i4', '--dry-run')
            self.assertIn('--ignored', out)
        self.assertEqual(self.recorded(), [])


if __name__ == '__main__':
    unittest.main(verbosity=2)
