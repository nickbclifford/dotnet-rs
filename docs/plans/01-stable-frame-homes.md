# Stable homes for addressable locals and arguments

**Status:** In progress
**Outcome:** A byref or native-int address taken from a local or argument keeps reading and
writing that variable across assignment, evaluation-stack growth and nested calls, and Miri
reports no UB. The evaluation-stack pointer fixup is deleted.

## Problem

Locals and arguments are `StackValue` slots inside the shared evaluation-stack `Vec`
(`BasePointer` in `crates/dotnet-vm-data/src/stack.rs`). `ldloca`/`ldarga`
(`crates/dotnet-vm/src/instructions/stack_ops.rs`) push a `PointerOrigin::Stack` byref that
caches a raw address: into the `Vec` for scalar slots, or into the value's own storage buffer
for value-type slots (`StackValue::data_location`, `crates/dotnet-value/src/stack_value.rs`).
When the `Vec` grows, or a frame is suspended or restored (`suspend_above`/`restore_suspended`),
`EvaluationStack::update_stack_pointers` rewrites every cached Stack pointer it can find. This
does not cover:

- `stloc` to a value-type local. It replaces the slot's value and its buffer. A Stack byref
  re-resolves its slot on each access, but an address cached from it (`conv.u`, `AsPointer`,
  `with_data`) goes stale (`p37_ldloca_value_type_survives_stloc`).

- addresses already converted to native integers (`ldloca; conv.u`). See the ignored tests
  `p01_ldloca_convu_stind_i4` and `p09_ldloca_convu_survives_callee_stack_growth` in
  `crates/dotnet-vm/src/pointer_provenance_tests.rs`.
- writes through such an address. Even without growth, they conflict with later `&mut` access
  to the `Vec` element (p01 under Tree Borrows).
- block operations on a local's address (`p15_initblk_stack_local_via_managed_ptr`).

The p-tests' outcomes are recorded in the [inventory](../pointer-provenance/INVENTORY.md).

Localloc had the same defect and was fixed by giving each frame non-moving `RawBlock` storage
(`FrameAllocations`, `crates/dotnet-vm-data/src/frame_alloc.rs`). This plan does the same for
addressable locals and arguments.

## Scope

**In scope:** locals and arguments whose address is taken; `ldloca`, `ldarga`, `ldloc`, `stloc`,
`ldarg`, `starg` on those variables; every consumer of `PointerOrigin::Stack` (about 28
non-test sites, mostly `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs`); frame
lifecycle (return, unwind, `tail.`/`jmp`, frame pooling, exception-filter suspension); deleting
the fixup.

**Out of scope, and why:**

