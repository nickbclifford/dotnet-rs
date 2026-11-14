# String interning

**Status:** Not started
**Outcome:** String literal identity, `string.Empty` identity and `String.Intern`/`IsInterned`
match `dotnet` in a diff fixture under both feature sets, including the same literal loaded on two
threads.

Closes the `string.Empty` bullet under the README's **Other missing or incorrect runtime
behavior** entry.

## Problem

- **`ldstr` never interns.** `ldstr` (`crates/dotnet-vm/src/instructions/objects/mod.rs`)
  allocates a new `CLRString` on every execution. ECMA-335 III.4.16 requires two `ldstr`
  instructions with the same characters to return the same object, so
  `ReferenceEquals("a", "a")` is false here and true on the CLR. This also holds for the same
  `ldstr` executed twice, such as in a loop.
- **`string.Empty` allocates per load.** `intrinsic_field_string_empty`
  (`crates/dotnet-intrinsics-string/src/accessors.rs`) pushes a fresh empty string, so
  `ReferenceEquals(string.Empty, string.Empty)` and `ReferenceEquals(string.Empty, "")` are both
  false. CoreLib code that returns `string.Empty` for empty results inherits this.
- **Empty results built by intrinsics.** The pointer string constructor
  (`crates/dotnet-intrinsics-string/src/constructors.rs`) builds `CLRString::new(Vec::new())` for
  a null pointer. The CLR's `new string((char*)null)` and constructors of zero length return
  `string.Empty`. Other intrinsics may build empty results through `push_string` with an empty
  buffer.
- **`String.Intern`/`IsInterned`.** The .NET 10.0.14 CoreLib forwards both to CoreCLR QCalls
  (`String_Intern`, `String_IsInterned`). The VM intercepts neither, so they likely fail today.
  Confirm in slice 1.

## Scope

In: `ldstr`, `string.Empty`, `String.Intern`, `String.IsInterned`, and VM intrinsics whose CLR
counterpart returns `string.Empty`, in single-threaded and `multithreading` builds.

Out:
- Interning computed strings. The CLR does not intern them either; only literals and explicit
  `String.Intern` calls are interned.
- Collecting interned strings. The CLR keeps its intern table for the process lifetime, and this
  plan roots interned strings until VM shutdown.
- `CompilationRelaxations.NoStringInterning`. The CLR ignores it and interns anyway, so the VM
  ignores it too.
- Frozen or preallocated string segments. That is a CLR allocation detail with no visible
  semantics.

## Design

- **One process-wide intern table** keyed by UTF-16 content, holding one canonical string object
  per key. Reuse plan 14's canonical-object mechanism (`SharedReflectionRegistry` in
  `crates/dotnet-vm/src/state.rs`):
  - The first thread to intern a key allocates the string in its own arena and publishes it with
    insert-if-absent. A thread that loses the race drops its string and uses the winner's.
  - Every arena traces the table as a root. Plan 09's retired-arena contract keeps the owning
    arena alive until shutdown, so a foreign interned string stays valid.
  - Single-threaded builds use the same table with a single owner. Keep one code path.
- **`ldstr`** looks up the table by its character slice. The lookup must not allocate a
  `CLRString` when it hits.
- **`string.Empty`** returns the interned `""`. Intrinsics whose CLR counterpart returns
  `string.Empty` (find them by grepping for empty `CLRString` construction and empty
  `push_string` results) return it too.
- **`String.Intern` and `String.IsInterned`** get ordinary `#[dotnet_intrinsic]` intercepts on
  the public methods themselves, like `Debugger.get_IsAttached`
  (`crates/dotnet-intrinsics-core/src/debugger.rs`), not the QCalls beneath them. The QCalls are
  CoreCLR-coupled plumbing, so the VM stubs above them. `Intern` throws
  `ArgumentNullException` on null, inserts if absent and returns the canonical object.
  `IsInterned` returns the canonical object or null.
- **Cost:** interning puts a hash lookup on every `ldstr`. Add a per-thread front cache, keyed
  by the instruction's method and offset (or by content), only if measurements show a cost. Plan
  14 did the same for `typeof`.

## Slices

1. **Baseline.** Write `strings/string_interning_42.cs` covering:
   - literal identity within a method, across methods and in a loop;
   - `string.Empty` against itself and against `""`;
   - `String.Intern` of a computed string returning the literal's object;
   - `IsInterned` of a non-literal returning null, then the canonical object after `Intern`;
   - `new string((char*)null)` and `new string('a', 0)` against `string.Empty`.

   Run it on `dotnet` and the VM and record which cases fail and how. Also record `string` and
   `runtime_primitives` benchmark `perf stat` instruction counts. The `string` benchmark's
   wall-clock time is sensitive to code placement (README), so compare instructions.
2. **Intern table, `ldstr` and `string.Empty`.** Add the table, route `ldstr` and
   `string.Empty` through it, register the fixture in the diff harness with the cases that now
   pass, and record benchmark deltas here. If `ldstr` instructions rise measurably, add the front
   cache in this slice.
3. **`String.Intern`/`IsInterned` and empty intrinsic results.** Add the `Intern`/`IsInterned` intercepts and
   route empty-result intrinsics to the interned `""`. Enable the remaining fixture cases.
4. **Cross-thread identity.** Add `threading/string_interning_mt_42.cs` (multithreading-only;
   list it in `crates/dotnet-cli/build.rs` with the other MT fixtures): two threads load the
   same literal and `string.Empty`, exchange them, and compare with `ReferenceEquals`. Also cover
   a `String.Intern` race between threads. Run the Miri tree-borrows lane that covers cross-arena
   roots and the Loom shared-table test if the table type compiles under `cfg(loom)`.

Each slice ends with `cargo clippy --workspace --all-targets -D warnings` (both feature sets),
the doc-drift check and the full `dotnet-cli` and `dotnet-vm` test matrices passing.

## Done when

- `string_interning_42` matches `dotnet` in both feature sets.
- `string_interning_mt_42` matches `dotnet` under `multithreading`.
- The benchmark deltas are recorded here and accepted.
- The README bullet is removed.

## Ask the maintainer only if

- Interning adds more than ~5% instructions to the `string` or `runtime_primitives` benchmark
  even with a front cache.
- Publishing a canonical string from a foreign arena needs a mechanism plan 14's registry and
  plan 09's retained arenas do not already provide.
