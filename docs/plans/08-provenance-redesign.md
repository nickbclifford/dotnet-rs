# Plan 08 — Provenance redesign

**Original gate:** `MIRIFLAGS="-Zmiri-strict-provenance"` runs green on at least the
`dotnet-value` and `dotnet-runtime-memory` Miri legs, and the flag is added to
`.github/workflows/miri.yml` for those legs.

**Status:** parked — implementation complete; original CI gate explicitly
closed by owner-directed deferral, not met (2026-08-06). **Parked, not queued**
— last in [the plan sequence](README.md) because reopening it needs a new,
explicitly authorized task, not because of any technical dependency on plans
01–07.

## Final disposition

The serialization, typed-reference, fuzzing, and measurement work landed while
preserving `ManagedPtr::SIZE` at three words. Managed Stack and Static reads now
require caller-supplied live bases; Heap and CrossArena reads derive data
addresses from live owner storage; Transient remains deliberately
non-deserializable. The managed-pointer fuzz targets exercise every serializable
origin plus offset, raw tag, checksum, and resolved-address preservation. A
bounded CallStack-owned Heap decode cache retains only traced `ObjectRef` handles,
is invalidated after each collection cycle, and never caches data pointers or
CrossArena leases.

The original strict-provenance gate did **not** land. A pinned-nightly local
`dotnet-runtime-memory` run passed, but that crate has no entry in the advisory
Miri matrix. A pinned-nightly `dotnet-value` run remains known-red at the
documented atomic GC-handle reconstruction in `gc_handle_from_addr`; continuation
triage also reaches the separate serialized `ObjectRef::read_unchecked` boundary.
On 2026-08-05 the owner directed that Phase 9 be closed as deferred rather than
enable a known-failing job or add a new matrix entry. Consequently no current CI
leg uses `-Zmiri-strict-provenance`. Reconsidering that decision requires a new,
explicitly authorized task beginning with a green local target leg.

Address-only unmanaged reconstruction is centralized in the unsafe
`pointer::unmanaged_ptr_from_addr` helper. Its callers are the explicit
Unmanaged, raw-storage, P/Invoke, native string, and `Unsafe.*` boundaries; no
managed `ManagedPtr` origin calls it. Four legacy provenance-API sites remain in
the separately documented managed storage boundaries: two in
`ObjectRef::read_unchecked`, one in `gc_handle_from_addr`, and one in the
lease-scoped CrossArena helper. Those exceptions explain the known-red
`dotnet-value` strict run and must not be mistaken for a completed global
one-helper/green-Miri claim.

The 102-site table below is the audited baseline at `6ebda249`, not a current
source inventory. Later phases removed or reclassified those sites; the final
disposition above is authoritative for the merged implementation state.

## Why this is first

Every other assurance instrument is downstream of it. Strict provenance is the
one UB class the project currently cannot check at all, in a codebase whose
central data structure is a serialized managed pointer. It is also the entry
requirement for every external tool that might ever help: RefinedRust
explicitly does not support pointer-integer casts, Kani does not check
provenance UB, and Charon/Aeneas cannot model MIR that launders addresses
through integers. The terminated proof study named pointer serialization as the
module where a proof layer would add the least confidence per unit effort
precisely because its axioms were the least falsifiable — this plan makes them
falsifiable instead.

## Current state (measured at `6ebda249`)

- **102 occurrences** of `expose_provenance` / `with_exposed_provenance` /
  `from_exposed_addr` / `nonnull_from_exposed_addr`, across 15 files.
  Concentration: `pointer/tests.rs` (20),
  `stack/raw_memory_ops_impl/mod.rs` (19), `pointer/serde.rs` (16),
  `runtime-memory/access.rs` (10), `pointer/mod.rs` (8), then a long tail of
  1–6 in `object/mod.rs`, `cts_cli_conversion.rs`, `resolver/factory.rs`,
  `pinvoke/call.rs`, `stack_value.rs`, `write_barrier.rs`, `unsafe_ptr.rs`,
  `vm-data/stack.rs`, `origin.rs`, `reflection/types/mod.rs`.
- **The Miri baseline is confirmed.** `dotnet-runtime-memory` passes all 13
  tests with `-Zmiri-strict-provenance`. `dotnet-value` fails at its first
  pointer-cast test, `cts_cli_conversion::tests::widening_preserves_pointer_and_reference_payloads`,
  on a bare cast at `crates/dotnet-value/src/cts_cli_conversion.rs:417`:

  ```text
  error: unsupported operation: integer-to-pointer casts and
  `ptr::with_exposed_provenance` are not supported with `-Zmiri-strict-provenance`
    --> crates/dotnet-value/src/cts_cli_conversion.rs:417:26
     |
  417|             NonNull::new(0x1234usize as *mut u8),
     |                          ^^^^^^^^^^^^^^^^^^^^^^ unsupported operation
  ```

  The command used the pinned `nightly-2026-05-27` toolchain and
  `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks -Zmiri-strict-provenance"`.
  A second bare cast in the same test is at line 451. Both are test-only and
  are intentionally not included in the 102-site count because they are `as`
  casts rather than matching the provenance API search.
