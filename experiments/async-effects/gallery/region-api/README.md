# VM access API: final decision

**Selected: `let mut vm = fx.vm().await?`.** `VmEffects` is the effect capability;
`VmAccess` is its RAII access loan; typed views such as `VmString` borrow that loan.
The owner selected this form after inspecting the concrete alternatives.

[Chosen source](raii.rs), [four-way comparison](index.html),
[ops implementation](../../layered/ops/src/raii_vm.rs),
[tests](../../layered/vm/src/region_api_tests.rs).
All four variants compile with the same six real operations plus the heap-slot probe.
The prototype exports `effect_intrinsics::chosen` and uses it in its default VM path.

```rust
let count = {
    let mut vm = fx.vm().await?;
    vm.string(&source).substring_len(start, length)?
};
while offset < count {
    let end = (offset + CHUNK_SIZE).min(count);
    {
        let mut vm = fx.vm().await?;
        vm.string(&source).copy_range(start + offset, start + end, &mut result)?;
    }
    offset = end;
}
```

The complete source includes the checked signed-index handling. No HRTB/GAT annotations
leak into the intrinsic body. Explicit `drop(vm)` also releases a scope; braces are preferred
for clarity. A view keeps the guard borrowed, and the guard exclusively borrows `fx`, so a
later VM effect while that loan remains live is a compiler error.

## Acquisition and the GC boundary

`vm()` probes the borrow-aware stop predicate and remaining native quantum. A request or
exhaustion performs the existing
SafePoint effect **before** returning access. Otherwise it is Ready in the current native
poll. This removes explicit stop-query/safe-point boilerplate from VM-access-heavy loops.
It does not replace explicit safe points in long pure-Rust loops with no VM access.

The chosen substring acquires before validation, each chunk and result allocation. The
permanently asserted mock stop flag therefore forces four collections for two chunks,
versus one explicit inter-chunk collection in the old callback body. Each character is
still copied exactly once. A request arriving during an access operation is handled at a
later boundary; keep operations bounded. Each poll permits at most 32 ready acquisitions; exhaustion yields to the outer executor
without requiring a GC request, preserving abort/fuzz-budget responsiveness. SafePoint must
not inline that yield away. The ordinary dispatch batch is unchanged.

`VmAccess` and `VmString` store no Gc, heap reference, raw context pointer, slice or lock.
Views are rooted proxies: each operation enters the same confined internal region, fetches
the stash, completes its checked heap work, drops actual guards, and returns. There is no
Deref to ObjectRef or `[u16]`. Scope grouping is an ergonomic/exclusivity boundary, not a
promise of one context fetch or a long heap lock. The real adapter cost must be benchmarked.

Reflection's guarded adapters use `VmAccess::with` to reuse branded marshaling/boxing
helpers. Its higher-ranked callback cannot return branded data. Callbacks remain inside
complex helper implementations, but intrinsic bodies use visible RAII scopes. No macro
makes the soundness rule implicit. The string helpers are fully written out in the ops
source; some reduction in body size is ordinary helper extraction.

`Drop` only ends the Rust loan; it never touches the poll-local context or runs asynchronous
cleanup. Forgetting this access loan forgets no heap guard: collection and later access can
continue safely, as tested. Forgetting a real GcScopeGuard is different and still rejected
by the existing scope counters. Cancellation during acquisition releases native roots
without entering the body. An escaped capability/view reaches the null slot check after
native completion/arena teardown before it can touch dead heap storage. The actual root,
generation and owner-lifetime gates for MT remain unchanged.

## Rejected alternatives

The historical method names are retained as hidden comparison helpers in this standalone
experiment; the production surface ships only `vm()` plus `VmAccess` methods/`with`.

| Alternative | Short example | Why it lost |
| --- | --- | --- |
| Synchronous callback | `fx.with_vm(\|cx\| read(cx))` | A clearer rename than enter, but still leaves callback-heavy bodies and manual stop checks. |
| Always-ready async callback | `fx.vm_ready(\|cx\| read(cx)).await` | Adds await syntax without doing checkpoint work or improving scope ergonomics. Originally trialed as `vm(callback)`. |
| Checkpointing fallible callback | `fx.checkpoint_vm(\|cx\| Ok(read(cx))).await?` | Sound and useful, but RAII gives ordinary method calls and lexical scopes without the callback/Ok wrappers in each body. |
| Async callback | `fx.vm_async(async \|cx\| read(cx)).await` | A callback holding untraced GC state must complete on its first poll. Runtime Pending rejection makes this a poor default for real VM effects. |
| Original enter | `fx.enter(\|cx\| read(cx))` | Names entering a scope rather than the VM access capability, and exposes the low-level region boundary directly in every intrinsic. |
| Bare heap-borrowing RAII | `let heap = fx.raw_heap().await?` | Not established by this trial: leaked/forgotten guards, collection exclusion and arena ownership need an additional lifetime/teardown proof. Rooted views preserve the existing confined proof. |

`gc(...)` was rejected as a name because access can also prepare calls or inspect metadata
and initialization state; it should not read as a collection request. `vm_guard` was the
selected variant's trial spelling and has been removed in favor of `vm`.

The lending async-closure experiment compiles on stable 1.95 without per-call annotations;
[AsyncFnOnce's standard-library documentation](https://doc.rust-lang.org/std/ops/trait.AsyncFnOnce.html)
marks the trait stable from 1.85. The test uses ordinary call syntax, not unstable associated
APIs. A ready helper borrowing a real scope/lock guard succeeds; a Pending helper panics
before region exit, and the guard is dropped there. Rooted RAII views can also span a ready
Rust await, but another VM effect must follow scope release. External Pending remains an
explicit non-goal.

## Evidence

Eleven focused runtime tests cover all four string forms under forced stops, first-poll Ready,
checkpoint-before-callback ordering, reflection success/throw, async-closure Ready/Pending,
RAII loan/view lifetime, forgetting the access loan, post-teardown rejection cancellation, and bounded ready-acquisition fairness.
Seven additional compile-fail doctests cover awaitable result escapes and RAII loans; with
the original eight ops negatives, diagnostics check fifteen failures for lifetime/borrow
reasons rather than missing APIs. The full experiment has 31 passing library tests and
17 doctests; ignored release cost tests measure baseline and chosen access costs.

[Full runner](../../evidence/verify.py) records stable tests, both utility configurations,
clippy, all-library pinned Tree Borrows Miri and release samples. The focused historical
`region-api-*.log` records are retained too. Neither run establishes real VM STW/SEH or
foreign-arena integration. The final design keeps those rollout gates explicit.
