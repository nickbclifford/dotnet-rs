"""One-host compile/code-size comparison; warmed dependencies, three fresh candidate builds."""
from pathlib import Path
import subprocess, time
BASE = Path(__file__).resolve().parents[1]
records = []
for candidate in ['concrete', 'gat']:
    target = BASE / 'target' / f'compile-{candidate}'
    prefix = ['cargo', 'build', '--offline', '--release', '--target-dir', str(target)]
    warm = subprocess.run(prefix + ['-p', 'effect-ops'], cwd=BASE, capture_output=True, text=True)
    assert warm.returncode == 0, warm.stderr
    for i in range(3):
        # Only generated artifacts under this experiment's target directory are removed.
        clean = subprocess.run(['cargo', 'clean', '--target-dir', str(target), '-p', 'effect-intrinsics', '--release'], cwd=BASE, capture_output=True, text=True)
        assert clean.returncode == 0, clean.stderr
        args = prefix + ['-p', 'effect-intrinsics', '--no-default-features', '--features', candidate, '--example', 'shape']
        start = time.monotonic()
        result = subprocess.run(args, cwd=BASE, capture_output=True, text=True)
        elapsed = time.monotonic() - start
        assert result.returncode == 0, result.stderr
        executable = target / 'release/examples/shape'
        records.append(f'{candidate} run {i+1}: {elapsed:.4f}s; executable {executable.stat().st_size} bytes\ncommand: {args!r}\n{result.stderr}')
(BASE / 'evidence/compile-cost.log').write_text('\n'.join(records))
print('\n'.join(r.splitlines()[0] for r in records))
