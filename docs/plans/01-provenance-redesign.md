# Plan 01 — Provenance redesign

**Gate:** `MIRIFLAGS="-Zmiri-strict-provenance"` runs green on at least the
`dotnet-value` and `dotnet-runtime-memory` Miri legs, and the flag is added to
`.github/workflows/miri.yml` for those legs.

**Status:** not started.

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

- **78 occurrences** of `expose_provenance` / `with_exposed_provenance` /
  `from_exposed`, across 15 files. Concentration: `pointer/tests.rs` (20),
  `stack/raw_memory_ops_impl/mod.rs` (19), `pointer/serde.rs` (16),
  `runtime-memory/access.rs` (10), `pointer/mod.rs` (8), then a long tail of
  1–6 in `object/mod.rs`, `cts_cli_conversion.rs`, `resolver/factory.rs`,
  `pinvoke/call.rs`, `stack_value.rs`, `write_barrier.rs`, `unsafe_ptr.rs`,
  `vm-data/stack.rs`, `origin.rs`, `reflection/types/mod.rs`.
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

1. **Identify the actual first blocker.** Run the `dotnet-value` Miri leg with
   `-Zmiri-strict-provenance` added and capture the first error verbatim. Do
   this before any code changes: `docs/CI.md`'s diagnosis says "dependency-level,
   during assembly parsing," but the dependency is not named, and the candidates
   in `dotnet-assemblies`' tree — `dashmap` → `parking_lot_core`,
   `crossbeam-utils`, `hashbrown`, all of which do tagged-pointer work — imply a
   different fix than `dotnetdll` would. Record the finding in this plan.
   *Do not assume the blocker is `dotnetdll` because the author owns it.*
2. **Pick the leg order from step 1.** If the blocker is a dependency reached
   only through assembly parsing, then `dotnet-value` and
   `dotnet-runtime-memory` may already be able to run strict provenance today,
   which makes them the gate and defers `dotnet-vm`. Confirm rather than assume.
3. **Classify all 78 sites** into: (a) genuinely at the storage boundary,
   (b) incidental — an address round-tripped for convenience where a pointer
   was available, (c) test-only. Expect (c) to be ~20 and (b) to be a
   substantial fraction of the tail files. Publish the classification as a table
   in this plan.
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
   round-trips, and promote both to blocking (see plan 03).
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
  (plan 05).

## Not in scope

- Changing `PointerOrigin`'s variant set or the GC's treatment of it.
- The `UnmanagedPtr` path, where exposure from an integer is the actual
  semantics of the operation (`Unsafe.*`, P/Invoke), not a laundering
  workaround. These become trust-register entries, not fixes.
- Any change to the 3-word *size*, which is observable to managed code.

## Related

- [`docs/ASSURANCE_ROADMAP.md`](../ASSURANCE_ROADMAP.md)
- [`docs/CI.md`](../CI.md) — the strict-provenance note this plan rewrites
- [`docs/plans/03-falsifier-portfolio.md`](03-falsifier-portfolio.md) — where
  the fuzz targets get promoted
- [`docs/ASSURANCE_BACKGROUND.md`](../ASSURANCE_BACKGROUND.md) — why this
  plan is first
- Archived study on the provenance risk this plan retires:
  [`08-risks.tex`](https://github.com/nickbclifford/dotnet-rs/blob/b5a5f65d67345b0682def83867b816ea86fa3152/docs/proof-dsl-feasibility/sections/08-risks.tex) §8.6
