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

- **Output:** `main.pdf` (40 pages)
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

Revision 5 records that Phase 0 (§9) is complete: commit `208a6c8b` in
`dotnet-rs` fixed both soundness defects from §2.3, promoted one Miri leg
and one fuzz target to blocking CI gates, and expanded the differential
harness from one fixture to seven. The `ves-proof` and `ves-proof-lean`
skeletons (WP-3) are unchanged and still pure scaffolds — no crate or
package has logic beyond a marker constant, and no Lean toolchain is
pinned yet. Phase 1, starting with `ves-syntax`, is the next action.

Revision 6 records that WP-4 (`ves-syntax`) is implemented in the
`ves-proof` repository (commit `9082659`, 2026-08-01): the script grammar,
`Script`/`Statement` AST, canonical serializer, and the
`statement_hash`/`subject_hash` SHA-256 contract, with `GRAMMAR_VERSION`
pinned at `"0.1.0"` and a golden corpus of valid and malformed `.ves`
scripts (33 tests, all passing).

Revision 7 records that WP-5 (`ves-vocabulary`) is implemented in the
`ves-proof` repository (commit `ad67707`, 2026-08-02): a declarative,
versioned source for the F1--F9 goal/premise/tactic inventory (77
constructors across 16 namespaces with trust and family annotations),
dependency and structural validation, and dual code generators emitting a
`no_std` Rust ghost-token module (`generated/tokens.rs`, handed to WP-6) and
a Lean declaration inventory (`generated/VesVocabulary.lean`, handed to
WP-13), both covered by golden-file tests (17 tests, all passing).
`VOCABULARY_VERSION` is now the canonical value embedded in both manifest
schemas and in statement hashes. The other three `ves-proof` crates
(`ves-tokens`, `ves-macros`, `ves-check`) and all five `ves-proof-lean`
packages are unchanged scaffolds. Phase 1 is underway; WP-6 (`ves-tokens`)
is next.

The study is grounded in a survey of the repository at HEAD `97e8f658`
(2026-07-31): 581 unsafe blocks, 620 SAFETY comments, the invariant-family
taxonomy in §3, and the two then-live soundness defects discussed in §2.3
(both fixed as of `208a6c8b`, 2026-08-01 — see §2.3 and §9). If those
numbers drift far from the current tree, re-survey before quoting them.
