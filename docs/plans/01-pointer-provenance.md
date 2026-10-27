# Unified pointer/provenance project plan

**Status:** In progress — partial M0 (VM probe harness and blocking controls)
**Prepared:** 2026-10-27
**Baseline:** `b49f5df8` (also HEAD when this plan was prepared)
**Scope:** Guest storage, pointer operations, GC transport/barriers, concurrency, and native-call boundaries.

Deliver a pointer model in which supported guest operations preserve host allocation provenance,
retain the required owners, and obey one host memory-access protocol. Valid IL must survive local
assignment, frame growth, repeated `localloc`, collection, and suspension without host UB or host panics.
Native-integer conversions remain available through an explicit exposed-provenance boundary.

This is the implementation plan derived from all three [reviews](../pointer-provenance/README.md).
The underlying historical pass archives and raw logs are not present in this checkout; current evidence
limits and source-recovery tasks are recorded in the [inventory](../pointer-provenance/INVENTORY.md).
The plan is **in progress, partial M0**: a VM probe harness and blocking control lane exist, but no
runtime repair or full M0 completion is claimed. Milestone IDs below are the work-item IDs; split them
into focused changes while retaining their acceptance criteria. Ownership is assigned by subsystem, not
by an assumed individual or schedule.

## Evidence and reconciled decisions

The historical pass-1/2/3 review archives are absent from this checkout. Their subject areas are
summarized in the [current evidence inventory](../pointer-provenance/INVENTORY.md): pass 1 covered
library invariants and strict Miri; pass 2 exercised real IL; pass 3 added independent IL, standalone
borrow/race models, native reproductions, and experimental patches. Probe IDs `p00`–`p20` below
always mean pass 3; pass-1 probes are identified by name. A standalone model establishes the model's
behavior, not an end-to-end reproduction in the VM. Source-review and audit-only claims require
confirmation in M0.

| Question | Unified decision | Basis and limit |
|---|---|---|
| Strict provenance everywhere? | Keep managed storage and GC pointer transport allocation-derived; isolate native-int/foreign reconstruction in audited helpers. | Pass 1's strict failures and pass 3's E-series distinguish unsupported strict reconstruction from actual permissive-Miri UB. Whole-VM strict Miri is not the target. |
| Native integers or shadow metadata? | Preserve native-integer byte/arithmetic semantics with explicit exposure. Retain actual pointers when already available. | Pass 3 E8/E8b shows byte round trips lose pointer provenance; foreign allocations need no VM registry entry. A registry may validate ranges or recover owners, but cannot manufacture provenance or silently root all native ints. |
| Cached pointer or logical location? | Stable raw-backed storage permits cached allocation-derived pointers. Retain origin, offset, extent, generation, and GC-role metadata; establish access rights and lifetime at use. | p04/p09 show that native ints require stable addresses regardless of managed re-resolution. Pass 1's transient and lease failures show that numerical stability alone is insufficient. |
| Does a mutable slice fix exposure? | Derive guest-visible pointers from retained allocation bases without intermediate payload references. | Pass 3 E12b/E13 distinguishes a mutable reborrow from a raw derivation that survives ordinary slot writes. Heap patch 3c2 covers only selected paths. |
| Are Relaxed atomics sufficient? | They are a candidate for aligned natural-width accesses. Decide a complete overlapping-access protocol before removing locks. | E9b passes; E9c rejects mixed-width overlaps; E9d rejects cached raw accesses racing locked access. Mixed-width host UB is an unresolved design constraint, not an accepted completion condition. |
| Delay all GC work until storage migration? | Repair ownership, transport, root publication, and teardown early; integrate barriers into the shared store/copy API. | Pass 1 proves transport provenance loss; pass 3 E11 models missing root barriers. These are independently actionable. |

Use pass 3's storage direction, with pass 1's ownership/access contract. Do not replace `gc-arena`
as a prerequisite, introduce a mandatory virtual address space, or add a registry to every load merely
to pursue strict native-int reconstruction. Preserve existing provenance-aware object codecs and pointer atomics.

## Target contract

1. **Storage:** `GuestBlock` owns aligned, non-resizing raw storage. Alignment is at least 16 and must
   satisfy each concrete layout/ABI, including larger requirements. Track payload extent separately
   from VM layout size and allocation-pressure accounting. Object, array, string, static, frame-home,
   and frame-allocation storage follow the same allocation/ownership rules.
