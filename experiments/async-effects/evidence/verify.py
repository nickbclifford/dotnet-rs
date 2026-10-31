"""Run bounded experiment checks, retaining exact commands, output, time and status."""
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
 ('miri', ['cargo', '+nightly-2026-05-27', 'miri', 'test', '--offline', '--workspace', '--lib'], {'MIRIFLAGS': '-Zmiri-tree-borrows'}),
 ('miri-mt-validation', ['cargo', '+nightly-2026-05-27', 'miri', 'test', '--offline', '--workspace', '--lib', '--features', 'effect-ops/multithreading,effect-ops/memory-validation'], {'MIRIFLAGS': '-Zmiri-tree-borrows'}),
]
for i in range(1, 4):
    commands.append((f'cost-{i}', ['cargo', 'test', '--offline', '--release', '--workspace', '--lib', 'cost', '--', '--ignored', '--nocapture', '--test-threads=1'], {}))
for name, args, extra in commands:
    start = time.monotonic()
    p = subprocess.run(args, cwd=BASE, env=os.environ | extra, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    elapsed = time.monotonic() - start
    (BASE / f'evidence/{name}.log').write_text((f'command: {args!r}\nenvironment: {extra!r}\nexit: {p.returncode}\nwall seconds: {elapsed:.3f}\n\n{p.stdout}').rstrip() + '\n')
    print(f'{name}: exit {p.returncode}, {elapsed:.3f}s', flush=True)
    if p.returncode: raise SystemExit(p.stdout)
