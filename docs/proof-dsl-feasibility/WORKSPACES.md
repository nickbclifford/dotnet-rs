# VesProof workspace handoff

This file turns the feasibility study's three orchestration lanes into explicit
workspace ownership. The study remains the canonical specification; the sibling
repositories are implementation scaffolds, not evidence that a phase is done.

| Lane | Repository | Package root | Reserved ownership | Current status |
| --- | --- | --- | --- |
| A | `~/Desktop/dotnet-rs` | repository root | VM integration, scripts at unsafe sites, `TRUSTED.toml`, fast `ves-check` CI, obligation production | Phase 0 complete (commit `208a6c8b`, 2026-08-01): both live defects fixed, `miri-value`/`fuzz-raw-memory-access` promoted to blocking CI, differential harness expanded to seven fixtures. VesProof integration (WP-9 onward) not started — blocked on lane B delivering the macros. |
| B | `~/Desktop/ves-proof` | `crates/` | `ves-syntax`, `ves-vocabulary`, `ves-tokens`, `ves-macros`, `ves-check`; manifest schemas live at `schemas/` | WP-4 `ves-syntax` (commit `9082659`, 2026-08-01) and WP-5 `ves-vocabulary` (commit `ad67707`, 2026-08-02) implemented: script grammar/AST/hashing, and a declarative 77-constructor F1--F9 vocabulary with dual Rust/Lean code generators. The remaining three crates and both v1 schema placeholders stay scaffolds; WP-6 `ves-tokens` is next |
| C | `~/Desktop/ves-proof-lean` | `packages/` | `VesCore`, `VesModel`, `RustAssumptions`, `VesProtocol`, `DotnetRsProofs` | All five independent Lake package skeletons exist; Lean toolchain intentionally not selected |

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
- A scaffold marker means “ownership reserved,” not “API accepted.” Follow the
  ordered work packages in `sections/09b-decomposition.tex` and replace markers
  only when that package's implementation session begins.

## First implementation sessions

The repository-seed work is complete only in the narrow scaffolding sense.
Lane B has implemented `crates/ves-syntax` (WP-4, commit `9082659`) and
`crates/ves-vocabulary` (WP-5, commit `ad67707`); WP-6 `ves-tokens` is next.
Lane C is now unblocked: WP-13 (`packages/VesCore`) requires only WP-5, which
has landed, so it may start against the checked-in `generated/VesVocabulary.lean`
golden file without waiting on `ves-tokens`, `ves-macros`, or `ves-check`.
Lane A must not bootstrap the dependencies until `ves-tokens`, `ves-macros`,
and structural `ves-check` have real test coverage. Cross-lane synchronization
points and tier guidance are in the decomposition section of the study.
