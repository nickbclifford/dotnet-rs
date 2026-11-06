# Plan 08 slice 4: `ldsfld` through initialization effects

**Status:** Notes. Fill in Done-when and open questions before starting.

## Consumer

`dotnet-vm/src/instructions/objects/fields.rs` (`ldsfld`, also `stsfld`/`ldsflda` callers of
`initialize_static_storage`) and `dotnet-vm/src/statics.rs` (`StaticInitResult::{Execute,
Initialized, Recursive, Failed, Waiting}`, `init`, `wait_for_init`).

## Introduced here

- Instruction completion action: the IP advances only after the load succeeds. Instructions
  start activations through the same entry helper. The instruction macro is not changed.
- `ensure_initialized(type, generics)` as a handler state machine: probe, claim, run the
  .cctor as a managed child, publish success or failure, wake waiters.

## Carried from the archive (and current code)

- MT wait graph (`statics.rs`, `wait_graph`, `WaitGraphEdgeGuard`). Hand the edge over to an
  owned wait registration atomically. Re-run the cycle check before waiting again after a GC
  stop removes the edge. A cycle follows the existing `Recursive` policy (ECMA-335 §II.10.5.3.3
  deadlock rules).
- An abandoned claim (abort, host error, cancellation) is published as failed, and waiters are
  woken on every terminal path.
- In ST, a same-thread `Waiting` remains unreachable, as today.

## Open questions

- Do `stsfld` and `ldsflda` migrate in this slice too, or stay on retry?
- What does `beforefieldinit` imply for timing?

## Done when (draft)

- Static-init fixtures pass in both configurations, including cross-thread init waits and an
  init cycle. The `ldsfld` retry path is deleted.
