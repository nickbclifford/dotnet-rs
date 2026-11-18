# String interning

**Status:** Complete
**Outcome:** String literal identity, `string.Empty` identity and `String.Intern`/`IsInterned`
match `dotnet` in a diff fixture under both feature sets, including the same literal loaded on two
threads.

## What was built

Before this plan, `ldstr` allocated a new string on every execution (ECMA-335 §III.4.16 requires
one object per literal), `string.Empty` allocated per load, and `String.Intern`/`IsInterned`
reached CoreCLR QCalls and threw `DllNotFoundException`.

- **One process-wide intern table** keyed by UTF-16 content holds one canonical string per key,
  using the canonical-object mechanism of [plan 14](14-canonical-reflection-objects.md). Every
  arena traces it as a root; single-threaded builds use the same table. Under `multithreading`,
  new literals and `String.Empty` are homed in the dedicated finalizer executor's arena through the
  [canonical allocation service](../THREADING_AND_SYNCHRONIZATION.md#canonical-allocation-service)
  ([plan 26](26-thread-lifecycle-and-multicast-fallback.md)).
- **`ldstr`** looks up the table by its character slice without allocating on a hit.
- **`string.Empty`** returns the interned `""`, and so do intrinsics whose CLR counterpart returns
  `string.Empty` (for example `new string((char*)null)` and `new string('a', 0)`).
- **`String.Intern` and `String.IsInterned`** are intercepted at the public methods, above the
  QCalls. `Intern` throws `ArgumentNullException` on null and publishes its supplied object when
  the key is absent, matching .NET's observed identity; that object's original arena then stays
  retained through shutdown. `IsInterned` returns the canonical object or null.

## Decisions

- Computed strings are not interned (the CLR does not either).
- The table lives for the process lifetime, as on the CLR.
- `CompilationRelaxations.NoStringInterning` is ignored, as on the CLR.
- Frozen or preallocated string segments are an unobservable CLR detail and not modeled.
- No per-thread front cache: whole-process instructions for `string --test` changed +0.03% and
  `eval_stack_push_i32_pop_safe` +0.99% (startup-dominated), measured with
  `perf stat -r 3 -e instructions:u` on the benchmark executables with `--test`.

## Remaining limit

There is no Loom model for the intern table (the shared-table models avoid strings because the
interner is global). Tracked in the plans README, "Not yet planned".

## Checks

`strings/string_interning_42.cs` (single-threaded and multithreaded) and
`threading/string_interning_mt_42.cs` (multithreaded only: two threads exchange literals and
`string.Empty`, and race `String.Intern`) are diff fixtures.
