# Pointer/provenance runtime work

**Status:** In progress — scoped repairs retained; M1e/M1g acceptance open
**Revised:** 2026-11-02
**Original baseline:** `b49f5df8`
**Scope:** Stable guest addresses, retained ownership, GC publication, coherent shared access,
and the enabled native-call boundary.

The outcome is that supported guest operations survive assignment, frame growth, allocation,
collection and suspension without host undefined behavior. Preserve guest native-integer
semantics through an explicit exposed-provenance boundary. A universal strict-provenance VM
or recovery of every historical experiment is not a prerequisite for a useful repair.

[The queue](README.md) sets current priority: after the scoped Executor ownership repair,
stable byref homes and their shared access paths are next. The M identifiers below remain finding
categories and historical references; they are not a mandate for separate agent sessions.
[The inventory](../pointer-provenance/INVENTORY.md) owns detailed measurements and R01–R13 evidence.

## Evidence and reconciled decisions

Retain the implemented M1a heap access, M1b arithmetic/conversion, M1c localloc storage and
M1d owner/guard repairs. Their regressions address actual memory or guest-semantic defects.
M1e/M1g retained transport, root publication and identity work has advanced, but R08 acceptance
is open. Do not equate a merged branch with a completed safety contract.

Original pass-1/2/3 archives, many model/FFI sources and the 3c2 patch are absent. Independent
recreations are evidence of their own behavior, not recovered originals. Prefer a fresh labelled
regression when historical identity is unnecessary; keep the missing evidence explicit.

| Concern | Required boundary |
|---|---|
| Host memory safety | Live owned storage, checked extents, alignment, initialization, aliasing and synchronization at every access |
| Guest semantics | Correct arithmetic/null/conversion behavior, writable local/argument homes, exceptions and copy results |
| GC | Provenance, roots, pinning, generation leases and publication barriers are separate obligations |
| Concurrency | All competing scalar, atomic, block and raw-alias accesses participate in one protocol |
| Native interop | ABI-sized/aligned storage, pin/root lifetime, retained temporaries and explicit shared-access rules |
| Strict provenance | Preserve allocation-derived pointers in managed/GC paths; isolate intentional integer/foreign reconstruction |

The filtered Tree Borrows lane permits exposed reconstruction and ignores leaks. It is bounded
dynamic evidence, not strict-provenance validation, leak-freedom or a soundness proof.

## Target contract

- Stable, aligned, non-resizing storage owns its allocation and records payload extent separately
  from layout and accounting. Reuse the localloc `RawBlock` contract where applicable; it does
  not by itself implement general `GuestBlock` storage.
- Addressable locals and arguments have layout-described, traced frame homes. Evaluation-stack
  temporaries are not a substitute for write-through homes. Frame ownership covers return,
  exceptions, tail-call replacement, suspension and reuse.
- Managed byrefs carry origin/owner, offset, target and applicable generation. Cached pointers
  derive from the retained allocation base. An observed address grants neither access nor rooting.
- Scoped access retains the necessary owner/lease/guard, checks the actual range and applies the
  common synchronization and write-barrier protocol. Release guards before scheduler parking.
- Retain real unmanaged pointers when available. Native integers preserve guest arithmetic and
  bytes; they do not automatically become roots or valid dereference permissions.
- Pointer-bearing stores/copies preserve representation and tracing. No unmediated mixed-size
  race or native escape is accepted merely because another access path takes a lock.

## Milestones and dependencies

The remaining work is organized around three complete outcomes. Keep changes reviewable, but
include supporting code and verification needed to make each path usable.

| Outcome | Existing identifiers | Observable completion |
|---|---|---|
| Executor and cross-arena lifetime | M1e/M1g, R08, plan 03 | Extraction cannot transfer TLS ownership accidentally; drain survives unwind; collection evidence has a valid rooting/brand contract |
| Stable homes and common access | M3/M4 plus affected M1f/M5 paths | Saved byrefs/native integers survive assignment and frame movement; shared scalar/atomic/copy paths agree on access and barriers |
| Supported native calls | M2, R06/R07 | Native fixtures establish correct payload addresses, scalar/aggregate returns, by-ref/out copies and retained lifetime |

Ownership is the next runtime repair. A shared-access decision precedes rollout to shared
storage; it need not precede independent single-threaded fixes. M2 can proceed independently
using established ownership/access contracts. Integrate copy/barrier fixes as their storage
paths migrate rather than postponing necessary correctness until a separate milestone.

