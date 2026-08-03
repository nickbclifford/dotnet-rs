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

- **Output:** `main.pdf` (43 pages)
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
versioned source for the F1--F9 goal/premise/tactic inventory,
dependency and structural validation, and dual code generators emitting a
`no_std` Rust ghost-token module (`generated/tokens.rs`, handed to WP-6) and
a Lean declaration inventory (`generated/VesVocabulary.lean`, handed to
WP-13), both covered by golden-file tests (17 tests, all passing).
`VOCABULARY_VERSION` is now the canonical value embedded in both manifest
schemas and in statement hashes.

Revision 8 records the first pass on WP-6 (`ves-tokens`, commit `73e6dea`,
2026-08-02) and corrects a vocabulary figure that revision 7 got wrong: the
inventory is **71 constructors across 20 namespaces** (4 trusted, 4 derived,
1 checked, 62 static), not 77 across 16. `ves-tokens` is now a `no_std`,
`forbid(unsafe_code)` crate assembled entirely from generated code — the
build script copies the vocabulary's `tokens.rs` into `OUT_DIR` and
synthesizes a `grants` module holding one `TrustGrant` const per
*registered* trusted constructor, selected by `VES_TRUSTED_TOML_PATH`. With
no registry supplied the module is empty, so a default build cannot mint any
trusted premise. Coverage is exhaustive by construction: a compile-time
`size_of == 0` assertion for all 72 generated types, plus a test that
re-derives the type list from the generated source and fails if the
assertion list drifts, plus an `#[inline(always)]` conformance test (5 tests,
6 under `--features memory-validation`, all passing).

Three items are carried forward rather than closed, and revision 8 assigns
each of them in §10 rather than leaving them in the crate's TODOs:
the `TRUSTED.toml` reader is a substring-match placeholder pending a schema
(reassigned to WP-9, which now owns both halves of that contract); trusted
constructors are discovered by text-scraping formatted Rust, which motivates
a new small work package **6b** emitting a machine-readable vocabulary
manifest for both `ves-tokens` and `VesCore`; and neither the
`debug_assert!` shadows nor the lifetime-branded tokens of §7.3 exist yet.
§7.3 also now states the one consequential divergence from the design
sketch: checked constructors are gated by a `ves-tokens` Cargo feature
rather than the guard's own `cfg`, so WP-9 must forward the `cfg` to the
feature and prove it with a guard-off CI leg, or the mechanism quietly
degrades to a static premise. The two remaining `ves-proof` crates
(`ves-macros`, `ves-check`) and all five `ves-proof-lean` packages are
unchanged scaffolds. Lane B's WP-7 (`ves-macros`) is the critical path and
lane C's WP-13 (`VesCore`) is unblocked; they can run concurrently.

Revision 9 records the first pass on WP-7 (`ves-macros`, commit `85a6abc`,
2026-08-03): the four proof-script proc macros are implemented over
`ves-syntax`. `proof!` and `proof_impl!` expand `have`/`trust` statements to
local `::ves_tokens` bindings and splice the `then` tokens verbatim,
realizing the token-identity property by construction; `exempt!` validates
its reason argument and emits nothing; `contract!` remains an
unchanged-input passthrough, with real contract-token generation deferred to
the contract-layer package (§10, item 10). `proof_impl!`'s premises
type-check inside an isolated anonymous-const checker function, since item
scope cannot receive local bindings directly — a real if narrow
expressiveness limit: that checker cannot capture caller-local values or the
protected item's own generics. Coverage is three unit tests, five
expansion-snapshot fixtures, and three `trybuild` compile-fail fixtures (a
feature-gated checked constructor, a derived constructor with an unmet
prerequisite, an unregistered trust path), all passing, and all three
failure cases are ordinary `rustc` type errors rather than macro-specific
diagnostics — exactly the diagnostics model §7's design describes. One item
is carried forward, not closed: the crate exports bare
`proof`/`proof_impl`/`exempt`/`contract` macros, not the `ves::`-namespaced
facade the study's examples use throughout; assembling that facade belongs
to the integration bootstrap (WP-9), which was already the session that
authors `TRUSTED.toml`'s schema and forwards the guard `cfg` to `ves-tokens`'
`memory-validation` feature. Four of the five `ves-proof` crates are now
implemented; `ves-check` (WP-8) is the sole remaining gate on WP-9, and all
five `ves-proof-lean` packages remain unchanged scaffolds.

The study is grounded in a survey of the repository at HEAD `97e8f658`
(2026-07-31): 581 unsafe blocks, 620 SAFETY comments, 62 documented
`unsafe fn`, the invariant-family taxonomy in §3, and the two then-live
soundness defects discussed in §2.3 (both fixed as of `208a6c8b`,
2026-08-01 — see §2.3 and §9). Re-measured at `bc266543` (2026-08-02):
583 unsafe blocks, 620 SAFETY comments, 62 documented `unsafe fn` — no
material drift. Revision 9's change is entirely in the sibling `ves-proof`
repository; `dotnet-rs` itself is unchanged since that measurement. If those
numbers drift far from the current tree, re-survey before quoting them.