2. **Addressable homes:** locals and arguments live in layout-described, GC-traced frame homes;
   evaluation-stack values are temporary and non-addressable. Frames retain their allocation blocks
   through normal return, exceptions, tail calls, and suspension. Generation checks protect reused identities.
3. **Pointers:** managed byrefs retain owner/origin, offset, target metadata and applicable generation.
   A cached raw pointer is derived from the retained allocation base. Unmanaged values carry an actual
   pointer where available; null is explicit, including arithmetic that wraps to zero. Do not settle the
   final Rust struct/codec layout until tracing, serde, and atomic representation constraints are reviewed.
4. **Access:** address observation, read, write, copy, and atomic operations have distinct contracts.
   A scoped access retains storage/arena guards, roots, and required safepoint exclusion through use.
   Validate checked ranges against actual payload extent before references or accesses; reject overflow
   and invalid widths/alignment. Address formation may wrap without granting dereference permission.
5. **GC:** provenance, rooting, pinning, generation leases, synchronization, and write barriers are
   separate obligations. Pointer-derived native ints do not automatically become strong roots. Typed
   reference stores and copies preserve pointer representation and publish the required barriers.
6. **Concurrency/native calls:** every interpreter, intrinsic, atomic, block, and FFI access must participate
   in the chosen memory protocol or a documented exclusion/pinning protocol. A lock on one path does
   not synchronize an unrelated raw path. Validate the contract in both feature configurations.

## Milestones and dependencies

| ID | Deliverable | Depends on | Primary subsystem |
|---|---|---|---|
| M0 | Executable evidence inventory and regression gates | None | VM/value tests, CI |
| M1 | Contain independent bounds, lifetime, arithmetic, and GC defects | M0 harness; land fixes individually | Value, VM-data, VM, utils GC |
| M2 | Correct native ABI and pinned-address boundary | M0; M1 access/owner rules | P/Invoke, core/unsafe intrinsics |
| M3 | Stable guest blocks and addressable frame homes | M1 contract; M4 protocol decision before shared-path rollout | Value storage, VM-data, VM stack |
| M4 | Unified host memory-access protocol | M0 lab; implementation uses M3 blocks | Runtime-memory, threading, all raw callers |
| M5 | Typed stores, bulk operations, GC barriers and resumption | M1 immediate repairs; M3/M4 final substrate | Runtime-memory, VM instructions, intrinsics |
| M6 | Close compatibility, CI, performance, and documentation gates | M1–M5 | All affected subsystems |

Start M4's design during M0/M1. Its implementation follows M3; its design decision precedes M3's
shared-storage rollout, avoiding a dependency cycle. M2 and independent M1 fixes can proceed separately.
An interim heap fix does not complete stack/static storage migration. Keep locks until all competing
access paths participate in the replacement protocol. Apply immediate barrier repairs during M1/M5
discovery rather than waiting for the final copy API.

### M0 — Make failures and controls executable

**Current progress — partial M0 only:** The durable [inventory](../pointer-provenance/INVENTORY.md)
maps R01–R13 and catalogs the 23 VM probes. Six bounded controls (`p00_smoke`,
`p02_ldloca_stind_i4_control`, `p03_ldloca_convu_ldind_i4`, `p05_single_localloc_roundtrip`,
`p07_localloc_one_past_end`, and `p11_ldelema_stind_heap_array_control`) each passed separate
revalidation in both no-features and multithreading configurations (12 recorded exact-name passes).
The six controls are enabled in ordinary tests and run by the blocking `miri-pointer-controls` CI job
as two configurations by three guarded shards under the pinned permissive Tree Borrows setup. The
other 17 probes remain ignored for diagnostic execution only. This filtered VM control lane is not a
full VM Miri suite, strict-provenance evidence, or a claim of runtime soundness.

- Keep the pass-3 IL p00–p20 probes and the two unique pass-2 cases (null through `&p`, wide stores)
  in the owning VM suite, retaining the mock corlib primitive typedefs. Recover and integrate the
  missing pass-1 library probes. Keep read/control tests alongside writes and dereference after
  mutation or decoding.
