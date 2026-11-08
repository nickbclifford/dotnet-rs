# Plan 08 slice 4: static fields through initialization effects

**Status:** Complete
**Outcome:** `ldsfld`, `stsfld` and `ldsflda` use `ensure_initialized` in both configurations
where they currently initialize static storage, without replaying the field instruction. A failing
initializer is captured and wrapped before an outer filter sees it. Other initialization
triggers remain on their existing path.

Step 4.4 depends on [retaining arenas after managed thread exit](../09-retired-arena-lifetime.md):
the original inner exception must remain accessible after its owner exits and through collection.

## Current paths and scope

`ldsfld`, `stsfld` and `ldsflda` in `instructions/objects/fields.rs` resolve the field and its
canonical static generic lookup before starting an `ensure_initialized` activation. `stsfld`
retains its operand in the traced completion action; `ldsfld` bypasses initialization for
intrinsic fields. The field operations preserve volatile ordering and generic-instantiation keying.
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
`Initialized`, `Recursive`, `Failed` or an owned `Waiting` registration, and claims the
initializer under its mutex. The effects handler runs owned initializers through `try_call`;
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
rooted exception and publishes the wrapper before outer filters run. This is the maintainer's
intended CLR-compatible change. The current `unwind_frame` wrap occurs after the exception search
has already run outer filters on the inner exception.

A waiting activation owns its registration outside the mutation. Transfer the wait-graph edge
atomically with `WaitGraphEdgeGuard::disarm`, remove it on every terminal path, and repeat the
cycle check before waiting again after a GC stop removed the edge. A cycle follows the existing
`Recursive` policy (ECMA-335 §II.10.5.3.3). An abandoned claim, including abort, cancellation or
host failure, is published as failed and wakes waiters on every terminal path; Rust `Drop` does
not execute managed code. Keep ST same-thread `Waiting` unreachable. Both effects-owned and
legacy initializers share the static state and wait graph and must interoperate when nested or
when one path waits for the other.

## Done when

- In ST and MT, all three field instructions use the new initialization path where applicable,
  including initial, already initialized, recursive, and failed cases. Their results, store operand lifetime,
  generic static keys, volatile behavior, static-address aliasing, intrinsic-field behavior and
  initialization timing remain correct. The field path no longer relies on instruction replay;
  the explicitly listed legacy callers retain their required retry and late-wrap machinery.
- Capturing `try_call` delivers a rooted exception only after child handlers and finally/fault
  cleanup, including an unhandled child. Child filters run before capture and outer filters after
  the wrapper is thrown. A CLR-diffed field-access fixture verifies that the outer filter sees
  `TypeInitializationException` with the original inner exception, not the inner exception first.
- Successful and failing cross-thread waits complete and wake all waiters, including mixed
  effects/legacy triggers; a GC stop while waiting rechecks the graph, and circular initialization
  retains the existing recursion policy. Tests cover success, failure and abandoned claims,
  including cleanup on abort/host failure, with no stale edges or permanently initializing type.
- Existing `cctor_*`, `circular_init*` and static-field fixtures remain green in ST and MT;
  add enabled wait-success and wait-failure fixtures and the CLR comparison above. Preserve
  multicast and reflection initializer-retry coverage and add focused mixed-path regressions.
  Check fixture discovery and expected skips; build fixtures first and run the full fixture set
  once in each configuration at phase end.
- Run focused real-frame exception and initializer tests in both configurations, the affected
  pinned Miri controls for any unsafe or pointer changes, doc drift, and `./check.sh --fast`.
  Update the CI Miri manifest and its checker if new enumerated controls are added. Record native,
  Miri, doctest and performance evidence separately; no benchmark result is implied by a fixture
  pass.
