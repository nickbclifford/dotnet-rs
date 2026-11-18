# Assurance background

The distilled output of a feasibility study (2026-08-01 to 2026-08-03, terminated) on
machine-checking the project's `// SAFETY:` comments with a Lean 4-backed proof DSL.

**Historical record, not instructions.** Current work is in [docs/plans/](plans/README.md); the
live successors of this study are the [invariant registry](INVARIANT_REGISTRY.md) and the
[pointer-provenance evidence](pointer-provenance/README.md). Counts below describe the code in
August 2026 and are not maintained.

The study proposed a separate assurance queue, since retired; its items are mentioned below only
for context: A01 (a named predicate registry, now the invariant registry), A03 (make atomic access
width a type parameter), A04 (an ECMA-335 correspondence table) and A06 (a table of trust classes).

- **Study (rev. 11), archived in git history** at
  [`docs/proof-dsl-feasibility/` @ `b5a5f65d`](https://github.com/nickbclifford/dotnet-rs/tree/b5a5f65d67345b0682def83867b816ea86fa3152/docs/proof-dsl-feasibility)
  — added in `f088cad0`, and `b5a5f65d` is the last commit whose tree contains
  it. Locally:
  `git show b5a5f65d:docs/proof-dsl-feasibility/main.pdf > /tmp/study.pdf`.
- **Prototype crates** were archived outside this repository; none of their code is here.

What remains useful is the families table (still the structure of the invariant registry) and the
process lessons.

## The invariant families

The study's durable finding: the prose `// SAFETY:` comments (about 620 in August 2026) do not
represent independent obligations. They factor into nine families, each one predicate stated once
and instantiated per site. A01 turned this skeleton into the named
[predicate registry](INVARIANT_REGISTRY.md).

| # | Family | The claim | What Rust already carries | What it doesn't |
| --- | --- | --- | --- | --- |
| F1 | Rooting / STW liveness | This allocation cannot be freed or moved during this access | `Gc<'gc>` handle or lock guard in scope; `Arc` refcount | The temporal premise: every mutator parked, `unregister_arena` waited for `active_leases == 0`, generation stamps match, GC is non-moving |
| F2 | Layout faithfulness | The descriptor says offset *k* holds type *X*, so reading these bytes as *X* is valid | Slice bounds; bytes written by the matching serializer | That `FieldLayoutManager`/`GcDesc` agree with ECMA-335 §II.10.7 / §I.8.5 for this instantiation |
| F3 | Eval-stack slot typing and stable homes | Slot views match their values; addressable variables live in non-moving frame homes | Enum discriminants and frame-owned raw blocks | Home addresses retain their allocation across stack growth; checked byrefs resolve a live serial and complete range |
| F4 | Atomic width / alignment | The pointer is valid and aligned for the selected atomic width, and no concurrent non-atomic access races it | `align_of`, guard exclusion | Which of three coexisting discharge mechanisms is actually being cited |
| F5 | Tracing completeness | This `trace` visits every contained GC reference | Match exhaustiveness, field enumeration | That the root set and `VmContinuation` state are complete — a universally quantified property |
| F6 | `gc-arena` brand discipline | This `'gc`-branded value cannot escape its arena | Invariant lifetimes, private constructors, `for<'a>` confinement | Almost nothing — this family is the clearest win for lifetime branding. One deliberate violation: cross-arena tracing mints `Gc::from_ptr` with the *wrong* arena's brand (`HeapManager::trace`), justified by F1 plus non-escape |
| F7 | Immutable-field publication | `owner_id` is written once at construction, so this lock-free read is race-free | Immutability | The happens-before edge publishing the initializing write |
| F8 | Lock order & safepoint discipline | Locks are acquired in DAG order; no safepoint poll or allocation while holding a heap borrow | `define_lock_order_dag!` with negative compile-time assertions | The four `GcScopeGuard` conventions — a rely-guarantee protocol |
| F9 | Leaked-`Box` metadata lifetime | `MetadataArena` solely owns these leaked allocations and outlives every descriptor into them | Conventional ownership reasoning | Little; low risk, low novelty |

What an ML-family type system without dependent types cannot say, and which
therefore lands in the residue above: value-dependent facts (`offset + size <=
len`), temporal/protocol facts (all mutators parked *for the duration of this
read*), history facts, correspondence facts (runtime data vs. a
specification-level function), and completeness facts.

### Two irreducible trust classes

Neither is provable; both must be isolated and named. They were seed rows for A06.

- **The managed program as adversary.** ECMA-335 itself declares behavior
  undefined when unverifiable IL supplies invalid operands to `cpblk`,
  `initblk`, `Unsafe.*`. These are *conditional* claims: if the input IL is
  verifiable in the §III.1.8 sense, or the host accepts the unverifiable-code
  trust boundary, then the implementation is safe.
- **The FFI boundary.** ABI agreement between a libffi CIF and an arbitrary
  native library is not provable in any model that stops at the process
  boundary. The pinning and liveness obligations *around* the call are F1; the
  call itself is an axiom per imported function shape.

## Why the proof track was terminated

**1. The missing arrow is not one arrow.** The study treated "Rust semantics →
premise" as a single edge to be supplied later by a verifier or conceded as
trusted correspondence. Decomposed per family, it is four edges of different
kinds, and only the cheap kinds lie inside any Rust verifier's remit:

| Premise kind | Families | Who can supply it |
| --- | --- | --- |
| Cross-thread temporal | F1, F7, F8 | No tool, on real code |
| ECMA-335 correspondence | F2, the non-moving-GC axiom | **No Rust verifier, ever** — half the statement is not in the program |
| Structural completeness | F5 | Rust itself (exhaustive match, derive audit) |
| Local value / spatial | F3, F4, F9 | Refinement, separation logic, or BMC — *or a type refactor* |

The families carrying this project's novelty (F1, F2, F6) need premises no Rust
verifier reaches; the families a verifier *would* close are the ones rated
lowest-novelty. F4, third-largest by site count, has a cheaper correct fix than
any proof: make the access width a type parameter so an `Int32` dispatch arm
cannot pass `4` to a 2-byte operation (A03).

**2. `cargo-anneal` is the same product, far better resourced.** Google's
zerocopy team's [`cargo-anneal`](https://crates.io/crates/cargo-anneal) pursues the same thesis
(specifications in doc comments, Lean 4 via Charon→Aeneas, `unsafe(axiom)` as a trust class) and
was pre-alpha at the time. The Rust→Lean arrow is a funded program elsewhere; building another one
solo was never the comparative advantage.

**3. A feature census eliminates every candidate verifier.** The code relies heavily on
closures, bounded generics, associated types, atomics, locks, threads and exposed provenance,
and hardly at all on unions, transmutes or `MaybeUninit`.

| Tool | Verdict | Blocking reason |
| --- | --- | --- |
| [RefinedRust](https://iris-project.org/pdfs/2024-pldi-refinedrust.pdf) | infeasible | Its own §1: no concurrency, recursive types, traits, closures, or unsized types; no aliasing model; no pointer-integer casts. Four of the five largest features above. Cost datum: 120 lines of `Vec` → 76 annotation lines → 1200 generated Rocq lines |
| Aeneas / Anneal | infeasible now | Safe-Rust functional translation; trait-bound generics don't extract; no concurrency story |
| [Verus](https://verus-lang.github.io/verus/guide/) | infeasible without rewrite | Raw pointers only under a simplified model with no full aliasing or provenance semantics; the `gc-arena` heap would be rewritten against `PointsTo`/`PPtr` |
| [Kani](https://model-checking.github.io/kani/limitations.html) | useful, not for the arrow | No concurrency (warns, compiles threads sequentially); no unwinding; no Stacked/Tree Borrows; no provenance UB |
| [VeriFast](https://verifast.github.io/verifast/rust-reference/) | only viable, not affordable | Genuinely handles multithreaded `unsafe` Rust. Its *partial* proof of std's `LinkedList` is 2,253 → 4,389 lines (+95%, 659 annotation lines) written by the tool's own authors, and it verifies only the immutable-bytes half of Rust's aliasing rules |
| [RustMC](https://arxiv.org/abs/2502.06293) | watch | GenMC stateless model checking of unsafe Rust, FFI and atomics — right shape for F1/F7/F8, but mixed-size accesses need the unmerged MIXER extension, and this VM's atomics are 1/2/4/8-byte accesses into shared object memory |

**Cost anchor.** The multi-institution
[`verify-rust-std`](https://arxiv.org/abs/2606.17374) campaign closed neither of its two
concurrency challenges.

**ECMA-335 has no mechanized formalization** that the study could find. The pen-and-paper prior art — Fruja's ASM model of
the CLR, Gordon–Syme's Baby IL type soundness, Kennedy–Syme on generics — remains
the best source text for edge cases, and is the reason
A04 proposed writing the correspondence down as a table rather than as theorems.

## What the exercise actually delivered

Phase 0 of the study was due diligence, not verification, and it shipped: two live soundness
defects fixed, `miri-value` and a raw-memory-access fuzz lane made blocking (today `miri-value`
runs in the fast core and the `fuzz_raw_memory_access` corpus replays in `fuzz-corpus-replay`;
see [CI](CI.md)), and the differential harness expanded. That came from the *survey*, not from
the proof layer.

Both defects were instances of one failure mode: **prose asserting a witness
that no code establishes.** One cited a `cfg`-gated no-op as an alignment
witness; the other justified a `Sync` implementation with a false claim. That
failure mode is a failure of *naming*, not of proving — a comment saying
"aligned" cannot be cross-checked, while one citing `F4.WidthAligned` can. Hence
A01, the invariant registry.

## Process lessons

Worth keeping, because they are not specific to proof assistants:

- **Test tools against the codebase, not against each other.** A comparison
  table organized by the tools' own published axes is not a feasibility study.
  The feature census above took minutes and eliminated three candidate
  architectures that a 44-page survey had left open for a later decision gate.
- **Decompose a deferred gap before deferring it.** As one edge, "Rust semantics
  → premise" was a risk to mitigate later. Decomposed per family, it was an
  architectural verdict available immediately.
- **Don't let the deliverable ladder outrun the load-bearing question.** Five
  crates were built while the package the whole design depended on — the goal
  elaborator, and behind it the ECMA-335 model — sat unstarted. Every phase was
  independently useful, which is exactly how that happens.
- **A document that agents will execute gets executed on its most enthusiastic
  section.** The study's risk sections were accurate about the semantic gap; its
  recommendation section spoke as if `rustc`-checked dataflow closed it. The
  recommendation is the one that drove implementation.
