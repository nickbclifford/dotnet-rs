"""Check negative examples fail for lifetime confinement, not unresolved mock APIs."""
from pathlib import Path
import re, subprocess, tempfile
BASE = Path(__file__).resolve().parents[1]
source = '\n'.join(p.read_text() for p in sorted((BASE / 'layered/ops/src').glob('*.rs')))
blocks = re.findall(r'(?://!|///) ```compile_fail\n(.*?)(?://!|///) ```', source, re.S)
lib = max((BASE / 'target/debug/deps').glob('libeffect_ops-*.rlib'), key=lambda p: p.stat().st_mtime)
gc = max((BASE / 'target/debug/deps').glob('libgc_arena-*.rlib'), key=lambda p: p.stat().st_mtime)
log = []
with tempfile.TemporaryDirectory(prefix='effects-compile-fail-') as tmp:
    for index, block in enumerate(blocks, 1):
        path = Path(tmp) / f'case_{index}.rs'
        path.write_text('\n'.join(line.lstrip().removeprefix('//! ').removeprefix('/// ') for line in block.splitlines()))
        args = ['rustc', '--edition=2024', '--crate-type=lib', str(path), '--out-dir', tmp,
                '-L', f'dependency={BASE / "target/debug/deps"}', '--extern', f'effect_ops={lib}', '--extern', f'gc_arena={gc}']
        result = subprocess.run(args, capture_output=True, text=True)
        error = result.stderr
        assert result.returncode != 0, index
        assert 'lifetime' in error or 'borrow' in error, (index, error)
        assert not re.search(r'E0(?:432|433|599)', error), (index, error)
        log.append(f'case {index}: expected failure\n{error}')
(BASE / 'evidence/compile-fail-diagnostics.log').write_text('\n'.join(log).rstrip() + '\n')
print(f'{len(blocks)} negative examples failed for lifetime/borrow reasons; see evidence/compile-fail-diagnostics.log')
