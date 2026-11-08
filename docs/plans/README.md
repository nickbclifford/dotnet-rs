# Plans

The project goal is a maintainable ECMA-335 runtime whose supported behavior and safety
boundaries are understandable. Prefer one working runtime path over more infrastructure,
audits or documents.

## Queue

In priority order. Each plan's own `Status:` line is the only record of its status.

| ID | Plan | Note |
|---|---|---|
| 09 | [Retain arenas after managed thread exit](09-retired-arena-lifetime.md) | Prerequisite of 08 static initialization effects |
| 05 | [Safe APIs whose soundness rests on unchecked caller premises](05-safe-api-premises.md) | After 09 |
| 06 | [Cross-arena collection fixture through production decoding](06-cross-arena-collection-fixture.md) | Prerequisite of 08 |
| 07 | [Nested filters keep earlier suspended frames](07-nested-filter-suspension.md) | Prerequisite of 08 |
| 08 | [Async/await VM effects](08-async-effects.md) | Slice notes in [08-async-effects/](08-async-effects/) |
| 04 | [Native continuations: Substring proof](04-native-continuations.md) | Measured large-call cost accepted; scheduler contract and behavior controls retained for 08 slice 1 |
| 03 | [Repair enum and named scalar frame homes](03-frame-home-enum-representation.md) | Done; scalar bounds, opcode/growth coverage and historical reproductions verified |
| 02 | [Stable homes for addressable locals and arguments](02-stable-frame-homes.md) | Done; measured primitive cost accepted |
| 01 | [Executor teardown ownership](01-executor-teardown-ownership.md) | Done; kept as a pointer to the contract |

Plan IDs normally follow dependency order. Never reuse an ID; a repair or prerequisite found
while executing a plan gets the next free ID, even when it must precede that plan.

## Not yet planned

Known problems without a plan. Write a plan for one only when it is chosen as the next work.
Evidence and IDs (p01…, R01…) are defined in the
[pointer-provenance inventory](../pointer-provenance/INVENTORY.md).

- **Unhandled worker exceptions:** CLR terminates the process when a managed thread's exception
  goes unhandled; `Thread.Join` instead rethrows it as `InvalidOperationException`
  (`threading_join_managed_thread` in `crates/dotnet-vm/src/stack/context_ops.rs`).
- **Cross-thread GC handles:** `intrinsic_gchandle_internal_get` and related operations index the
  calling arena's local handle table, so a `WeakReference` created on one thread cannot reliably
  read its target from another, even while both arenas remain registered.
- **`Thread.IsBackground`:** unimplemented, so every managed thread is treated as foreground at
  exit (plan 09).
- **Negative and wide managed-pointer offsets:** `ManagedPtr` stores a compact unsigned offset,
  so byref arithmetic that goes below the base or past the compact range (for example
  `Unsafe.Subtract`, or `Unsafe.Add` with a negative count) fails with `NotImplemented`
  (`managed_offset_error` in `crates/dotnet-value/src/stack_value.rs`).
- **Static field addresses:** `ldsflda; conv.u` writes and static payload alignment
  (`p12_ldsflda_convu_stind_static`).
- **Instance field alignment:** `FieldStorage` uses `Vec<u8>`, which does not promise the
  alignment required by `get_field_mut_local`; exception stack-trace writes can trip its
  alignment assertion under Miri.
- **Native call boundary:** payload addresses, ABI returns, by-ref/out copy-back and pinned
  lifetime for P/Invoke (R06/R07). The current cross-arena raw-escape and by-value
  managed-pointer aggregate rejections stay until this is done.
- **Shared raw-memory access:** one protocol for competing plain, volatile, atomic, block and
  raw-alias accesses to shared storage (heap, statics) under `multithreading` (R09/R12).
  `FieldStorage`'s aligned byte-to-integer atomic path also discards pointer provenance when
  storing object references; a subsequent static-storage GC trace fails under Miri.
- **Safe byte writes into reference slots:** `Vector::get_mut`, `FieldStorage::with_data_mut` and
  `ObjectRef::with_data_mut` hand out `&mut [u8]` over storage the collector traces, so safe code
  can write forged pointer bytes; `ObjectPtr::with_data_mut` also skips the owner barrier
  (`ThreadSafeLock::borrow_mut_unbarriered`). Fold into R09/R12 and R10/R11.
- **Resumable bulk operations and barriers:** `cpblk`/`initblk`/`Memmove`/`Array.Copy`/`Fill`
  that keep progress across safepoints and apply GC write barriers to reference slots (R10/R11).
- **Thread-pool scheduling:** `ThreadPool.UnsafeQueueUserWorkItem` is an accepted no-op
  (`dotnet-intrinsics-threading/src/thread.rs`), so guest code whose continuations need the
  thread pool (`Task.Run`, timers, `Task.Delay`) cannot complete. Guest `async`/`await` that
  completes synchronously or through `TaskCompletionSource` works (fixtures under
  `crates/dotnet-cli/tests/fixtures/async/`).
- **CI feedback:** remaining steps of the [test hierarchy](../CI.md#test-hierarchy-and-migration).

## Writing a plan

A plan is a short, current document an agent can act on. Keep detailed history in git.
Retain validation conclusions, reproducible check commands, accepted costs and unresolved
limits in the relevant plan or subsystem doc. The pointer inventory records current
safety coverage; dated transcripts do not define current requirements.

- **Header:** `Status:` (Not started / In progress / Parked / Complete) and a one-sentence
  **Outcome** that a test can check.
- **Problem:** what goes wrong today, with code paths and test names.
- **Scope:** what is in, and what is out with the reason. Use the spec (ECMA-335 section
  numbers) to rule out cases that valid programs cannot produce.
- **Design:** the chosen approach, briefly. Reuse existing mechanisms where one fits.
- **Slices:** ordered increments. Each slice ends with the VM using the new path and its tests
  enabled. A slice never builds support that only a later slice uses.
- **Done when:** the checkable exit conditions.
- **Ask the maintainer only if:** a short list of specific triggers.

If a slice turns out to need much more than its description (new modules, or support for
cases outside the stated scope), stop and revise the plan rather than splitting the work into
more preparatory steps.
