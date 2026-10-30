# Unified pointer/provenance project plan

**Status:** In progress — scoped M1b pointer semantics, M1c stable localloc storage and M1d owner/guard work complete; partial M0 (VM probe harness and controls)
**Prepared:** 2026-10-29
**Baseline:** `b49f5df8` (also HEAD when this plan was prepared)
**Scope:** Guest storage, pointer operations, GC transport/barriers, concurrency, and native-call boundaries.

Deliver a pointer model in which supported guest operations preserve host allocation provenance,
retain the required owners, and obey one host memory-access protocol. Valid IL must survive local
assignment, frame growth, repeated `localloc`, collection, and suspension without host UB or host panics.
Native-integer conversions remain available through an explicit exposed-provenance boundary.

This is the implementation plan derived from all three [reviews](../pointer-provenance/README.md).
The underlying historical pass archives and raw logs are not present in this checkout; current evidence
limits and source-recovery tasks are recorded in the [inventory](../pointer-provenance/INVENTORY.md).
The scoped M1a heap-access repairs, M1b pointer-semantics work, M1c stable frame-local `localloc`
storage and M1d owner/guard work have been implemented and recorded, but the broader M0 evidence
inventory and remaining M1/M2 work remain open. No broader milestone exit is claimed. Milestone IDs below are the work-item IDs; split
them into focused changes while retaining their acceptance criteria. Ownership is assigned by subsystem,
not by an assumed individual or schedule.

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

**Current progress — partial M0:** The durable [inventory](../pointer-provenance/INVENTORY.md)
maps R01–R13 and the harness catalogs 39 VM entries. The guarded lane has 33 exact blocking controls in both
no-features and multithreading configurations across shards 1–17; all passed the M1d final sweep.
The other 6 catalogued probes remain ignored and diagnostic-only. Shards 1–16 each contain two
controls; shard 17 contains p36. Shard 16 pairs p34 with the M1d p35 control.
These are filtered permissive Tree Borrows results, not a full VM Miri suite, strict-provenance
evidence, or a claim of runtime soundness.

- Keep the pass-3 IL p00–p20 probes and the two unique pass-2 cases (null through `&p`, wide stores)
  in the owning VM suite, retaining the mock corlib primitive typedefs. Recover the original pass-1
  library probe sources where possible; the independent M1a recreations do not establish source
  identity. Keep read/control tests alongside writes and dereference after mutation or decoding.
- Port lab E1–E13 and `gc_model::e11*` with clear labels for model tests. Repair the backing-storage
  aliasing in the two ignored atomic tests; distinguish that UB from their historical Miri ICE reasons.
- Preserve the existing per-probe classification; classify recovered library/model/FFI evidence and
  future regressions as native, permissive TB, strict-core TB, supplemental Stacked Borrows, or native
  FFI as appropriate. Tests that deliberately exhibit UB stay ignored with a finding/work-item ID
  until fixed; do not execute UB as ordinary native tests. Keep known-failure reproductions in the
  explicit diagnostic lane and run failing Miri reproducers separately.
- The 33 passing pointer controls have a blocking lane, but they do not close M0. Add permanent
  gates for recovered evidence and expand VM coverage beyond `jmp_tests`, `tail_calls`, and
  `fault_tests`. Recover exact strict filters and reasons where possible; current dependency traces and
  historical filters are unavailable, and exposed reconstruction remains an explicit strict-provenance
  boundary.
- Inventory the 73 reported provenance-lint hits before denying `fuzzy_provenance_casts` and
  `lossy_provenance_casts` in first-party code. Use explicit exposure/reconstruction helpers; record any
  narrow exceptions. Do not deny everything before migration and call unrelated dependency failures fixes.
- Review native benchmark comparisons using the [benchmark workflow](../BENCHMARK_WORKFLOW.md),
  including memory/unsafe_buffer/span/dispatch and field/byref/native-int loops. The M1a
  `m1a-before` comparisons are recorded; repeated no-features regressions remain open for follow-up.

