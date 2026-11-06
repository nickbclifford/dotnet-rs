# Safe APIs whose soundness rests on unchecked caller premises

**Status:** Not started
**Outcome:** Safe Rust code in the workspace cannot make `FieldStorage`, `Object`, `StackValue` or the
memory-operation helpers decode a non-reference slot as an `ObjectRef`, read bytes without the
synchronization their contract needs, or call a raw-pointer copy/fill helper without an `unsafe`
block; a test for each fails on today's code.

These are safe functions that call an unsafe decoder or raw operation and justify it by a
premise the signature cannot express. Sites that were checked and found sound are listed under
Refuted so they are not re-raised.

## Problem

- **Typed field access does not check the slot.** `FieldStorage::field_at_offset::<T>` checks only
  that `offset + size_of(T)` is in range, and `FieldStorage::field::<T>` asserts only that the
  named field has the same size as `T` (an `i64` field passes for `ObjectRef`). `FieldRef<ObjectRef>::read` then calls
  `FieldType::read_from`, which calls `ObjectRef::read_unchecked` (unsafe: the bytes must be a
  serialized reference). A safe caller can read an `i64` field, or any offset, as an `ObjectRef`
  and forge a `Gc`. `write` can plant a pointer in a slot the collector does not trace.
  `crates/dotnet-assemblies/src/support_contract.rs` checks the scalar kind in `field` but not in
  `reinterpreted_field`, which is same-width only. `LayoutManager::trace`/`resurrect`,
  `FieldLayoutManager::trace`/`resurrect` and `ArrayLayoutManager::resurrect` are `pub` and take
  an arbitrary `&[u8]`; their only callers are `FieldStorage` and `Vector`, which pair a layout
  with its own bytes.
- **Unsynchronized slices outside a stop-the-world pause.** `raw_data_unsynchronized` is
  `unsafe` with the contract "no writer or reallocation while the slice is live". Its
  collector uses (`FieldStorage::trace`/`resurrect`) hold it. `Object::size_bytes` and
  `StackValue::data_location` (value-type case) also call it with only a `ThreadSafeLock` read
  borrow or none, so the contract is not established there. `FieldStorage::data_len` (reads the
  `Vec` length under the read guard) and `raw_data_ptr` (no slice formed) exist.
- **Raw-pointer helpers with safe signatures.** `chunked_copy_with_safe_point` and
  `chunked_fill_with_safe_point` (`dotnet-intrinsics-unsafe/src/mem_helpers.rs`) are safe
  `pub(crate)` functions taking `*const u8`/`*mut u8` and a length; their SAFETY comments rely on
  "the caller-validated range". Five call sites in `buffer.rs` and `unsafe_ptr.rs`.
- **`RawMemoryOps::read_unaligned`/`write_unaligned` document more than they need.** The trait
  says the caller must prove `offset` is valid for `origin`. For `Heap`, `Static`, `Transient` and
  cross-arena origins the implementation checks bounds and returns `BoundsCheck`, which is what
  `ldelem`/`stelem` and the field instructions rely on. For `Unmanaged` it does not, and for
  `Stack` it forms the address with `NonNull::new_unchecked(base.add(offset))`.
  `read_from_inline_storage`/`write_to_inline_storage` compute `offset + size` unchecked.

## Scope

In: the four problems above.

Out, with reasons:

- Safe byte-level writes into reference slots (`Vector::get_mut`, `FieldStorage::with_data_mut`,
  `ObjectRef::with_data_mut`) can plant forged pointer bytes, and bulk copies of reference slots
  need barriers. Fixing that is the shared raw-memory protocol and resumable-bulk-operations work
  already listed under "Not yet planned" (R09/R12, R10/R11); it is added there as a bullet.
- The `Stack`-origin unchecked address is plan 02's frame and locals work.
- Stop-the-world reads aliasing a parked writer's guard is not a defect today; see Refuted. It is clarified in `INVARIANT_REGISTRY.md`, not planned.
- `ObjectPtr::with_data_mut` is a safe function over a raw pointer that skips the owner barrier
  because no owning `Gc` exists under the caller's brand. It overlaps R09/R12 and stays there.

## Design