- Port lab E1–E13 and `gc_model::e11*` with clear labels for model tests. Repair the backing-storage
  aliasing in the two ignored atomic tests; distinguish that UB from their historical Miri ICE reasons.
- Preserve the existing per-probe classification; classify recovered library/model/FFI evidence and
  future regressions as native, permissive TB, strict-core TB, supplemental Stacked Borrows, or native
  FFI as appropriate. Tests that deliberately exhibit UB stay ignored with a finding/work-item ID
  until fixed; do not execute UB as ordinary native tests. Keep known-failure reproductions in the
  explicit diagnostic lane and run failing Miri reproducers separately.
- The six passing pointer controls now have a blocking lane. Add appropriate gates for each repaired
  regression and expand VM coverage beyond `jmp_tests`, `tail_calls`, and `fault_tests`. Recover exact
  strict filters and reasons where possible; current dependency traces and historical filters are
  unavailable, and exposed reconstruction remains an explicit strict-provenance boundary.
- Inventory the 73 reported provenance-lint hits before denying `fuzzy_provenance_casts` and
  `lossy_provenance_casts` in first-party code. Use explicit exposure/reconstruction helpers; record any
  narrow exceptions. Do not deny everything before migration and call unrelated dependency failures fixes.
- Capture native benchmark baselines using the [benchmark workflow](../BENCHMARK_WORKFLOW.md),
  including memory/unsafe_buffer/span/dispatch and field/byref/native-int loops.

**Exit:** every finding in the coverage table below has a permanent test or a specific reproduction task;
all current passing controls run in CI. Evidence distinguishes VM tests, models, source claims, and
dependency blockers. No green filtered lane is described as a green full strict suite.

#### Recorded follow-ups (not completed)

The [evidence inventory](../pointer-provenance/INVENTORY.md) maps R01–R13 to exact probes,
evidence limits, owners and reproduction tasks. The historical pass-1/2/3 archive directories,
raw transcripts, library/model/FFI sources and patch sources are absent from this tree; their
external recovery location is unknown. Recover an external backup if available or independently
reproduce the missing evidence. The integrated p20 source came from a patched tree; its unpatched
baseline result remains unknown. The six filtered permissive-TB controls do not establish full M0,
strict Miri or runtime soundness.

- Recover and integrate pass-1 library probes, standalone E1–E13 models and `gc_model::e11*`,
  preserving the distinction between model, library and real-VM evidence. Recover the missing address
  and libffi fixtures and reproduce the p20 baseline as described in the inventory.
- Identify the two historically reported atomic tests, distinguish test-backing aliasing from their
  historical Miri ICE reasons, and repair and validate them (M0/M6); their identities remain unknown.
- Inventory and migrate the reported 73 provenance-lint hits before denying `fuzzy_provenance_casts`
  and `lossy_provenance_casts`; document narrow exceptions (M0). The count is not a fresh audit.
- Capture native benchmark baselines for memory/unsafe_buffer/span/dispatch and field/byref/native-int
  paths, recovering the excluded addressing and field performance probes as needed (M0).
- Recover or re-establish strict-core evidence and exact selectors after diagnosing dependency
  blockers; the historical filters and first-failure dependency traces are unavailable. Separately
  scope unrelated VM Miri expansion (M0/M6). Whole-VM strict Miri is not a claim of this harness.
  Observe hosted CI cold/cache timings before changing the provisional 45-minute control timeout.
- Complete the M1–M6 pointer, storage, GC, ABI and concurrency repairs and their existing exits; no
  runtime repair or milestone completion follows from this harness.

### M1 — Contain independently reproducible defects

- **M1a, heap access:** use actual payload lengths in `with_data`; eliminate shared slices from address
  observation and use root-derived wrapping address formation. Apply explicit exposure at conversion
  sites, including their separate implementations. The historical 3c2 patch source is absent; recover it
  before reviewing its exact changes, or independently validate the same design. Rewriting stale safety
  comments and the remaining stack fallback would still be required; the historical patch is not a
  wholesale fix.
  Gates: p10/p13/p14/p16/p20 plus pass-1 one-byte heap read and safe-OOB observation probes.
