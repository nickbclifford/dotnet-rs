# Correctness backlog: byte-read bounds, pointer slots, feature forwarding, thread IDs and the timer flake

**Status:** Complete
**Outcome:** An overflowing heap byte read returns `BoundsCheck`, every CTS write into a pointer
slot either stores the address or fails loudly, the standalone reflection feature compiles, the
managed-ID counter goes through `dotnet_utils::sync`, and `threading_timer_callback_pool_thread_42`
is closed with captured evidence.

## What changed

- **Checked heap byte reads.** `RawMemoryAccess::read_bytes` and the other pre-check additions in
  `crates/dotnet-runtime-memory/src/access.rs` use checked arithmetic, so offset `usize::MAX`
  returns `MemoryAccessError::BoundsCheck` instead of panicking.
- **Pointer-slot writes.** Callers choose the native-word encoding from the destination type
  (`CliToCts::pointer_slot_address`), and the pointer arm of `CTSValue::write` is exhaustive: a
  `ManagedPtr::SIZE` slot gets the serialized pointer, a one-word slot the address, and nothing is
  silently zero-filled.
- **Feature forwarding.** `dotnet-intrinsics-reflection/generic-constraint-validation` forwards to
  `dotnet-types`.
- **Managed-ID counter.** `NEXT_MANAGED_ID` (`crates/dotnet-vm/src/threading/local.rs`) uses the
  `dotnet_utils::sync` atomics with a checked `fetch_update`, so Loom builds swap it out;
  allocation stays monotonic and panics on exhaustion.
- **Fuzzing fixtures.** The fuzzing build's sandbox denies every native entry point by design.
  `fixture_test!` now skips a fixture under `fuzzing` when its failure is a denied `SystemNative_*`
  call (or the `TimerQueue` type-initializer wrapper), instead of a hand-maintained ignore list;
  any other failure still fails.

## Timer flake (not reproduced)

`threading_timer_callback_pool_thread_42` failed twice in full parallel runs and never in
isolation. Two multithreaded and two single-threaded full `dotnet-cli` runs alongside 24
CPU-saturating processes passed all 1,872 tests, with no cause found. The diff harness now prints
the VM's stderr on an exit-code mismatch and reports partial stdout and stderr when a separately
timed CLR or VM subprocess exceeds its deadline, so a recurrence captures its message.

## Checks

```bash
cargo check -p dotnet-intrinsics-reflection --no-default-features --features generic-constraint-validation
cargo run -p xtask -- matrix test-features      # the ten feature configurations
```

`pointers/pointer_slot_writes_42.cs` is a diff fixture; `pointer_stobj_writes_native_word`
exercises `stobj` of a pointer (which C# cannot express). The thread-ID changes are covered by the
exhaustion and concurrent-allocation tests and the `thread_identity_start_42` and
`thread_cross_start_42` diff fixtures.
