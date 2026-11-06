# Plans

The project goal is a maintainable ECMA-335 runtime whose supported behavior and safety
boundaries are understandable. Prefer one working runtime path over more infrastructure,
audits or documents.

## Queue

In priority order. Each plan's own `Status:` line is the only record of its status.

| ID | Plan | Note |
|---|---|---|
| 05 | [Safe APIs whose soundness rests on unchecked caller premises](05-safe-api-premises.md) | Next |
| 06 | [Cross-arena collection fixture through production decoding](06-cross-arena-collection-fixture.md) | Prerequisite of 08 |
| 07 | [Nested filters keep earlier suspended frames](07-nested-filter-suspension.md) | Prerequisite of 08 |
| 08 | [Async/await VM effects](08-async-effects.md) | Blocked on 06 and 07; slice notes in [08-async-effects/](08-async-effects/) |
| 04 | [Native continuations: Substring proof](04-native-continuations.md) | Done; superseded by 08 slice 1 once that lands |
| 03 | [Repair enum and named scalar frame homes](03-frame-home-enum-representation.md) | Done; repair found during 02 slice 5 |
| 02 | [Stable homes for addressable locals and arguments](02-stable-frame-homes.md) | Done; measured primitive cost accepted |
| 01 | [Executor teardown ownership](01-executor-teardown-ownership.md) | Done; kept as a pointer to the contract |

Plan IDs follow dependency order: a plan's prerequisites have lower IDs. Never reuse an ID; a
repair or follow-up found while executing a plan gets the next free ID.

## Not yet planned

Known problems without a plan. Write a plan for one only when it is chosen as the next work.
Evidence and IDs (p01…, R01…) are defined in the
[pointer-provenance inventory](../pointer-provenance/INVENTORY.md).

- **Negative and wide managed-pointer offsets:** `ManagedPtr` stores a compact unsigned offset,
  so byref arithmetic that goes below the base or past the compact range (for example
  `Unsafe.Subtract`, or `Unsafe.Add` with a negative count) fails with `NotImplemented`
  (`managed_offset_error` in `crates/dotnet-value/src/stack_value.rs`).
- **Static field addresses:** `ldsflda; conv.u` writes and static payload alignment
  (`p12_ldsflda_convu_stind_static`).
- **Native call boundary:** payload addresses, ABI returns, by-ref/out copy-back and pinned
  lifetime for P/Invoke (R06/R07). The current cross-arena raw-escape and by-value
  managed-pointer aggregate rejections stay until this is done.
- **Shared raw-memory access:** one protocol for competing plain, volatile, atomic, block and
  raw-alias accesses to shared storage (heap, statics) under `multithreading` (R09/R12).
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

A plan is a short, current document an agent can act on. Keep history in git and evidence in
`docs/pointer-provenance/` or `docs/ci-evidence/`.

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