- **M1b, pointer semantics:** wrapping unmanaged arithmetic with explicit null handling; comparison arms;
  correct `Unsafe.ByteOffset` sign and pointer acceptance for `conv.i`/native-int stores. Validate native
  signed/unsigned widths. Gates: p06/p07/p08/p17/p18 and new signed conversion/store/load/dereference tests.
- **M1c, frame allocation:** replace the growing `localloc` byte vector with stable aligned per-frame
  blocks; checked size/alignment arithmetic and cleanup on all frame exits. Reuse the M3 block contract.
  Gates: p04/p05, repeated allocations, alignment, unwind and tail-call lifetime tests.
- **M1d, owners/guards:** replace naked-pointer escape from `pop_ptr`, `normalize`/`from_info_full`, and
  `resolve_address` with retained ownership or scoped access. Hold static/cross-arena stabilization
  through use. Unsupported ownership conversions must fail explicitly. Gates: pass-1 transient
  destruction probes, static access, and arena teardown/access races.
- **M1e, GC transport/publication:** carry opaque pointers or validated handles through recording,
  harvest, coordinator sets and mark commands. Numeric identity keys may coexist with retained pointers.
  Use root-barrier publication (`mutate_root` where required), and unregister/drain outstanding leases
  before freeing an arena. Gate transport without unrelated exposure, E11 plus a real VM cross-arena
  collection test, and teardown stress. Confirm the source-only teardown diagnosis before changing it.
- **M1f, immediate instruction correctness:** negative array indices produce managed exceptions;
  `cpblk`/`initblk` use their specified unsigned count width with checked range arithmetic. Repair
  consumed-operand yield/retry paths before claiming block-op correctness; see M5 for progress/GC tests.
- **M1g, identity encoding:** reproduce arena ID 40000, remove sign-extension failure, and define
  supported arena/address ranges in release builds. Test boundary IDs and reuse. If widening the codec
  requires M3, enforce the interim supported range rather than silently decoding a live object as null.

**Exit:** corresponding regression tests execute and pass in their designated lane. Owners survive actual
accesses; address formation alone never performs out-of-bounds in-bounds arithmetic. Stack/static exposure
and local assignment interleaving remain M3 work until their tests pass.

### M2 — Repair the native boundary

- `AddrOfPinnedObject` returns payload data for objects, arrays, strings and boxed values, derived from
  retained raw bases. Validate handle state and pin/root lifetime; add IL coverage beyond the Rust address test.
- Allocate libffi return storage to the ABI's required size and alignment, at least `ffi_arg` for widened
  scalar results. Decode according to signedness and return type. An unqualified “8-byte buffer” does
  not cover larger aggregates or every ABI. Test guard bytes around sub-word slots and aggregate results.
- Confirm and repair source-reviewed by-ref/out pointer-slot indirection, type-sized copy-in/out,
  typedref temporary relocation, null managed arguments, and string termination/encoding. Keep all
  argument and temporary storage alive and immobile for the call. Release resources on error paths.
- Exercise a small native fixture library for scalar returns, by-ref/out, null, string, boxed data and
  enough arguments to force temporary-container growth. Define how native access coordinates with
  guest threads and collection; reconcile that boundary with M4 before enabling shared raw access.

**Exit:** native fixture and guard-byte tests pass on supported CI ABIs; pinned-address tests access real
payloads without corrupting metadata. Native-only libffi evidence stays separate from Miri results.

### M3 — Migrate storage and pointer representation

- Introduce `GuestBlock` with allocation/deallocation, initialization, layout, tracing and raw-base
  invariants. Migrate heap payloads and statics without obtaining long-lived guest addresses from slices.
  Do not expose overlapping Rust payload references while raw guest accesses can occur.
- Move locals/arguments into stable segmented frame homes, preserving initialization/type layouts,
  tracing, byref-like values and transient ownership. Audit reserve/suspend/restore, calls, tail calls,
  exceptions and deep recursion. Remove stack relocation/fixup machinery only after all consumers migrate.
- Carry unmanaged pointers through `PointerOrigin`/`get_ptr_info` and access dispatch. Unify native-int
  conversion and storage acceptance. Finalize managed owner/origin metadata and codec representation
  together; preserve the current pointer-aware word and atomic storage behavior.
- Centralize explicit pointer-to-integer exposure and address-only reconstruction. Audit `conv.i`,
  `conv.u`, `conv.u8`, `AsPointer`, unmanaged serde, spans, and native imports. Raw pointers already
  available from FFI or VM storage should not be discarded merely to reconstruct them.
