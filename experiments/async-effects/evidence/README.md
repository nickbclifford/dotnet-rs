# Final evidence and limits

These are historical experiment results. Current implementation scope is in
[plan 02](../../../docs/plans/02-native-continuations.md), and verification selection follows
[CI policy](../../../docs/CI.md#test-hierarchy-and-migration). Re-run an experiment only when it
answers a current uncertainty; this full runner is not required for each implementation step.

To reproduce the experiment, run from `experiments/async-effects`:

```bash
python3 evidence/verify.py
python3 gallery/build_gallery.py
```

The runner records exact commands, environment overrides, exit status, elapsed wall time
and full output. Default workspace membership includes the original prototype and all three
layered crates. Nothing adds workspace members/dependencies to the real VM.

**Chosen API: `fx.vm().await?` returns a VmAccess RAII loan with rooted views.** The default
prototype VM uses `effect_intrinsics::chosen`; old callback/GAT bodies still compile as
comparison evidence. The selected API grants at most 32 ready acquisitions per native poll,
then returns to the outer executor even with no pending GC request.

| Log | What it establishes |
| --- | --- |
| [test.log](test.log) | Stable workspace tests/doctests, including the default RAII path and all retained gallery variants. |
| [compile-fail-diagnostics.log](compile-fail-diagnostics.log) | Fifteen ops negative examples fail for brand/borrow confinement, including actual ThreadSafeLock read/write guards, GcScopeGuard, and the selected access/view loan. |
| [fmt.log](fmt.log), [clippy.log](clippy.log) | rustfmt and warnings-denied clippy for all workspace targets. |
| [test-mt-validation.log](test-mt-validation.log), [clippy-mt-validation.log](clippy-mt-validation.log) | Same model with real utility threaded locks and memory-validation enabled. |
| [miri.log](miri.log), [miri-mt-validation.log](miri-mt-validation.log) | All experiment library tests on nightly-2026-05-27, MIRIFLAGS=-Zmiri-tree-borrows, under both utility configurations. |
| [cost-1.log](cost-1.log), [cost-2.log](cost-2.log), [cost-3.log](cost-3.log) | Three serial release samples (`--test-threads=1`), including chosen guard acquisition, scalar/rooted-string view reads, and baseline region/spawn tests. |
| [compile-cost.log](compile-cost.log) | Historical dependency-warmed comparison of the callback and GAT facades, not the chosen RAII surface. |
| [environment.txt](environment.txt) | Compiler and CPU/platform details. |

Final inventory: original toy 3 passing + 1 ignored cost test; ops bridge 3 passing; VM 25
passing + 2 ignored cost tests. Total **31 passing library tests**, **17 passing doctests**
(16 compile-fail, including the original toy's negative), and 3 ignored release cost tests
which run separately in each of the three samples. Miri runs library tests, not doctests.
The diagnostic checker inspects all ops source files and rejects incidental missing APIs.

The focused historical `region-api-*.log` and `verify_region_api.py` remain as records of
the access comparison. The full logs above supersede their smaller test inventory. See the
[final access decision](../gallery/region-api/README.md) for source and candidate semantics.

Tests cover forced-GC copy progress with no repeated prefix; inline completion; managed/native
nesting; Capture and Transparent exception routing; root transfer before source teardown;
reflection boxing, box identity and byref slot assignment on success/throw; and wait effects
with deterministic mock completion. Guard-specific cases cover borrowing across a ready
Rust await, compile rejection of another VM effect while access remains borrowed, view
escape, forgotten access loans versus forgotten actual GC scopes, post-arena-teardown
rejection, cancelled acquisition, and an exhausted no-GC quantum followed by abort/budget
cancellation. The real GC scope and lock types are used in both utility configurations.

These are not real-VM tests. The utility `multithreading` feature selects threaded locks,
not the VM's OS thread manager/STW coordinator. The model has no actual CLR metadata/layouts,
SEH filters/finally/fault, cctor unwinding, or foreign-arena transport. No integrated MT root
publication, generation/owner-exit guarantee, lost-wakeup/timeout-race result or complete
feature-matrix claim follows. Strict provenance was not attempted. The original toy's 0.7
result is inherited; this extension pins and tests exactly gc-arena 0.6.0.

Release access measurements include the access-quantum poll/reply round trips, but no GC:
the microdriver directly acknowledges its SafePoint request. They use a tiny host, not real
GCHandle/VesContext setup. Scalar spawn does not include per-reference root-cell allocation.
The compile-cost experiment was a small sequential callback/GAT build, including linker and
driver overhead; GAT was slightly faster/smaller there. It is not a chosen-RAII/workspace
build-time comparison. Production budgets in DESIGN.md remain acceptance gates.
