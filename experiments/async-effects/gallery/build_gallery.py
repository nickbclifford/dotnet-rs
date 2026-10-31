"""Regenerate the offline comparison page and verbatim current-code snapshots."""
from pathlib import Path
from html import escape
import re
BASE = Path(__file__).resolve().parent
REPO = BASE.parents[2]

def function(text, name):
    match = re.search(r'(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+' + name + r'\b', text)
    if not match:
        raise ValueError(name)
    opening = text.index('{', match.end())
    level = 1
    end = opening + 1
    while level:
        level += (text[end] == '{') - (text[end] == '}')
        end += 1
    return text[match.start():end]

def current(path, name):
    text = (REPO / path).read_text()
    body = function(text, name)
    line = text[:text.index(body)].count('\n') + 1
    return f'// Verbatim snapshot: {path}:{line}\n{body}\n'

sources = {
    'substring': ('crates/dotnet-intrinsics-string/src/search.rs', 'intrinsic_string_substring'),
    'wait_finalizers': ('crates/dotnet-vm/src/intrinsics/gc.rs', 'intrinsic_gc_wait_for_pending_finalizers'),
    'monitor_enter': ('crates/dotnet-intrinsics-threading/src/monitor.rs', 'intrinsic_monitor_enter_obj'),
    'multicast': ('crates/dotnet-intrinsics-delegates/src/invoke.rs', 'invoke_delegate'),
    'ensure_static': ('crates/dotnet-vm/src/statics.rs', 'wait_for_init'),
}
for name, (path, symbol) in sources.items():
    (BASE / 'current' / f'{name}.rs.txt').write_text(current(path, symbol))
path = 'crates/dotnet-intrinsics-reflection/src/methods.rs'
lines = (REPO / path).read_text().splitlines()
reflection = f'// Verbatim snapshot: {path}:375\n' + '\n'.join(lines[374:445])
reflection += '\n\n' + current(path, 'unmarshal_invoke_params')
reflection += '\n// Byref implementation (same source):826\n' + '\n'.join(lines[825:1017])
path = 'crates/dotnet-vm/src/stack/context.rs'
reflection += f'\n// Return completion: {path}:377\n' + '\n'.join((REPO / path).read_text().splitlines()[376:410])
(BASE / 'current' / 'reflection_invoke.rs.txt').write_text(reflection + '\n')
(BASE / 'current' / 'heap_slot_after_wait.rs.txt').write_text('// No exact existing intrinsic. Probe for a rooted heap slot after suspension.\n// Arbitrary ManagedPtr transport remains excluded; see Monitor.ReliableEnter\n// in crates/dotnet-intrinsics-threading/src/monitor.rs:141-200.\n')
names = ['substring', 'wait_finalizers', 'monitor_enter', 'reflection_invoke', 'multicast', 'ensure_static', 'heap_slot_after_wait']
html = '''<!doctype html><meta charset="utf-8"><title>Native continuation syntax gallery</title>
<style>body{font:16px system-ui;margin:2rem;color:#202124;background:#fafafa}a{color:#315b9c}.grid{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:1rem}pre{font:12px/1.5 ui-monospace,monospace;background:#f1f2f3;padding:1rem;overflow:auto;max-height:44rem;border:1px solid #ddd}h2{margin-top:3rem}h3{font-size:16px}header{max-width:70rem} @media(max-width:1000px){.grid{grid-template-columns:1fr}}</style>
<header><h1>Native continuation syntax gallery</h1><p>Final choice: async VM access with RAII rooted views. Seven matching operations. Left: current VM source. Middle: chosen RAII bodies. Right: the superseded callback facade. The ports model metadata, not a drop-in real VM migration. See also <a href="region-api/index.html">the four-way VM access comparison</a>. Read <a href="README.md">the review and limits</a> and <a href="../DESIGN.md">the design</a>.</p><p>Lexical RAII scopes delimit VM access; short internal regions confine GC brands. Awaits perform effects before access is granted. Reflection preparation/completion is shared in <a href="../layered/intrinsics/src/lib.rs">the intrinsics crate</a>.</p></header>'''
for name in names:
    html += f'<section><h2 id="{name}">{name}</h2><div class="grid">'
    for folder, label in [('current', 'Today'), ('region-api', 'Chosen · async RAII'), ('concrete', 'Rejected · callback facade')]:
        if folder == 'current':
            code = (BASE / folder / f'{name}.rs.txt').read_text()
        elif folder == 'region-api':
            code = function((BASE / folder / 'raii.rs').read_text(), name)
        else:
            code = function((BASE / folder / 'ports.rs').read_text(), name)
        html += f'<article><h3>{label}</h3><pre>{escape(code)}</pre></article>'
    html += '</div></section>'
(BASE / 'index.html').write_text(html)

# Region-API refinement: same seven operations, four independently compiled spellings.
region_html = '''<!doctype html><meta charset="utf-8"><title>VM access API comparison</title>
<style>body{font:16px system-ui;margin:2rem;color:#202124;background:#fafafa}a{color:#315b9c}.grid{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:1rem}pre{font:12px/1.5 ui-monospace,monospace;background:#f1f2f3;padding:1rem;overflow:auto;max-height:40rem;border:1px solid #ddd}h2{margin-top:3rem}h3{font-size:16px}@media(max-width:1000px){.grid{grid-template-columns:1fr}}</style>
<header><h1>VM access: final decision</h1><p>D is selected: <code>let mut vm = fx.vm().await?</code>, an RAII access loan lending rooted views after honoring a pending stop. <a href="README.md">Read the decision and safety contract</a>.</p><p>All four columns compile for the same seven operations. A/B/C are retained alternatives. The previous always-ready callback uses vm_ready; vm() is reserved for the selected RAII API. The rejected async-closure probe is covered separately in tests.</p></header>'''
for name in names:
    region_html += f'<section><h2 id="{name}">{name}</h2><div class="grid">'
    for filename, label in [('with_vm.rs', 'A · with_vm'), ('vm_await.rs', 'B · vm_ready(...).await'), ('checkpoint.rs', 'C · access with checkpoint'), ('raii.rs', 'D · vm().await? (chosen)')]:
        code = function((BASE / 'region-api' / filename).read_text(), name)
        region_html += f'<article><h3>{label}</h3><pre>{escape(code)}</pre></article>'
    region_html += '</div></section>'
(BASE / 'region-api/index.html').write_text(region_html)