- Validate managed and unmanaged codecs using live allocations and dereference/mutation after decoding.
  Cover arithmetic and byte store/load round trips at the native boundary, partial overwrites, null,
  address/ID reuse and stale generations. Specify GC lifetime separately from integer value preservation.

**Exit:** p01/p02/p03/p09/p12/p15 and assignment-interleaving tests pass for stack, heap and static
locations; deep frame growth and suspension preserve earlier addresses. Strict-core pointer paths retain
provenance. No owner is lost during normalization, serialization or pointer extraction.

### M4 — Establish one host memory model

Write an access matrix for plain/volatile/Interlocked operations, unaligned and mixed-width accesses,
reference slots, block operations, unmanaged aliases, and FFI. Include allowed overlap and ordering.
Prototype before rollout: aligned natural-size Relaxed atomics can implement ordinary word accesses,
but E9c means this alone cannot implement arbitrary overlapping sizes safely. Choose a common
synchronization/granularity scheme or explicit mediation for unsupported overlaps. Any guest restriction
must be enforceable before host UB, documented as a compatibility limit, and tested.

Then route all relevant origins and intrinsics through that protocol. Preserve pointer provenance for
reference slots; do not replace pointer atomics with integer encodings. Implement `volatile.` ordering
and `Interlocked` ordering consistently, including statics; use wrapping arithmetic where required for
`Interlocked.Add`. Do not use an unaligned memcpy fallback for concurrent shared atomics.

**Exit:** native threaded stress and seeded Miri VM tests cover field vs field, Interlocked vs ordinary
store, fixed/unmanaged vs array access, block vs scalar, and mixed-width/unaligned overlaps. E9a/E9d
negative models still demonstrate why the old protocol was invalid; positive implementations are clean.
No unmediated mixed-size race is accepted as a completed soundness fix. Resolve the audit-only static
atomic and sync-block-creation race claims here.

### M5 — Integrate GC-aware stores and resumable bulk operations

- Centralize typed store/copy operations, applying owner recovery, `Gc::write` and cross-arena recording
  according to reference layout. Cover raw/unmanaged aliases as well as heap/static/frame destinations.
  Do not hide typed reference copying behind byte-only APIs that discard provenance.
- Migrate `cpblk`, `initblk`, `CopyBlock`, `Buffer.Memmove<T>`, `Array.Copy`, and `Fill`. Define overlap,
  alignment, count, reference-layout and prefix behavior per operation. Audit byte writes that overlap
  reference slots and explicitly define their supported behavior.
- Retain owners, guards, operands and progress across chunk safepoints. Choose a saved continuation
  for partial operations or a proven retry-before-side-effects strategy. Blind replay of a partially
  completed overlapping copy is not sufficient. Avoid retaining guards that prevent GC from progressing.
- Test forced yield/GC between chunks, exceptions/cancellation, concurrent access under M4, fresh-reference
  copies during non-MT incremental marking, and real cross-arena copies in multithreading builds.

**Exit:** results match uninterrupted execution, progress is preserved, reference targets remain reachable,
and both incremental and cross-arena collection tests pass without unrelated statics masking missing roots.

### M6 — Close the gates

- Make all repaired pointer/GC lanes blocking. Run strict managed-core TB suites and permissive whole-VM
  pointer suites separately, with default and `--no-default-features`/multithreading configurations as
  appropriate. Keep external dependency limitations visible; narrow remaining exceptions explicitly.
- Run supplemental Stacked Borrows probes on previously divergent stack cases. Use multiple seeds for
  alignment/concurrency and fuzz lifecycle/codec/arithmetic/copy sequences. A finite green corpus is evidence,
  not proof of universal soundness.
- Re-run end-to-end benchmarks against M0 with consistent profiles and noise controls. Target no more
  than ~3% regression on memory/unsafe_buffer/span/dispatch; investigate deltas before accepting them.
  Investigate shared layout metadata in `get_layout_from_owner` to avoid deep `HashMap` clones; pass 3's
  microbench numbers guide profiling and are not a promise of final throughput.
- Update architecture, GC/memory safety, threading, invariant registry, compatibility, validation and CI
  documentation. Remove obsolete rebase guarantees and reconcile any remaining ignored-test reasons.
