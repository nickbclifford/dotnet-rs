# Plan 08 slice 4: static fields through initialization effects

**Status:** Complete
**Outcome:** `ldsfld`, `stsfld` and `ldsflda` check published initialization state and use
`ensure_initialized` for slow states in both configurations, without replaying the field
instruction. [Plan 16](../16-retire-legacy-continuations.md) slice 7 moved the other triggers
(`newobj`, eligible calls, `RunClassConstructor`) onto the same effect.

## Current design

`ldsfld`, `stsfld` and `ldsflda` (`instructions/objects/fields.rs`) resolve the field and its
canonical static generic lookup, then `VesContext::start_static_field_effect` completes the field
operation synchronously when the acquire read reports `Initialized`, or `Initializing` owned by the
current thread. Every other state starts an `ensure_initialized` activation. `stsfld` keeps its
operand in the traced completion action only when an activation is used; intrinsic fields bypass
initialization. The instruction IP advances once, after the field operation succeeds; an
exception or host failure discards the pending operation without a partial store. `ldsflda`
produces a `PointerOrigin::Static` byref.

`StaticStorageManager::init` (`crates/dotnet-vm/src/statics.rs`) keys storage by type and generic
lookup; `InitState` encodes state and owner in one atomic word with compare-and-swap transitions.
`Execute` transfers an `InitializerClaim` that the effect retains; explicit publication or an
abandoned claim (abort, cancellation, host failure) publishes success or failure and wakes
waiters, and its destructor runs no managed code (`F6.InitializerClaimLifetime`).

`ensure_initialized` runs the `.cctor` through the capturing `try_call`: the child's handlers,
filters and cleanup run first, then Rust receives the rooted exception and publishes a
`TypeInitializationException` with the original inner exception before the caller's filter runs.
The failure carrier is traced (`F6.InitializerFailureRoot`). Under `multithreading`, a waiting
activation owns its wait-graph edge outside the mutation, removes it on every terminal path and
repeats the cycle check after a GC stop; a cycle follows the `Recursive` policy (ECMA-335
§II.10.5.3.3). Field access initializes on access regardless of `beforefieldinit`.

The initialized/recursive shortcut was justified by measurement: on a mixed load/store/address
workload it improved every pair, by −39.37% (ST) and −34.17% (MT) on average.

## Verification

- CLR comparisons: `statics/cctor_filter_42.cs` (the filter sees the wrapper and its inner
  exception) and the other `statics/cctor_filter_*_42.cs` triggers; `cctor_wait_{success,failure}_42`
  and `cctor_wait_triggers_{success,failure}_42` under `multithreading`; `circular_init_42` and
  `circular_init_mt_42`.
- Pinned Tree Borrows Miri singletons in `executor::native_substring_tests`:
  `static_store_operand_released_after_abandonment` and
  `native_initializer_failure_is_published_with_original_inner` (both configurations),
  `native_initializer_failure_survives_owner_retirement_and_collection` and
  `native_initializer_wait_rechecks_cycle_after_real_collection` (multithreading). Selectors and
  flags are in [CI](../../CI.md#miri). This is permissive Tree Borrows with leaks ignored, not
  strict-provenance evidence.
