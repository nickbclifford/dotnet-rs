# Stable homes for addressable locals and arguments

**Status:** Complete (measured primitive cost accepted)
**Outcome:** A byref or native-int address taken from a local or argument keeps reading and
writing that variable across assignment, evaluation-stack growth and nested calls, and Miri
reports no UB. The evaluation-stack pointer fixup is deleted.

## What was built

Locals and arguments used to be slots in the shared evaluation-stack `Vec`; `ldloca`/`ldarga`
cached raw addresses into it, and `update_stack_pointers` tried to rebase them on growth. That
missed `stloc` to value-type locals, native-int addresses (`ldloca; conv.u`) and block operations.
The design, now described in [Frame-owned homes](../GC_AND_MEMORY_SAFETY.md#frame-owned-homes):

1. `build_frame_homes` (`crates/dotnet-vm/src/frame_homes.rs`) scans each method instantiation
   once and homes every local named by `ldloca` and argument named by `ldarga`; other variables
   stay ordinary slots.
2. Each frame owns one zeroed, non-moving `RawBlock` (`FrameHomeLayout`,
   `crates/dotnet-vm-data/src/frame_homes.rs`), laid out by the field-layout machinery and
   released with the frame. Value-type constructor receivers get caller-owned temporary homes.
3. `ldloc`/`stloc`/`ldarg`/`starg` on a homed variable read and write the home bytes.
4. A byref to a home has the checked origin `PointerOrigin::FrameHome` (per-allocation serial plus
   offset, within the unchanged 24-byte encoding); `frame_home_window` rejects a returned frame or
   an out-of-range access. `TypedReference` address-taking is rejected with
   `InvalidProgramException` (strict ECMA policy).
5. `StackFrame` traces the references and managed pointers in its home block.
6. The `Stack` origin, `update_stack_pointers`, `apply_reallocation_fixup`, the scratch caches and
   the registry predicate `F3.InteriorPointerRebased` were deleted (now `F3.StableFrameHomes`).

Block operations on homes (`cpblk`/`initblk`, `Memmove`, `Unsafe.CopyBlock`/`InitBlock`) resolve
their complete range through `RetainedPtr::checked_range`. The byref-placement checks this plan
added (a `by_ref` field only in a byref-like type; `box` of a byref-like value throws
`InvalidProgramException`) are in [Managed pointer placement](../GC_AND_MEMORY_SAFETY.md#managed-pointer-placement).

## Out of scope

- **Byrefs in heap containers.** ECMA-335 §II.14.4.2 and §III.1.1.5.2 forbid them, so arrays,
  boxes, statics and finalization are not home-aware.
- **Cross-thread access** to a home address, and a shared raw-access protocol.
- **Dangling native-int addresses** after the frame returns: once converted with `conv.u` they are
  `Unmanaged` addresses, trusted like `localloc` addresses. Byrefs stay checked.
- **Static field addresses** (`p12_ldsflda_convu_stind_static`) and bulk-operation barriers; see
  the plans README, "Not yet planned".

An earlier attempt that first made every heap container home-aware grew to 24 preparatory steps
without enabling anything and was abandoned; the scope rules above came from it.

## Accepted cost

All 18 end-to-end cases measured then improved in both feature configurations against the
pre-home baseline. The isolated Rust stack control showed an approximately 4% generated-copy
cost whose magnitude varies with launch conditions; three bounded source experiments did not
improve it repeatably, and the maintainer accepted it. Detailed measurements are in git.

## Completion validation

At completion every reviewed pointer control passed pinned Tree Borrows Miri in both
configurations, and the native pointer suites and library Miri (`dotnet-value`, `dotnet-vm-data`)
passed. These are bounded controls, not a soundness proof; exposed-provenance warnings limit
pointer-bug detection. Static addresses, cross-thread native addresses and general
bulk-operation barriers remain outside this plan.

Reproduce:

```bash
bash scripts/pointer_provenance.sh batch --configuration no-features --batch N      # and multithreading
bash scripts/pointer_provenance.sh verify-batches
./check.sh --fast
```

Library Miri uses the pinned runner flags ([CI](../CI.md#miri)). The pointer-provenance
[inventory](../pointer-provenance/INVENTORY.md) records R02 and the p01/p03/p09/p15/p37–p42
controls.
