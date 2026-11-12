Work in `/home/nick/Desktop/dotnet-rs` on branch `supervised/ide-inspections`. dotnet-rs is an ECMA-335 CLI runtime/VM in Rust. The workspace separates CLI/tests, VM/execution, VM operation traits/data, value/type/layout/GC utilities, assembly loading, runtime memory/resolution, intrinsic families, macros/build tools and benchmarks. See docs/ARCHITECTURE.md and Cargo.toml for ownership; rust-toolchain.toml owns the toolchain pin.

The goal is to investigate and meaningfully resolve RustRover general/Rust quality inspections: improve readability, maintainability and declarativity, **not move bad patterns around**. XML exports and `.descriptions.xml` are untracked at the repository root. Preserve them; never delete, move or commit them. Query problem XML with Python ElementTree, using file/highlighted_element/context, not offsets alone. REVIEW.md supplies a complete per-export ordinal ledger and aggregate duplicate classifications. The profile descriptions contain an XML-forbidden control byte; sanitize only in memory if parsing them. Feature coverage in an IDE export is incomplete.

CONTRIBUTING.md governs this work: use dotnet_utils::sync lock/atomic types; gate MT at item/module boundaries; every unsafe block/impl has a local `// SAFETY: F<n>.<Name> — ...` registry-backed proof; safe invariant reliance uses `// INVARIANT:`. Preserve unconditional Send/Sync reasoning and test guards. State facts once, describe the present, cite durable symbols in docs, and explain why in comments. Handle discoveries by fixing within this slice, recording Not yet planned in docs/plans/README.md, refuting with evidence, or asking only for consequential guest-behavior/public-compatibility/scope/safety decisions. Never weaken safety or gating for an IDE warning.

Plans 15/16 establish active Task/pool/timer and effects architecture; retain the deliberately kept multicast fallback. CHECKLIST.md defines ordered enabled slices for this refactor. Register the next free durable docs/plans plan and queue pointer as part of the first substantive slice; preserve its order/boundaries thereafter. Cite its durable slice section in execution records. REVIEW.md supports that plan, not a parallel architecture graph.

.NET SDK 10.0.114 and nextest were observed installed during kickoff. Recheck prerequisites if the environment changes. Managed fixtures require `cargo run -p xtask -- fixtures build`; then export `DOTNET_USE_PREBUILT_FIXTURES=1` for enabled test runs. DOTNET_SKIP_BUILD=1 is suitable for metadata/docs/lint checks, **not** evidence that managed integration worked. Read docs/CI.md, check.sh and the actual workflow/xtask matrix before choosing verification. Kickoff baseline utils no-feature all-target clippy passed; the full matrix was not run in planning. Std try_update exists on the pin, but Loom 0.7.2 lacks it; do not blindly rename facade calls.

{{TOOL_CAPABILITIES}}

## Assignment

Complete step {{STEP_ID}} — {{STEP_TITLE}}.

{{STEP_DESCRIPTION}}

Phase context: {{PHASE_NUMBER}} — {{PHASE_TITLE}}. Execute only the assigned flat-ID step, not every item shown for context.

## Phase items

{{CHECKLIST_ITEMS}}

## Review evidence for the assignment

{{REVIEW_SECTION}}

## Checklist context

{{CHECKLIST_SNAPSHOT}}

## Prior session memory

{{PRIOR_MEMORY}}

## Mandatory procedure