Prefer making the safe API enforce its premise; use `unsafe fn` only where the premise is a raw
pointer range that no type can carry.

1. `field_at_offset` and `field` take the slot from the layout.
   For `T::SCALAR` of `ObjectRef` or `ManagedPtr` it returns `None` unless the layout has a scalar
   of that kind starting at `offset`; for plain scalars the existing range check stays. Then
   `FieldRef` is the only safe route to `FieldType::read_from`. Narrow the layout `trace` and
   `resurrect` methods to `pub(crate)`.
2. `Object::size_bytes` uses `FieldStorage::data_len`. `StackValue::data_location` uses
   `raw_data_ptr`, which forms a pointer and no slice. `raw_data_unsynchronized` keeps only its
   two collector callers and the resurrection invariant check.
3. The two chunked helpers become `unsafe fn` with a `# Safety` section: `src` and `dst` valid
   for `total_count` bytes with the right access mode and synchronization, and `dst` not aliased
   by a live reference. Their callers already validate; each gets a SAFETY comment stating it.
4. Rewrite the `RawMemoryOps` `# Safety` text per origin. `Heap`, `Static`, `Transient`,
   `CrossArenaObjectRef`: no caller premise beyond a matching `layout`; out-of-range yields
   `BoundsCheck`. `Unmanaged`: `offset` is a live address. `Stack`: unchanged until plan 02.
   Use `checked_add` in the two inline-storage helpers so a wrapped sum is an error.

## Slices

1. **Typed field access checks the slot.** Design item 1, with tests: `field::<ObjectRef>` and `field_at_offset::<ObjectRef>`
   at an `i64` field, at a misaligned offset inside a reference, and at a reference slot of a
   different kind each return `None`; the supported `support_contract` accessors still pass. Enabled:
   every `FieldRef` in the VM goes through the check.
2. **No unsynchronized slice outside stop-the-world.** Design item 2, with a test that
   `data_location` on a value-type slot returns the storage base and `size_bytes` agrees with
   `total_size` while a `FieldStorage` read guard is held on another thread (`multithreading`).
3. **Chunked helpers are `unsafe`.** Design item 3. The compiler enforces it; add no new test.
4. **Memory-op contract matches the code.** Design item 4, with tests that an out-of-range offset
   on `Heap`, `Static` and `Transient` origins returns `BoundsCheck` for read and write, and that
   `usize::MAX`-adjacent offsets do not wrap.

## Refuted

- **SIMD length preconditions.** `dotnet_simd::copy_nonoverlapping` asserts equal lengths and
  `&mut`/`&` exclude overlap; the architecture modules are private and reachable only through it.
- **Stop-the-world GC reading storage aliased by a parked writer.** A thread parks only in
  `ThreadManager::safe_point`, which returns without parking while
  `has_active_thread_safe_write_guard()` is true, and `request_stop_the_world` asserts the same
  (tested by `request_stop_the_world_rejects_active_thread_safe_write_guard`). `FieldStorage` and
  static-map guards are held by closures (`with_data(_mut)`, `StaticStorageManager` shard
  accessors) that contain no safepoint poll; polls occur between instructions and copy chunks.
  That second half is convention, now stated in the `F1.StwParked` registry row. The SAFETY
  comment in `FieldStorage::resurrect` claimed a parked thread might hold a guard; it was corrected.
  Making `FieldStorage` guards join the safepoint-exclusion counter would turn the convention into
  enforcement; it is not needed for the outcome.
- **`ThreadSafeLock::trace` in release.** It uses `try_read` and panics rather than aliasing.

## Done when

- Each slice's tests pass in no-features and `multithreading`; Miri passes the new
  `dotnet-value` field-access tests (Miri command in [CI](../CI.md)).
- `bash scripts/check_doc_drift.sh` passes.
- `grep -rn "raw_data_unsynchronized" crates` finds only `FieldStorage::trace`, `resurrect` and
  `validate_resurrection_invariants`.

## Ask the maintainer only if

- Slice 1 needs `FieldRef<ObjectRef>` at a non-reference slot for a supported guest program
  (none is known); that would need a typed reinterpretation API, not a weaker check.
- A caller of a chunked helper cannot state its range premise, which means the intrinsic itself
  is unchecked and belongs in R09/R12.