### M0 — Make failures and controls executable

The catalog has 39 VM entries: 33 enabled controls and six diagnostic-only probes. Current YAML
runs controls as 4 batches per configuration on selected push/PR runs and as 33 singleton shards per
configuration in full sweeps and after a batch failure; [CI.md](../CI.md) describes the existing
selection and the planned hierarchy. Preserve native regression coverage, discovery guards,
known-failure isolation and full scheduled sweeps when changing job placement.

Each active finding needs a reproducer, a refutation or a bounded compatibility decision.
Source-derived concerns may justify a repair; do not require unsafe native execution to prove
one. Recover missing archives only when they resolve a specific uncertainty blocking current work.
Broad VM Miri expansion and supplemental strict/model research are separate investigations.

#### Recorded follow-ups (not completed)

Missing historical artifacts, strict dependency traces and lab sources remain evidence limits.
Do not make their recovery an umbrella exit condition. Record the current source and test for
each new repair. Existing performance concerns remain tracked; measure an affected path when
its result can change an implementation or compatibility decision.

#### M0 lint and atomic status (2026-10-31)

The provenance-lint migration classified 59 first-party casts and made exposure/reconstruction
explicit. Numeric GC transport was subsequently replaced by retained pointer/identity transport.
The source lint is not dynamic strict-provenance evidence. Two atomic tests had invalid backing
or aliasing; corrected tests passed recorded Tree/Stacked Borrows selections in both modes.
Those test repairs do not establish a protocol for mixed-width overlapping runtime accesses.
See the [inventory](../pointer-provenance/INVENTORY.md) for commands and limits.

### M1 — Contain independently reproducible defects

#### M1a scoped status and coverage (2026-10-28)

Heap p10/p13/p14/p16/p20 had recorded pre-repair Miri UB at an oversized slice and passed after
exact-range access and address-observation repairs. Independently recreated library regressions
cover one-byte reads, safe out-of-bounds observation and retained bases across typed writes.
Non-heap access and the wider storage model remain outside that result.

#### M1b status and coverage (2026-10-29)

Recorded arithmetic, comparison, null/wrapping and native-integer round-trip controls passed
for their asserted paths. Negative/wider managed offsets still have interim restrictions;
remaining narrow/overflow-checking conversions and negative-index exception behavior are open.

#### M1e/M1g status and coverage (2026-11-01)

Retained transport carries an allocation-derived pointer and exact registration identity through
tracing, harvest and marking. Owner roots use publication barriers. Unsigned owner decoding,
release-enforced codec bounds, registration generations and non-reused tokens have regressions.
The compact object codec still requires a live original allocation and lacks a generation field.
Its lower-48-bit address/16-bit owner limits and the VM's 65,535 lifetime arena limit remain real
constraints, not the final representation.

The scoped ownership outcome in [plan 03](03-executor-teardown-ownership.md) is accepted;
broader R08 acceptance remains open, including the real-VM collection fixture's foreign-branded
`Gc`. That concern was accepted by a maintainer decision, not made safe by that approval.
Replace or justify the fixture's actual contract before using it as closure evidence. A lease
prevents arena teardown, not object collection.

#### M1c status and coverage (2026-10-29)

Frame-owned non-resizing localloc blocks fixed the recorded p04 failure. Recorded growth,
alignment, nested-call, return/pooling, tail-call, unwind and teardown controls passed. Stable
local/argument homes, general heap/static storage and nested-filter suspension remain open.

#### M1d status and coverage (2026-10-30)

Shared Transient ownership and `RetainedPtr` fixed recorded owner-loss and dangling-access
regressions. Selected cross-arena accesses hold leases through use. p35 reads a snapshot and
does not prove stack-home write-through. p36 refuted a one-byte static lifetime hypothesis;
p12 instead hit an alignment assertion. Fill's repaired observation/access gap has library
evidence, while end-to-end reachability remains unconfirmed.

### M2 — Repair the native boundary

Use a small native fixture library to establish payload addresses, pin/root lifetime, ABI return
size/alignment and signedness, aggregate returns, by-ref/out slot indirection, null/string
marshalling, temporary growth and cleanup. Keep native libffi results separate from Miri.
Agreement with an arbitrary native import's real ABI and the safety of library initializers
remain explicit trust assumptions; fixture coverage cannot validate arbitrary foreign code.

