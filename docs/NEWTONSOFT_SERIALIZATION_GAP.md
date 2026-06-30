# Newtonsoft host-mode serialization gap — root cause + fix plan

Date: 2026-06-29 · Branch: `supervised/nuget-host-runner` · Status: **diagnosed, not yet fixed**

This is the standing **rung-2** blocker (CHECKLIST 5.15): host-mode Newtonsoft prints `{}` instead of
`{"name":"test","value":42}` (exit code 42 matches; only stdout diverges). Earlier steps 5.16–5.20
fixed *synthetic* repros derived from the investigation but never moved the real probe off `{}`. This
doc records the **empirically confirmed** root cause and a concrete, templated fix so the next step can
go straight at it.

## The acceptance test (still failing)

```bash
# rebuild the ephemeral probe if /tmp was cleared (see REVIEW.md F-TEST-001 for the csproj/Program.cs)
bash scripts/diff_run.sh /tmp/nuget-probe/App.csproj
# stock dotnet : {"name":"test","value":42}   exit 42
# dotnet-rs    : {}                            exit 42   ← only stdout wrong
```

## Confirmed root cause: reflection objects are not equality/identity-stable across calls

Newtonsoft's `DefaultContractResolver.GetSerializableMembers` discovers members with reflection and
then uses `List<MemberInfo>.Contains` / set operations (which call `MemberInfo.Equals` /
`GetHashCode`) to merge and de-duplicate. dotnet-rs returns a **fresh** `PropertyInfo` object on every
`GetProperties()` call, and those objects are reference-equal only to themselves, so every membership
check fails and **every property is dropped → `{}`**.

### Differential probe (reproducible, no Newtonsoft needed)

`/tmp/refl_probe.cs` (run with `bash scripts/diff_run.sh /tmp/refl_probe.cs`):

```csharp
using System; using System.Linq; using System.Reflection; using System.Collections.Generic;
public class Item { public string name { get; set; } public int value { get; set; } }
public class Program {
    public static int Main() {
        var t = typeof(Item);
        var f = BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance;
        var p1 = t.GetProperties(f);
        var p2 = t.GetProperties(f);
        Console.WriteLine("count1=" + p1.Length + " count2=" + p2.Length);
        Console.WriteLine("refEq=" + ReferenceEquals(p1[0], p2[0]));
        Console.WriteLine("valEq=" + p1[0].Equals(p2[0]));
        Console.WriteLine("hashEq=" + (p1[0].GetHashCode() == p2[0].GetHashCode()));
        var list = new List<MemberInfo>(p1);
        Console.WriteLine("contains=" + list.Contains(p2[0]));
        Console.WriteLine("distinctUnion=" + p1.Concat(p2).Cast<MemberInfo>().Distinct().Count());
        return 42;
    }
}
```

| primitive | stock `dotnet` | dotnet-rs (now) |
|---|---|---|
| `GetProperties().Length` | 2 | **2 ✓** (5.13 works) |
| `ReferenceEquals` across 2 calls | True | **False** |
| `PropertyInfo.Equals` | True | **False** |
| `GetHashCode` equal | True | **False** |
| `List<MemberInfo>.Contains` | True | **False** |
| `Distinct()` of union | 2 | **4** |
| `CanRead/CanWrite/Get/SetMethod/MemberType/DeclaringType` | ✓ | ✓ (all match) |

The **only** broken primitive is equality/identity. Everything else about property reflection is
correct, which is why this is the precise next thing to fix.

### Why methods/fields don't hit this (the asymmetry to copy)

`GetMethods()` / `GetFields()` already return identity-stable objects because their reflection objects
are **cached**:

- `crates/dotnet-intrinsics-reflection/src/common.rs` — `get_runtime_method_obj` and
  `get_runtime_field_obj` check a cache first (`reflection_cached_runtime_method_obj` /
  `…_field_obj`) and only allocate + cache on miss.
- Cache storage: `crates/dotnet-vm/src/state.rs` — `runtime_method_objs` / `runtime_field_objs`
  (`HashMap<(MethodDescription|FieldDescription, GenericLookup), ObjectRef>`).
- Trait surface: `crates/dotnet-intrinsics-reflection/src/lib.rs` (`reflection_cached_/cache_…`).
- Impl: `crates/dotnet-vm/src/stack/reflection_ops_impl.rs`.