1. Read AGENT_MEMORY.md and the governing CONTRIBUTING.md, docs/plans/README.md and docs/CI.md. Preserve existing work and settled decisions. Read the registered refactor plan's assigned slice and linked subsystem/invariant contracts.
2. Inspect relevant code and verify review claims before acting. Investigate stale references, incomplete explanations and routine design questions; correct supporting records when useful and continue within authorized scope. XML line numbers and nearest-function labels are navigation aids, not authority.
3. Finish the assigned usable behavior, including callers, enabled tests, removals and directly affected docs. Decide reasonable mechanics yourself; do not ask users to reapprove authorized work. Keep unrelated improvements out. Confirm the result really improves readability, maintainability or declarativity rather than relocating a pattern; check each touched inspection under all relevant feature configurations. Every new path must be in use by the VM/application or enabled tests in this slice. No dormant support, private activation flags or deferred activation. Passing isolated helpers does not prove integration. Never put scratch names, finding IDs, checklist numbers or milestone codes in code, test names, ignore reasons or runtime messages; describe present invariants with durable symbols/spec references.
4. Implement, run appropriate checks, diagnose failures and fix regressions until acceptance holds. Use actual application/enabled-test paths where integration matters. Distinguish preexisting failures, unavailable checks, setup failures and regressions; never report these as passes or invent unrelated scope. A zero-match filter, unexpected ignore, timeout or failed fixture setup is not success.
5. Tick the assigned CHECKLIST.md box only when required behavior, integration, removals and verification are complete, not because a session ended or useful partial work happened. Update relevant REVIEW.md ledger dispositions and summary counts with actual outcomes and evidence; every original export entry remains accounted for.
6. Append one concise session entry to AGENT_MEMORY.md with changes, commands/outcomes and relevant limits/follow-ups; preserve earlier entries. Routine discoveries belong in implementation/explanation and are not reasons to stop.
7. Keep necessary support in this slice. Record unrelated findings in memory or docs/plans/README.md Not yet planned per policy; do not automatically add checklist items. Do not separately complete staging and integration halves. Changes to approved outcomes/scope/slice boundaries need an explicit plan decision. If an assumption fails, investigate authorized alternatives; propose a governing-plan revision rather than splitting a blocked step. newChecklistItems is a legacy proposal signal, not authority to change the plan.
8. Persist through actionable work. Complexity, effort, imperfect review and failing tests mean investigate and continue, not hand back the assignment. Block only when human intervention is necessary: missing authority/access, incompatible requirements or a consequential governing-plan decision that authorized alternatives cannot resolve. State evidence, attempts, exact intervention and recommendation. Respect explicit user stops without inventing a stop-on-uncertainty rule.
9. Call the actual runtime completion tool (`step_complete` in this API, sometimes written `step.complete`) as the **final action**, with one JSON object. Use completed only when finished. Pending is for an interrupted run or user-requested unfinished handoff, not routine partial delivery, early stopping or choosing supervisor commit timing. Leave unfinished steps unchecked and record the next concrete action. The supervisor owns commits and advancement. Report actual verification results: skipped/failed/unavailable are not pass. Example shape: `{"status":"completed","summary":"Implemented and verified the assigned behavior","verificationCommand":"<actual project command>","verificationResult":"pass","concerns":[]}`.

## Verification scope

Commands below are templates: substitute the affected crate and supported feature strings. For crate-scoped commands, do not pass a feature that the crate does not define; use its VM/CLI consumer for propagated configurations. The source of truth is docs/CI.md and `.github/workflows/ci.yml`, including SHARED_FEATURE_MATRIX in xtask, not this snapshot. Query it with `cargo run -p xtask -- matrix test-features --format lines`. Omit `--features` for the empty row. Build fixtures before tests needing managed assemblies and use DOTNET_USE_PREBUILT_FIXTURES=1. Run compile-fail doctests separately because nextest does not run doctests.

