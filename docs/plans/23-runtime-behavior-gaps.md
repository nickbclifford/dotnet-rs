# Runtime behavior gaps: static field reflection, Math FCalls, Array.Fill, Span.CopyTo, signed byref offsets and generic arity

**Status:** Not started
**Outcome:** Differential fixtures for static `FieldInfo.GetValue`/`SetValue`, the extern `Math`
methods, `Array.Fill`, overlapping `Span<T>.CopyTo`, negative `Unsafe.Add`/`Unsafe.Subtract` and
leaked-generics calls all match `dotnet`. The buffered-stdout entry is either reproduced and
fixed or closed with its repro fixture as evidence.

Closes these README "Not yet planned" entries: **Static field reflection**, **Support
`Span<T>.CopyTo` is a per-element loop**, **Negative and wide managed-pointer offsets**, and every
sub-bullet of **Other missing or incorrect runtime behavior** (buffered stdout, extern
`System.Math`, `Array.Fill<T>`, generic-arity checks). The access violation in
`RuntimeType.GetNativeTypeHandle` is only reached through `DynamicMethod`, so
[plan 24](24-reflection-emit-investigation.md) covers it.

## Problem

- **Static field reflection.** `DotnetRs.FieldInfo.GetValue`
  (`crates/dotnet-intrinsics-reflection/src/fields.rs`, around line 230) pushes null for a static
  non-literal field. `SetValue` (around line 338) throws `NotImplementedException`. The CLR reads
  and writes the static storage, runs the type initializer first if it hasn't run, and throws
  `FieldAccessException` from `SetValue` on a static `initonly` field once the type is initialized.
  `AttributeCollection.GetDefaultAttribute` currently relies on the null. Plan 21 slice 4's
  pointer-slot fixture covers instance pointer fields only for this reason.
- **Extern `System.Math`.** `crates/dotnet-intrinsics-core/src/math.rs` registers only `Abs`,
  `Min`, `Max`, `Pow` and `Sqrt` (some overloads). Every other `Math` or `MathF` method that is an
  FCall with no IL body (`Ceiling`, `Floor`, `Round(double)`, `Truncate`, `Sin`, `Cos`, `Tan`,
  `Atan2`, `Exp`, `Log`, `Log10`, `Cbrt`, `FusedMultiplyAdd`, …) fails with "no body in executing
  method" (`call_ops_impl.rs`). `Stopwatch.Elapsed.TotalMilliseconds` reaches `Math.Round`'s
  callees.
- **`Array.Fill<T>`.** The support `DotnetRs.Array`
  (`crates/dotnet-assemblies/src/support/Array.cs`) replaces `System.Array` and has no `Fill`
  overloads, so callers get `MethodNotFound`.
- **`Span<T>.CopyTo`.** `Span<T>.CopyTo` and `ReadOnlySpan<T>.CopyTo` (`support/Span.cs` lines
  87 and 229) copy one element at a time in interpreted IL and always go forward. That is slow
  for large spans, and when the spans overlap with the destination after the source, it smears
  the source. The CLR's `Buffer.Memmove` handles overlap. `TryCopyTo` shares the bug if it is
  implemented the same way.
- **Signed managed-pointer offsets.** `ManagedPtr` stores a compact unsigned offset, so
  `StackValue` arithmetic (`managed_offset` in `crates/dotnet-value/src/stack_value.rs`) returns
  `NotImplemented` for any byref below its base, even transiently (`Unsafe.Subtract`,
  `Unsafe.Add(ref r, -1)`, `ref Unsafe.Add(ref r, i - 1)` loops). CoreLib's span helpers use these
  patterns.
- **Generic arity.** Intrinsics that read method generics use `MethodDescription::own_method_arg`,
  except `Activator.CreateInstance<T>` (`native_effects.rs` around line 800, which reads
  `entry_generics.cloned_method_arg(0)`). Intrinsics that read *type* generics
  (`Span<T>.get_Item`, `EqualityComparer<T>.Default`, …) are not checked against the declaring
  type's arity. Constrained calls and effect resumption have not been audited for caller generics
  leaking into callees. Commits 31f94e72 and 0e971baa fixed the two instances found so far.
