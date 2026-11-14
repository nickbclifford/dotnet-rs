# Correctness backlog: byte-read bounds, pointer slots, feature forwarding, thread IDs and the timer flake

**Status:** In progress (slice 1 complete)
**Outcome:** An overflowing heap byte read returns `BoundsCheck`, every CTS write into a pointer
slot either stores the address or fails loudly, the standalone reflection feature compiles, the
managed-ID counter goes through `dotnet_utils::sync`, and `threading_timer_callback_pool_thread_42` is either fixed
with a recorded cause or closed with captured evidence.

Closes these README "Not yet planned" entries: **Heap byte-read overflow**, **Standalone
reflection validation feature**, **Managed-thread local ID synchronization facade**, **Pointer
statics and `ManagedPtr` serialization** (the open remainder), and **Timer pool-thread fixture under load**.
String interning (the `string.Empty` bullet) is [plan 22](22-string-interning.md).

## Problem

- **Heap byte-read overflow.** `RawMemoryAccess::read_bytes`
  (`crates/dotnet-runtime-memory/src/access.rs`) computes `start + dest.len()` unchecked, so a
  heap-origin `VesContext::read_bytes` (`crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs`)
  with offset `usize::MAX` and a one-byte destination panics instead of returning
  `MemoryAccessError::BoundsCheck`. The static and frame-home origins already use
  `checked_window`/`frame_home_window`.
- **Pointer-slot writes after 611fcccb.** `ldsfld`/`stsfld` of pointer statics now use native
  words. Other `new_cts_value(..).write(..)` callers can still build a `CTSValue::Value(Pointer)`
  for a pointer-typed destination: reflection `FieldInfo.SetValue`
  (`crates/dotnet-intrinsics-reflection/src/fields.rs`), `stobj` without references
  (`instructions/objects/mod.rs`), `stfld` (`instructions/objects/fields.rs`), array element stores
  (`crates/dotnet-intrinsics-core/src/array_ops.rs`) and enum/type queries
  (`types/type_queries.rs`). `CTSValue::write` (`crates/dotnet-value/src/object/types.rs`)
  handles a one-word slot by storing the address, but any other slot shorter than
  `ManagedPtr::SIZE` is silently zero-filled, and an unobservable address is silently written as
  0.
- **Standalone feature.** `cargo check -p dotnet-intrinsics-reflection --features
  generic-constraint-validation` fails: the crate's feature is `[]`, but it calls
  `GenericLookup::validate_constraints`, which `dotnet-types/generic-constraint-validation`
  defines. Only `dotnet-vm`'s feature forwards both.
- **Managed-ID counter.** `threading/local.rs::NEXT_MANAGED_ID` is a `std::sync::atomic::AtomicI32`
  that uses `try_update`, bypassing the `dotnet_utils::sync` facade that Loom builds swap out.
  `threading/basic.rs::allocate_managed_thread_id` already uses the facade with `fetch_update`
  (Loom 0.7.2 has no `try_update`).
- **Timer fixture flake.** `threading_timer_callback_pool_thread_42` failed twice in full
  parallel `dotnet-cli` runs on 2026-11-11 and never in isolation. The failure message was not
  captured: the diff harness's VM exit-code assertion prints no VM stderr, and a hang ends as a
  nextest timeout with no indication of where it stopped.

## Scope

In: the five items above. String interning is [plan 22](22-string-interning.md).

Out:
- The broader shared raw-memory access protocol (R09/R12) and the native call boundary (R06/R07).
  Slice 1 only fixes arithmetic, not access ordering.
- Unrelated timer or pool redesign. If slice 5 finds the cause outside the timer/pool path, fix it
  only if the fix stays within one slice. Otherwise record the cause in the README and stop.

## Design

- **Bounds:** use checked arithmetic for every `offset + len` in `access.rs` that runs before a
  bounds check (`read_bytes`, and audit the rest of the file, e.g. the field walk near
  `f_start + field.layout.size()`). Overflow returns the same `BoundsCheck` error as an
  out-of-range access. Move `checked_window` down to `dotnet-runtime-memory` if that lets both
  layers share it. Keep the owner and write-barrier distinctions unchanged.