- **One chokepoint already exists.** `nonnull_from_exposed_addr` at
  `crates/dotnet-value/src/pointer/mod.rs:30` is a single `pub(crate)` helper
  wrapping `with_exposed_provenance_mut`, with 30 call sites. Reconstruction is
  therefore already centralized; only *exposure* is scattered.
- **The typed origin already exists.** `PointerOrigin`
  (`crates/dotnet-value/src/pointer/origin.rs:16`) is a discriminated union —
  `Heap(ObjectRef)`, `Stack(StackSlotIndex)`, `Static(Arc<StaticMetadata>)`,
  `Unmanaged`, `CrossArenaObjectRef(ObjectPtr, ArenaId)`,
  `Transient(Box<Object>)`. The project has the registry it needs; the
  serialized form just does not route through it.
- **The serialized form is three words** (`ManagedPtr::SIZE = ObjectRef::SIZE
  * 3`): pointer word, owner word, checksum word, with tag bits OR'd into a
  word (`w0 = ptr.expose_provenance() | 5` in `serde.rs`).
- **CI documents the blocker as upstream.** `docs/CI.md:261`: strict provenance
  "is currently infeasible for `dotnet-vm` because dependency-level
  integer-to-pointer casts are reached during assembly parsing before VM unsafe
  sites execute."
- **Two falsifiers already target this exact code**:
  `fuzz_managed_ptr_roundtrip` and `fuzz_managed_ptr_offset` in
  `crates/dotnet-value/fuzz/fuzz_targets/`.

### 102-site classification

| File | Count | Test-only | Incidental | Storage-boundary | Kept/out-of-scope |
|------|------:|----------:|-----------:|-----------------:|------------------:|
| `pointer/tests.rs` | 20 | 20 | — | — | — |
| `stack/raw_memory_ops_impl/mod.rs` | 19 | — | 17 | — | 2 (Unmanaged origin) |
| `pointer/serde.rs` | 16 | — | — | 16 | — |
| `runtime-memory/access.rs` | 10 | — | 10 | — | — |
| `pointer/mod.rs` | 8 | 4 (Arbitrary) | 3 | — | 1 (helper definition) |
| `object/mod.rs` | 6 | — | 4 | 2 (`ObjectRef`) | — |
| `cts_cli_conversion.rs` | 6 | 1 (line 438) | 4 | 1 (`LoadType::Object`) | — |
| `runtime-resolver/factory.rs` | 4 | — | 1 | 3 | — |
| `dotnet-pinvoke/call.rs` | 4 | — | — | — | 4 (genuine P/Invoke) |
| `stack_value.rs` | 2 | — | 1 | — | 1 (NativeInt cast) |
| `runtime-memory/write_barrier.rs` | 2 | — | 2 | — | — |
| `dotnet-intrinsics-unsafe/unsafe_ptr.rs` | 2 | — | — | — | 2 (genuine `Unsafe.*`) |
| `dotnet-vm-data/stack.rs` | 1 | — | — | 1 | — |
| `pointer/origin.rs` | 1 | — | 1 | — | — |
| `intrinsics-reflection/types/mod.rs` | 1 | — | 1 | — | — |
| **TOTAL** | **102** | **25** | **44** | **23** | **10** |

**Classification definitions:** test-only sites are in `#[test]` or fuzzing
`Arbitrary` implementations; incidental sites already have a live pointer and
should use it (or `ptr::addr()`); storage-boundary sites serialize the address
and require an origin-handle redesign; kept/out-of-scope sites are genuine
P/Invoke, `Unsafe.*`, or `Unmanaged` semantics, or separately handled
`ObjectRef` serialization.

### Factory raw-storage reconciliation

The four `factory.rs` sites were re-audited against their actual dataflow. The
`StackValue::ManagedPtr` case at line 342 already preserves its live pointer;
only the alternate `StackValue::UnmanagedPtr` input is an incidental
integer-to-pointer round-trip. The short `ValuePointer` representation and both
`TypedReference` words are raw-storage boundaries: deserialization receives only
integer bytes, not a recoverable owner or pointer.

`TypedReference` is not limited to a P/Invoke call path. Its VM storage format
is `[value address, Arc::as_ptr(TypeDescription)]`; consequently, recovering its
type pointer also relies on the original Arc allocation remaining live. Replacing
those reconstructions requires a stable type handle plus a recoverable value
origin, rather than a local pointer-cast substitution.