- **Buffered stdout on internal-error exit.** Reported but never reproduced. The VM has no stdout
  buffer of its own (`Console` writes go to `write(2)` through the native shim, and the CLI flushes
  on every exit path).

## Scope

In: the items above.

Out:
- R10/R11 GC write barriers for bulk copies. `CopyTo` for reference-containing `T` stays an
  element-wise copy, which already goes through the ordinary barriered store, and gains only the
  overlap direction. Only reference-free `T` routes to `Buffer.Memmove`, as `Clear` already routes
  to `initblk`.
- Byref offsets past the compact range in the *positive* direction, if the compact width is
  already wide enough for every valid object (ECMA-335 III.1.1.5.2 does not let a byref outlive
  its object's extent). Widening goes in only if a fixture needs it.
- Dereferencing an out-of-range byref. That stays a bounds-checked error, and only forming and
  carrying the byref becomes legal.
- Reflection access checks beyond `initonly`/literal (`FieldInfo` from `GetField` already applies
  visibility through binding flags).
- Other missing `DotnetRs.Array` members. Record any found in the README, but add only `Fill`.

## Design

- **Static field reflection:** resolve the field's owning concrete type through the existing
  `resolve_runtime_field` lookup, then reuse the static-storage path that `ldsfld`/`stsfld`
  (`instructions/objects/fields.rs`) use, including the type-initializer check. The initializer
  runs through the same native-effect suspension `ldsfld` uses (plan 16 retired the static-init
  retry). Box the value for `GetValue`, and unbox or convert through the same `CTSValue` path the
  instance branch uses for `SetValue`. Throw `FieldAccessException` for `initonly` statics after
  initialization, matching the CLR from .NET Core 3.0 on.
- **Math:** register handlers with `#[dotnet_intrinsic]` for each `Math`/`MathF` method whose
  CoreLib definition has no IL body. Get the list from the loaded CoreLib (methods with
  `InternalCall`/no body), not by hand: add a unit test, next to the existing `classify` test in
  `crates/dotnet-vm/src/intrinsics/metadata.rs`, that walks `System.Math` and `System.MathF` and
  asserts that no bodiless method classifies as `Missing`. Use Rust `f64`/`f32` methods. Where Rust
  and the CLR differ (`Round` is banker's rounding through `round_ties_even`, and `Math.Round` with
  `MidpointRounding` has an IL body), follow the CLR.
- **Array.Fill:** add `Fill<T>(T[], T)` and `Fill<T>(T[], T, int, int)` to `support/Array.cs` with
  the CLR's argument checks, implemented as `new Span<T>(array, start, count).Fill(value)` if
  support `Span<T>.Fill` exists, or as a plain loop otherwise.
- **CopyTo:** for `!RuntimeHelpers.IsReferenceOrContainsReferences<T>()`, call
  `Buffer.Memmove(ref dest, ref src, (nuint)_length)` (registered in
  `crates/dotnet-intrinsics-unsafe/src/buffer.rs`). For reference-containing `T`, keep the loop
  but copy backward when the destination starts after the source (`Unsafe.IsAddressGreaterThan`),
  as `Buffer.Memmove` does. Apply the same to `TryCopyTo`.
- **Signed offsets:** make the compact offset signed (or store a signed displacement from the
  base) so `checked_offset` accepts negative results, and keep every dereference going through the
  existing bounds check against the owner's extent. Check that `ManagedPtr::SIZE` and the
  serialized layout don't change. If they must change, stop and ask (see below). Pointer
  comparison and `ByteOffset` must stay correct across zero.
- **Generic arity:** switch `Activator.CreateInstance<T>` to `own_method_arg`. Add
  `MethodDescription::own_type_arg` (checked against `parent.definition().generic_parameters`,
  mirroring `own_method_arg`) and move the type-generic reads in intrinsics onto it. Audit
  constrained calls (`constrained.` prefix resolution) and effect resumption (what generic lookup a
  resumed intrinsic sees). Add a leaked-generics fixture for each path, modelled on the Memmove
  checks from b0021220.
- **Buffered stdout:** write the repro the README asks for: a fixture that writes to stdout, then
  hits an `Internal VM error`. Use a debug-only trigger if no natural one exists, and check it
  under both a pipe and a file. If the output survives, delete the entry and cite the fixture. If
  it doesn't, fix the flush path.

## Slices

Each slice ends with the VM using the new path, its fixtures enabled in the diff harness, and
`cargo clippy --workspace --all-targets -D warnings` (both feature sets), the doc-drift check and
the full `dotnet-cli` + `dotnet-vm` matrix (`cargo run -p xtask -- matrix test-features`) passing.

1. **Math FCalls.** Add the bodiless-method coverage test, then the handlers it requires. Add a
   `math_extern_42` differential fixture covering rounding modes, NaN/±∞/−0, and
   `Stopwatch.Elapsed.TotalMilliseconds` (print only a sanity comparison, never the value).
2. **`Array.Fill`.** Add both overloads and an `array_fill_42` fixture: value and reference
   element types, range overload, argument exceptions.
3. **`Span<T>.CopyTo`.** Route reference-free `T` to `Memmove` and add the overlap direction for
   reference `T`. Add a `span_copyto_overlap_42` fixture (forward and backward overlap, both `T`
   kinds, `ReadOnlySpan`, `TryCopyTo`) and a benchmark or timing note for a large byte copy.
4. **Signed byref offsets.** Change the representation and add unit tests in `stack_value.rs`
   (subtract below base, add back, compare, `ByteOffset` sign). Add a `byref_negative_offset_42`
   fixture using `Unsafe.Subtract` and `Unsafe.Add(…, -n)` over arrays, fields and locals. Run the
   pointer-provenance Miri controls, because this touches `ManagedPtr`.
5. **Generic arity.** Make the `CreateInstance<T>` fix, add `own_type_arg` and migrate callers,
   audit constrained calls and effect resumption, and add a fixture per path. Record the audit
   result (paths checked, fixes) in this plan.
6. **Static field reflection.** Implement `GetValue`/`SetValue` for statics, with
   type-initializer triggering and the `initonly` check. Add a `reflection_static_fields_42`
   fixture: primitive, struct, reference, pointer static, cctor side-effect ordering and the
   `initonly` exception. Extend plan 21's pointer-slot fixture to cover static pointer fields.
   Rerun the Newtonsoft and EF host rungs, because `AttributeCollection` defaults change behavior.
7. **Buffered stdout.** Add the repro fixture, then fix the flush path or close the entry.
8. **Docs.** Remove the closed README entries, update `docs/COMPATIBILITY.md` where it lists any of
   these as unsupported, and mark this plan complete.

## Done when

- Every fixture named above matches `dotnet` in the diff harness in both feature sets.
- The bodiless-`Math`/`MathF` coverage test passes.
- No intrinsic reads a method or type generic except through `own_method_arg`/`own_type_arg`.
- The Newtonsoft and EF host rungs still match `dotnet`.
- The README entries listed at the top are gone, and the buffered-stdout one has either a fix or
  its repro cited.

## Ask the maintainer only if

- Signed offsets need a change to `ManagedPtr::SIZE` or its serialized layout.
- Static field reflection needs a new suspension mechanism rather than the one `ldsfld` uses.
- The bodiless-`Math` list includes methods whose CLR results are platform-dependent
  (for example transcendental functions where Rust's libm and the CLR's CRT disagree in the
  last ulp) and a fixture can't be made exact.