#### M2 handoff: temporary cross-arena native-escape restriction (step 8.5, 2026-11-01)

Preserve the enforced rejection of unsupported cross-arena raw escapes and by-value aggregates
containing managed pointers. Do not remove it until actual calls demonstrate native-call rooting,
owner lifetime, stable storage, synchronization and correct copy-back. A predicate-only test
or a short lease cannot establish the native-call boundary.

### M3 — Migrate storage and pointer representation

Deliver stable traced frame homes first, preserving write-through semantics and addresses
retained as native integers. Carry storage ownership through suspend/restore, nested calls,
unwind, pooling and reuse. Remove relocation/fixup code only after its consumers migrate.

Prefer explicit owner/offset/pointer state over further compact encoding exceptions. Separate
live representation from its codecs, reviewing tracing, pointer atomics, layouts, serialization,
fuzz inputs and benchmarks together. Widening may reduce restrictions but has migration cost;
a logical handle alone cannot fix addresses already exposed to native integers or FFI.

**Exit:** p01/p09 write-through and growth/suspension regressions, p12 static alignment and
applicable p02/p03/p15 controls pass; decoding retains owners and live allocation provenance.
Existing supported behavior is preserved or a compatibility change is explicitly decided.

### M4 — Establish one host memory model

Use a common coarse synchronization protocol as the initial candidate for managed shared
blocks, retaining existing locks until every competing access path participates. Include
plain/volatile/Interlocked operations, mixed sizes, unaligned accesses, raw aliases and FFI.
Natural-width atomics alone do not establish arbitrary overlapping access safety. If a native
alias cannot participate, establish enforceable exclusion before access; do not assume a
per-object lock protects external native code.

**Exit:** focused models and VM tests cover scalar/atomic/block/raw overlaps and ordering,
including statics. No host race is waived by green controls or an undocumented guest restriction.
Finer-grained or lock-free access requires measured need and equivalent correctness evidence.

### M5 — Integrate GC-aware stores and resumable bulk operations

Migrate typed stores/copies with their storage/access paths. Apply reference-layout-aware
barriers and cross-arena publication; preserve provenance through copies and specify byte writes
that overlap reference slots. Cover cpblk/initblk, Memmove, Array.Copy and Fill as affected.
Retain operands, owners and progress across safepoints, releasing exclusion before parking.
Evaluate the experiment's saved-progress mechanism when this outcome is selected; the retired
native branch is not a landed dependency. Avoid building parallel frameworks by default.
A proven retry-before-side-effects path is also acceptable.

**Exit:** forced stop/GC between chunks preserves results and progress, including overlap,
cancellation and fresh-reference reachability in incremental and cross-arena collection.

### M6 — Close the gates

Close each delivered outcome with the [relevant validation layers](../CI.md#test-hierarchy-and-migration),
reviewed performance on affected paths and matching compatibility/safety documentation.
Broad storage/access changes require full affected suites before acceptance. Do not require
all historical models or unrelated diagnostic fleets for every scoped repair. Keep audit-only
findings open until reproduced, refuted or covered by an explicit enforceable limit.

## Finding-to-work coverage

| Findings | Remaining owner and outcome |
|---|---|
| R01/R02/R05 | M3 stable homes/storage and codecs; retain completed heap, arithmetic and localloc regressions |
| R03/R04/R08 | Retain accepted plan 03 ownership repair; valid cross-arena collection fixture; M3 remaining codecs |
| R06/R07 | M2 native fixture and supported ABI/lifetime boundary |
| R09/R12 | M4 common access/ordering; confirm source-derived sync/handle findings |
| R10/R11 | Affected M1f/M5 copies, count/range handling and negative-index exception behavior |
| R13 | Retain repaired atomic tests; target further strict/core evidence to specific uncertainties |

## Execution and reporting

Follow [CONTRIBUTING.md](../../CONTRIBUTING.md#engineering-decisions-and-plan-authority).
Use focused regressions during implementation, then one integration pass for the completed
outcome. Carry relevant evidence forward on unchanged code; rerun when dependencies or new
findings invalidate it. Preserve exact discovery and known-UB diagnostic isolation. Update the
owning status and evidence once at integration rather than reproducing the whole inventory.