### TypedReference stable storage and P/Invoke ABI (step 4.4)

`System.TypedReference` needs a representation of its own. It must not reuse
the old two-word byte pair, and it must not be treated as a `ManagedPtr` merely
because it contains a byref. A typed reference has two independently live
parts: a typed value origin and a type-descriptor owner.

ECMA-335 confines `typedref` to parameter and local signatures: it is created
by `mkrefany`, copied as a typed reference, and read by `refanyval` /
`refanytype`. It shall not be boxed, used as a field or array-element type, or
used as a return type. This is also the correct ownership boundary for this VM:
parameters, locals, and evaluation-stack slots are stored as `StackValue`, not
as raw field bytes.

#### VM slot representation

The durable representation is a frame-owned typed slot, conceptually:

```rust
struct TypedReferenceSlot<'gc> {
    value: StackManagedPtr<'gc>,
    type_desc: Arc<TypeDescription>,
}
```

`StackValue::TypedRef` already carries this exact ownership shape. The
implementation work will make it the sole VM representation for a non-null
`typedref` (with a separate uninitialized-local sentinel where a zero-initialized
local is required), rather than converting it through `ValueType::TypedRef` or
`CTSValue` bytes. `value` retains the full `ManagedPtr` and hence its
`PointerOrigin`, offset, and live pointer provenance; `type_desc` retains a
strong `Arc<TypeDescription>`, whose cloned `ResolutionS` keeps the metadata
arena alive. A typed-reference copy clones these two owned handles. No
`Arc::as_ptr`, type-address registry, or integer-to-pointer reconstruction is
part of VM storage.

This representation preserves every `PointerOrigin` directly rather than
serializing an address. `Heap`, `Stack`, `Static`, and (when enabled)
`CrossArenaObjectRef` continue to resolve through their existing owned origin
handles; the CrossArena path continues to require its scoped arena lease.
`Transient` and `Unmanaged` remain subject to their existing lifetime and
unsafe/P/Invoke contracts. Verification must prevent a transient home from
outliving its stack frame, but a valid slot or parameter copy keeps the live
`ManagedPtr` itself and does not create a new raw-storage boundary.

All non-slot uses are invalid-layout errors before bytes are written or read:

* `CTSValue::write`, `ValueType::TypedRef`, and resolver construction must not
  serialize a typed reference into object, boxed-value, field, static-field, or
  vector storage.
* `System.TypedReference` must receive a dedicated layout classification rather
  than falling through normal value-type field layout. Generic boxing and array
  paths reject it, and GC layout tracing never treats a byte range as a typed
  reference.
* A method return declared `typedref` is rejected during signature / call
  validation. This includes the current P/Invoke typed-reference return path.

The current two-word representation confirms why this separation is required:
`ValueType::TypedRef::size_bytes()` and `CTSValue::write` produce
`[value address, Arc::as_ptr(type)]`, while `ManagedPtr::SIZE` is already three
words. The factory decoder's assertion that those two widths are equal and its
subsequent `ManagedPtr::SIZE` slice are therefore not a valid decoder for the
16-byte payload. The migration removes that decoder; it must not enlarge the
managed `TypedReference` layout or invent a new raw-storage format to preserve
an operation that ECMA-335 disallows.

#### P/Invoke translation

P/Invoke is the one boundary that needs a physical pair. For an *input
parameter* only, marshalling translates a live `TypedReferenceSlot` into a
call-scoped `#[repr(C)]` temporary:

```rust
struct PInvokeTypedReferenceAbi {
    value: *mut u8,
    type_handle: *const TypeDescription,
}
```

This preserves the existing two-pointer libffi shape, but it is an ABI
translation rather than VM storage:

* The marshaller resolves `value` from the slot's live `ManagedPtr` origin and
  pins or leases that origin for the complete synchronous libffi call. It does
  not obtain an address from serialized storage.
* `type_handle` is `Arc::as_ptr(&slot.type_desc)` as a *borrowed* native token.
  The slot (or an explicit marshalling-owned `Arc` clone) remains alive until
  libffi returns, so the native pointer has a defined call-scoped lifetime.
* Native code may inspect the two fields during that call, but must neither
  retain, free, nor manufacture either pointer. In particular it must never
  call `Arc::from_raw` on `type_handle`.
* `typedref` P/Invoke returns are rejected rather than imported. A returned
  `{ value, type_handle }` pair cannot recover a managed `PointerOrigin`, and
  treating a native `type_handle` as an `Arc` is unsound. A future explicit
  native-handle registry would be a new ABI with its own lifetime contract; it
  is not an implicit fallback for this two-pointer ABI.

Consequently, the direct `Arc::as_ptr` export can remain only inside the
call-scoped ABI codec, while the current P/Invoke `Arc::from_raw` return path
and both factory raw reconstructions are removed during the same migration. The
native ABI remains two pointers; VM lifetime and provenance are retained by the
frame-owned slot.

