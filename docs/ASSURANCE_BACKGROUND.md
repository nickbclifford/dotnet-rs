# Assurance background

Why the assurance plans in [`docs/plans/`](plans/README.md) look the way they
do. This
is the distilled output of a feasibility study on machine-checking the project's
`// SAFETY:` comments with a Lean 4-backed proof DSL, run 2026-08-01 → 2026-08-03
and terminated. The full 44-page study and its five implemented Rust crates are
history, not documentation:

- **Study (rev. 11), archived in git history** at
  [`docs/proof-dsl-feasibility/` @ `b5a5f65d`](https://github.com/nickbclifford/dotnet-rs/tree/b5a5f65d67345b0682def83867b816ea86fa3152/docs/proof-dsl-feasibility)
  — added in `f088cad0`, and `b5a5f65d` is the last commit whose tree contains
  it. Locally:
  `git show b5a5f65d:docs/proof-dsl-feasibility/main.pdf > /tmp/study.pdf`.
- **Prototype crates:** `~/Desktop/ves-proof` (`ves-syntax`, `ves-vocabulary`,
  `ves-tokens`, `ves-macros`, `ves-check`) and `~/Desktop/ves-proof-lean`, both
  archived read-only. No code from either is in this repository.

Everything below is current: it is the rationale for the plan queue, not a
record of an abandoned plan.

## The invariant families

The study's durable finding. 620 prose `// SAFETY:` comments across 583 `unsafe`
blocks do not represent 620 independent obligations — they factor into nine
families, each one predicate stated once and instantiated per site. This is the
skeleton [plan 01](plans/01-layer-invariant-specs.md) turns into a named
predicate registry.

| # | Family | Sites | The claim | What Rust already carries | What it doesn't |
| --- | --- | --- | --- | --- | --- |
| F1 | Rooting / STW liveness | ~70 | This allocation cannot be freed or moved during this access | `Gc<'gc>` handle or lock guard in scope; `Arc` refcount | The temporal premise: every mutator parked, `unregister_arena` waited for `active_leases == 0`, generation stamps match, GC is non-moving |
| F2 | Layout faithfulness | ~60 | The descriptor says offset *k* holds type *X*, so reading these bytes as *X* is valid | Slice bounds; bytes written by the matching serializer | That `FieldLayoutManager`/`GcDesc` agree with ECMA-335 §II.10.7 / §I.8.5 for this instantiation |
| F3 | Eval-stack slot typing | ~20 | This slot was pushed as *X*; this raw view is valid, and recorded interior pointers survive `Vec` reallocation | Enum discriminants, `Vec` growth semantics | The history fact — *this offset was recorded from that slot at pointer-creation time* — plus re-establishment after every `apply_reallocation_fixup()` |
| F4 | Atomic width / alignment | ~40 | The pointer is valid and aligned for the selected atomic width, and no concurrent non-atomic access races it | `align_of`, guard exclusion | Which of three coexisting discharge mechanisms is actually being cited |
| F5 | Tracing completeness | 22 `unsafe impl Collect` | This `trace` visits every contained GC reference | Match exhaustiveness, field enumeration | That the root set and `VmContinuation` state are complete — a universally quantified property |
| F6 | `gc-arena` brand discipline | ~10 | This `'gc`-branded value cannot escape its arena | Invariant lifetimes, private constructors, `for<'a>` confinement | Almost nothing — this family is the clearest win for lifetime branding. One deliberate violation: cross-arena tracing mints `Gc::from_ptr` with the *wrong* arena's brand (`heap.rs:436–445`), justified by F1 plus non-escape |
| F7 | Immutable-field publication | ~8 | `owner_id` is written once at construction, so this lock-free read is race-free | Immutability | The happens-before edge publishing the initializing write |
| F8 | Lock order & safepoint discipline | — | Locks are acquired in DAG order; no safepoint poll or allocation while holding a heap borrow | `define_lock_order_dag!` with negative compile-time assertions | The four `GcScopeGuard` conventions — a rely-guarantee protocol |
| F9 | Leaked-`Box` metadata lifetime | ~14 | `MetadataArena` solely owns these leaked allocations and outlives every descriptor into them | Conventional ownership reasoning | Little; low risk, low novelty |

What an ML-family type system without dependent types cannot say, and which
therefore lands in the residue above: value-dependent facts (`offset + size <=
len`), temporal/protocol facts (all mutators parked *for the duration of this
read*), history facts, correspondence facts (runtime data vs. a
specification-level function), and completeness facts.

### Two irreducible trust classes

Neither is provable; both must be isolated and named. They are seed rows for
[plan 06](plans/06-trust-register.md).

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
cannot pass `4` to a 2-byte operation — see
[plan 03](plans/03-width-generic-atomics.md) and the same note in
[`GC_AND_MEMORY_SAFETY.md`](GC_AND_MEMORY_SAFETY.md).

**2. `cargo-anneal` is the same product, far better resourced.** Google's
zerocopy team ([`0.1.0-alpha.24`](https://crates.io/crates/cargo-anneal), 7 June
2026) states the same thesis in the same words, with specifications in
`/// ```anneal` doc comments, **Lean 4** as the checker via Charon→Aeneas, a
shipping Lean spatial memory model (`Allocation`, `Referent`,
`FitsInAllocation`, `IsContiguous`, `HasStaticLayout`), and `unsafe(axiom)` as a
trust class. Not adoptable today — pre-alpha and self-described as unsound in
places, no union support, trait-bound generics don't extract, and "extend Aeneas
to handle separation logic" is still open at 18/71 tasks on its tracking issue.
But the Rust→Lean arrow is now a funded program at Google, MPI-SWS and KU
Leuven. Building a fourth one solo was never the comparative advantage.

**3. A feature census eliminates every candidate verifier.** Measured over
`crates/*/src` (61,395 lines of Rust; approximate greps): ~920 closures, 489
generic fns with bounds, 75 associated types, 30 `dyn`, 256 atomic uses, 154
`Mutex`/`RwLock`, 16 `thread::spawn`, 78 `expose_provenance`/`from_exposed`; and
— more happily — 0 unions, 9 transmutes, 1 `MaybeUninit`.

| Tool | Verdict | Blocking reason |
| --- | --- | --- |
| [RefinedRust](https://iris-project.org/pdfs/2024-pldi-refinedrust.pdf) | infeasible | Its own §1: no concurrency, recursive types, traits, closures, or unsized types; no aliasing model; no pointer-integer casts. Four of the five largest features above. Cost datum: 120 lines of `Vec` → 76 annotation lines → 1200 generated Rocq lines |
| Aeneas / Anneal | infeasible now | Safe-Rust functional translation; trait-bound generics don't extract; no concurrency story |
| [Verus](https://verus-lang.github.io/verus/guide/) | infeasible without rewrite | Raw pointers only under a simplified model with no full aliasing or provenance semantics; the `gc-arena` heap would be rewritten against `PointsTo`/`PPtr` |
| [Kani](https://model-checking.github.io/kani/limitations.html) | useful, not for the arrow | No concurrency (warns, compiles threads sequentially); no unwinding; no Stacked/Tree Borrows; no provenance UB |
| [VeriFast](https://verifast.github.io/verifast/rust-reference/) | only viable, not affordable | Genuinely handles multithreaded `unsafe` Rust. Its *partial* proof of std's `LinkedList` is 2,253 → 4,389 lines (+95%, 659 annotation lines) written by the tool's own authors, and it verifies only the immutable-bytes half of Rust's aliasing rules |
| [RustMC](https://arxiv.org/abs/2502.06293) | watch | GenMC stateless model checking of unsafe Rust, FFI and atomics — right shape for F1/F7/F8, but mixed-size accesses need the unmerged MIXER extension, and this VM's atomics are 1/2/4/8-byte accesses into shared object memory |

**Cost anchor.** The AWS/Rust Foundation
[`verify-rust-std`](https://arxiv.org/abs/2606.17374) campaign ran 16 months
with 450+ pull requests from 21+ contributors across four institutions, with cash
rewards, and reached ~35% of std's ~33,955 functions with 989 verified against
contracts — closing **neither of its two concurrency challenges**, and finding no
previously unknown memory-safety bugs.

**Also still true:** no mechanized formalization of ECMA-335 exists anywhere
(re-confirmed 2026-08-03). The pen-and-paper prior art — Fruja's ASM model of
the CLR, Gordon–Syme's Baby IL type soundness, Kennedy–Syme on generics — remains
the best source text for edge cases, and is the reason
[plan 04](plans/04-model-correspondence.md) writes the correspondence down as a
table rather than as theorems.

## What the exercise actually delivered

Phase 0 of the study was due diligence, not verification, and it shipped
(`208a6c8b`, 2026-08-01): two live soundness defects fixed, `miri-value` and
`fuzz-raw-memory-access` promoted to blocking CI legs, and the differential
harness expanded from one fixture to seven. That came from the *survey*, not
from the proof layer — which is the observation the plan queue is built on.

Both defects were instances of one failure mode: **prose asserting a witness
that no code establishes.** One cited a `cfg`-gated no-op as an alignment
witness; the other justified a `Sync` implementation with a false claim. That
failure mode is a failure of *naming*, not of proving — a comment saying
"aligned" cannot be cross-checked, while one citing `F4.WidthAligned` can. Hence
plan 01.

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
