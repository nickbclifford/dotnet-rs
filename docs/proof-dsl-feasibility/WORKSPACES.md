# VesProof workspace handoff

This file turns the feasibility study's three orchestration lanes into explicit
workspace ownership. The study remains the canonical specification; the sibling
repositories are implementation scaffolds, not evidence that a phase is done.

| Lane | Repository | Package root | Reserved ownership | Current status |
| --- | --- | --- | --- | --- |
| A | `~/Desktop/dotnet-rs` | repository root | VM integration, scripts at unsafe sites, `TRUSTED.toml`, fast `ves-check` CI, obligation production | Phase 0 complete (commit `208a6c8b`, 2026-08-01): both live defects fixed, `miri-value`/`fuzz-raw-memory-access` promoted to blocking CI, differential harness expanded to seven fixtures. VesProof integration (WP-9 onward) not started — blocked on lane B delivering the macros. WP-9's scope has grown: it now also owns the `TRUSTED.toml` schema and parser, and the guard-`cfg`→feature forwarding rule for checked tokens. |
| B | `~/Desktop/ves-proof` | `crates/` | `ves-syntax`, `ves-vocabulary`, `ves-tokens`, `ves-macros`, `ves-check`; manifest schemas live at `schemas/` | WP-4 `ves-syntax` (`9082659`), WP-5 `ves-vocabulary` (`ad67707`), WP-6 `ves-tokens` (`73e6dea`, first pass), and WP-7 `ves-macros` (`85a6abc`, first pass) implemented: script grammar/AST/hashing; a declarative 71-constructor, 20-namespace F1--F9 vocabulary with dual Rust/Lean code generators; the generated `no_std` ZST token crate with a build-time `grants` module; and the four proof-script proc macros with expansion-snapshot and compile-fail coverage. `ves-check` and both v1 schema placeholders stay scaffolds; WP-8 `ves-check` is next and is now the sole gate on the lane-A integration bootstrap (WP-9) |
| C | `~/Desktop/ves-proof-lean` | `packages/` | `VesCore`, `VesModel`, `RustAssumptions`, `VesProtocol`, `DotnetRsProofs` | All five independent Lake package skeletons exist; Lean toolchain intentionally not selected. Unblocked: WP-13 `VesCore` may start now |

All paths in the rest of this file are repository-relative. From this study's
directory, the sibling roots are `../../../ves-proof` and
`../../../ves-proof-lean`; from the `dotnet-rs` repository root they are
`../ves-proof` and `../ves-proof-lean`.

## Phase locations

| Phase | Existing implementation location(s) |
| --- | --- |
| 0 — Hygiene | Lane A: `dotnet-rs` source, tests, scripts, and CI |
| 1 — Embedded layer | Lane B: `ves-proof/crates/{ves-syntax,ves-vocabulary,ves-tokens,ves-macros,ves-check}` and `ves-proof/schemas/`; Lane A for VM integration |
| 2 — Model | Lane C: `ves-proof-lean/packages/VesModel/` |
| 3 — Hard theories | Lane C: `ves-proof-lean/packages/{VesCore,VesProtocol,RustAssumptions}/`; Lane B's `ves-check` participates in the manifest round trip |
| 4 — Proof campaign | Lane C: `ves-proof-lean/packages/DotnetRsProofs/`; Lane A supplies obligations and enforces the ratchet |

## Why two sibling repositories

The study sometimes uses “the `ves-proof` repository” as a collective name for
the spun-off verifier, while its decomposition requires one branch-owning
repository per concurrent lane. The concrete split above follows that later,
execution-specific requirement. It does not change a package interface: the
Rust and Lean halves already communicate through generated vocabulary and
versioned serialized manifests.

## Non-duplication rules

- Do not recreate any of the ten reserved packages in `dotnet-rs`.
- Do not add a second parser: `ves-syntax` is shared by `ves-macros` and
  `ves-check`.
- Do not create a proof kernel or Rust-side tactic evaluator. Rust extracts and
  checks structure; Lean elaborates and kernel-checks proofs.
- Do not make `dotnet-rs` invoke Lean or depend on a Lean package.
- Do not make a Lean package depend directly on `dotnet-rs` source.
- Treat `obligations.v1.json`, `proofs.v1.json`, vocabulary versions, and
  statement hashes as cross-repository contracts.
- Do not re-derive vocabulary facts by parsing a generated artifact. WP-6's
  build script currently scrapes trusted-constructor names out of formatted
  Rust; WP-6b replaces that with a machine-readable manifest emitted by
  `ves-vocabulary`, and no new consumer should add a second scraper in the
  meantime.
- Do not hand-write a semantic token type in `ves-tokens`. Its `lib.rs` is a
  doc comment and two `include!`s by design; new tokens are added to the
  vocabulary source and regenerated.
- A scaffold marker means “ownership reserved,” not “API accepted.” Follow the
  ordered work packages in `sections/09b-decomposition.tex` and replace markers
  only when that package's implementation session begins.

## Next implementation sessions

Lane B has implemented `crates/ves-syntax` (WP-4, `9082659`),
`crates/ves-vocabulary` (WP-5, `ad67707`), `crates/ves-tokens` (WP-6,
`73e6dea`), and `crates/ves-macros` (WP-7, `85a6abc`). One lane-B session
remains before the lane-A integration bootstrap can start; it runs
independently of lane C's ongoing work:

- **Lane B, WP-8 `ves-check`** — now the sole gate on WP-9. Lane B's order is
  strict (4→8), so this is the only lane-B session that can run next.
  Implement extraction, structural rules, manifest comparison, and ratchet
  budgets against synthetic fixture crates.
- **Lane C, WP-13 `VesCore`** — unblocked by WP-5 and the longest-lead item
  in the plan, runnable concurrently with WP-8 since the two touch different
  repositories. It may start against the checked-in
  `generated/VesVocabulary.lean` golden file. Prefer running WP-6b first if
  convenient: that inventory carries no trust class or invariant family, so
  without the manifest the elaborator has to be told by hand which
  propositions are grant-axiomatized.

**WP-6b (vocabulary manifest)** is a small lane-B session that can be slotted
in before or interleaved with WP-8; it closes the text-scraping dependency in
`ves-tokens` and supplies WP-13.

Lane A must still not bootstrap the dependencies until structural `ves-check`
has real test coverage; `ves-macros` already does (three unit tests, five
expansion-snapshot fixtures, three `trybuild` compile-fail fixtures, all
passing). When lane A resumes, note that WP-9 is the one session that
legitimately edits two repositories: the `TRUSTED.toml` schema is authored in
`dotnet-rs`, but its parser replaces the placeholder in
`ves-proof/crates/ves-tokens/build_support.rs`. WP-9 also owns assembling the
`ves::`-namespaced facade module that the study's examples assume; the
implemented macros are plain `ves_macros::proof!`/`proof_impl!`/`exempt!`/
`contract!` with no such facade yet. Cross-lane synchronization points and
tier guidance are in §10 of the study.
