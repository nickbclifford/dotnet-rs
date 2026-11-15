# Runtime behavior gaps: static field reflection, Math FCalls, Array.Fill, Span.CopyTo, signed byref offsets and generic arity

**Status:** Complete
**Outcome:** The enabled differential fixtures for static `FieldInfo.GetValue`/`SetValue`, bodiless
Math/MathF methods, `Array.Fill`, overlapping `Span<T>.CopyTo`, signed byref offsets and generic
context match `dotnet` in both base feature sets. The buffered-stdout report is closed with its
pipe/file repro evidence.

Closes these README "Not yet planned" entries: **Static field reflection**, **Support
`Span<T>.CopyTo` is a per-element loop**, the negative-offset portion of **Negative and wide
managed-pointer offsets**, and every sub-bullet of **Other missing or incorrect runtime behavior**
(buffered stdout, extern `System.Math`, `Array.Fill<T>`, generic-arity checks). The positive compact
range limit remains documented. The access violation in
`RuntimeType.GetNativeTypeHandle` is only reached through `DynamicMethod`, so
[plan 24](24-reflection-emit-investigation.md) covers it.

## Resolved behavior

The tracked runtime gaps are implemented and exercised by enabled CLR differential fixtures.
Static field reflection uses the declaring type's initialized static storage; Math and MathF
handlers cover the loaded CoreLib's bodiless methods; both `Array.Fill<T>` overloads are present;
span copies handle overlap with `Memmove` for reference-free elements and direction-aware typed
stores for reference-containing elements. Managed-pointer arithmetic carries signed compact
displacements, and generic readers validate callee-owned type and method slots while constrained
calls and resumed native effects preserve the target context. The stdout repro confirms guest
bytes survive the internal-error exit path without a flush change. The enabled fixtures and their
verification are recorded below; the compatibility surface and its limits are summarized in
[`COMPATIBILITY.md`](../COMPATIBILITY.md).

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
   **Audit record.** `call_constrained` resolves the implementation on every dispatch using a
   metadata-only static-constrained cache, then builds the implementation's type lookup from the
   closed constraint (or the declaring interface lookup for a default body), retaining method
   arguments. `resolve_static_constrained_method_uncached` resolves MethodImpls in the constraint
   context and compares declaration and candidate signatures in their respective contexts.
   `callvirt_constrained` uses separate declaration and constraint lookups for direct value
   methods, boxes when necessary, and resolves reference and boxed receivers virtually; its final
   lookup follows the selected declaring type, including inherited generic bases. The virtual
   resolver's `method_for_lookup`, ancestor substitution,
   MethodImpl and cached paths were checked alongside these routes.

   `start_native_inner` retains the entry descriptor, lookup and roots in `NativeActivation`;
   `advance_native_effect_once`, `dispatch_pending_native_call`, `deliver_native_return` and
   `deliver_native_exception` construct regions from that activation, not the child frame.
   `VmEffects::call`/`try_call` and `ensure_initialized` carry the requested child lookup and
   generation through call, capture, initializer completion and wait/retry. Reflection's
   `Adapter::reflection_call` resolves virtual targets with receiver type arguments before
   rebinding to the selected declaring method. Reflection target-context construction is distinct
   from callee-slot reads: its arity-limited enumeration and signature/declaring-type construction are
   not intrinsic reads of the entry callee's generic slots.

   Enabled differential fixtures `constrained_context_42` and `resumed_intrinsic_context_42`
   cover nested interface instantiations, explicit and implicit static methods, default bodies,
   direct values, boxing and inherited reference methods, managed constructor and virtual child
   calls, collection, initializer completion and exception capture. Repeated calls exercise cache
   hits. `static_dispatch_uses_target_type_and_preserves_method_arguments` and
   `resumed_native_reflection_rejects_surplus_lookup_after_child` supply synthetic surplus entries;
   the latter crosses a managed child, collection and yield before checking entry arity.

   **Verification:** The fixture build passed (NuGet vulnerability-feed warning). All four generic
   differential registrations passed in both base configurations; the constrained fixture also
   passed with the static cache disabled and with cache shadow checking enabled in both. The
   types/resolver/VM suites passed 287 tests without features and 418 with multithreading, five
   skipped in each, including both surplus controls and `byte_memmove_ignores_leaked_method_generics`.
   Both workspace clippy configurations and doc drift (59 checks) passed. The CLI/VM nextest matrix
   actually ran all ten configurations in xtask order: 687/57 skipped, 942/6, 687/57, 942/6,
   688/56, 687/57, 687/57, 942/6, 943/5 and 514/106. Fuzzing excludes differential subprocess
   tests; the separate base differential runs above provide that evidence.
