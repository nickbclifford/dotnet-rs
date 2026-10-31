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
    1: ["p00_smoke", "p02_ldloca_stind_i4_control"],
    2: ["p03_ldloca_convu_ldind_i4", "p05_single_localloc_roundtrip"],
    3: ["p07_localloc_one_past_end", "p11_ldelema_stind_heap_array_control"],
    4: ["p10_ldelema_convu_stind_heap_array", "p20_heap_convu_then_typed_write_then_raw_store"],
    5: ["p13_ldelema_managed_add_far_out_of_bounds_then_conv_u", "p14_initblk_heap_array_via_managed_ptr"],
    6: ["p16_cpblk_heap_arrays_via_managed_ptr", "p_null_through_pointer_local_address"],
    7: ["p06_localloc_minus_one_then_plus_one", "p08_localloc_overshoot"],
    8: ["p17_localloc_ptr_ceq_self", "p18_localloc_ptr_clt"],
    9: ["p21_localloc_signed_extreme_offsets_roundtrip", "p22_localloc_wrap_to_zero_roundtrip"],
    10: ["p23_localloc_pointer_loop_unsigned_branch_compare", "p24_unsafe_byte_offset_direction_and_deref"],
    11: ["p25_convi_and_stind_i_pointer_roundtrip_deref", "p26_native_int_boundary_store_load_roundtrip"],
    12: ["p27_stind_i_fresh_pointer_through_local_slot_dereferences", "p28_stind_i_fresh_pointer_through_heap_memory_dereferences"],
    13: ["p04_double_localloc_write_first", "p29_localloc_blocks_survive_growth_and_are_16_byte_aligned"],
    14: ["p30_localloc_block_survives_nested_callee_localloc", "p31_localloc_blocks_released_on_return_and_pooling"],
    15: ["p32_localloc_blocks_released_on_tail_call_replacement", "p33_localloc_blocks_released_on_exception_unwind"],
    16: ["p34_localloc_blocks_released_on_executor_teardown", "p35_ldflda_on_stack_value_type_then_ldind"],
    17: ["p36_static_address_survives_forced_collection"],
}
CONTROLS = [name for names in CONTROLS_BY_SHARD.values() for name in names]
FAKE = r'''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
a = sys.argv[1:]
with open(os.environ['CALLS'], 'a') as f:
    f.write(json.dumps({'args': a, 'flags': os.environ.get('MIRIFLAGS')}) + '\n')
names = ['pointer_provenance_tests::' + n for n in json.loads(os.environ['CONTROLS'])]
mode = os.environ.get('MODE', '')
if '--list' in a:
    if mode == 'missing': names.pop(0)
    if mode == 'typo': names[0] += '_typo'
    if mode == 'duplicate': names.append(names[0])
    for n in names: print(n + ': test')
elif 'nextest' in a:
    cases = {n: {'kind': 'test', 'ignored': mode == 'ignored' and i == 0,
                 'filter-match': {'status': 'mismatch' if mode == 'filter-mismatch' and i == 0 else 'matches'}}
             for i, n in enumerate(names)}
    print(json.dumps({'rust-suites': {'dotnet-vm': {'package-name': 'dotnet-vm', 'kind': 'lib',
          'status': 'skipped' if mode == 'not-listed' else 'listed', 'testcases': cases}}}))
else:
    name = a[a.index('--exact') + 1]
    print('fake stderr marker', file=sys.stderr)
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
                        CALLS=str(self.calls), CONTROLS=json.dumps(CONTROLS), TMPDIR=str(self.path))

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

    def test_guard_good_and_stale_or_ignored(self):
        for config in ('no-features', 'multithreading'):
            count = 'six' if len(CONTROLS) == 6 else str(len(CONTROLS))
            self.assertIn(f'{count} exact non-ignored controls', self.invoke('verify-controls', '--configuration', config))
            for mode in ('missing', 'typo', 'duplicate', 'ignored', 'filter-mismatch', 'not-listed'):
                self.assertIn('guard failed', self.invoke('verify-controls', '--configuration', config, mode=mode, ok=False))
        for call in self.recorded():
            self.assertNotIn('--ignored', call['args'])

    def test_caller_log_directory_retains_both_streams_and_failure(self):
        logdir = self.path / 'durable logs'
        self.env['POINTER_PROVENANCE_LOG_DIR'] = str(logdir)
        args = ('controls', '--configuration', 'no-features', '--shard', '17')
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

    def test_reject_invalid_control_table_before_cargo(self):
        original = SCRIPT.read_text()
        shard, names = next(iter(CONTROLS_BY_SHARD.items()))
        full = 'pointer_provenance_tests::' + names[0]
        table_row = f"'{full}|{shard}'"
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
                self.script.write_text(catalog_only.replace(table_end, f"'{full}|{shard}'\n" + table_end))
                self.env['CONTROLS'] = json.dumps(CONTROLS + [name])
                count = len(CONTROLS_BY_SHARD.get(shard, [])) + 1
                output = self.invoke('controls', '--configuration', 'no-features', '--shard', str(shard))
                self.assertIn(f'total: {count} passed, 0 failed', output)
                calls = self.recorded()
                self.assertEqual(len(calls), count + 2)
                self.assertTrue(all('--ignored' not in c['args'] for c in calls))
                self.assertIn(full, [c['args'][c['args'].index('--exact') + 1] for c in calls[2:]])
                self.calls.unlink(missing_ok=True)
                self.invoke('diagnostic', '--configuration', 'no-features', '--probe', name, ok=False)
                self.assertEqual(self.recorded(), [])

    def test_all_dry_runs_do_not_invoke_cargo(self):
        for config in ('no-features', 'multithreading'):
            for shard, names in CONTROLS_BY_SHARD.items():
                out = self.invoke('controls', '--configuration', config, '--shard', str(shard), '--dry-run')
                self.assertEqual(out.count('--exact'), len(names))
                self.assertNotIn('--ignored', out)
                self.assertEqual('--features multithreading' in out, config == 'multithreading')
            out = self.invoke('diagnostic', '--configuration', config, '--probe', 'p01_ldloca_convu_stind_i4', '--dry-run')
            self.assertIn('--ignored', out)
        self.assertEqual(self.recorded(), [])


if __name__ == '__main__':
    unittest.main(verbosity=2)
