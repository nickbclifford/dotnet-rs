# VM inline test review

Baseline `8431c1ab`. [Reviewed symbols](vm-inline-inventory.md) enumerate every test in the
assigned files, including adjacent feature/debug/Loom gates. All bodies, setup and relevant
production helper paths were inspected. Evidence is inspection; no VM tests were run by this
partition. Main VM and provenance reviews are separate records.

## Findings

### VI-1 — thread-local record smoke uses only null (low; rewrite or remove)

`threading::local::tests::records_are_parent_local_and_start_unstarted` inserts one null-keyed
record with null `thread_start`, then reads that field. It does not distinguish parent-local
state, retention/pruning, or a constructor always storing null. Its name refers to an unstarted
condition not observed by its assertions. F5 requires real thread/delegate/argument roots while
a worker may use them. Replace with distinguishable live records in two states and retention
observations, or remove this smoke if integration already protects that contract. Verify
`cargo test -p dotnet-vm records_are_parent_local` in both configurations and retained-record
integration; do not count the current test as thread-start or GC evidence.

### VI-2 — canonical tests miss claimed terminal outcomes (medium; strengthen)

`canonical_homing_cancel_after_publication` finishes successfully, drops its only waiter, then
checks an empty queue and a new non-ready request. A completion incorrectly changed to Canceled
on waiter drop passes. `canonical_homing_shutdown` checks Shutdown before calling `finish(Ok(()))`
on taken work, but never checks the original completion afterward. A regression overwriting
Shutdown with success passes. Retain a Completion handle and assert original success/Shutdown
after the competing operation, with existing queue/admission checks. These queue units need not
allocate real canonical objects. Verify `cargo test -p dotnet-vm canonical_homing_` with temporary
overwrite and cancellation faults. The shared-request test protects overlapping pending requests,
without forcing concurrent threads or actual object materialization.

### VI-3 — duplicate enum and dummy-handler assertions (low; consolidate)

`intrinsics::metadata::tests::{test_intrinsic_kind_categories,test_intrinsic_classification_categories}`
repeat enum inequalities without calling classification. `test_metadata_constructors` and
`test_intrinsic_metadata_reason_field` repeat kind/reason storage;
`test_intrinsic_metadata_handler_storage` uses Missing for both inputs, so always storing Missing
passes. Preserve useful constructor checks with distinct meaningful IDs and expected fields in
one case; remove redundant enum inequalities. Keep real loaded-CoreLib registry/classification
tests. Verify `cargo test -p dotnet-vm intrinsics::metadata::tests` in normal/fuzzing configurations.

### VI-4 — blocking registry guard keys on generated hashes/totals (medium; rewrite)

`intrinsics::async_registry_tests::blocking_intrinsics_have_no_sync_fallback` selects generated
names by hexadecimal suffix and fixes overload totals. The requirement is that blocking
operations use async dispatch without sync fallbacks. A valid naming/hash refactor or added
supported overload fails, while record presence does not establish dispatch association.
Select semantic signatures and check required overloads plus no fallback on applicable discovered
methods, retaining an independent nonempty baseline. Coordinate with VM-4; keep
`async_only_intrinsic_signatures_are_transportable`, whose shape policy protects suspension
transport. Verify async registry and blocking guest tests in both configurations. This is an
overload census, not a suite-size quota.

### VI-5 — STW regressions rely on scheduling sleeps (medium; strengthen)

`registered_initiator_waits_out_registration_without_parking` sleeps after signaling an initiator
while holding coordination; `queued_initiator_reasserts_stop_request_after_previous_resume` sleeps
before dropping the first guard. Neither observes that the second request reached the contested
acquisition. A thread scheduled only after release passes without exercising that regression.
Observe the actual production wait/probe boundary before releasing, then assert the outcome with
bounded cleanup. Keep stress as supplementary evidence. `registration_waits_until_stopped_world_resumes`
uses a stronger production hook. Verify the named tests and STW models with controlled delayed
start and a fault in the contested path.

