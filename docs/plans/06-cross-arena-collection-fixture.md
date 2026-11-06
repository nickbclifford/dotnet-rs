# Cross-arena collection fixture through production decoding

**Status:** Complete
**Outcome:** The three multithreaded real-VM cross-arena collection tests obtain arena B's
object in arena A only through the production storage codec, contain no `Gc::from_ptr`, and
pass natively and under pinned Tree Borrows Miri.

## Problem

`crates/dotnet-vm/src/executor_collection_tests.rs` drives real `Executor` arenas through the
production coordinator, but builds A's reference to B's object by calling `Gc::from_ptr` on a
raw `ObjectPtr` handed across threads (`executor_collection_tests.rs:143`; the module comment at
line 11 says production never does this). Affected tests:

- `arena_lifetime_vm_collection`
- `arena_lifetime_vm_collection_unrooted_negative_control` (uses the same harness)
- `arena_lifetime_vm_collection_fixed_point_payload_and_reclamation`

Production obtains a foreign reference differently. A live reference written to storage is
encoded as Tag 5 with its owner arena ID (`ObjectRef` encode, `dotnet-value/src/object/mod.rs:537–569`).
Reading it back goes through `try_from_storage_ptr` (`object/mod.rs:612–651`), which leases the
owner, calls `record_cross_arena_object` and only then brands the pointer in the reader's
mutation. The shortcut skips the lease, the record and the codec, so the tests do not show that
production brand discipline keeps a foreign object alive. The R08 row in
`docs/pointer-provenance/INVENTORY.md` and the limitation in
`docs/GC_AND_MEMORY_SAFETY.md` (cross-arena root tracing) record this as accepted and unresolved.
[Plan 01](01-executor-teardown-ownership.md) excluded it.

[Plan 08](08-async-effects.md) roots foreign references in native activations under
`multithreading`. Its tests need a foreign reference obtained the way production obtains one.

## Scope

In:
- One shared test helper that moves B's reference into A through the production codec: B
  encodes the reference into storage in its own mutation, and A decodes it in A's mutation
  with the same code a field or static load uses. A real static field is preferred. If the bare
  loader used by these tests cannot provide one, use a field of an A-owned object.
- Port the three tests to the helper and keep their assertions: survival across two
  collections, sweep without A's root, the fixed-point A→B→C chain, payload write-back and
  bounded reclamation.
- Update the R08 row and the GC safety limitation text.

Out:
- Any change to the coordinator, the codec or `ObjectRef::trace`. If a production defect shows
  up, record the failing test and stop (see below).
- The no-features control (`arena_lifetime_vm_collection` without `multithreading`). It has a
  single arena, so there is no foreign reference.
- Other R08 items (65,535-arena ID limit, the compact ObjectRef live-allocation contract).

## Design

The helper performs one codec round trip per edge. B (or C) writes with the production encoder
during its mutation. The bytes cross threads as plain data. A decodes during its mutation,
under the owner lease that decoding already acquires, and stores the result in a normal GC
handle. Payload checks stay in the owner arena through the existing weak-handle witness, so no
test reads a possibly swept object.

## Slices

1. Add the helper and port `run_vm_collection` (two tests). Enabled when both pass natively
   and under pinned Miri with `--features multithreading`.
2. Port the fixed-point test (two cross edges) and update the docs.

## Done when

- `executor_collection_tests.rs` has no `Gc::from_ptr` and no foreign-branding `SAFETY:`
  comment.
- All three tests pass natively and under the pinned Tree Borrows Miri command used for
  other R08 tests, with `--features multithreading`.
- The negative control still observes a sweep.
- INVENTORY R08 and the GC safety limitation no longer describe the fixture as
  foreign-branded.

## Ask the maintainer only if

- A ported test fails, meaning production decoding does not keep the object alive. That is a
  runtime defect, not a fixture problem. Do not weaken the assertion.
- No production storage path can carry the reference without loading a fixture assembly.

## Implementation evidence

Both slices now use production-encoded `FieldStorage`, loaded through a receiver-owned
instance field; the temporary carrier slot is cleared after decoding. No fixture assembly
is needed. The module contains no `Gc::from_ptr`. Native collection-module checks passed
(5 multithreaded tests and 1 no-features control). Pinned Tree Borrows Miri passed
all three multithreaded collection tests (3 passed, zero ignored; 158.45 s) and the
no-features control (1 passed, zero ignored; 40.56 s). Clippy with warnings denied,
formatting and diff whitespace checks passed.

Reproduce the collection checks with `cargo test -p dotnet-vm --lib
--no-default-features --features multithreading
executor::executor_collection_tests::arena_lifetime_vm_collection -- --test-threads=1`.
For Miri replace `cargo test` with `cargo +nightly-2026-05-27 miri test` and set
`MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`.
Omit `--features multithreading` for the single-arena control. These are permissive
Tree Borrows results, not strict-provenance or full-VM soundness evidence.
