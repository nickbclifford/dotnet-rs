# Canonical reflection objects across threads

**Status:** Complete
**Outcome:** Under `multithreading`, every managed thread observes the same object for the same
type, method, field, property or assembly: `ReferenceEquals(typeof(T), typeof(T))` is true across
threads, and so is `lock (typeof(T))` mutual exclusion through the object's own monitor.

## What was built

The CLR guarantees one `Type` object per type, and `==`, `ReferenceEquals`, `ceq`, `lock`,
`ConditionalWeakTable` and the default `Dictionary<Type, _>` comparer rely on it. The VM already
had process-wide keys (`SharedReflectionRegistry` indices) but allocated objects per arena.

- `SharedReflectionRegistry` (`crates/dotnet-vm/src/state.rs`) now holds one canonical object per
  type, method, field, property and assembly key, plus a shared reverse map from assembly objects
  to their resolutions. Every arena traces the tables as roots, alongside static storage (see
  [Shared reflection roots](../GC_AND_MEMORY_SAFETY.md#shared-reflection-roots)).
- `ReflectionLocalState` stays a per-thread front cache holding the canonical (possibly foreign)
  `ObjectRef`, so `typeof` on the hot path is a local hash lookup.
- The support `RuntimeType`, method, constructor and field wrappers carry only a registry index;
  assigning a foreign object's sync-block index writes a scalar without a foreign owner barrier.
- The per-type monitor workaround (`runtime_type_monitors`) was removed; the canonical object's own
  sync block is correct.

This plan let the first requester allocate the object in its own arena;
[plan 26](26-thread-lifecycle-and-multicast-fallback.md) moved canonical objects into the dedicated
finalizer executor's arena through the
[canonical allocation service](../THREADING_AND_SYNCHRONIZATION.md#canonical-allocation-service).

Not chosen: index-based equality (fixes `==` and `Dictionary` but not `ReferenceEquals`, `ceq`,
`lock` or `ConditionalWeakTable`) and a new type representation (the key was already correct).

## Out of scope

User-defined `Type` subclasses and `TypeDelegator` (guest-owned identity); collectible types and
unloading (canonical objects live until shutdown, as on the CLR).

## Remaining limit

Member identity uses the existing cache keys. A property accessor is not identical to the object
`GetMethod` returns for the same method, and the CLR's `ReflectedType` behavior (distinct
`MethodInfo`s for one method reached through different types) is not modeled. Tracked in the plans
README, "Not yet planned".

## Accepted cost and checks

The single-threaded `runtime_type` benchmark (30,000 `typeof` lookups per iteration) measured
52.292 ms before and 51.864 ms after (−0.82%, 95% interval −1.28% to −0.28%), within noise:

```bash
cargo bench -p dotnet-benchmarks --bench runtime_type --no-default-features
```

`crates/dotnet-cli/tests/fixtures/threading/canonical_type_objects_42.cs` and
`canonical_member_objects_42.cs` cover both owner directions, dictionary lookup, concurrent
publication, use after the creator exits and collection, and type and static-synchronized
monitors. They pass under `multithreading` and `multithreading,validation-all` and match `dotnet`
(`scripts/diff_run.sh`).
