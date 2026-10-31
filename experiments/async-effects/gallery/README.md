# Final syntax gallery

**Chosen: `let mut vm = fx.vm().await?`, with an RAII access loan and rooted views.**
Open [index.html](index.html) for today's code beside the chosen bodies and the earlier
callback facade. [Chosen source](region-api/raii.rs) compiles as `effect_intrinsics::chosen`
and is the default path in the prototype VM.

The [four-way access comparison](region-api/index.html) retains the synchronous callback,
always-ready async callback, checkpointing callback and chosen RAII versions of all seven
operations. The [decision record](region-api/README.md) explains their semantics and costs.
No public API choice remains pending.

| Shape | Source | Verdict |
| --- | --- | --- |
| Concrete VmEffects, async `vm()`, RAII VmAccess and rooted views | [raii.rs](region-api/raii.rs) | Chosen: visible lexical scopes, ordinary method calls, await before access, no lifetime annotations in bodies. |
| Concrete callback facade | [ports.rs](concrete/ports.rs) | Superseded: same underlying brand proof, but callbacks and manual stop checks clutter intrinsic bodies. |
| Generic GAT host | [ports.rs](gat/ports.rs) | Rejected: viable after lifetime repair, but no demonstrated need for multiple real hosts. It is not an established performance loss. |

All bodies compile in the intrinsics crate, whose only dependency is ops. Current-code
`.rs.txt` files are verbatim excerpts with source coordinates, not compilable modules.
Reflection includes Invoke setup, byref marshaling and the remote return hook. Static
initialization shows its existing wait helper; the new body also includes the trigger.
`python3 gallery/build_gallery.py` refreshes both comparison pages and snapshots.

## What the chosen version looks like in context

Substring now reads as validation, chunk copies and result allocation. Each `vm()` await
honors a pending stop or exhausted native quantum before granting access; each block releases the effects loan before
another acquisition/effect. Offset and owned output survive suspension. The view's actual
validation/copy methods are fully written out in `layered/ops/src/raii_vm.rs`; some apparent
brevity is ordinary helper extraction, not an inherent consequence of RAII.

Reflection has two short access scopes around the managed call. Its guarded helper adapters
reuse the same metadata/unboxing/boxing logic as the other candidates, internally through
`VmAccess::with`. No real Gc or byref enters the native future. Transparent exceptions retire
the continuation during managed unwind rather than resuming return boxing. The model is
not evidence that the real two-pass SEH integration already works.

Multicast is a loop of prepared managed calls; no VM access scope is needed for each call
once the metadata/arguments are prepared. The real implementation must preserve the
reusable argument buffer, target binding and single-target fast path. The mock's argument
cloning is not justification for regressing those optimizations.

Static readiness lives in a short scope followed by a call/wait outside that scope. The
handler owns wait-graph registrations, not a scope guard borrowed from the context. Tiny
finalizer/monitor waits remain straight effect calls; production handlers probe readiness
before parking, and explicit synchronous shims retain fast paths.

Readers can see where an effect may yield at `.await`, where a VM access loan is active in
the lexical block, and where complex helpers still enter a generative internal region.
No macro hides these boundaries. Guards/views do not expose arbitrary borrowed heap data.

## Retained GAT evidence

Successful GAT bodies require no per-region closure annotations. The first two-lifetime
attempt, saved under `gat/rejected/`, hit the stable compiler's higher-ranked associated-type
bound limitation (#100013). The repaired one-brand facade compiles. A backend function-item
inference failure at `Arena::new(DynamicRootSet::new)` is handled with one explicit closure,
not annotations repeated by intrinsic authors.

The historical one-host compile-cost experiment slightly favored GAT over the callback
facade. It did not measure the chosen RAII facade. The decision is scope/API simplicity
for one VM host; multiple-host duplication is a cost model, not a measured workspace slowdown.

## Model limits

These are compiling adaptations, not drop-in CLR implementations. `InvokeSpec` replaces
metadata lookup; i64 boxes and object[] slots model value/reference byrefs. Existing box
identity and write-through reference-slot assignment are tested on success and throw.
Generic resolution, TypedReference, binding flags and actual CLR constructor layouts remain
outside the mock; constructor mode assumes a preallocated rooted receiver.

Condition waits complete deterministically. There is no OS wait graph, STW coordinator,
real SEH/filter engine or foreign-arena object transport in this model. The seventh probe
uses a rooted owner/index descriptor, not a universal ManagedPtr stash. General byrefs and
MT enablement retain the gates documented in DESIGN.md.
