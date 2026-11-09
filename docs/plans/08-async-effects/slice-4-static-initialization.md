# Plan 08 slice 4: static fields through initialization effects

**Status:** Complete
**Outcome:** `ldsfld`, `stsfld` and `ldsflda` check published initialization state and use
`ensure_initialized` for slow states in both configurations, without replaying the field instruction.
Field-triggered exception wrapping is verified against CLR for the
`statics/cctor_filter_42.cs` filter shape. Other initialization triggers remain on their existing
path.

This slice builds on the completed [retained-arena lifetime prerequisite](../09-retired-arena-lifetime.md):
the original inner exception remains accessible after its owner exits and through collection.

## Current paths and scope

`ldsfld`, `stsfld` and `ldsflda` in `instructions/objects/fields.rs` resolve the field and its
canonical static generic lookup before checking the initialization state. `VesContext::start_static_field_effect`
completes the field operation synchronously when the acquire read reports `Initialized` or
`Initializing` owned by the current thread. All other states use the `ensure_initialized`
activation, including failed types and waits on another owner. `stsfld` retains its operand in
the traced completion action only when using an activation; `ldsfld` bypasses initialization for
intrinsic fields. Both completion paths share the field operations, preserving volatile ordering
and generic-instantiation keying.
`ldsflda` produces a `PointerOrigin::Static`, not a heap or frame-home byref.

The other callers stay **legacy** in this slice:

- `newobj` initializes before allocating the non-intrinsic object and pushing its constructor;
  intrinsic constructor branches bypass this call. Its constructor operands and existing retry
  behavior must remain intact.
- `dispatch_method_with_mode`, through `prepare_method_initialization`, initializes before
  eligible static, value-type instance and constructor calls only when the type is not
  `beforefieldinit`. This includes pending targets of native calls, multicast and reflection;
  preserve their initializer retry and return-action handling.
- `RuntimeHelpers.RunClassConstructor` resolves its peeked `RuntimeTypeHandle`, initializes,
  and pops the argument only after success. It keeps the synchronous intrinsic path and retry.

These triggers have different completion and operand contracts from field instructions. Moving
them is not required for the field consumer and would expand the migration into method dispatch,
constructor and intrinsic completion. Keep their existing initialization timing and semantics;
the filter-order improvement is required for the migrated field-triggered path, not claimed for
these legacy triggers. In particular, preserve the legacy late wrapping in `unwind_frame` and
its `.cctor` return handling while those callers need them. Remove field-initialization replay,
not shared `back_up_ip`, `yield_and_retry`, `RetryInstruction` or auto-invoked `.cctor` handling
that the legacy callers still use. Do not let that legacy handling double-publish or rewrap an
initializer owned by `ensure_initialized`.

`StaticStorageManager::init` keys storage by type and generic lookup, returns `Execute`,
`Initialized`, `Recursive`, `Failed` or an owned `Waiting` registration. `InitState` encodes state
and owner in one atomic word; transitions use compare-and-swap, with an acquire-only initialized
fast-path read. `Execute` transfers an `InitializerClaim`: effects retain it in their future,
while legacy dispatch attaches it to `StackFrame::initializer_claim`. Explicit return or failure
publishes completion; frame disposal and pooling fail an abandoned claim and wake waiters. The
claim retains synchronization state only, not GC references, and its destructor runs no managed code. The effects handler runs owned initializers through `try_call`;
its traced failure carrier and abandoned-claim cleanup are shared with legacy callers. MT
waits use atomically checked wait-graph registrations, while legacy `wait_for_init` retains its
in-mutation condvar wait. Effect waits run outside the mutation and re-probe after a GC stop.
Same-thread recursion proceeds without waiting; in ST a `Waiting` result is unreachable.
Field access and `newobj` currently initialize on access regardless of `beforefieldinit`.

## Design

Use the instruction entry and completion action in the [main plan](../08-async-effects.md),
without changing the instruction macro. Advance the instruction IP exactly once, only after its
load, store or address operation succeeds. Preserve a store value across a suspended initializer
in traced VM-owned storage, including supported value types and references; do not place a branded
value in the future or replay the instruction. Preserve static-address origin and aliasing without
transporting it through a native byref descriptor or manufacturing an unmanaged origin. An
exception or host failure must discard the pending operation without a partial store or output.