| Change | Narrowest correct checks and escalation |
|---|---|
| Formatting only | `cargo fmt --check`; confirm diff changes no behavior. |
| Comments/docs only | `bash scripts/check_doc_drift.sh`; `DOTNET_SKIP_BUILD=1 RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps --no-default-features`. No runtime/Miri merely for prose. |
| Single crate internal code | `cargo clippy -p <crate> --all-targets --no-default-features -- -D warnings`; repeat with supported `--features multithreading` and relevant validation combinations. After fixtures, `DOTNET_USE_PREBUILT_FIXTURES=1 cargo nextest run -p <crate> --no-default-features` and matching features; test dependent VM/CLI activation where relevant. Run `./check.sh --fast` once per coherent change per contribution policy. |
| cfg, MT, diagnostics or release-only paths | Query shared matrix and run affected crate/consumer clippy and tests on every relevant row, including no-feature paths. Optional bench-instrumentation, heap-diagnostics and deadlock-diagnostics guards are specified in check.sh --full. For release use `DOTNET_SKIP_BUILD=1 cargo clippy --release --all-targets --no-default-features -- -D warnings` and repeat with `--features multithreading` as CI clippy-release does. Run changed release behavior tests where compile-only cannot establish it. |
| Loom/synchronization | `DOTNET_SKIP_BUILD=1 RUSTFLAGS="--cfg loom" cargo nextest run -p dotnet-vm --test loom_stw --no-default-features`, plus focused native threading/event tests. Loom is a rustc cfg, not a Cargo feature. The STW model does not cover every production event/registry transition. Preserve facade backend compatibility. |
| Unsafe, raw memory, GC or synchronization primitives | Add focused enabled invariant regressions; run relevant native tests and Miri selectors from ci.yml (`miri-value`, `miri-frame-blocks`, `miri-owner-retention`, `miri-cross-arena`, pointer controls/batches). Set `NIGHTLY` from docs/CI.md and `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`. Examples: `cargo +"$NIGHTLY" miri test -p dotnet-value -- --test-threads=1`; `cargo +"$NIGHTLY" miri test -p dotnet-vm-data --lib --no-default-features frame_alloc:: -- --test-threads=1`. Pointer lane: `bash scripts/pointer_provenance.sh verify-controls --configuration no-features`, `bash scripts/pointer_provenance.sh batch --configuration multithreading --batch 2` (select actual relevant batch/control); cross-arena uses workflow selectors, not an invented generic suite. Run `bash scripts/check_provenance_lints.sh`, doc drift and Loom for sync changes. Never run known-UB ignored probes natively. If nightly/component unavailable, install the documented pin if authorized, otherwise record the exact unavailable lane and seek required access; native success is not a Miri pass. |
| SAFETY/INVARIANT or owning contract changes | Update matching docs/INVARIANT_REGISTRY.md rows in the same slice when predicates change; `bash scripts/check_doc_drift.sh` checks both citation directions. Run relevant runtime/safety lanes as well as prose checks; do not weaken markers to satisfy the script. |
| Cargo dependency edges | `cargo tree -p <crate> -e features` with relevant no-default/features variants, all-target clippy/build/test across full shared matrix and optional guards; include build dependencies, benches, macros and independent fuzz workspaces. Use cargo udeps/machete if installed, otherwise manual source/expansion/feature audit. No major upgrades authorized by this checklist. |
| Macros/build.rs/generated tables | `bash scripts/check_build_script_regressions.sh`, `cargo run -p xtask -- verify-slots`, generator tests and actual dependent-crate compilation/tests across feature rows. Edit generator/definitions, never OUT_DIR output. Fixtures require the SDK. Preserve the intentional __rerun_probe.rs fixture. |
| Workflow YAML | Check actual referenced action.yml inputs; actionlint if available and YAML parse with a parser that supports workflow syntax. Python PyYAML was absent at kickoff. Hosted expressions, permissions, artifacts and runners cannot be fully verified locally; do not dispatch workflows without authorization or call static validation a hosted pass. All original flagged inputs were verified declared. |
| Independent fuzz crates | Use explicit manifests or documented working directories in docs/FUZZING.md, not only workspace cargo checks. `cargo +"$NIGHTLY" check --manifest-path crates/dotnet-value/fuzz/Cargo.toml --all-targets` and VM fuzz equivalent; replay affected nonempty corpus using cargo-fuzz and ASAN_OPTIONS as FUZZING.md specifies. Example from crates/dotnet-value: `ASAN_OPTIONS=detect_leaks=0 cargo +"$NIGHTLY" fuzz run fuzz_managed_ptr_offset fuzz/corpus/fuzz_managed_ptr_offset -- -runs=0`; from crates/dotnet-vm/fuzz: corresponding fuzz_executor corpus/fuzz_executor replay. Prerequisites include cargo-fuzz and .NET runtime resolution. |
| Cross-cutting/closure | `./check.sh --fast` for each coherent change; `./check.sh --full` for closure, broad imports/dependencies or optional-feature-sensitive changes. These build fixtures, verify slots and run matrix clippy/nextest, but do not replace fmt, release clippy, Miri, Loom, doc/provenance lint or fuzz replay. Run those separately for touched paths. Reconcile every XML occurrence/duplicate group and record deferred work durably. |

## Closing instruction

Your very last action is the runtime `step_complete` call with truthful status and verification evidence. Do not emit text or invoke another tool afterward.