- **Pointer slots:** make the pointer case of `CTSValue::write` exhaustive. A slot of exactly
  `ManagedPtr::SIZE` gets the serialized pointer, a one-word slot gets the address, and any other
  length is a caller bug and gets a `debug_assert!`/`unreachable!`, not a fill. An unobservable
  address (transient, or an origin with no stable address) must not silently become null. Either
  resolve each caller to the native-word path, as `stsfld` now does, or return an error from a
  fallible write, so the destination type decides the encoding rather than the codec guessing from
  the slot length.
- **Feature forwarding:** support standalone features. Make
  `dotnet-intrinsics-reflection/generic-constraint-validation` forward to
  `dotnet-types/generic-constraint-validation` (and to `dotnet-runtime-resolver` if it calls into
  it), as `dotnet-runtime-resolver` already does.
- **Managed ID:** move `NEXT_MANAGED_ID` to `dotnet_utils::sync` atomics with `fetch_update` and
  `checked_add`. Allocation stays monotonic and still panics on exhaustion. Reconcile the
  `threading/basic.rs` comment that explains why that counter uses `fetch_update`.

## Slices

1. **Checked heap byte reads (complete).** Fix `read_bytes` and any other unchecked pre-check additions in
   `access.rs`. Add a `dotnet-vm` unit test where a heap-origin `VesContext::read_bytes` at
   `usize::MAX` with a one-byte destination returns `BoundsCheck`, plus the multithreading
   cross-arena origin if it shares the path. Run the `miri-value` lane and
   `fuzz-raw-memory-access` smoke.
2. **Feature forwarding.** Fix the manifest. Then `cargo check -p dotnet-intrinsics-reflection
   --features generic-constraint-validation` and `--no-default-features` pass. Add that check to
   CI next to the existing feature checks if a per-crate feature matrix already exists. Otherwise
   record the command in this plan.
3. **Managed-ID facade.** Migrate the counter. `cargo test -p dotnet-vm` and the Loom lane (the
   `cfg(loom)` crates that compile today) pass. Thread-identity fixtures
   (`thread_identity_start_42`, `thread_cross_start_42`) still match.
4. **Pointer-slot writes.** Write a `pointers/pointer_slot_writes_42.cs` diff fixture: reflection
   `FieldInfo.SetValue`/`GetValue` on instance and static pointer fields via
   `System.Reflection.Pointer.Box`, `stobj` of a pointer, pointer-element arrays, a struct with
   pointer fields stored through `stobj`, and boxed `IntPtr`/`UIntPtr` round trips. Fix whatever
   it exposes by making each caller choose the native-word encoding from the destination type.
   Then make `CTSValue::write`'s pointer arm exhaustive as described in Design.
5. **Timer flake.** First make failures diagnosable. Diff-harness exit-code assertions print the
   VM's stderr, and a VM run that exceeds a harness-side deadline is killed with its partial
   output reported, separately from the CLR run. Then reproduce under load: loop the full
   multithreading `dotnet-cli` integration suite (`cargo nextest run -p dotnet-cli --features
   multithreading`) alongside a CPU-saturating job, and run the single-threaded suite too, since
   one recorded failure was single-threaded. Allow about 1,000 fixture executions under load or
   one working day, whichever comes first.
   - *Reproduced:* diagnose from the captured output, fix it with a regression fixture or a
     deterministic unit test, and record the cause here.
   - *Not reproduced:* record the commands, counts and the improved harness output here, remove
     the README entry and note that a recurrence now captures its message.

Each slice ends with `cargo clippy --workspace --all-targets -D warnings` (both feature sets),
the doc-drift check and the full `dotnet-cli` and `dotnet-vm` test matrices passing, because past
plan commits repeatedly skipped these.

## Done when

- `pointer_slot_writes_42` matches `dotnet` in the diff harness, in both feature sets where
  applicable.
- The `usize::MAX` heap-read test returns `BoundsCheck`, and no `access.rs` addition can overflow
  before its bounds check.
- `CTSValue::write` has no silent zero-fill path for pointers.
- The standalone reflection feature compiles.
- `NEXT_MANAGED_ID` uses the sync facade.
- The timer entry is resolved by one of slice 5's two outcomes.
- The README entries listed at the top are removed or updated in the slice that closes each one.

## Ask the maintainer only if

- A pointer-slot caller cannot know its destination type, so the codec must stay guessing from
  slot length.
- The timer flake's cause lies in a subsystem another plan owns (STW, pool scheduling) and needs
  more than one slice to fix.