- Confirm `refanytype`'s reported dangling `Arc::as_ptr` and repair ownership if reproduced. Close every
  source/audit-only item with a regression, a documented refutation, or an explicit bounded compatibility
  decision; do not silently drop findings because the initial probe suite is green.

**Exit:** the coverage inventory is resolved, required CI lanes pass, native ABI and threaded tests pass,
performance is reviewed, and documented limits match enforced behavior. Optional checked-address mode
may follow; it is not a prerequisite for closing valid native-pointer support.

## Finding-to-work coverage

Evidence: **VM-M** = real VM Miri, **LIB-M** = library Miri, **N** = native reproduction,
**MODEL** = standalone lab, **R/A** = source/audit claim awaiting implementation-level confirmation.

| Finding group | Evidence in reviews | Work and decisive regression |
|---|---|---|
| Oversized heap slice; shared-derived writable address; safe OOB formation | Pass 1 LIB-M; pass 3 VM-M p10/p13/p14/p16, 3c2 controls | M1a/M3: one-byte real read, p20, writes and checked access |
| Slot assignment invalidates byref; stack growth invalidates native int | Pass 1 LIB-M; pass 3 VM-M/N p09, MODEL E12b/E13 | M3: assign home then dereference, recursion/suspend/restore |
| Transient destruction; expired static/arena guards | Pass 1 LIB-M for pop/normalize; R for resolver guards | M1d/M3: access after pop/codec and concurrent teardown |
| Unmanaged serde loses exposure; integer GC transport loses provenance | Pass 1 LIB-M; pass 3 p05 refutes unconditional failure | M1e/M3: decode and dereference without unrelated exposure; strict GC transport |
| Repeated localloc; unsafe arithmetic; null/wrapping and comparison gaps | Pass 2 VM-M/N; pass 3 p04/p06/p08/p17/p18 | M1b/M1c: p04–p08, comparisons, zero result |
| Pinned payload address; FFI return overwrite | Pass 2 R; pass 3 Rust address test/N libffi | M2: IL pinned cases and native fixture guard bytes |
| By-ref/out, temp growth, null and string ABI | Pass 3 R | M2: native type-sized copy and lifetime fixtures |
| Cross-arena root barrier/teardown; arena/address packing | Pass 3 MODEL E11/R; pass 1 LIB-M ID 40000 | M1e/M1g/M5: actual VM GC, lease/ID reuse, codec boundaries |
| Host races/alignment; mixed-size overlaps; static atomics | Pass 3 MODEL E9/E10; R/A for VM paths | M3/M4: seeded VM races and layout alignment |
| Block barriers, count width and yield state | Passes 1–3 R | M1f/M5: forced chunk yield/GC and fresh-reference copies |
| Negative index; ByteOffset sign; conv.i/native-int stores | Pass 3 N p19 and R; pass 1 low-level store panic | M1b/M1f: managed exception, signed offset and storage round trips |
| Volatile ignored; Interlocked overflow; sync-block race; refanytype lifetime | Pass 3 R/A | M4/M6: ordering/overflow/concurrent identity and handle lifetime tests |
| Atomic tests' backing aliasing; strict dependency gaps; missing blocking VM coverage | Pass 1 LIB-M; pass 3 CI review | M0/M6: repaired test storage and explicit CI lane inventory |

## Execution and reporting

Implement M0 first, then small M1/M2 changes with their own reproducer and passing controls. Each change
must state which access paths it repairs and which migration obligations remain. Re-run only the relevant
tests during iteration; reserve the full matrix and benchmarks for milestones and final validation.

Use the pinned `nightly-2026-05-27` and recorded flags initially for comparable evidence; changing
Miri versions requires recording both versions' outcomes. VM Miri builds/probes are expensive (the
historical estimate is roughly ten minutes to build and four per IL probe, not a fresh measurement);
shard passing suites and run known failures individually. Native libffi tests require their own runner.
The 23 current VM probes are wired, but the missing pass-1 library and standalone lab/FFI sources are
not; recover or reproduce those separately before claiming their coverage.

Track milestone progress here and update the [queue](README.md) when the overall status changes.
Completion requires passing accesses after state changes, not address-only equality or metadata checks.
