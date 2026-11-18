# Safe APIs whose soundness rests on unchecked caller premises

**Status:** Complete
**Outcome:** Safe Rust code in the workspace cannot make `FieldStorage`, `Object`, `StackValue` or
the memory-operation helpers decode a non-reference slot as an `ObjectRef`, read bytes without the
synchronization their contract needs, or call a raw-pointer copy/fill helper without an `unsafe`
block.

## Enforced premises

1. **Typed field access checks the slot.** `FieldStorage::field` and `field_at_offset` return
   `None` for an `ObjectRef` or `ManagedPtr` request unless the layout has a scalar of that kind
   at the offset. `FieldType` is sealed and its raw-byte `read_from` is unsafe; layout decoders
   are `pub(crate)` except `FieldLayoutManager::trace`, which frame homes call across crates and
   is public unsafe with a storage-validity contract.
2. **No unsynchronized slice outside stop-the-world.** `Object::size_bytes` reads
   `FieldStorage::data_len`. `raw_data_unsynchronized` (`crates/dotnet-value/src/storage.rs`)
   is `pub(crate) unsafe` and used only by `FieldStorage::trace`, `resurrect` and the resurrection
   invariant check.
3. **Raw chunk helpers are unsafe.** `copy_one_chunk` and `fill_one_chunk`
   (`crates/dotnet-intrinsics-unsafe/src/mem_helpers.rs`) are `unsafe fn` with a `# Safety`
   range contract; callers state their validated range.
4. **`RawMemoryOps` safety per origin.** `Heap`, `Static`, `Transient` and
   `CrossArenaObjectRef` need no caller premise beyond a matching layout (out of range yields
   `BoundsCheck`); `Unmanaged` requires a live address; `FrameHome` checks the live frame and full
   range internally. Inline-storage helpers use `checked_add`.

## Refuted (do not re-raise)

- **SIMD length preconditions.** `dotnet_simd::copy_nonoverlapping` asserts equal lengths, and
  `&mut`/`&` exclude overlap; the architecture modules are reachable only through it.
- **Stop-the-world GC reading storage aliased by a parked writer.** A thread does not park while
  it holds a `ThreadSafeLock` write guard, and `request_stop_the_world` asserts the same
  (`request_stop_the_world_rejects_active_thread_safe_write_guard`). `FieldStorage` and static-map
  guards are held only in closures with no safepoint poll; that half is convention, stated in the
  `F1.StwParked` registry row.
- **`ThreadSafeLock::trace` in release** uses `try_read` and panics rather than aliasing.

## Out of scope

Safe byte writes into reference slots (`Vector::get_mut`, `FieldStorage::with_data_mut`,
`ObjectRef::with_data_mut`, and `ObjectPtr::with_data_mut`, which also skips the owner barrier)
are tracked as "Safe byte writes into reference slots" in the plans README, "Not yet planned".

## Checks

- Field-access regressions (wrong kind, interior offset, overflow, nested structs, arrays) and
  owned-origin bounds regressions through a real `VesContext`, in both feature configurations and
  under pinned Miri ([CI](../CI.md#miri)).
- Compile-fail doctests for the raw decoder and the sealed trait.
- `grep -rn raw_data_unsynchronized crates` finds only the definition and the uses in item 2.
- `bash scripts/check_doc_drift.sh`.
