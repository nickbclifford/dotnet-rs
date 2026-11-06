# Plan 08 slice 4: `ldsfld` through initialization effects

**Status:** Notes. Fill in Done-when and open questions before starting.

## Consumer

`dotnet-vm/src/instructions/objects/fields.rs` (`ldsfld`, also `stsfld`/`ldsflda` callers of
`initialize_static_storage`) and `dotnet-vm/src/statics.rs` (`StaticInitResult::{Execute,
Initialized, Recursive, Failed, Waiting}`, `init`, `wait_for_init`).

## Decided

- `ensure_initialized` uses the capturing `try_call` (maintainer decision, 2026-11-05).
  `try_call` is introduced in this slice on top of slice 2's native boundary and mailbox. The
  child's handlers and cleanup run, Rust receives `Err(RootedException)`, publishes the failure,
  and throws `TypeInitializationException(inner)`. Outer filters therefore see the TIE, as in
  CLR. Today the wrap happens in `unwind_frame` (`stack/context.rs:466-497`) after the search
  pass has already run filters against the inner exception (`dotnet-exceptions/src/lib.rs:672-679`).
  This is an intended observable change; verify it with a CLR-diffed fixture that has a filter.

## Verified facts (HEAD 069c8294, read from code, not reproduced)

- Callers of `initialize_static_storage`: `fields.rs:242` (`ldsfld`), `:283` (`stsfld`, before
  popping the value), `:488` (`ldsflda`); `instructions/objects/mod.rs:245` (`newobj`);
  `call_ops_impl.rs:557-571` (`dispatch_method`); `type_construction.rs:47`
  (`RunClassConstructor`). Implementation: `context_ops.rs:834-923`.
- Retry: after a `.cctor` push the IP is untouched and `handle_return` skips the increment for
  `is_cctor` frames (`context.rs:295-298,337-339`); after a yield `back_up_ip`
  (`context_ops.rs:925-934`) restores the snapshot taken at `dispatch/mod.rs:146-149`.
- `WaitGraphEdgeGuard::disarm` (`statics.rs:138-142`) exists, is unused and is marked
  `#[allow(dead_code)]`. It is the intended hook for handing the edge to a wait registration.
  `wait_for_init` (`statics.rs:489-546`) blocks inside the mutation on a condvar with 10 ms
  timeouts and is `unreachable!` in ST.
- `beforefieldinit` is honored only in `dispatch_method`. Field access and `newobj` always
  initialize on first access, so this slice changes no timing.
- Fixtures (`statics/`): `cctor_failure_42`, `cctor_trigger_42`, `cctor_trigger_vt_42`,
  `circular_init_42`, `circular_init_mt_42` (two executors over shared statics). Missing: a
  non-cycle cross-thread `Waiting` that ends in success, one that ends in failure, and a TIE
  seen by an outer filter.

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

- `newobj`, `dispatch_method` and `RunClassConstructor` also call `initialize_static_storage`.
  Migrate each or list it explicitly as remaining on the legacy path; the retry path and the
  late TIE wrap are deleted only if nothing else uses them.

(Answered: `stsfld` and `ldsflda` migrate with `ldsfld`, since they share the helper and the
retry path cannot be deleted otherwise. `beforefieldinit` implies no timing change; see above.)

## Done when (draft)

- Static-init fixtures pass in both configurations, including cross-thread init waits and an
  init cycle. The `ldsfld` retry path is deleted.
