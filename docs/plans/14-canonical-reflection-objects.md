# Canonical reflection objects across threads

**Status:** In progress
**Outcome:** Under `multithreading`, every managed thread observes the same object for the same
type, method, field, property or assembly: `ReferenceEquals(typeof(T), typeof(T))` is true across
threads, and so is `lock (typeof(T))` mutual exclusion through the object's own monitor.

## Problem

The CLR guarantees one `Type` object per type in the process: `System.Type` documents that two
`Type` references denote the same type only when they are the same object. `RuntimeType`'s
`==`, `object.ReferenceEquals`, `ceq` on object references, `lock`, `ConditionalWeakTable` and
the default `Dictionary<Type, _>` comparer (`RuntimeHelpers.GetHashCode`) all rely on that
identity.

dotnet-rs keeps the type key process-wide but the object per arena:

- `SharedReflectionRegistry` (`crates/dotnet-vm/src/state.rs`) maps each `RuntimeType`, method
  and field to one process-wide index.
- `ReflectionLocalState` (`crates/dotnet-vm/src/state.rs`) maps the same keys to `ObjectRef`s
  allocated in the calling thread's arena. `common::get_runtime_type`
  (`crates/dotnet-intrinsics-reflection/src/common.rs`) allocates a fresh object the first time
  each thread asks, as do `get_runtime_method_obj`, `get_runtime_field_obj`, the property cache
  and `runtime_asms`.

So two threads compare equal by index but not by identity. Visible today:

- `typeof(T) == typeof(T)` and `ReferenceEquals` are false when the two sides come from
  different threads.
- `Dictionary<Type, _>` filled on one thread misses lookups from another.
- `lock (typeof(T))` and static `[MethodImpl(Synchronized)]` methods lock different objects
  per thread. A monitor-only workaround hides this:
  `runtime_type_monitors` publishes one sync block per `RuntimeType`, and Monitor on a type
  object redirects to it. Identity is still split.

Without `multithreading` there is one arena, so the problem doesn't occur.

## Scope

In:
- `Type` objects created by the VM: `ldtoken` + `GetTypeFromHandle`, `object.GetType()`,
  `Type.GetType(string)`, `MakeGenericType`/`MakeArrayType`/`MakeByRefType`/
  `MakePointerType` and anything else that reaches `get_runtime_type`.
- `MethodInfo`/`ConstructorInfo`, `FieldInfo`, `PropertyInfo` and `Assembly` objects from the
  same per-arena caches. Two `GetMethod` calls returning one object is the CLR's observed
  behavior, and code that keys dictionaries or `ConditionalWeakTable`s on members relies on it.

Out:
- User-defined `Type` subclasses and `TypeDelegator`: guest code allocates these, and identity
  is the guest's own.
- Collectible types and unloading: the VM has no collectible `AssemblyLoadContext`, so canonical
  objects live until shutdown, as on the CLR.
- Cross-thread GC handles (the plans README "Not yet planned" bullet). That bullet only becomes
  a dependency if a reflection object stores a `GCHandle` that another thread dereferences.
  Check this in slice 1.

## Design

Keep one canonical object per key for the whole process, published in a shared table that
every arena traces as a root. This is the same mechanism shared static storage uses today:
`StaticStorageManager` is traced from each arena's `CallStack` root, and references it holds into
other arenas are marked through the cross-arena fixed-point phase.

- The first thread to request a key allocates the object in its own arena and publishes it with
  insert-if-absent. A thread that loses the race drops its object, which becomes garbage, and
  uses the winner's.
- Owner lifetime needs no new protocol: an arena that owns canonical objects is retained until
  VM shutdown (plan 09's retired-arena contract), and the shared table roots the objects.
- `ReflectionLocalState` stays as a per-thread front cache, holding the canonical (possibly
  foreign) `ObjectRef`. That keeps `typeof` on the hot path a local hash lookup.
- `resolve_runtime_type` and the index field are unchanged: the object still carries its
  process-wide index.
- Remove `runtime_type_monitors`: the canonical object's own sync block is correct.

Not chosen:
- **Index-based equality:** patching `Type.op_Equality`, `Equals` and `GetHashCode` fixes
  `==` and `Dictionary` only. It doesn't fix `ReferenceEquals`, `ceq`, `lock` or
  `ConditionalWeakTable`.
- **A new type representation:** the key is already process-wide and correct. Only the
  object's owner is wrong.

## Slices

1. **Canonical `Type` objects.**
   - Add the shared traced table for `RuntimeType`, route `get_runtime_type` through it, and
     delete `runtime_type_monitors`.
   - Confirm a foreign `ObjectRef` held in `ReflectionLocalState` is traced as a cross-arena
     reference. Confirm that the support `RuntimeType`'s fields stay valid when written from a
     non-owner thread (write barrier) and hold no per-arena `GCHandle`.
   - Fixture (`multithreading`), on a worker and main: `ReferenceEquals(typeof(T), typeof(T))`,
     `obj.GetType() == typeof(T)` with `obj` made on the other thread, a `Dictionary<Type, int>`
     filled on one thread and read on the other, `lock (typeof(T))` mutual exclusion, and a
     static `Synchronized` method still excluding.
   - Compare the fixture against the CLR with `scripts/diff_run.sh`.
   - Implemented: the shared type-object table is traced from each arena, and local caches retain
     its canonical objects. The support object contains only a registry index; foreign monitor
     assignment writes a scalar without applying a foreign owner barrier. See
     [GC and memory safety](../GC_AND_MEMORY_SAFETY.md#shared-reflection-roots).
   - `canonical_type_objects_42.cs` covers both owner directions, dictionary lookup, concurrent
     publication, collection after the creator exits, and type/static synchronized monitors. It
     matches the CLR via `scripts/diff_run.sh` and passes the multithreading integration tests
     with and without `validation-all`.
2. **Canonical member and assembly objects.** Route `get_runtime_method_obj`,
   `get_runtime_field_obj`, the property cache and `runtime_asms` through the same table. The
   fixture compares `GetMethod`/`GetField`/`GetProperty`/`Assembly` results by reference across
   threads.
   - Implemented: member and assembly caches publish canonical objects through shared traced
     tables. Assembly resolution has a shared reverse lookup for objects passed between threads.
     `canonical_member_objects_42.cs` exercises both owner directions, concurrent publication,
     and use after the creator exits and collection runs.
   - The existing member cache keys are retained. A property accessor is not currently identical
     to the result of looking it up with `GetMethod`; reconciling those metadata paths and
     `ReflectedType` behavior remains separate from cross-thread publication.

## Done when

- Both fixtures pass under `multithreading` and `multithreading,validation-all`, and match
  `dotnet`.
- `runtime_type_monitors` and every other per-type identity workaround are gone.
- `docs/GC_AND_MEMORY_SAFETY.md` describes the shared reflection table as a root alongside
  static storage.
- The single-threaded `typeof` benchmark shows no regression beyond noise.

## Ask the maintainer only if

- A canonical object needs a dedicated shared arena instead of the first requester's.
- A reflection object stores a `GCHandle` or other per-arena state that a foreign thread must
  dereference. That pulls in cross-thread GC handles.
- Matching the CLR requires member identity to differ from what the CLR documents. For example,
  `ReflectedType` makes the CLR return distinct `MethodInfo`s for the same method reached
  through different types.