- **Byrefs in heap containers.** ECMA-335 II.14.4.2 / III.1.1.5.2: managed pointers cannot be
  stored in statics, array elements, or fields of objects or value types. The exception,
  `ref` fields of byref-like types, lives only in locals, arguments and the evaluation stack,
  and those are covered here. Do not make arrays, boxes, statics or finalization
  home-aware. Today nothing enforces this: `LayoutFactory::collect_fields` gives a `by_ref`
  field `Scalar::ManagedPtr` without checking that the declaring type is byref-like, and `box`
  does not reject byref-like types (see
  [Managed pointer placement](../GC_AND_MEMORY_SAFETY.md#managed-pointer-placement)). Slice 1
  adds both checks, rejecting invalid IL with the VM's existing invalid-program error.
- **Cross-thread access and a shared-access protocol.** A frame belongs to one thread. An
  address leaked to another thread through a native integer has the same status as a
  leaked localloc address today. A general protocol for raw, atomic and bulk access is a
  separate future plan.
- **Dangling native-int addresses after the frame returns.** Once a home address is converted
  to a native integer (`conv.u`) it is an Unmanaged-origin address, trusted like a localloc
  address (`PointerOrigin::Unmanaged` branches in `raw_memory_ops_impl`). This applies only to
  native integers; byrefs stay checked (design step 4). Quarantine or ABA protection would apply to both and is
  separate work.
- **Static field addresses** (`p12_ldsflda_convu_stind_static`, alignment). Separate small fix.
- **Bulk-operation chunking/GC barriers** beyond what is needed for `p15` to pass.
- **Changing `ManagedPtr::SIZE`** (24 bytes, `crates/dotnet-value/src/pointer/serde.rs`).
  The Stack word-0 encoding has spare bits. If the design truly cannot fit, stop and ask;
  do not widen.

## Design

1. **Which variables get a home.** Scan each method's IL once for `ldloca`/`ldarga` (cache
   it with the method's other per-method info). Only those variables get a home. All others stay
   ordinary evaluation-stack slots, so the hot path for most methods does not change.
2. **Storage.** Each frame owns one non-moving, zeroed `RawBlock` holding its homes, laid out
   with the runtime's existing field-layout machinery and aligned to max(16, largest home
   alignment). Allocate it at frame entry and copy addressable arguments into it from the
   caller's evaluation-stack slots. Release it with the frame, exactly as `FrameAllocations`
   is released today. Access is only through raw pointers derived from the block base, never
   through `&mut` to the bytes.
3. **Loads and stores.** `ldloc`/`stloc`/`ldarg`/`starg` on a homed variable read and write the
   home bytes through the same layout-driven read/write the VM uses for value-type fields.
4. **Byref representation.** A byref to a home is a managed pointer with its own checked
   origin, `PointerOrigin::FrameHome`, carrying a frame identity and the home offset, plus a
   cached pointer that never needs rebasing because the block does not move. The frame
   identity is a per-frame serial number, not a depth, so a stale byref cannot resolve into a
   newer frame at the same depth. Resolution checks that the frame with that serial is still
   live and that the range lies inside its home block; a stale or out-of-range byref is an
   error, never an access. Home byrefs never use the `Unmanaged` origin, which is for addresses
   the VM cannot check. Encode the origin within the existing 24 bytes (tag 7 has free
   sub-tags; follow the Stack and static encodings in `crates/dotnet-value/src/pointer/serde.rs`).
5. **Tracing.** Frames trace the GC references and managed pointers inside their home block
   using its layout, so `StackFrame`'s `Collect` impl gains the home block.
6. **Fixup removal.** Once no Stack-origin pointer targets the evaluation-stack `Vec`, delete
   `apply_reallocation_fixup`, `update_stack_pointers` (including its calls from suspend/restore),
   the scratch location caches and the serialized Stack-pointer rebasing, along with their tests
   and metrics. Replace the registry predicate `F3.InteriorPointerRebased` with one describing
   stable homes, and update its `// SAFETY:` citations. If a real eval-temporary
   case still needs an address, home it as well rather than keep the fixup.

## Slices

Each slice ends with the VM running guest code through the new path and its tests enabled.
Do not build a later slice's support in an earlier slice.

1. **Homed locals, single frame (done).** The two placement checks above, then steps 1–3 for
   locals only, in one frame. It used the `Unmanaged` origin for home byrefs; slice 2 replaces
   that. Enable `p01` and
   `p03`, and add a regression for a byref to a value-type local that survives `stloc`. `p02`,
   `p35` and the existing local and value-type tests keep passing.
2. **Checked home byrefs and homed arguments (done).** Step 4: add `FrameHome`, move `ldloca` on homed
   locals to it, and add a regression that a byref to a returned frame's home is rejected
   rather than accessed. Then home arguments: `ldarga`, `starg`, `ldarg`, copying arguments
   into the block at frame entry. Enable `p09`.
3. **Frame lifecycle (done).** Return, unwind, `tail.`/`jmp`, pooling and filter suspension
   release or retain homes correctly. The p31–p34 lifecycle controls now include homes;
   p39 covers filter acceptance/rejection, checked suspended-home access and collection;
   p40 covers modified scalar/value-type arguments and fresh serials across `jmp`.
   `jmp` reads current argument homes, and home lookup includes suspended frames. Filter
   restoration saves the filter-owning frame's instruction pointer and absolute stack height.
4. **Block ops and fixup deletion.** Route `cpblk`/`initblk`, `Buffer.Memmove` and
   `Unsafe.CopyBlock`/`InitBlock` on a `FrameHome` byref through `frame_home_window`; today
   they use its cached address unchecked. Enable `p15`. Then step 6. The remaining
   Stack-origin producers are evaluation-stack temporaries (`constructor_frame`, the
   value-type `newobj` intrinsic) and byref-typed or `TypedReference` variables; home them
   too and delete the Stack origin with the fixup if nothing else needs it.
5. **Measure.** Compare `runtime_primitives` and `end_to_end` against `main` before slice 1. A
   regression on methods without `ldloca`/`ldarga` is a defect. Report other costs.

## Done when

- `p01`, `p03`, `p09` and `p15` are un-ignored and pass natively and in the pointer-provenance
  Miri lane (both configurations), and the existing controls still pass.
- `update_stack_pointers` and its supporting code no longer exist.
- Every byref to a home uses the `FrameHome` origin, and a byref to a returned frame's home
  is rejected (regression test).
- `docs/GC_AND_MEMORY_SAFETY.md` describes frame homes, and `docs/pointer-provenance/INVENTORY.md`
  marks R02 resolved.

Validation follows [CI.md](../CI.md#test-hierarchy-and-migration): focused tests while iterating,
then one `./check.sh --fast` per slice, and the selected deep lanes before merge.

## Previous attempt

A November 2026 attempt began by making every container that can hold a `ManagedPtr` (fields,
arrays, boxes, statics, finalizer resurrection, atomics) support home pointers before any frame
produced one. It grew to 24 preparatory steps without enabling anything and was abandoned.
The scope and slice rules above exist to prevent that.

## Ask the maintainer only if

- fitting the byref encoding requires changing `ManagedPtr::SIZE`;
- a home byref seems to need an unchecked origin (such as `Unmanaged`) to work;
- a supported program (not invalid IL) stores a home-origin byref in a heap location;
- slice 5 shows a regression on methods that take no addresses.