**Properties have no such cache.** `create_runtime_property_obj`
(`crates/dotnet-intrinsics-reflection/src/types/type_members.rs:1108`) allocates a fresh
`DotnetRs.PropertyInfo` every call. That is the whole bug.

## Recommended fix (Route 1 — cache property objects; mirrors methods/fields)

Add a property-object cache exactly like the field cache. This fixes `refEq`, `valEq`, `hashEq`,
`contains`, and `distinct` in one shot, and keeps reflection-object architecture consistent.

Key choice: `PropertyCandidate { name, getter: Option<MethodDescription>, setter: Option<MethodDescription> }`.
A property always has ≥1 accessor, and `MethodDescription` is already `Hash + Eq` (it keys the method
cache). **Key the property cache on `(accessor, GenericLookup)` where `accessor = getter ?? setter`.**
Two `GetProperties()` calls produce candidates with equal accessor `MethodDescription`s → cache hit →
same `ObjectRef`.

Edit sites (each mirrors the existing field-cache code):

1. `crates/dotnet-vm/src/state.rs`
   - add field `runtime_property_objs: RefCell<HashMap<(MethodDescription, GenericLookup), ObjectRef<'gc>>>`
     next to `runtime_field_objs` (~line 619), init it in `new` (~633),
   - **and add a GC-trace loop** `for o in self.runtime_property_objs.borrow().values() { o.trace(cc); }`
     beside the existing field/method trace loops (~656–661). **This is mandatory** — without it the
     cached property objects can be collected and you reintroduce the `Invalid magic in ObjectInner`
     class of bug fixed in 5.11.
   - add `property_objs_read()/write()` accessors (~754/760, mirror `field_objs_*`).
2. `crates/dotnet-intrinsics-reflection/src/lib.rs` — add
   `reflection_cached_runtime_property_obj(&self, accessor: &MethodDescription, lookup: &GenericLookup)`
   and `reflection_cache_runtime_property_obj(...)` to `ReflectionIntrinsicHost` (mirror the field pair).
3. `crates/dotnet-vm/src/stack/reflection_ops_impl.rs` — implement the two trait methods (mirror ~443–463).
4. `crates/dotnet-intrinsics-reflection/src/types/type_members.rs::create_runtime_property_obj` — at the
   top compute `let accessor = property.getter.clone().or_else(|| property.setter.clone());` and if
   `Some`, check the cache → early-return on hit; before `Ok(...)`, insert into the cache.

### Alternative (Route 2 — value equality)
Override `Equals`/`GetHashCode` on `DotnetRs.PropertyInfo` (support `.cs` + intrinsic) comparing the
cached accessor `MethodInfo` references (those are already identity-stable) or the property metadata
identity. More faithful to how .NET *defines* reflection equality, but does not fix `refEq` and is more
surface area. Prefer Route 1; consider Route 2 only if a path constructs `PropertyInfo` outside the
cache.

## Verify the fix against the REAL probe, not a synthetic one

This is the cautionary lesson from 5.16–5.20. After implementing:

1. `bash scripts/diff_run.sh /tmp/refl_probe.cs` → all rows must match stock (refEq/valEq/hashEq/contains True, distinct 2).
2. `bash scripts/diff_run.sh /tmp/nuget-probe/App.csproj` → **must** print `{"name":"test","value":42}` exit 42.
3. `DOTNET_USE_PREBUILT_FIXTURES=1 cargo nextest run --no-default-features -p dotnet-cli` → 158/158, no regressions.

If (2) still diverges after (1) passes, member discovery is fixed and the next blocker is **downstream**
of it (e.g. reading property values via the getter `MethodInfo` — see the 5.17–5.19 invoke saga — or
contract caching). Record that as the next step rather than chasing a synthetic repro.

## Secondary finding (tangential — NOT on the rung-2 critical path)

The diagnostic probe also tripped a separate crash when formatting an enum via an **interpolated
string** (`$"{someEnum}"`): `System.InvalidCastException` in `System.Enum.TryFormatUnconstrained` via
`DefaultInterpolatedStringHandler.AppendFormatted<T>`. This is a *different* path than the
`System.Enum::ToString()` intrinsic added in 5.20. The `Item` probe has no enums, so this does **not**
block rung-2 here; it is a real but independent gap (tracked as CHECKLIST 5.22). Fix opportunistically,
not as part of the property-cache work, to avoid re-fragmenting the rung-2 effort.
