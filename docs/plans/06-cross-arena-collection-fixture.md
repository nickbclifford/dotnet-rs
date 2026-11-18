# Cross-arena collection fixture through production decoding

**Status:** Complete
**Outcome:** The three multithreaded real-VM cross-arena collection tests obtain arena B's object
in arena A only through the production storage codec, contain no `Gc::from_ptr`, and pass
natively and under pinned Tree Borrows Miri.

## What was built

`crates/dotnet-vm/src/executor_collection_tests.rs` drives real `Executor` arenas through the
production coordinator. Its tests used to brand a raw `ObjectPtr` from another thread with
`Gc::from_ptr`, skipping the owner lease, the cross-arena record and the codec. Now the owning
arena encodes the reference into `FieldStorage` with the production encoder; the bytes cross
threads as plain data; and the receiving arena decodes them through a receiver-owned instance
field (the same path a field load uses), then clears the temporary carrier slot. Payload checks
stay in the owner arena through a weak-handle witness.

- `arena_lifetime_vm_collection`: survival across two collections.
- `arena_lifetime_vm_collection_unrooted_negative_control`: B's object is swept without A's root.
- `arena_lifetime_vm_collection_fixed_point_payload_and_reclamation`: the A→B→C fixed point,
  payload write-back and bounded reclamation.

The no-features `arena_lifetime_vm_collection` is a single-arena control. Other R08 items (the
65,535-arena ID cap and the compact `ObjectRef` live-allocation contract) were out of scope.

## Checks

```bash
cargo test -p dotnet-vm --lib --no-default-features --features multithreading \
  executor::executor_collection_tests::arena_lifetime_vm_collection -- --test-threads=1
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
  cargo +nightly-2026-05-27 miri test -p dotnet-vm --lib --no-default-features --features multithreading \
  executor::executor_collection_tests::arena_lifetime_vm_collection -- --test-threads=1
```

Omit `--features multithreading` for the single-arena control. These are permissive Tree Borrows results, not strict-provenance or
full-VM soundness evidence.