## Retained families

Individual symbols are in the inventory; each row covers all listed cases in that family.

| Source family | Contract and evidence |
|---|---|
| GC coordinator | Keep registration, pressure thresholds/snapshots, fixed-point identity/rediscovery, command-slot handling, unregister races, precondition rejection and panic cleanup through production coordinator/guards. Fake workers model command processing, not heap tracing. Stress is bounded exploration. Ordered-drop probe observes coordinator state before the wrapped real STW guard resumes. |
| ThreadManager basic | Keep actual registration hook, stale-generation harvest with positive control, checked ID exhaustion, STW flags/coordination, writer rejection, debug lock-order checks, armed/disarmed cleanup and state decoding. Injected flags isolate guards; collection tests supply integration. VI-5 fixes scheduling evidence. ID bounds are representation requirements. |
| Canonical queue | Keep cancellation/admission, shared pending requests, terminal failure and owner/inactive/shutdown distinctions through production service methods, subject to VI-2. Canonical allocation integration is in vm.md. |
| Local records/IDs | Keep monotonic/exhaustion and Loom uniqueness through the real allocator; VI-1 addresses record smoke. Production retention tracing was inspected to establish its limited claim. |
| Worker pool | Keep cap/reservation/idle accounting and request injection at actual probe/registration points. Counts express configured resource policy. Setup supplies workers, without claiming managed thread creation. |
| Events | Keep auto/manual grants, expiry/order, reset-after-grant, cancellation transfer and invalid IDs. Injecting a registered deadline isolates grant-survives-expiry, not wall-clock execution. |
| Worker registry | Keep native completion/outcome/retirement/finalization/quiescence/background/shutdown/join cases. Constructed entries test real transitions; spawn and late-spawn shutdown connect to ownership. Cleanup closure is held under real completion locking. Ready signals do not force every wait-before-publication schedule; production Loom models complement native tests. |
| Statics | Keep explicit independent state/event table, stale CAS rejection, claim success/fail/drop and nested edge cleanup normally/on unwind. Constructed state isolates transitions; guest fixtures execute cctors. Test-only mark_initialized cannot substantiate production publication. |
| Shared state | Keep first-fatal-wins/abort, cloned signal, distinct-arena finalizer accounting and cache reports. Counts protect idempotency. Front-cache checks express deliberate tuning choices, changeable with policy revisions. Registry consistency does not prove all caches were declared. AbortSignal Send/Sync guards its cross-thread contract. |
| Intrinsic metadata | Keep loaded CoreLib classification for Marvin seed, recursive-body fallback, managed span copying, array-data overloads and bodiless Math/MathF; these depend on installed framework shape. VI-3/VI-4 address weak units. |
| GC handles | Keep encoding/decoding consistency and reserved-tag rejection. IDs are opaque; a particular index mapping is not an external requirement. |
| Numeric generic slots | Keep production handlers on a real stack with distinct type slots, literal results and missing-slot rejection. Matching positive operations constrain the final broad error check. |
| Synchronized helper | Keep pending-at-IP0 versus completed/no-pending decisions as narrow protocol checks, not lock-operation evidence. |
| Raw-memory access | Keep complete-range/overflow rejection, unchanged storage, final-slot positive access, pointer atomics, retired home rejection and distinct thread-static owner restore. Real access and meaningful bytes supplement serialization consistency. |
| Executor entrypoint | Keep complete_entrypoint_return with empty/nonempty pending finalizers. Null sentinel witnesses queue nonemptiness and is removed before teardown; this is not finalizer-liveness evidence. |
| Fixture macros | make_test_method builds supplied instructions/locals; make_test_assembly supplies assembly/System.Object identity. They are setup builders; reachability remains each caller's responsibility. |

## Completion and verification limits

All cases in this partition have an inspection disposition. Proposed validation belongs to 27b;
none is recorded as executed here. Overall VM completion also depends on vm.md and
vm-provenance.md, including their helper, diagnostic, macro and feature coverage.