`ensure_initialized(type, generics)` probes and claims through the static manager, executes an
owned `.cctor` through capturing `try_call`, publishes success or a
`TypeInitializationException` with its inner exception, and wakes waiters. A capturing boundary
first lets the child's handlers, filters and cleanup run; after the child unwinds, Rust receives a
rooted exception and publishes the wrapper before the caller's filter runs. This ordering is
verified against CLR only for the `statics/cctor_filter_42.cs` shape, whose filter returns true
for the wrapper and its original inner exception. It does not establish behavior when a filter
itself throws: ECMA-335 §III.3.34 requires that exception to be intercepted and the filter to
return `exception_continue_search` (the separate gap recorded as
[plans README, "Control flow and filters"](../README.md#not-yet-planned)). The legacy `unwind_frame` wrap occurs
after exception search has already run outer filters on the inner
exception.

A waiting activation owns its registration outside the mutation. Transfer the wait-graph edge
atomically with `WaitGraphEdgeGuard::disarm`, remove it on every terminal path, and repeat the
cycle check before waiting again after a GC stop removed the edge. A cycle follows the existing
`Recursive` policy (ECMA-335 §II.10.5.3.3). An abandoned claim, including abort, cancellation or
host failure, is published as failed and wakes waiters on every terminal path; Rust `Drop` does
not execute managed code. Keep ST same-thread `Waiting` unreachable. Both effects-owned and
legacy initializers share the static state and wait graph and must interoperate when nested or
when one path waits for the other.

## Static-field throughput

The [baseline measurement](../10-static-field-evidence.json) exceeds the 1% regression
threshold and authorizes the initialized/recursive shortcut. Four alternating `bench-fat` pairs
with CPU 0 and SHA-verified fixtures measure the shortcut against the same branch without it:
the mixed load/store/address workload improves in every pair, with mean pair changes of
−39.37% ST and −34.17% MT. Whole-process user instructions decrease by approximately 29%.
The workload includes setup and dispatch; these are not per-instruction costs. Host contention
limits precise timing conclusions. Raw samples, hashes and counter diagnostics are in
[the fast-path evidence](../10-static-field-fast-path-evidence.json), which also records the
control-workload limits.

## Done when

- In ST and MT, all three field instructions use the new initialization path where applicable,
  including initial, already initialized, recursive, and failed cases. Their results, store operand lifetime,
  generic static keys, volatile behavior, static-address aliasing, intrinsic-field behavior and
  initialization timing remain correct. The field path no longer relies on instruction replay;
  the explicitly listed legacy callers retain their required retry and late-wrap machinery.
- Capturing `try_call` delivers a rooted exception only after child handlers and finally/fault
  cleanup, including an unhandled child. Child filters run before capture and the wrapper is
  thrown afterward. The registered CLR comparison for `statics/cctor_filter_42.cs` covers this
  field-access shape.
- Successful and failing cross-thread waits complete and wake all waiters, including mixed
  effects/legacy triggers; a GC stop while waiting rechecks the graph, and circular initialization
  retains the existing recursion policy. Tests cover success, failure and abandoned claims,
  including cleanup on abort/host failure, with no stale edges or permanently initializing type.
- Existing `cctor_*`, `circular_init*` and static-field fixtures remain green in ST and MT;
  add enabled wait-success and wait-failure fixtures and the CLR comparison above. Preserve
  multicast and reflection initializer-retry coverage and add focused mixed-path regressions.
  Check fixture discovery and expected skips; build fixtures first and run the full fixture set
  once in each configuration as final verification.
- Run focused real-frame exception and initializer tests in both configurations, the affected
  pinned Miri controls for any unsafe or pointer changes, doc drift, and `./check.sh --fast`.
  Update the CI Miri manifest and its checker if new enumerated controls are added. Record native,
  Miri, doctest and performance evidence separately; no benchmark result is implied by a fixture
  pass.

## Verification (2026-11-09)

- Native: after the initializer state-machine change, the VM native/transition/collection
  selectors passed 89/89 ST and 124/124 MT; static/cctor/circular-init CLI selections passed
  14/14 ST and 20/20 MT, and static-field selections passed 18/18 ST and 24/24 MT. The fast
  matrix passed 819/819 ST, 1012/1012 MT and 1027/1027 MT+validation (14/7/9 existing skips).
  A later `./check.sh --fast` run passed 831/831, 1031/1031 and 1046/1046 after the remaining fixture and
  mutation work.
- CLR: static/cctor/circular-init selections, including the `statics/cctor_filter_42.cs` shape,
  passed their registered CLR comparisons.
- Miri: pinned Tree Borrows transition, explicit/abandoned-claim and frame-retirement
  singletons passed 1/1 each in ST and MT; the worker-waiter singleton passed 1/1 MT. The
  initialized and recursive byte-field shortcut tests later passed 1/1 each in ST and MT.
  A later rerun passed `static_store_operand_released_after_abandonment` and
  `native_initializer_failure_is_published_with_original_inner` in ST and MT, and
  `native_initializer_failure_survives_owner_retirement_and_collection` and
  `native_initializer_wait_rechecks_cycle_after_real_collection` in MT: each exact singleton
  ran 1/1 with no ignores (214 other tests filtered in ST, 321 in MT). The selectors, pinned
  flags and reproduction command are in [CI](../../CI.md#miri). This is permissive Tree
  Borrows with isolation disabled and leaks ignored, not strict-provenance or leak checking;
  dependency integer-to-pointer warnings limit provenance conclusions.
- Doctests: VM ran 0 tests with 2 existing ignored examples in each configuration; vm-data ran
  0 with none ignored. Performance: the four-pair static-field remeasurement improved in every
  pair (mean −39.37% ST and −34.17% MT); details and limits are in the linked fast-path evidence.
- Documentation: doc drift, formatting, diff and cross-arena manifest checks passed during the
  implementation. The current documentation-only edits are verified separately.
