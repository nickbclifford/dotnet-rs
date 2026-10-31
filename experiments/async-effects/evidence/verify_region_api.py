"""Verify the region API trials; retain earlier performance and full baseline Miri logs."""
from pathlib import Path
import os, subprocess, time
BASE = Path(__file__).resolve().parents[1]
commands = [
 ('test', ['cargo', 'test', '--offline', '--workspace'], {}),
 ('compile-fail', ['python3', 'evidence/check_compile_fail.py'], {}),
 ('fmt', ['cargo', 'fmt', '--all', '--check'], {}),
 ('clippy', ['cargo', 'clippy', '--offline', '--workspace', '--all-targets', '--', '-D', 'warnings'], {}),
 ('test-mt-validation', ['cargo', 'test', '--offline', '--workspace', '--features', 'effect-ops/multithreading,effect-ops/memory-validation'], {}),
 ('clippy-mt-validation', ['cargo', 'clippy', '--offline', '--workspace', '--all-targets', '--features', 'effect-ops/multithreading,effect-ops/memory-validation', '--', '-D', 'warnings'], {}),
 ('miri', ['cargo', '+nightly-2026-05-27', 'miri', 'test', '--offline', '-p', 'effect-vm', '--lib', 'region_api_tests'], {'MIRIFLAGS': '-Zmiri-tree-borrows'}),
 ('miri-mt-validation', ['cargo', '+nightly-2026-05-27', 'miri', 'test', '--offline', '-p', 'effect-vm', '--lib', '--features', 'effect-ops/multithreading,effect-ops/memory-validation', 'region_api_tests'], {'MIRIFLAGS': '-Zmiri-tree-borrows'}),
]
for name, args, extra in commands:
    start = time.monotonic()
    p = subprocess.run(args, cwd=BASE, env=os.environ | extra, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    elapsed = time.monotonic() - start
    (BASE / f'evidence/region-api-{name}.log').write_text((f'command: {args!r}\nenvironment: {extra!r}\nexit: {p.returncode}\nwall seconds: {elapsed:.3f}\n\n{p.stdout}').rstrip() + '\n')
    print(f'{name}: exit {p.returncode}, {elapsed:.3f}s', flush=True)
    if p.returncode: raise SystemExit(p.stdout)