**Exit:** every finding in the coverage table below has a permanent test or a specific reproduction task;
all current passing controls run in CI. Evidence distinguishes VM tests, models, source claims, and
dependency blockers. No green filtered lane is described as a green full strict suite.

#### Recorded follow-ups (not completed)

The [evidence inventory](../pointer-provenance/INVENTORY.md) maps R01–R13 to exact probes,
evidence limits, owners and reproduction tasks. The historical pass-1/2/3 archive directories,
raw transcripts, most original library/model/FFI sources and patch sources are absent from this tree;
their external recovery location is unknown. Two M1a library regressions independently recreate the missing test intents, and the p20 baseline
was recorded, but neither recovers the missing historical source or archive. The
33 filtered permissive-TB controls do not establish full M0, strict Miri or runtime soundness.

- Recover and integrate the remaining pass-1 library probes, standalone E1–E13 models and
  `gc_model::e11*`, preserving the distinction between model, library and real-VM evidence. Recover
  the missing address and libffi fixtures; retain the recorded p20 result as current-tree evidence.
- Identify the two historically reported atomic tests, distinguish test-backing aliasing from their
  historical Miri ICE reasons, and repair and validate them (M0/M6); their identities remain unknown.
- Inventory and migrate the reported 73 provenance-lint hits before denying `fuzzy_provenance_casts`
  and `lossy_provenance_casts`; document narrow exceptions (M0). The count is not a fresh audit.
- Review the recorded native benchmark comparisons for memory/unsafe_buffer/span/dispatch and
  field/byref/native-int paths (M0). Matching M1a comparisons are recorded in the
  [benchmark report](../BENCHMARK_WORKFLOW.md); the repeated no-features regressions still need
  follow-up.
- Recover or re-establish strict-core evidence and exact selectors after diagnosing dependency
  blockers; the historical filters and first-failure dependency traces are unavailable. Separately
  scope unrelated VM Miri expansion (M0/M6). Whole-VM strict Miri is not a claim of this harness.
  Observe hosted CI cold/cache timings before changing the provisional 45-minute control timeout.
- Complete the M1–M6 pointer, storage, GC, ABI and concurrency repairs and their existing exits; the
  scoped M1a heap repair does not itself complete any broader milestone exit.

### M1 — Contain independently reproducible defects

- **M1a, heap access:** scoped heap repairs now use actual payload lengths for `with_data`, separate
  heap address observation from reads, derive observed addresses from retained allocation bases with
  wrapping arithmetic, and expose pointer provenance at integer-conversion boundaries. `conv.u` and
  `conv.u8`, the unsafe pointer helpers, and the shared `StackValue::as_ptr`/VM address paths were
  separately covered. The historical 3c2 patch source is absent; these changes were independently
  validated and do not claim identity with that patch. See the coverage table below. The wider M0/M1
  exits and the follow-on obligations listed there remain open.
  Gates: p10/p13/p14/p16/p20 plus independently recreated pass-1 one-byte heap read and safe-OOB
  observation tests.

#### M1a scoped status and coverage (2026-10-28)

The heap-access portion of M1a is implemented and has passed the following pinned
`nightly-2026-05-27`, permissive Tree Borrows checks in both no-features and multithreading
configurations. This status does not close M0 or the broader M1 milestone.