## The unavoidable core, and the avoidable rest

A VES must store a managed pointer as bytes in managed memory — IL can read
those bytes. So *at the storage boundary* an address genuinely becomes an
integer, and no redesign changes that. What is avoidable is recovering
provenance **from** that integer. The strict-provenance-clean pattern is to
reconstruct from a provenance-carrying source keyed by the stored data, rather
than from the stored address itself. `PointerOrigin` is already that source.

So the target shape is: the serialized triple carries an **origin handle** plus
an offset, and deserialization resolves the handle through the origin registry
to obtain a pointer with real provenance, using the stored address only as a
cross-check (which is what the checksum word is already for).

## Steps

1. **Address the confirmed first blocker.** `dotnet-value` first fails on the
   test-only bare cast at `cts_cli_conversion.rs:417` (with a second such cast
   at line 451), not during assembly parsing. Fix these before rerunning the
   leg and recording the next failure.
2. **Use the confirmed leg order.** `dotnet-runtime-memory` already passes
   strict provenance; make the `dotnet-value` test-only fixes first, then work
   through its subsequent failures. Continue to defer `dotnet-vm`, whose
   dependency-level parsing blocker remains outside this plan's gate.
3. **Use the published 102-site classification** above: test-only, incidental,
   storage-boundary, and kept/out-of-scope sites have distinct remediation
   paths. The two bare test casts are tracked separately from the API-site count.
4. **Eliminate class (b)** with `ptr::map_addr` / `with_addr`, or by threading
   the pointer instead of the address. These are strict-provenance-clean and
   need no design change. This is the bulk of the mechanical work and is a
   good supervised-refactor candidate.
5. **Redesign the class (a) encoding.** Give `PointerOrigin` a stable
   serializable handle, make `serde.rs` write `(handle, offset, checksum)`, and
   make reconstruction resolve the handle. Keep the tag bits — they are cheap
   and already tested. The checksum word becomes a genuine integrity check
   against the resolved pointer rather than decoration.
6. **Narrow the remaining exposure to one documented function**, replacing
   `nonnull_from_exposed_addr`, with a `// SAFETY:` comment that states exactly
   which premise is being assumed and why no alternative exists. If the redesign
   succeeds, that function should have zero callers outside the
   `Unmanaged`/P-Invoke path, where exposure is genuinely correct.
7. **Extend the two existing fuzz targets** to assert provenance-preservation
   round-trips, and promote both to blocking (see plan 02).
8. **Turn the flag on in CI** for the legs that pass, and rewrite
   `docs/CI.md:261` from an infeasibility note into a scope statement naming
   which legs run strict provenance and why the others do not.

## Risks and known unknowns

- **The blocker may be a dependency you cannot fix.** If it is inside
  `parking_lot_core` or `hashbrown`, the options are: a Miri suppression scoped
  to that crate if one exists, swapping `dashmap` for a strict-provenance-clean
  map in the parse path, or accepting that `dotnet-vm` never runs the flag while
  the leaf crates do. All three are acceptable outcomes; only silence is not.
- **The handle indirection has a runtime cost** on every managed-pointer
  deserialization, which is on a hot path. Measure with the existing benchmark
  suite before and after; if the cost is real, the fallback is to keep the
  address word for fast-path use and resolve the handle only under
  `memory-validation`, which preserves the *checkability* while not paying for
  it in release builds. Note this weakens the gate to "strict provenance passes
  under the validation feature," which is still strictly better than today.
- **`Transient(Box<Object>)` origins have no stable handle** by construction —
  they are eval-stack-resident value types. These may need to stay in the
  exposure path, which is fine if documented and counted in the trust register
  (plan 06).

## Not in scope

- Changing `PointerOrigin`'s variant set or the GC's treatment of it.
- The `UnmanagedPtr` path, where exposure from an integer is the actual
  semantics of the operation (`Unsafe.*`, P/Invoke), not a laundering
  workaround. These become trust-register entries, not fixes.
- Any change to the 3-word *size*, which is observable to managed code.

## Related

- [`docs/plans/README.md`](README.md)
- [`docs/CI.md`](../CI.md) — the strict-provenance note this plan rewrites
- [`docs/plans/02-falsifier-portfolio.md`](02-falsifier-portfolio.md) — where
  the fuzz targets get promoted
- [`docs/ASSURANCE_BACKGROUND.md`](../ASSURANCE_BACKGROUND.md) — why this
  plan is first
- Archived study on the provenance risk this plan retires:
  [`08-risks.tex`](https://github.com/nickbclifford/dotnet-rs/blob/b5a5f65d67345b0682def83867b816ea86fa3152/docs/proof-dsl-feasibility/sections/08-risks.tex) §8.6
