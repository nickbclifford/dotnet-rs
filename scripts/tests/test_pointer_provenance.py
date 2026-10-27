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
CONTROLS = [
    "p00_smoke", "p02_ldloca_stind_i4_control", "p03_ldloca_convu_ldind_i4",
    "p05_single_localloc_roundtrip", "p07_localloc_one_past_end",
    "p11_ldelema_stind_heap_array_control",
]
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
                 'filter-match': {'status': 'mismatch' if mode == 'ignored' and i == 0 else 'matches'}}
             for i, n in enumerate(names)}
    print(json.dumps({'rust-suites': {'dotnet-vm': {'package-name': 'dotnet-vm', 'kind': 'lib',
          'status': 'listed', 'testcases': cases}}}))
else:
    name = a[a.index('--exact') + 1]
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
        self.env = dict(os.environ, PATH=str(self.path) + os.pathsep + os.environ['PATH'],
                        CALLS=str(self.calls), CONTROLS=json.dumps(CONTROLS), TMPDIR=str(self.path))

    def invoke(self, *args, mode='', ok=True):
        env = dict(self.env, MODE=mode)
        result = subprocess.run(['bash', str(SCRIPT), *args], cwd=ROOT, env=env,
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
        ignored = set(re.findall(r'^#\[ignore = "[^\n]+"\]\nfn (p(?:\d\d_|_)[a-z0-9_]+)\(', text, re.M))
        self.assertEqual(ignored, set(source) - set(CONTROLS))
        self.assertEqual(len(rows), 23)
        self.assertEqual({r[0].split('::')[-1] for r in rows}, set(source))
        self.assertEqual(len(set(r[0] for r in rows)), 23)
        for shard, pair in enumerate((CONTROLS[:2], CONTROLS[2:4], CONTROLS[4:]), 1):
            self.assertEqual([r[0].split('::')[-1] for r in rows if r[3] == str(shard)], pair)
        self.assertEqual([r[0].split('::')[-1] for r in rows if r[2] == 'yes'],
                         ['p17_localloc_ptr_ceq_self', 'p18_localloc_ptr_clt', 'p19_ldelem_negative_index'])
        self.assertEqual(self.recorded(), [])

    def test_guard_good_and_stale_or_ignored(self):
        for config in ('no-features', 'multithreading'):
            self.assertIn('six exact non-ignored controls', self.invoke('verify-controls', '--configuration', config))
            for mode in ('missing', 'typo', 'duplicate', 'ignored'):
                self.assertIn('guard failed', self.invoke('verify-controls', '--configuration', config, mode=mode, ok=False))
        for call in self.recorded():
            self.assertNotIn('--ignored', call['args'])

    def test_controls_count_and_guard(self):
        base = ('controls', '--configuration', 'no-features', '--shard', '1')
        self.assertIn('total: 2 passed, 0 failed', self.invoke(*base))
        calls = self.recorded()
        self.assertEqual(len(calls), 4)
        self.assertTrue(all('--ignored' not in c['args'] for c in calls))
        self.assertEqual([c['args'][c['args'].index('--exact') + 1] for c in calls[2:]],
                         ['pointer_provenance_tests::' + c for c in CONTROLS[:2]])
        for mode in ('zero', 'ignored-run'):
            self.assertIn('zero-ignored-or-unverified-count', self.invoke(*base, mode=mode, ok=False))
        before = len(self.recorded())
        self.invoke(*base, mode='ignored', ok=False)
        self.assertEqual(len(self.recorded()) - before, 2)  # no Miri after guard failure

    def test_each_control_shard_uses_exact_names_and_pinned_flags(self):
        for config in ('no-features', 'multithreading'):
            for shard, pair in enumerate((CONTROLS[:2], CONTROLS[2:4], CONTROLS[4:]), 1):
                self.calls.unlink(missing_ok=True)
                self.invoke('controls', '--configuration', config, '--shard', str(shard))
                calls = self.recorded()
                self.assertEqual(len(calls), 4)
                self.assertTrue(all('--ignored' not in c['args'] for c in calls))
                self.assertEqual([c['args'][c['args'].index('--exact') + 1] for c in calls[2:]],
                                 ['pointer_provenance_tests::' + name for name in pair])
                for call in calls[2:]:
                    self.assertIn('+nightly-2026-05-27', call['args'])
                    self.assertEqual(call['flags'], '-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks')
                    self.assertEqual('--features' in call['args'], config == 'multithreading')

    def test_reject_unknown_and_native_before_cargo(self):
        for args in [('--probe', 'p00_smok'), ('--probe', ''), ('--probe', 'p01_ldloca_convu_stind_i4', '--lane', 'native'),
                     ('--probe', 'p00_smoke'),
                     ('--all', '--lane', 'native'), ('--probe', 'pointer_provenance_tests::p01_ldloca_convu_stind_i4')]:
            self.invoke('diagnostic', '--configuration', 'no-features', *args, ok=False)
        self.assertEqual(self.recorded(), [])
        self.invoke('diagnostic', '--configuration', 'no-features', '--probe', 'p17_localloc_ptr_ceq_self', '--lane', 'native')
        call = self.recorded()[0]['args']
        self.assertIn('--ignored', call)
        self.assertNotIn('miri', call)

    def test_diagnostic_continues_and_distinguishes_failures(self):
        for mode, classification in [('fail-first', 'miri-ub'),
                                     ('setup-first', 'compile-setup-or-process-failure'),
                                     ('panic-first', 'assertion-or-panic'),
                                     ('timeout-first', 'timeout-or-killed')]:
            self.calls.unlink(missing_ok=True)
            output = self.invoke('diagnostic', '--configuration', 'multithreading', '--all', mode=mode, ok=False)
            self.assertIn(classification, output)
            self.assertIn('total: 16 passed, 1 failed', output)
            calls = self.recorded()
            self.assertEqual(len(calls), 17)
            names = {c['args'][c['args'].index('--exact') + 1].split('::')[-1] for c in calls}
            self.assertEqual(len(names), 17)
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

    def test_all_dry_runs_do_not_invoke_cargo(self):
        for config in ('no-features', 'multithreading'):
            for shard in ('1', '2', '3'):
                out = self.invoke('controls', '--configuration', config, '--shard', shard, '--dry-run')
                self.assertEqual(out.count('--exact'), 2)
                self.assertNotIn('--ignored', out)
                self.assertEqual('--features multithreading' in out, config == 'multithreading')
            out = self.invoke('diagnostic', '--configuration', config, '--probe', 'p01_ldloca_convu_stind_i4', '--dry-run')
            self.assertIn('--ignored', out)
        self.assertEqual(self.recorded(), [])


if __name__ == '__main__':
    unittest.main(verbosity=2)