6. **Static field reflection.** Implement `GetValue`/`SetValue` for statics, with
   type-initializer triggering and the `initonly` check. Add a `reflection_static_fields_42`
   fixture: primitive, struct, reference, pointer static, cctor side-effect ordering and the
   `initonly` exception. Extend plan 21's pointer-slot fixture to cover static pointer fields.
   Rerun the Newtonsoft and EF host rungs, because `AttributeCollection` defaults change behavior.
7. **Buffered stdout.** Add the repro fixture, then fix the flush path or close the entry.

   **Verification:** The rebuilt fixture has several writes and a final fragment without a
   newline. `stdout_before_internal_error_matches_dotnet` compares its exact output and exit
   status against the CLR. The debug-only `DOTNET_RS_DEBUG_TEST_POST_EXECUTION_ERROR` hook
   replaces only a successful executor result at the CLI result/error boundary (no stable natural
   post-write VM-error trigger was found). The enabled
   `stdout_is_preserved_on_internal_error_exit_with_pipe_and_file_capture` subprocess test runs
   that fixture through a draining pipe and direct file redirection. Both captures contain every
   expected byte; stderr is the `Internal VM error` diagnostic and the process exits with status
   1. The hook is absent from release builds and inactive unless the explicit test variable is
   set. No flush change was needed: writes survive executor shutdown and the existing `exit_fast`
   path. The injected failure runs deliberately do not compare the CLR's successful exit result
   with the VM's injected failure exit.

   The fixture build passed (NU1900 vulnerability-feed warning only). The generated fixture, normal
   differential test and pipe/file subprocess regression passed 3/3 in both base configurations.
   Both workspace clippy configurations, doc drift (59 checks), formatting, `git diff --check`,
   and the release CLI check passed. The xtask matrix printed all ten configurations and every
   CLI/VM nextest run completed successfully. Fuzzing intentionally omits the differential and
   injected subprocess tests; its generated Console fixture is explicitly ignored because that
   mode denies the native calls used by console output.
8. **Docs (complete).** Removed the closed entries from the plans README while retaining the
   positive compact-offset limit, the Reflection.Emit/`RuntimeType.GetNativeTypeHandle` item, and
   bulk-operation barrier work. Updated `docs/COMPATIBILITY.md` with the validated surface and
   remaining limits, cited the stdout differential and pipe/file repro tests, and reconciled the
   directly affected pointer-slot and memory-safety records.

## Completion evidence

- Rebuilt the C# fixtures successfully with `cargo run -p xtask -- fixtures build`; the build
  reported only the existing NU1900 vulnerability-feed warning. With
  `DOTNET_USE_PREBUILT_FIXTURES=1`, the enabled plan fixtures passed as real CLR differential
  comparisons: 14 tests in no-default-features and 15 with multithreading, including the stdout
  pipe/file subprocess assertions and the thread-static field fixture where enabled.
- `math_and_mathf_bodiless_methods_have_intrinsic_handlers` passed 1/1 in each base configuration.
  The generic-reader audit found raw `GenericLookup::method_arg`/`type_arg` access only inside the
  checked `MethodDescription` accessors and their tests; intrinsic callee-slot readers use
  `own_method_arg`/`own_type_arg`. Direct generic-vector inspection in reflection and dispatch
  constructs or substitutes a separate target lookup, as described in the slice 5 audit record.
- Fresh-CLI Newtonsoft and clean EF InMemory host rungs matched `dotnet` in both configurations
  (four host comparisons passed).
- Both workspace clippy configurations passed. `scripts/check_doc_drift.sh` passed all 59 checks;
  rustdoc broken-intra-link lint passed (it reports an unrelated existing private-intra-doc-link
  warning in `managed_ptr_decode_cache.rs`).
- The full CLI/VM matrix was executed with freshly rebuilt prebuilt fixtures in all ten printed
  configurations: no features (691 passed, 57 skipped); multithreading (947/6); simd (691/57);
  multithreading+simd (947/6); generic-constraint-validation (692/56); memory-validation (691/57);
  metadata-validation (691/57); multithreading+memory-validation (947/6);
  multithreading+validation-all (948/5); fuzzing (514/107). Every run passed. Fuzzing excludes
  differential subprocess tests; the explicit base runs above provide that evidence.

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
