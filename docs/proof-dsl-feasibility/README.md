# Proof-DSL Feasibility Study

A typeset feasibility study on adding a second-layer, machine-checked
verification of `// SAFETY:` proofs in `dotnet-rs` via a dependently-typed
(Curry–Howard) proof DSL, with Lean 4 as the recommended backend.

The recommended design (rev. 2) is **VesProof**: an embedded,
tactic-oriented proof-script DSL in the lineage of dotnetdll's `asm!`
macro — block-shaped `ves::proof!` scripts wrapping each unsafe site,
premises acquired as zero-sized ghost tokens that rustc itself checks
(existence, cfg-liveness, dataflow), deterministic elaboration to Lean 4
statements, Lean's kernel as the sole trusted checker, and a
compartmentalized package architecture (five Rust crates + five Lean
packages) whose only contact with dotnet-rs is two build dependencies and
two content-hashed manifest artifacts.

- **Output:** `main.pdf` (39 pages)
- **Build:** `./build.sh` (requires XeLaTeX + latexmk + bibtex; fonts are
  loaded by filename from the TeX Live tree, no system fontconfig needed)
- **Sources:** `main.tex`, `preamble.tex`, `references.bib`,
  `sections/*.tex`

Revision 4 adds §10, a decomposition of the whole effort into ordered
supervised-refactor work packages for the nasin-pali orchestrator
(three lanes: dotnet-rs / ves-proof Rust crates / Lean packages), sized
by checklist class, tier mix, verification-signal quality, and
supervisor gate density rather than calendar time. It also records the
now-created repository and package skeletons at their concrete locations:
`../ves-proof/crates/` for the Rust/tooling lane and
`../ves-proof-lean/packages/` for the Lean lane.

The three lanes now have explicit workspace reservations. See
[`WORKSPACES.md`](WORKSPACES.md) before starting an implementation session;
it records which sibling repository owns each package and which parts remain
design-only.

The study is grounded in a survey of the repository at HEAD `97e8f658`
(2026-07-31): 581 unsafe blocks, 620 SAFETY comments, the invariant-family
taxonomy in §3, and the two then-live soundness defects discussed in §2.3.
If those numbers drift far from the current tree, re-survey before quoting
them.
