# VesProof workspace handoff

This file turns the feasibility study's three orchestration lanes into explicit
workspace ownership. The study remains the canonical specification; the sibling
repositories are implementation scaffolds, not evidence that a phase is done.

| Lane | Repository | Package root | Reserved ownership | Current status |
| --- | --- | --- | --- | --- |
| A | `~/Desktop/dotnet-rs` | repository root | VM integration, scripts at unsafe sites, `TRUSTED.toml`, fast `ves-check` CI, obligation production | Phase 0 complete (commit `208a6c8b`, 2026-08-01): both live defects fixed, `miri-value`/`fuzz-raw-memory-access` promoted to blocking CI, differential harness expanded to seven fixtures. VesProof integration (WP-9 onward) not started — blocked on lane B delivering the macros. WP-9's scope has grown: it now also owns the `TRUSTED.toml` schema and parser, and the guard-`cfg`→feature forwarding rule for checked tokens. |
| B | `~/Desktop/ves-proof` | `crates/` | `ves-syntax`, `ves-vocabulary`, `ves-tokens`, `ves-macros`, `ves-check`; manifest schemas live at `schemas/` | All five crates implemented: WP-4 `ves-syntax` (`9082659`), WP-5 `ves-vocabulary` (`ad67707`), WP-6 `ves-tokens` (`73e6dea`, first pass), WP-6b vocabulary-manifest follow-up (`c23ede8`), WP-7 `ves-macros` (`85a6abc`, first pass), and WP-8 `ves-check` (`2c93b4b`/`e06a34b`/`da52ee5`/`bb7ea0a`) implemented: script grammar/AST/hashing; a declarative 71-constructor, 20-namespace F1--F9 vocabulary with triple Rust/Lean/manifest code generators; the generated `no_std` ZST token crate with a build-time `grants` module driven by the golden-pinned vocabulary manifest instead of scraped Rust; the four proof-script proc macros with expansion-snapshot and compile-fail coverage; and the standalone extractor/checker CLI (extraction, structural rules, manifest diffing, ratchet budgets — 57 tests, all passing) with the finalized v1 schemas it emits. Lane B's only carried-forward item is the `TRUSTED.toml` schema and its real parser, both reassigned to WP-9. Lane B is idle; the lane-A integration bootstrap (WP-9) is unblocked and is the sole gate on wiring this workspace into `dotnet-rs` |
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
  build script originally scraped trusted-constructor names out of formatted
  Rust; WP-6b (`c23ede8`) replaced that with a machine-readable
  `generated/vocabulary.manifest` emitted by `ves-vocabulary`, and no new
  consumer — including WP-13's `VesCore` — should add a second scraper
  instead of reading that manifest.
- Do not hand-write a semantic token type in `ves-tokens`. Its `lib.rs` is a
  doc comment and two `include!`s by design; new tokens are added to the
  vocabulary source and regenerated.
- A scaffold marker means “ownership reserved,” not “API accepted.” Follow the
  ordered work packages in `sections/09b-decomposition.tex` and replace markers
  only when that package's implementation session begins.

## Next implementation sessions

Lane B has implemented all five reserved crates: `crates/ves-syntax`
(WP-4, `9082659`), `crates/ves-vocabulary` (WP-5, `ad67707`), the WP-6b
vocabulary manifest follow-up (`c23ede8`), `crates/ves-tokens` (WP-6,
`73e6dea`), `crates/ves-macros` (WP-7, `85a6abc`), and
`crates/ves-check` (WP-8, `2c93b4b`/`e06a34b`/`da52ee5`/`bb7ea0a`). Lane
B's strict order (4→8) is therefore complete and lane B is idle. Two
sessions can start now, in different repositories:

- **Lane A, WP-9 integration bootstrap** — unblocked now that lane B has
  delivered `ves-check` with real test coverage (57 tests: unit coverage
  of every module, golden extraction against synthetic fixture crates,
  and CLI exit-code integration tests, all passing). This is the one
  session that legitimately edits two repositories: the `TRUSTED.toml`
  schema is authored in `dotnet-rs`, but its parser replaces the
  placeholder in `ves-proof/crates/ves-tokens/build_support.rs` *and*
  the unconditional acceptance in `ves-check/src/rules.rs`'s
  `check_premise_validity` (marked `TODO(WP-9)`) — both discovery paths
  land on the same schema. WP-9 also owns assembling the
  `ves::`-namespaced facade module that the study's examples assume; the
  implemented macros are plain `ves_macros::proof!`/`proof_impl!`/`exempt!`/
  `contract!` with no such facade yet, and it owns forwarding
  `dotnet-rs`'s guard `cfg` to `ves-tokens`'s `memory-validation`
  feature.
- **Lane C, WP-13 `VesCore`** — unblocked by WP-5 and WP-6b, and the
  longest-lead item in the plan, runnable concurrently with WP-9 since the
  two touch different repositories. It may start against the checked-in
  `generated/VesVocabulary.lean` golden file plus the now-available
  `generated/vocabulary.manifest`, which carries the trust class and
  invariant family the Lean inventory alone does not — no need to tell the
  elaborator by hand which propositions are grant-axiomatized.

Cross-lane synchronization points and tier guidance are in §10 of the
study.