| Coverage | Current result | Boundary |
|---|---|---|
| `dotnet-value` regressions `pointer::tests::m1a_one_byte_heap_read`, `m1a_safe_oob_address_observation`, and `m1a_heap_pointer_survives_typed_write` | Each exact test passed in both configurations (1 passed, 0 failed, 0 ignored per run). | The first two independently recreate missing test intents; the third is a new interleaving test. Original pass-1 sources are absent, so no identity claim. |
| VM heap probes p10, p13, p14, p16, p20 | All five are promoted controls and passed individually in both configurations. | Original IL and assertions are unchanged. p13's `memory-validation` native bounds trap remains an intentional conditional ignore; the separate far-OOB `conv.u8` regression is conditionally ignored for the same reason. |
| Blocking VM pointer-control lane | At the M1a snapshot, 11 exact controls passed in each configuration across shards 1–6. At M1b completion, the lane had 24 controls across shards 1–12 (31 catalog entries, 7 diagnostic-only). At M1c completion, the lane had 31 exact controls across shards 1–16 and 37 entries (6 diagnostic-only). The current M1d lane has 33 controls across 17 shards and 39 entries (6 diagnostic-only). | Filtered permissive Tree Borrows only; see the [M1b coverage table](#m1b-status-and-coverage-2026-10-29), [M1c status and coverage](#m1c-status-and-coverage-2026-10-29), and [CI shard layout](../CI.md). |
| Heap access paths | Exact payload-extent reads; slice-free heap observation in `PointerLike`, `StackValue::as_ptr`, VM address resolution, `conv.u`/`conv.u8`, Unsafe helpers, buffer and P/Invoke heap branches; retained-base typed-write interleaving covered. | Observation does not retain an owner, root, or guard; strings have no new writable contract. |

Remaining obligations are explicit and are not implied complete by this table:

- **M3:** stack/static storage migration and the legacy `ManagedPtr::with_data` stack fallback;
  pointer-codec and Rust struct-layout redesign.
- **M4:** shared-memory/atomic access protocol, including synchronization for escaped raw pointers.
- **M1f/M5:** block-operation correctness: unsigned count width, checked ranges, reference barriers,
  and yield/resumption state.
- **M1b:** The scoped pointer arithmetic, comparison, `Unsafe.ByteOffset`, conversion and native-integer
  store/load work is implemented; see [M1b status and coverage](#m1b-status-and-coverage-2026-10-29).
  This does not close the remaining storage, ownership, representation, ABI, access-protocol or strict-
  provenance obligations listed in that status and below.
- **M1d:** scoped owner/guard work is complete; see [M1d status and coverage](#m1d-status-and-coverage-2026-10-30). Ownership alone does not satisfy the remaining GC, access-protocol, ABI or codec obligations.
- **M1e/M1g:** GC transport/lease behavior and arena/address identity encoding.
- **M0/M6:** provenance-lint migration and recovery of strict-provenance evidence. Permissive Tree
  Borrows results do not satisfy this obligation.

#### M1b status and coverage (2026-10-29)

The scoped M1b pointer-arithmetic, comparison, and native-integer changes are implemented. The
following results distinguish native tests, the filtered permissive Tree Borrows lane, native
diagnostics, and source/audit claims; Phase 6 control outcomes are recorded in detail in the
[inventory](../pointer-provenance/INVENTORY.md#m1b-phase-6-control-outcome-comparison-2026-10-29).

| Coverage | Current result | Boundary |
|---|---|---|
| Eight `dotnet-value` `pointer::tests::m1b_` regressions | Native tests and pinned permissive-TB Miri each reported 8 passed, 0 failed, 0 ignored in both no-features and multithreading configurations. | With `memory-validation`, 7 passed and 1 intentionally ignored by the F-TEST-003 guard for deliberate heap-owner out-of-bounds arithmetic. These results are not strict-provenance evidence. |
| Blocking VM pointer-control lane | At M1b completion, 24 exact non-ignored controls had passed across shards 1–12 in both configurations; `verify-controls` found 24 controls and the catalog had 31 entries (7 diagnostic-only). At M1c completion, it had 31 controls across shards 1–16 and 37 entries (6 diagnostic-only). The current M1d lane has 33 controls across 17 shards and 39 entries (6 diagnostic-only). | The M1d final sweep passed all shards in both configurations. Discovery is metadata, not execution; see [M1d status](#m1d-status-and-coverage-2026-10-30) for limits. |
| Address arithmetic, comparisons and `Unsafe.ByteOffset` | p06/p08, p17/p18, p21–p24 and `p_null_through_pointer_local_address` passed as controls in both configurations. | Address formation on these covered paths is wrapping, not allocation-bounded in-bounds arithmetic; observation alone does not grant dereference permission. Stable frame-local `localloc` storage and its tested lifecycle paths are covered by M1c; stack homes and broader static storage remain M3 work (see [M1c status and coverage](#m1c-status-and-coverage-2026-10-29)). |
| Pointer conversion and native-integer round trips | p25–p28 passed as controls in both configurations; the assertions include store/load round trips followed by real dereferences. | This covers only the asserted `conv.i`/`conv.i8`, `stind.i` and native-width cases; it does not prove every conversion opcode or ABI boundary. |
| Fresh-pointer `stind.i` controls p27 and p28 (shard 12) | Under `bash scripts/pointer_provenance.sh controls --configuration CFG --shard 12`, both passed: no-features p27 296 s / p28 302 s; multithreading p27 348 s / p28 321 s; each configuration totaled 2 passed, 0 failed. | Filtered permissive-TB Miri evidence; p27 tests the local-slot path and p28 the heap-memory path. The measurements are per-probe runner wall seconds, not native timings. |
| Native diagnostics and source/audit | The p17/p18 pre-repair native assertion failures are historical diagnostics; no new native diagnostic result is claimed here. Source review confirms `managed_ptr_to_raw` uses the M1a retained-base observer and the covered address-formation paths no longer use in-bounds arithmetic. | Native diagnostics, where permitted, are distinct from Miri. The current documentation does not claim a native diagnostic pass for the M1b control lane. |

A final audit also repaired arithmetic from a canonical unmanaged-origin managed null: nonzero results reconstruct through exposed provenance, while zero stays null. Managed-offset calculations now widen the compact unsigned offset before signed addition, avoiding a 32-bit sign flip. The extended value regression passed natively and under pinned permissive Tree Borrows in both feature configurations on x86_64, including an exposed-address dereference. After these repairs, guarded VM shard 5 (p13/p14) also passed in both configurations; the other shards were not rerun. No 32-bit target was run, and strict-provenance evidence remains open.

Remaining M1b boundaries and related obligations are explicit:

- **M1c:** scoped stable frame-local `localloc` storage is implemented and covered by the blocking p04 and p29–p34 controls; see [M1c status and coverage](#m1c-status-and-coverage-2026-10-29). This does not close p09/stack-home lifetime or broader static-storage/address work (M3).
- **M1d:** scoped owner/guard work is complete; see [M1d status and coverage](#m1d-status-and-coverage-2026-10-30) for remaining boundaries.
- **M1f/M5:** block operations (`cpblk`/`initblk`, count width, ranges, barriers and yield state) and the negative array-index managed-exception case.
- **M3:** signed/wider managed offsets and final pointer structs/codecs. Negative or beyond-compact managed offsets currently return an interim `NotImplemented` error; the final representation is not implemented.
- **M4:** shared host memory-access protocol; **M2:** native ABI and pinned-address boundary.
- **M0/M6:** strict-provenance evidence and broader strict-core/Miri work remain outstanding. Filtered permissive Tree Borrows is not strict-provenance evidence, not a full VM Miri suite, and not a soundness proof.
- Pointer cases left unchanged include the remaining narrow/overflow-checking `conv.*` forms (notably managed-pointer narrow conversions and `conv.ovf.*` pointer conversions); no general conversion claim is made.
- **PERF:** repeated M1a no-features regressions remain an open, separately tracked performance item; see the [native benchmark report](../BENCHMARK_WORKFLOW.md#m1a-heap-access-comparison-2026-10-28). Native Criterion measurements are not Miri evidence.
- **M1c, frame allocation:** scoped stable frame-local `localloc` storage is implemented: each request
  owns a zeroed, aligned, non-resizing block, with checked sizing and cleanup on frame lifecycle paths.
  The p04/p05, repeated-allocation/alignment, nested-call, return, unwind, tail-call and teardown
  regressions passed and are recorded in the M1c table. This reuses the M3 block contract but does not
  migrate stack homes, statics or heap storage.
- **M1d, owners/guards:** scoped work complete: `RetainedPtr` retains popped operands, Transient
  origins share their owner and survive normalization/reconstruction, and `resolve_address` borrows its
  origin. Cross-arena accesses use scoped leases; covered block operations retain leases through
  synchronous access. Unsupported Transient native-integer conversion and fallible memory stores are
  explicitly rejected. Independent owner/access regressions, p35/p36 and controlled teardown results
  are recorded below; the original pass-1 probes were not recovered. This does not complete M0/M1
  or the remaining GC, synchronization, static/stack, codec and native-call obligations.
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

#### M1c status and coverage (2026-10-29)

The scoped M1c work replaces the growing byte vector with stable, zeroed, 16-byte-aligned
frame-owned blocks, and releases them on the tested frame-lifecycle paths. The table records the
per-probe runner walls for the baseline and first passing runs in the pinned `nightly-2026-05-27`
permissive Tree Borrows lane (`-Zmiri-ignore-leaks`); the p29–p34 rows are the first diagnostic
passes, and these probes were subsequently promoted and passed in blocking shards 13–16 in both
configurations. These results are not strict-provenance, leak-check, full-VM Miri, or soundness evidence.

| Coverage | no-features | multithreading | Boundary |
|---|---|---|---|
| p04 pre-repair baseline | `miri-ub`, dangling pointer without provenance at `access.rs:1172:25` (239 s) | Same UB/location (224 s) | Observed before the stable-block repair. |
| p04 after repair | Pass, 1 passed / 0 failed / 0 ignored (255 s) | Pass, 1 passed / 0 failed / 0 ignored (240 s) | The repeated-allocation write-first regression now passes in both configurations. |
| p05 single-allocation control | Pass in shard 2 (235 s); test body unchanged | Pass in shard 2 (231 s); test body unchanged | Retained as the single-allocation control. |
| p29 stable blocks across growth and 16-byte alignment | Pass (341 s) | Pass (330 s) | Reads earlier allocations after later requests; includes a zero-size request. |
| p30 block survives nested callee allocation | Pass (329 s) | Pass (292 s) | Caller allocation remains readable after the callee's `localloc`. |
| p31 release on return and pooling | Pass (365 s) | Pass (315 s) | Live-block gauge and pooled frame state verify cleanup without expired-pointer dereference. |
| p32 release on tail-call frame replacement | Pass (387 s) | Pass (314 s) | Caller block is released before the replacement callee allocates. |
| p33 release on exception unwind | Pass (357 s) | Pass (314 s) | Unwound frames release blocks before the catch handler observes state. |
| p34 release on executor teardown | Pass (283 s) | Pass (248 s) | Live blocks return to baseline after dropping the executor arena. |
| `raw_block::` library tests | 7 passed / 0 failed / 0 ignored (1.413 s) | 7 passed / 0 failed / 0 ignored (1.475 s) | Allocation layout, alignment, initialization, bounds, failures and release gauge. |
| `frame_alloc::` library tests | Initial 7 passed (0.958 s); finalizer 8 passed / 0 failed / 0 ignored (1.105 s) | Initial 7 passed (0.969 s); finalizer 8 passed / 0 failed / 0 ignored (1.036 s) | Request/frame/thread budgets, allocation failure, accounting cleanup, and frame suspension/restoration. Initial walls include the log/count pipeline; finalizer walls are direct commands. |
| Remaining storage and ownership obligations | — | — | M1c does not close p09 or addressable stack homes, static alignment or heap/static `GuestBlock` migration (M3). M1d subsequently completed its scoped owner/guard work; pointer representation/codecs, other lifetime obligations and strict-provenance evidence remain open. |

At M1c completion, final local verification discovered and passed all 31 guarded controls in each configuration across
16 shards. Four sequential batches of four parallel shards took 4222.037 s (no-features) and
3871.943 s (multithreading), excluding standalone discovery; slowest shard walls were 1103.821 s
and 1011.151 s. See the [inventory](../pointer-provenance/INVENTORY.md#m1c-final-verification) for
per-shard results and timing qualifications. `./check.sh --fast` passed in 88.442 s; nextest reported
554 passed / 9 skipped, 625 / 7, and 639 / 9 for no features, multithreading, and
multithreading,validation-all respectively. Hosted CI timing remains unmeasured.

**Exit:** corresponding regression tests execute and pass in their designated lane. Owners survive actual
accesses. On covered M1a/M1b paths, address formation uses wrapping operations rather than
allocation-bounded in-bounds pointer arithmetic; formation alone does not authorize dereference. Scoped
stable frame-local `localloc` storage passes the M1c regressions and blocking controls. Stack-home and
local-assignment interleaving (including p09), static exposure/storage (including p12), and the
remaining ownership, GC and access-protocol obligations remain open; this does not close the broader
milestone exits.

#### M1d status and coverage (2026-10-30)

The scoped M1d repairs are complete, not the broader M1 exit. The following are **measured**
pre-repair and post-repair results from independent recreations; the missing pass-1 sources were not
recovered, so no test-identity claim is made. Miri runs use `nightly-2026-05-27` with
`-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks`, in no-features and
multithreading unless noted. Commands, diagnostics and per-shard timings are recorded in the
[durable inventory](../pointer-provenance/INVENTORY.md#m1d-baseline-and-outcomes). Native and Miri outcomes are distinct; permissive Tree
Borrows is neither strict-provenance evidence nor a soundness proof.

| Coverage | Current result | Boundary |
|---|---|---|
| Normalize, from-info and stack-pop owner/access regressions | Before repair, native liveness assertions failed in both configurations (owner count 1 instead of 2); pinned Miri found dangling reads after normalization/pop and a dangling reference after from-info. After repair, the corresponding real-access tests passed native and Miri in both configurations. | The pop tests now use `pop_retained_ptr`. Transient origins share the same owner and survive normalize/from-info; this does not prove a root, range, synchronization or arbitrary codec round trip. |
| Transient clone and ownership conversion | Pre-repair native clone identity assertions failed in both configurations; the original and clone had different owner-storage addresses. Pre-repair fallible store accepted undecodable metadata, and address observation returned a Transient address. Repaired clone, store-rejection and native-integer-rejection regressions passed native and pinned Miri in both configurations. | The fallible memory store rejects Transient before writing; `conv.i/u/i8/u8`, `Unsafe.AsPointer`, and CLI-to-CTS numeric narrowing reject its ownership escape. Infallible codecs and final representation remain M3; not every byte copy is covered. |
| Library blocking lanes | `dotnet-value` `m1d_` after the CLI-to-CTS audit repair: 11 passed, 0 ignored without features; 14 passed, 1 ignored historical UB diagnostic with multithreading. `dotnet-vm-data` `stack::tests::m1d_`: 8 passed, 0 ignored in each configuration. Before the audit regression was added, the full default `dotnet-value` Miri suite reported 73 unit tests passed and 3 ignored, plus 1 integration test passed. | Cross-arena tests exist only with multithreading; the historical Fill reproducer stays ignored and must not run natively. Stack-level P/Invoke handoff models are not actual libffi calls. |
| p35 `ldflda` on a stack value | Before repair, both Miri configurations produced `System.NullReferenceException` type-resolution failure, not Miri UB; after repair, both passed the diagnostic and then the blocking control. | The exact pre-repair origin/throw site was not established. The passing probe reads a snapshot, not stack-home write-through; p09 and addressable homes remain M3. |
| Static p36 and p12 | p36's one-byte static write/read after forced collection passed pinned Miri in both configurations and is blocking. The earlier p12 diagnostic hit a four-byte alignment `debug_assert` panic in both configurations, not Miri UB. | p36 refutes the proposed allocation-lifetime failure only for that controlled collection and direct static path; p12 is not a teardown proof. Static alignment/`GuestBlock` and legacy Static access remain M3; synchronization remains M4. |
| Cross-arena teardown and synchronous operations | Pre-repair multithreading Miri reported use-after-free accessing the freed owner lock; post-repair teardown rejection and the in-flight lease control passed Miri. `RetainedPtr` lease tests passed multithreading Miri, including with `memory-validation`. | Fresh accesses fail after unregister begins; in-flight leases drain before teardown completes. Leases do not root objects, prove ranges or synchronize payloads, and must not span scheduler parking. Arena-ID reuse remains M1g. No-features has no CrossArena variant. |
| Fill fast-path hazard | A pre-repair-pattern diagnostic measured a dangling write under multithreading Miri; the production byte fast path now retains a lease through synchronous fill. | IL reachability is source-inferred, not VM-confirmed; no end-to-end Miri fixture exercises the fixed Fill path. Yield resumption remains M1f/M5. |
| VM control sweep | All 33 exact controls passed in both configurations across 17 shards under pinned permissive Miri; 39 catalog entries include 6 diagnostic-only probes. Shard 16 contains p34/p35 and shard 17 p36. | Filtered controls are not the full VM suite; discovery alone is not execution. Existing 31 controls remain included. |

**Remaining obligations, not claimed fixed:**

- **M3:** p09 stack-home relocation and write-through (Transient is a snapshot), final pointer
  codecs and struct layout, legacy Stack/Static access and static `GuestBlock` alignment (p12).
- **M1e/M1g:** GC transport, root publication and teardown ordering; arena-ID/generation reuse and
  identity encoding. Per-access leases are not a generation or GC-root proof.
- **M4:** a shared-memory/atomic protocol, including raw pointers escaping scoped access; leases are
  allocation-lifetime protection, not synchronization.
- **F-LIFE-004:** nested-filter suspended-frame overwrite remains source-only, unreproduced and open.
- **M2:** P/Invoke pin/guard lifetime, CrossArena native access, callbacks and ABI remain open; the
  Transient clone tests model the handoff but do not exercise libffi.
- **M1f/M5:** `cpblk`/`initblk`/`Memmove` and Fill resumability after `Yield`, count/range and barrier
  correctness remain open.
- **M0/M6:** strict-provenance evidence and wider Miri coverage remain open; no strict lane is
  claimed by the M1d results.

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
| Transient destruction; expired static/arena guards | Historical pass-1 LIB-M reports; original sources absent. Independent M1d library recreations measured pre-repair UAFs and now pass; controlled cross-arena teardown was confirmed, while p36 refuted the proposed static allocation-lifetime failure for its one-byte scenario. p35 pre-repair reported an exception-resolution failure, not Miri UB. | **Scoped M1d owner/guard repairs complete**, including real access after owner/info consumption and per-access leases; see [coverage](#m1d-status-and-coverage-2026-10-30). M3 codecs, stack homes and static alignment; M1e/M1g GC/identity; M4 synchronization and M2 native guards remain open. |
| Unmanaged serde loses exposure; integer GC transport loses provenance | Pass 1 LIB-M; pass 3 p05 refutes unconditional failure | M1e/M3: decode and dereference without unrelated exposure; strict GC transport |
| Repeated localloc; unsafe arithmetic; null/wrapping and comparison gaps | Phase 6 controls p06, p08, p17, p18, p21–p24 and p_null passed in both configurations; p07 remains a passing control. The pre-repair p06/p08 diagnostics were VM-M `miri-ub`, and p17/p18 had native/VM-M assertion failures. M1c controls p04 and p29–p34 passed in both configurations. | **M1b covered arithmetic/comparison/null paths closed** for wrapping, signed extremes, wrap-to-zero, comparisons, subtraction and dereference. **M1c scoped stable frame-local `localloc` storage and tested lifecycle paths closed**; see the M1c table. **M3 remains open** for p09/stack homes, statics (including p12), signed/wider managed offsets, heap/static storage migration and final pointer structs/codecs; M1d scoped owner/guard work is complete, with separate boundaries above. Unrepresentable negative or beyond-compact managed offsets return interim `NotImplemented`. |
| Pinned payload address; FFI return overwrite | Pass 2 R; pass 3 Rust address test/N libffi | M2: IL pinned cases and native fixture guard bytes |
| By-ref/out, temp growth, null and string ABI | Pass 3 R | M2: native type-sized copy and lifetime fixtures |
| Cross-arena root barrier/teardown; arena/address packing | Pass 3 MODEL E11/R; pass 1 LIB-M ID 40000 | M1e/M1g/M5: actual VM GC, lease/ID reuse, codec boundaries |
| Host races/alignment; mixed-size overlaps; static atomics | Pass 3 MODEL E9/E10; R/A for VM paths | M3/M4: seeded VM races and layout alignment |
| Block barriers, count width and yield state | Passes 1–3 R | M1f/M5: forced chunk yield/GC and fresh-reference copies |
| Negative index; ByteOffset sign; conv.i/native-int stores | Phase 6 controls p24–p28 and p_null passed in both configurations; p26 covers native-width sign/zero extension, and p25–p28 include pointer/store-load paths followed by dereferences. p19 remains a reported native panic; `p_wide_store_into_int_local` remains unverified. | **M1b ByteOffset direction, asserted pointer conversions/native-int stores, and native-integer round trips closed** for the covered paths. **M1f/M5 remain open** for the negative-index managed exception and block operations. **M3 remains open** for signed/wider managed offsets and final structs/codecs. |
| Volatile ignored; Interlocked overflow; sync-block race; refanytype lifetime | Pass 3 R/A | M4/M6: ordering/overflow/concurrent identity and handle lifetime tests |
| Atomic tests' backing aliasing; strict dependency gaps; missing blocking VM coverage | Pass 1 LIB-M; pass 3 CI review | M0/M6: repaired test storage and explicit CI lane inventory |

## Execution and reporting

Implement M0 first, then small M1/M2 changes with their own reproducer and passing controls. Each change
must state which access paths it repairs and which migration obligations remain. Re-run only the relevant
tests during iteration; reserve the full matrix and benchmarks for milestones and final validation.

Use the pinned `nightly-2026-05-27` and recorded flags initially for comparable evidence; changing
Miri versions requires recording both versions' outcomes. The blocking pointer lane currently runs
33 exact controls in each of two configurations across shards 1–17; the per-shard test names are in
[CI](../CI.md). The six remaining VM catalog entries are diagnostic-only; run known failures
individually, and treat setup failures/timeouts separately from UB. The three M1a library regressions
run in the blocking `miri-value` job's default (multithreading) suite and its separate no-features
selector. These are filtered permissive Tree Borrows lanes: they are not a full VM Miri suite,
strict-provenance evidence, or proof of soundness. Miri reports warnings around exposed-provenance
integer reconstruction; they do not make this lane strict evidence. Native libffi tests require their
own runner. The original pass-1 library sources and pass-1/2/3 archives remain unavailable; the three
M1a library tests are independent recreations, while standalone lab/FFI sources still need recovery or
reproduction before broader coverage is claimed.

The native benchmark review compares matching `m1a-before` and `m1a-before-nofeatures` Criterion
baselines at `bench-fat`, 10 samples, on the same Ryzen 9 9900X / `powersave` host. Default-feature
end-to-end results were neutral or faster for the reviewed hot cases; repeated no-features regressions
remain an open performance concern, with intervals and caveats in the
[benchmark report](../BENCHMARK_WORKFLOW.md#m1a-heap-access-comparison-2026-10-28). These native
measurements are not Miri evidence.
The finished-tree M1b comparison was not measured because the host was not idle; see the [M1b PERF report](../BENCHMARK_WORKFLOW.md#m1b-pointer-semantics-comparison-2026-10-29).

Track milestone progress here and update the [queue](README.md) when the overall status changes.
Completion requires passing accesses after state changes, not address-only equality or metadata checks.
