# RustRover code-quality review

## Executive Summary

- **High:** raw-memory windows use a shared checked contract while preserving provenance and local proofs.
- **Medium:** exception construction, event transitions, allocation accounting and native-effect test observation use owning contracts across their callers.
- **Low:** qualification noise, assertion diagnostics and unnecessarily elaborate signatures/control flow obscure the important invariants. Fix selectively, preserving macro hygiene, named arena relationships and synchronization identity.
- **Informational:** workflow input warnings, detached fuzz/generated files and the intentional rerun probe are IDE limitations. Major dependency notices are separate migration work, not authorization to change GC or synchronization semantics.

The ledger preserves the original inspection dispositions. “Fix” denotes an authorized resolution; rows marked **resolved** have been implemented and verified as recorded below. All 29 duplicate-fix occurrences are resolved. Every exported problem has an individual ledger row; aggregate duplicate fragments are not counted twice. XML inputs are unchanged.

## Methodology

Read CONTRIBUTING.md and the plan queue before generating deliverables. Consulted plans 16 and 15, CI/check.sh/xtask, architecture, GC/memory, synchronization, delegates/dispatch, code generation, invariant registry and fuzz docs. Plans 15/16 constrain the refactor: active Task/pool/timer support and effects remain enabled; do not resurrect legacy continuation or yield-retry plumbing. Plan 16 is complete and deliberately retains a restricted multicast fallback.

Listed root XML exports, parsed all problem documents with Python ElementTree and counted by inspection/crate/file. Parsed all 32 aggregate duplicate groups and read both/all fragments with surrounding current source. Their 70 fragments correspond to the 70 DuplicatedCode entries. The hidden profile `.descriptions.xml` is not a diagnostic export: it contains an illegal ESC at line 1581. Its descriptions were parsed after removing XML-forbidden control bytes **in memory**, never modifying the input. Descriptions confirm that trait ordering, lifetime elision and batch annotator errors are distinct from compiler failures.

Reproduce counts without scratch files:

```python
import collections, glob, xml.etree.ElementTree as E
for name in sorted(glob.glob('*.xml')):
    problems = E.parse(name).findall('.//problem')
    paths = [p.findtext('file', '').removeprefix('file://$PROJECT_DIR$/') for p in problems]
    print(name, len(problems), collections.Counter(paths))
for n, group in enumerate(E.parse('DuplicatedCode_aggregate.xml').findall('duplicate'), 1):
    print(n, [fragment.attrib for fragment in group])
```

Enumerated every small-category site and inspected its surrounding source. Sampled qualifications and TLS across crates/high-density files rather than claiming a manual proof of all 371 occurrences. All entries nevertheless receive a policy-based decision in the ledger, with mandatory feature validation during implementation. Symbol names supplement snapshot lines; locate highlighted text and cfg context rather than trusting old offsets. Nearest preceding functions in the generated ledger are navigation aids, not AST ownership claims.

Inspected git log through d52b860c and blame on hot spots. Byte Min/Max decoding was added in 8bac30f9: its accepted integer variants and truncation are behavior, not accidental duplication. The rerun probe and regression script explicitly establish why an apparently detached source file exists.

Feature reasoning covers SHARED_FEATURE_MATRIX in xtask: no features, multithreading, simd, multithreading+simd, generic-constraint-validation, memory-validation, metadata-validation, multithreading+memory-validation, multithreading+validation-all, fuzzing; also bench-instrumentation/heap/deadlock diagnostics, release-only code, cfg(loom) and cfg(miri). These were considered, not all executed during planning. Item/module cfg boundaries and dotnet_utils::sync identity remain mandatory.

**Observed environment and baseline:** .NET SDK 10.0.114; pinned rustc 1.95.0; nextest 0.9.137; CI nightly-2026-05-27 installed. `cargo clippy -p dotnet-utils --all-targets --no-default-features -- -D warnings` passed. A stdin rustc type-check probe with metadata to stdout verified std atomic try_update on the pin; bundled std docs say stable since 1.95.0. An initial /dev/null-output probe failed because rustc tried to create a temporary directory in /dev, not because the API was unavailable. Installed loom 0.7.2 exposes fetch_update, not try_update. The same utils clippy check with multithreading also passed. `bash scripts/check_doc_drift.sh` passed (59 checks), and `DOTNET_SKIP_BUILD=1 RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps --no-default-features` passed. No full fixture/test/Miri run is claimed. cargo-machete was not found and Python PyYAML was unavailable. Workflow inputs were checked remotely with urllib and an inputs-block parser, not a full YAML execution. Some upstream CHANGELOG URLs returned 404; release APIs and crates.io metadata supplied additional evidence. RustRover itself was not rerun.

## Inspection disposition table

The generated table below partitions each problem into **fix / refute / keep / defer**. Keep means a reasoned readability/convention decision, not a new allow attribute. Exact entries and per-file tallies follow the findings. `.descriptions.xml` contains profile definitions and DuplicatedCode_aggregate.xml groups the same duplicate occurrences; neither adds problem counts.

| Inspection | Count | Crates/areas (count) | Fix / refute / keep / defer | Findings |
|---|---:|---|---|---|
| Annotator | 5 | dotnet-assemblies (2), dotnet-vm (3) | 0 / 5 / 0 / 0 | F-INFRA-001 |
| CargoUnusedDependency | 4 | dotnet-benchmarks (2), dotnet-cli (1), dotnet-value (1) | 4 / 0 / 0 / 0 | F-DEP-001 |
| DuplicatedCode | 70 | dotnet-intrinsics-core (2), dotnet-intrinsics-reflection (3), dotnet-intrinsics-span (8), dotnet-intrinsics-string (4), dotnet-simd (2), dotnet-utils (2), dotnet-value (1), dotnet-vm (48) | 29 (29 resolved) / 0 / 41 / 0 | F-DUP-001 / F-DUP-002 / F-DUP-003 / F-DUP-004 / F-DUP-005 |
| NewCrateVersionAvailable | 12 | Cargo.toml (3), dotnet-benchmarks (1), dotnet-macros (1), dotnet-macros-core (1), dotnet-value (1), dotnet-vm (4), xtask (1) | 0 / 0 / 0 / 12 | F-DEP-002 |
| RsAssertEqual | 19 | dotnet-value (17), dotnet-vm (2) | 19 (19 resolved) / 0 / 0 / 0 | F-IDIOM-002 |
| RsDeprecation | 4 | dotnet-utils (2), dotnet-vm (2) | 3 (3 resolved) / 0 / 1 / 0 | F-DEPR-001 |
| RsDetachedFile | 33 | dotnet-value (7), dotnet-vm (26) | 0 / 33 / 0 / 0 | F-INFRA-001 |
| RsLift | 4 | dotnet-assemblies (1), dotnet-vm (1), dotnet-vm-ops (2) | 4 (4 resolved) / 0 / 0 / 0 | F-IDIOM-002 |
| RsNeedlessLifetimes | 22 | dotnet-intrinsics-reflection (3), dotnet-intrinsics-string (1), dotnet-intrinsics-unsafe (2), dotnet-runtime-memory (3), dotnet-value (9), dotnet-vm (4) | 11 (11 resolved) / 0 / 11 / 0 | F-IDIOM-002 |
| RsSortImplTraitMembers | 11 | dotnet-benchmarks (1), dotnet-cli (3), dotnet-vm (6), xtask (1) | 0 / 0 / 11 / 0 | F-IDIOM-003 |
| RsThreadLocalStableMethodCanBeUsed | 75 | dotnet-benchmarks (1), dotnet-utils (2), dotnet-value (4), dotnet-vm (64), dotnet-vm-data (4) | 71 (71 resolved) / 0 / 4 / 0 | F-IDIOM-001 |
| RsUnnecessaryCast | 2 | dotnet-assemblies (2) | 2 (2 resolved) / 0 / 0 / 0 | F-IDIOM-002 |
| RsUnnecessaryParentheses | 4 | dotnet-utils (2), dotnet-vm-ops (2) | 4 (4 resolved) / 0 / 0 / 0 | F-IDIOM-002 |
| RsUnnecessaryQualifications | 296 | dotnet-benchmarks (1), dotnet-intrinsics-reflection (2), dotnet-intrinsics-string (2), dotnet-intrinsics-threading (2), dotnet-intrinsics-unsafe (4), dotnet-macros (2), dotnet-macros-core (11), dotnet-metrics (1), dotnet-pinvoke (4), dotnet-runtime-memory (4), dotnet-runtime-resolver (3), dotnet-tracer (2), dotnet-types (4), dotnet-utils (10), dotnet-value (23), dotnet-vm (209), dotnet-vm-data (1), dotnet-vm-ops (11) | 277 (277 resolved) / 0 / 19 / 0 | F-QUAL-001 |
| RsUnusedMut | 1 | dotnet-vm (1) | 0 / 1 / 0 / 0 | F-IDIOM-002 |
| UndefinedParamsPresent | 109 | .github (109) | 0 / 109 / 0 / 0 | F-CI-001 |


## Findings

<a id="F-DUP-001"></a>
#### F-DUP-001 — Intrinsic decoding and validated reflection slots

**Medium; high effort; resolved.** `byte_operand` defines the accepted `StackValue` representations once for `intrinsic_math_min_byte` and `intrinsic_math_max_byte`, retaining pop order, truncation and type-error text. `reflection_index_field` selects the validated typed slot for `read_reflection_index` and `write_reflection_index` by canonical wrapper name. The read retains its `Result` and construction retains its expectation; no field borrow is cached and the generated support-slot registry remains authoritative. RuntimeType, MethodInfo, ConstructorInfo and FieldInfo are exercised by an enabled CLI fixture.

Group 3 is parallel specialization: reflection Prim includes floats, CTS scalar kind includes native integers. Keep distinct domains instead of exporting a misleading common scalar enum. Groups 4–8 already use meaningful span/string host seams; aliases for error-return plumbing would only relocate text.

[Plan 17's intrinsic selection slice](docs/plans/17-inspection-quality-refactor.md#11-unify-intrinsic-operand-and-reflection-slot-selection) records the scope. Unit tests exercise truncation, type errors and pop order; CLI fixtures exercise byte extremes and all four wrappers. See AGENT_MEMORY.md for executed checks. Existing numeric width/sign semantics remain out of scope.

<a id="F-DUP-002"></a>
#### F-DUP-002 — Checked memory windows and frame-home codec selection

**High; high effort; resolved in code.** `checked_window` validates inline typed reads and
writes, slice copies and pointer-sized atomic slots, returning the original bounds payload even
on arithmetic overflow. It proves extent only; each raw access retains its own safety proof and
storage guard. `frame_home_access_layout` selects the full managed-pointer encoding for native-int
indirect access to a byref home; the actual read and write each resolve the live serial and
complete range separately.

Enabled VM tests cover inline, frame-home and static/transient byte windows, zero and exact-end
ranges, overflow payloads, retired serials and an IL-level indirect byref roundtrip. See
[plan 17's memory-window slice](docs/plans/17-inspection-quality-refactor.md#12-make-raw-memory-windows-and-byref-layout-selection-explicit)
and AGENT_MEMORY.md for verification and limits. Both strict release-clippy configurations pass.
The independently discovered heap byte-read overflow is recorded under Not yet planned; it is
not an inline-storage window.

<a id="F-DUP-003"></a>
#### F-DUP-003 — Exception field initialization and lifecycle operations

**Medium; high effort; resolved.** `VesContext::initialize_exception_string` shares string
allocation, registration and optional field writes across synchronous throws and native effects,
without merging their control protocols. A present parameter-name field is checked as a reference
slot; the typed write avoids the existing byte-vector alignment assertion. Empty messages remain
unset. `Executor::publish_pending_finalizers` is called at both original lifecycle positions.
`object_write_error` preserves stale-home host errors and AccessViolation messages for all store
and initialization paths, while `address_error` retains InvalidProgram versus host-error mapping.
The unsafe writes remain separate. Group 24's monitor polls remain distinct because readiness can
change between them. Enabled sync/effect collection, optional-field, address, object-write and
teardown tests exercise the contracts; execution evidence is in the session memory.

<a id="F-DUP-004"></a>
#### F-DUP-004 — Event transitions and allocation accounting

**Medium; high effort; resolved.** `State::expire_waiters` and
`State::deliver_auto_reset_signal` run under the existing event lock at both Set and canceled-grant
handoff. Waiters expired before selection cannot receive a new grant, while existing grants survive
deadlines and Reset; an auto-reset signal goes to the first live waiter or remains pending.
`VesContext::account_value_type_write` centralizes the three stack-slot write
charges, leaving frame-home writes uncharged and keeping metric classification aligned with GC debt.
Enabled executor cancellation, timeout ordering and stack-write tests cover the transitions and
per-write accounting; manual-reset coverage remains enabled. The Loom STW model is separate from
production event coverage. See [plan 17's event/accounting slice](docs/plans/17-inspection-quality-refactor.md#14-centralize-event-transitions-and-value-type-allocation-accounting)
and AGENT_MEMORY.md for execution evidence.

<a id="F-DUP-005"></a>
#### F-DUP-005 — Keep parallel specializations and explicit test premises

**Low; default effort for disposition; no rewrite.** The group inventory classifies every retained pair. Architecture-local SIMD scalar tails have target-feature and local safety premises; genericizing tiny unsafe tails obscures them. Cross-arena lease tests and executor collection/teardown/fault tests intentionally expose root ownership, arena lifetime and scheduling. Generic fixture helpers for a few lines could hide what each scenario proves. Keep explicit fixture boilerplate and per-operand uses of existing APIs. No blanket duplicate-code suppression.

Verification: reconcile the inventory against all aggregate fragments, retain enabled tests and safety markers. A text-similarity refutation does not establish a new runtime pass. Future subsystem redesign can introduce a real fixture vocabulary when independently justified.

<a id="F-IDIOM-001"></a>
#### F-IDIOM-001 — Thread-local access reveals a boundary observation protocol

**Resolved.** `BoundaryObservation` owns the per-thread reset, activation numbering, resume and drop events, and snapshots in `native_substring_tests`. Numbered witnesses preserve child-before-parent drop order; the synchronous-throw witness counts its drop without adding an order entry. Enabled tests check thread isolation, Rust unwind cleanup and VM exception/abort timing.

Trivial `LocalKey<Cell>` operations use `get`, `set`, or `replace`; managed-thread guards restore the previous value on unwind. The four retained read-modify-write closures (`LIVE_BLOCKS`, `COLLECTION_LOCK_DEBUG_DEPTH`, and `INLINE_CALLS`) keep the operation within one TLS access; borrow-based hooks retain their existing reentrancy boundaries. See the [execution record](AGENT_MEMORY.md) for feature and test evidence.

<a id="F-IDIOM-002"></a>
#### F-IDIOM-002 — Better assertions and simpler signatures/control flow

**Resolved.** All 19 value comparisons use `assert_eq!` or `assert_ne!`; custom messages, both directional equality witnesses, and operand evaluation are retained. The two receiver casts are removed while `instantiation_data` still receives its reference-shaped resolver argument. Redundant arithmetic parentheses are removed. PE classification, nullable unboxing and native-effect branches return their simple match/if expressions; the same exception rooting, nullable construction, claim completion, and early exits remain.

Eleven uninformative lifetime names are elided on byte/string/size queries, raw-address observation and independent span-input methods. Named arena input/output conversions, pointer offset and decode relationships, and the short-lifetime witness remain named. Elision does not imply `'static` or relax arena branding. The existing `bit_index` mutation remains because the following `record_bit!` expansion increments it; its ledger row records the source evidence.

**Refute RsUnusedMut:** VM pointer_provenance_tests.rs:2346 bit_index is incremented by following record_bit! expansion. Removing mut breaks enabled compilation. No allow required.

Risk: low for syntax, moderate for future/control-flow/lifetime edits. Verify full-feature clippy/tests and applicable compile-fail doctests for signature changes. No broad autofix without cfg inspection.

<a id="F-IDIOM-003"></a>
#### F-IDIOM-003 — Semantic implementation grouping beats literal trait order

**Low; keep all 11; default effort.** CLI Visit impls at fixtures.rs:89,185 and reflection_tail.rs:95 group required record_debug with meaningful record_str; managed_ptr_serde.rs:100 groups resolver homes. VM call/context/exception/native/raw-memory impls group operations by execution concern; xtask's slot visitor prioritizes relevant accessor calls. Trait-declaration order is not a correctness rule. Reordering large impls would churn structurally edited files without clearer behavior. Preserve grouping, no suppression attributes. Verification is final disposition plus normal fmt/clippy for substantively changed files.

<a id="F-QUAL-001"></a>
#### F-QUAL-001 — Imports should expose the domain, not maximize short names

**Resolved.** Recurring runtime, metadata, pointer and test-domain names use explicit imports at their owning module or test boundary; the raw-memory implementation's imports remain scoped. The sweep adds no wildcard imports or lint allowances. The standard-atomic changes retain the standard types, and runtime lock/atomic imports continue to identify `dotnet_utils::sync` explicitly.

Nineteen qualifications remain for stated reasons: syntax-tree paths in macro/core code preserve expansion context; three parent-qualified lock-order traits make the bound's owner clear; two `Ordering` paths identify the sync facade; and the stack-module intra-doc target resolves the intended `ExecutionEngine` link. These decisions are per-entry in the ledger. Generic names remain unqualified only where a specific import makes their domain visible.

Cfg-sensitive test imports are gated at module/item boundaries. Clippy and nextest pass every shared row. Under fuzzing, native-dependent fixtures are skipped and the reflection path test retains its other cases; those fixtures pass without fuzzing. Optional diagnostic/instrumentation guards pass. The full check's separate async build-script probe refers to absent VM API methods and is recorded under Not yet planned; this sweep changes no macro output. See AGENT_MEMORY.md for the validation record.

Existing workspace clippy denies undocumented unsafe blocks and multiple unsafe ops per block. Rust warnings already cover unused mut/parentheses; clippy covers needless lifetimes/unnecessary casts. bool_assert_comparison is closer to comparison assertions than manual_assert, which concerns panic branches. No new global unused_qualifications, needless_lifetimes or pedantic deny is recommended without a full-matrix clean run and no noisy allows. The planning baseline is insufficient evidence for new lint policy.

<a id="F-DEPR-001"></a>
#### F-DEPR-001 — try_update is stable on the pin, not on all backends

**Resolved.** The two checked-counter controls in `dotnet-utils` and `threading/local::next_managed_id` use standard-atomic `try_update`, preserving checked-add overflow and exhaustion behavior. The runtime allocation path `threading/basic::allocate_managed_thread_id` retains the facade's `fetch_update`: `dotnet_utils::sync` reexports Loom atomics under `cfg(loom)`, and Loom 0.7.2 has no `try_update`. Its 1..=65535 bound and backend identity remain unchanged.

The other standard production atomics remain separate from this method rename; changing their backend is a synchronization-model decision. The individual ledger rows distinguish the three standard sites from the retained Loom-compatible facade site.

<a id="F-DEP-001"></a>
#### F-DEP-001 — Remove unused direct dependency edges, preserve feature routes

**Low; default effort.** Searches show no tracing/tracing-subscriber uses in benchmark source/benches, no lru use in value, and no dotnet_value use in CLI source/tests/build script. Real LruCache consumers are types::generics and vm::cache. Remove four unused direct edges after checking macro expansions and forwarding lists. Never remove workspace tracing/lru with real consumers, or add fake uses to pacify the IDE.

Compare cargo tree -e features before/after: CLI feature propagation must continue through VM/actual consumers, including validation/fuzzing. Verification: shared matrix all-target builds/clippy/tests, optional flags, benches, standalone fuzz closure and feature trees. CLI tests require prebuilt fixtures. udeps/machete if available supplement, not replace, cfg inspection. Risk: workspace feature unification can mask missing direct dependencies.

<a id="F-DEP-002"></a>
#### F-DEP-002 — Major dependency migrations are scoped follow-ups

**Medium; defer all 12; no upgrade step.** crates.io metadata confirms advertised major lines exist, but compatible MSRV does not imply API or safety compatibility. Register grouped Not yet planned follow-ups with these manifest families/evidence in the dependency-removal slice. Declining unrelated major migrations is a routine scope decision; no user question is needed.

| Dependency | Upstream evidence and repository surface | Why separate |
|---|---|---|
| gc-arena 0.7 | [Changelog](https://github.com/kyren/gc-arena/blob/master/CHANGELOG.md) fetched; Gc/Collect/opaque transport and both fuzz manifests | Direct T pointer representation and pacing change from shallow bytes to pointer counts require safety/pacing/performance review. 0.6.1's equal-alignment cast requirement merits a focused audit, not a blind version bump. |
| syn 3 | [Releases API](https://api.github.com/repos/dtolnay/syn/releases), crates.io metadata; CHANGELOG URL returned 404; macros/core, VM build.rs, xtask visitor | Major AST/parser migration needs emitted-code, signature-scan and slot-consumer validation; source compatibility not established. |
| parking_lot 0.13 | [Changelog](https://github.com/Amanieu/parking_lot/blob/master/CHANGELOG.md) fetched; sync facade, lock order and GC locks | Per-acquisition raw guard state, fairness and recursive-lock changes affect synchronization semantics despite compatible MSRV. |
| phf/phf_codegen 0.14 | [Release](https://github.com/rust-phf/rust-phf/releases/tag/v0.14.0), registry metadata; VM runtime and build generator | Coupled table/runtime formats must migrate together and pass generation/dispatch probes. |
| lru 0.19 | [Changelog](https://github.com/jeromefroe/lru-rs/blob/master/CHANGELOG.md) fetched; types generic cache, VM cache | Shared iterator Sync requirements need trait review; unused value edge removal is independent. |
| criterion 0.8 | [Releases](https://github.com/criterion-rs/criterion.rs/releases), registry metadata and changelog request; benchmarks | Multiple minor API generations from 0.5 plus harness/measurement comparability; separate smoke/measurement validation. |

These are dated investigation sources, not normative version policy. No lockfile migration is authorized here. Future GC/sync upgrades require relevant Miri/Loom and safety arguments in addition to compilation.

<a id="F-CI-001"></a>
#### F-CI-001 — Every flagged key is a declared action input

**Informational; refute all 109; default effort.** Mapped every XML line in ci.yml/miri.yml/fuzz.yml/valgrind.yml to its preceding uses action, fetched `https://raw.githubusercontent.com/<owner>/<action>/<ref>/action.yml`, and compared flagged keys against its inputs block. Zero unknown keys. The workflow references are mutable tags/branches, not presumed SHA pins.

| Actual reference | Flagged input counts |
|---|---|
| actions/checkout@v4 | fetch-depth 1 |
| dtolnay/rust-toolchain@stable | components 4 |
| dtolnay/rust-toolchain@master | toolchain 14, components 9 |
| actions/setup-dotnet@v4 | dotnet-version 15 |
| actions/cache@v4 | path 4, key 4, restore-keys 2 |
| actions/upload-artifact@v4 | name 12, path 12, include-hidden-files 1, retention-days 11, if-no-files-found 11 |
| actions/download-artifact@v4 | name 1, path 1 |
| Swatinem/rust-cache@v2.8.2 | key 3, workspaces 2, shared-key 2 |

This is schema/resolution noise; do not remove functioning with inputs or add per-line suppressions. The comparison verifies spelling, not authentication, expression evaluation, branch protection or hosted execution. Recheck actual references if workflows later change. Final closure retains valid configuration.

<a id="F-INFRA-001"></a>
#### F-INFRA-001 — Independent fuzz workspaces and intentional/generated files

**Informational; refute; default effort.** Both crates/dotnet-{value,vm}/fuzz/Cargo.toml have their own workspace root, cargo-fuzz metadata and explicit bin targets. Four detached reports are those binaries, 28 are generated target output. Attach manifests in the IDE; exclude target/obj rather than adding runtime module declarations or merging workspaces.

The remaining report is `crates/dotnet-vm/src/intrinsics/__rerun_probe.rs:1`, explicitly a tracked code-free build-script fixture. scripts/check_build_script_regressions.sh at :122 preserves it while creating a distinct temporary probe. Keep it; deletion would violate the test contract and registering an empty module adds nothing.

All five Annotator reports are generated/non-Rust material: two support/obj/project.packagespec.json diagnostics and three libffi texinfo errors under fuzz target output. No source doc syntax defect was demonstrated. Never edit vendor/generated output. Optional durable IDE guidance belongs in contributor/fuzz docs, not committed machine-local settings.

The [registered plan](docs/plans/17-inspection-quality-refactor.md) defines the ordered slices and enabled-path exit conditions. This review is evidence, not a parallel architecture plan; plans 15/16 remain constraints. Closure reconciles the plan, marks its Status only on evidence and records dependency/std-atomic follow-ups.

## Duplicate group inventory

Group numbers mean XML document order; hash/cost are not unique identities. They are review evidence labels, never production/test/comment names. Each group lists all current source fragments with snapshot lines and nearby symbols.

### Aggregate group 1 — fix (resolved)

Shared byte-compatible decoding is used by both distinct Min/Max operations.

- `crates/dotnet-intrinsics-core/src/math.rs:581` — `intrinsic_math_min_byte`.
- `crates/dotnet-intrinsics-core/src/math.rs:632` — `intrinsic_math_max_byte`.

### Aggregate group 2 — fix (resolved)

Canonical reflection wrapper/index-slot selection is shared while preserving read/write failure contracts.

- `crates/dotnet-intrinsics-reflection/src/common.rs:100` — `read_reflection_index`.
- `crates/dotnet-intrinsics-reflection/src/common.rs:135` — `write_reflection_index`.

### Aggregate group 3 — keep

Parallel specialization: Prim includes floats; CTS kinds include native integers. Keep separate domains.

- `crates/dotnet-intrinsics-reflection/src/invoke_args.rs:45` — `from_base`.
- `crates/dotnet-value/src/cts_cli_conversion.rs:58` — `from_enum_underlying`.

### Aggregate group 4 — keep

Parallel error propagation through existing read_span_reference_from_ptr; keep operand-specific plumbing, no alias helper.

- `crates/dotnet-intrinsics-span/src/conversions.rs:598` — `intrinsic_span_get_pinnable_reference`.
- `crates/dotnet-intrinsics-span/src/conversions.rs:656` — `intrinsic_memory_marshal_get_reference`.
- `crates/dotnet-intrinsics-span/src/equality.rs:327` — `intrinsic_span_get_item`.

### Aggregate group 5 — keep

Parallel error propagation through existing read_span_length_from_ptr; keep, no new concept to extract.

- `crates/dotnet-intrinsics-span/src/conversions.rs:602` — `intrinsic_span_get_pinnable_reference`.
- `crates/dotnet-intrinsics-span/src/conversions.rs:730` — `intrinsic_span_slice`.
- `crates/dotnet-intrinsics-span/src/equality.rs:318` — `intrinsic_span_get_item`.

### Aggregate group 6 — keep

Parallel a/b operands through existing span-reference API; keep pairing/error order explicit.

- `crates/dotnet-intrinsics-span/src/equality.rs:176` — `intrinsic_memory_extensions_sequence_equal`.
- `crates/dotnet-intrinsics-span/src/equality.rs:182` — `intrinsic_memory_extensions_sequence_equal`.

### Aggregate group 7 — keep

Parallel string span-length observation using existing host seam; keep rather than rename one expression.

- `crates/dotnet-intrinsics-string/src/constructors.rs:62` — `readonly_span_to_vec`.
- `crates/dotnet-intrinsics-string/src/operations.rs:449` — `intrinsic_string_concat_three_spans`.

### Aggregate group 8 — keep

Parallel native selectors with independent thresholds/operation policy; keep.

- `crates/dotnet-intrinsics-string/src/operations.rs:526` — `string_hash_select`.
- `crates/dotnet-intrinsics-string/src/search.rs:708` — `whitespace_select`.

### Aggregate group 9 — keep

Parallel architecture specialization: keep short scalar tails and co-located unsafe proofs.

- `crates/dotnet-simd/src/aarch64.rs:67` — `copy_nonoverlapping_neon`.
- `crates/dotnet-simd/src/x86.rs:90` — `copy_nonoverlapping_sse2`.

### Aggregate group 10 — keep

Test boilerplate: scenario-local cross-arena writer locks; keep synchronization premises visible.

- `crates/dotnet-utils/src/gc/cross_arena.rs:1128` — `stress_arena_lifecycle_concurrency`.
- `crates/dotnet-utils/src/gc/cross_arena.rs:1233` — `stress_stw_flag_transitions_with_lease_acquisition`.

### Aggregate group 11 — keep

Test hook boilerplate: distinct wait phases/reentrancy; keep without generic callback framework.

- `crates/dotnet-vm/src/executor.rs:1065` — `run_inner`.
- `crates/dotnet-vm/src/threading/basic.rs:323` — `register_thread`.

### Aggregate group 12 — fix (resolved)

`Executor::publish_pending_finalizers` preserves both publication positions.

- `crates/dotnet-vm/src/executor.rs:1379` — `unregister`.
- `crates/dotnet-vm/src/executor.rs:1398` — `drop`.

### Aggregate group 13 — keep

Test boilerplate: root encoding from scenario-specific owner arena; keep.

- `crates/dotnet-vm/src/executor_collection_tests.rs:118` — `run_vm_collection`.
- `crates/dotnet-vm/src/executor_collection_tests.rs:322` — `spawn_owner`.

### Aggregate group 14 — keep

Test boilerplate: explicit decoding/rooting with local lifetime premises; keep.

- `crates/dotnet-vm/src/executor_collection_tests.rs:150` — `run_vm_collection`.
- `crates/dotnet-vm/src/executor_collection_tests.rs:398` — `spawn_owner`.

### Aggregate group 15 — keep

Test boilerplate: replacement/foreign identity witnesses need explicit live arena evidence; keep.

- `crates/dotnet-vm/src/executor_collection_tests.rs:555` — `arena_lifetime_mark_command_rejects_stale_registration`.
- `crates/dotnet-vm/src/executor_collection_tests.rs:577` — `arena_lifetime_mark_command_rejects_stale_registration`.

### Aggregate group 16 — keep

Test boilerplate: retired-worker setup keeps thread ownership observable; keep.

- `crates/dotnet-vm/src/executor_teardown_tests.rs:141` — `retired_shutdown_waits_for_inflight_collection_and_lease`.
- `crates/dotnet-vm/src/executor_teardown_tests.rs:1208` — `retired_owner_stays_registered_while_another_owner_finalizes`.

### Aggregate group 17 — keep

Test boilerplate: entrypoint frame setup with local GC witness; keep, no generic unsafe fixture framework.

- `crates/dotnet-vm/src/fault_tests.rs:120` — `run_fault_entrypoint`.
- `crates/dotnet-vm/src/tail_calls_tests.rs:178` — `run_tail_chain_and_measure_max_depth`.

### Aggregate group 18 — keep

Test boilerplate: local return-value observation in distinct instruction scenarios; keep.

- `crates/dotnet-vm/src/fault_tests.rs:155` — `run_fault_entrypoint`.
- `crates/dotnet-vm/src/jmp_tests.rs:178` — `test_jmp_instruction`.

### Aggregate group 19 — keep

Test boilerplate: constructor metadata in distinct fault scenarios; keep local IL construction.

- `crates/dotnet-vm/src/fault_tests.rs:255` — `test_fault_handler_executed_on_exception`.
- `crates/dotnet-vm/src/fault_tests.rs:352` — `test_fault_inside_finally_resumes_outer_handler`.
- `crates/dotnet-vm/src/fault_tests.rs:440` — `test_fault_inside_finally_with_pending_exception`.

### Aggregate group 20 — fix (resolved)

`object_write_error` translates stale-home and other failures for both writes; unsafe operations stay separate.

- `crates/dotnet-vm/src/instructions/objects/mod.rs:368` — `stobj`.
- `crates/dotnet-vm/src/instructions/objects/mod.rs:410` — `initobj`.

### Aggregate group 21 — fix (resolved)

`address_error` maps layout failures consistently without changing argument/local lookup.

- `crates/dotnet-vm/src/instructions/stack_ops.rs:81` — `ldarga`.
- `crates/dotnet-vm/src/instructions/stack_ops.rs:115` — `ldloca`.

### Aggregate group 22 — fix (resolved)

`initialize_exception_string` registers and writes optional parameter names for both producers.

- `crates/dotnet-vm/src/stack/exception_ops_impl.rs:67` — `throw_argument_out_of_range`.
- `crates/dotnet-vm/src/stack/native_effects.rs:577` — `exception`.

### Aggregate group 23 — fix (resolved)

`initialize_exception_string` shares message construction and skips empty or absent fields.

- `crates/dotnet-vm/src/stack/exception_ops_impl.rs:95` — `throw_by_name_with_message`.
- `crates/dotnet-vm/src/stack/native_effects.rs:563` — `exception`.

### Aggregate group 24 — keep

Parallel temporal readiness phases: keep both polls to close state-change races.

- `crates/dotnet-vm/src/stack/native_effects.rs:1736` — `advance_monitor_wait`.
- `crates/dotnet-vm/src/stack/native_effects.rs:1748` — `advance_monitor_wait`.

### Aggregate group 25 — fix (resolved)

`checked_window` supplies the common extent check; reads and writes retain separate safety premises.

- `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:33` — `write_to_inline_storage`.
- `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:63` — `read_from_inline_storage`.

### Aggregate group 26 — fix (resolved)

Both slice directions use the checked range and preserve the BoundsCheck payload.

- `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:86` — `bounds_checked_slice_copy`.
- `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:102` — `bounds_checked_slice_read`.

### Aggregate group 27 — fix (resolved)

`frame_home_access_layout` selects the byref codec for both access directions.

- `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:164` — `write_unaligned`.
- `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:261` — `read_unaligned`.

### Aggregate group 28 — fix (resolved)

Pointer-sized slots use `checked_window` while retaining their storage guards.

- `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:487` — `object_atomic`.
- `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:518` — `object_atomic`.

### Aggregate group 29 — fix (resolved)

All three stack-slot writes use `account_value_type_write`; homed writes retain their separate path.

- `crates/dotnet-vm/src/stack/stack_ops_impl.rs:106` — `set_local`.
- `crates/dotnet-vm/src/stack/stack_ops_impl.rs:297` — `set_argument`.
- `crates/dotnet-vm/src/stack/stack_ops_impl.rs:335` — `set_slot`.

### Aggregate group 30 — fix (resolved)

`State::expire_waiters` marks eligible deadlines at both locked transition sites.

- `crates/dotnet-vm/src/threading/events.rs:65` — `set`.
- `crates/dotnet-vm/src/threading/events.rs:197` — `drop`.

### Aggregate group 31 — fix (resolved)

`State::deliver_auto_reset_signal` grants the first eligible waiter or retains the signal.

- `crates/dotnet-vm/src/threading/events.rs:77` — `set`.
- `crates/dotnet-vm/src/threading/events.rs:202` — `drop`.

### Aggregate group 32 — keep

Test boilerplate: explicit RegistryState scenario fixtures; keep without lifecycle factory.

- `crates/dotnet-vm/src/threading/workers.rs:544` — `completed_outcomes_remain_consumable_after_retirement`.
- `crates/dotnet-vm/src/threading/workers.rs:586` — `abort_prevents_retired_worker_from_starting_finalizers`.
- `crates/dotnet-vm/src/threading/workers.rs:614` — `finalizing_preserves_managed_completion`.
- `crates/dotnet-vm/src/threading/workers.rs:648` — `shutdown_requires_retirement_and_records_failed_drain`.



## Complete export ledger and per-file tallies

Each row is one exported problem, identified by inspection and one-based XML ordinal. Parenthesis tokens at one site remain separate rows. Snapshot lines and nearest preceding functions are navigation aids. Generated output may be absent. Qualification choices must pass cfg checks; if current enabled-code evidence refutes a planned shortening, update that ledger decision and the summary partition explicitly. Nothing is silently dropped.

### Annotator — 5 entries

**Per-file tally:** `crates/dotnet-assemblies/src/support/obj/project.packagespec.json` (2); `crates/dotnet-vm/fuzz/target/debug/build/libffi-sys-457cb2393b93073d/out/libffi-build/doc/texinfo.tex` (1); `crates/dotnet-vm/fuzz/target/debug/build/libffi-sys-e9433e4b55dc4387/out/libffi-build/doc/texinfo.tex` (1); `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/libffi-sys-5009394cc4b3e651/out/libffi-build/doc/texinfo.tex` (1)

| Entry | Snapshot site and nearby symbol | Decision | Reason / finding |
|---|---|---|---|
| 1 | `crates/dotnet-assemblies/src/support/obj/project.packagespec.json:1` — `(module context)` | refute | F-INFRA-001; `:` |
| 2 | `crates/dotnet-assemblies/src/support/obj/project.packagespec.json:1` — `(module context)` | refute | F-INFRA-001; `:` |
| 3 | `crates/dotnet-vm/fuzz/target/debug/build/libffi-sys-457cb2393b93073d/out/libffi-build/doc/texinfo.tex:78` — `(module context)` | refute | F-INFRA-001; `\end` |
| 4 | `crates/dotnet-vm/fuzz/target/debug/build/libffi-sys-e9433e4b55dc4387/out/libffi-build/doc/texinfo.tex:78` — `(module context)` | refute | F-INFRA-001; `\end` |
| 5 | `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/libffi-sys-5009394cc4b3e651/out/libffi-build/doc/texinfo.tex:78` — `(module context)` | refute | F-INFRA-001; `\end` |

### CargoUnusedDependency — 4 entries

**Per-file tally:** `crates/dotnet-benchmarks/Cargo.toml` (2); `crates/dotnet-cli/Cargo.toml` (1); `crates/dotnet-value/Cargo.toml` (1)

| Entry | Snapshot site and nearby symbol | Decision | Reason / finding |
|---|---|---|---|
| 1 | `crates/dotnet-benchmarks/Cargo.toml:44` — `(module context)` | fix | F-DEP-001; `tracing = "0.1"` |
| 2 | `crates/dotnet-benchmarks/Cargo.toml:45` — `(module context)` | fix | F-DEP-001; `tracing-subscriber = "0.3"` |
| 3 | `crates/dotnet-cli/Cargo.toml:40` — `(module context)` | fix | F-DEP-001; `dotnet-value = { workspace = true }` |
| 4 | `crates/dotnet-value/Cargo.toml:14` — `(module context)` | fix | F-DEP-001; `lru = { workspace = true }` |

### DuplicatedCode — 70 entries

**Per-file tally:** `crates/dotnet-intrinsics-core/src/math.rs` (2); `crates/dotnet-intrinsics-reflection/src/common.rs` (2); `crates/dotnet-intrinsics-reflection/src/invoke_args.rs` (1); `crates/dotnet-intrinsics-span/src/conversions.rs` (4); `crates/dotnet-intrinsics-span/src/equality.rs` (4); `crates/dotnet-intrinsics-string/src/constructors.rs` (1); `crates/dotnet-intrinsics-string/src/operations.rs` (2); `crates/dotnet-intrinsics-string/src/search.rs` (1); `crates/dotnet-simd/src/aarch64.rs` (1); `crates/dotnet-simd/src/x86.rs` (1); `crates/dotnet-utils/src/gc/cross_arena.rs` (2); `crates/dotnet-value/src/cts_cli_conversion.rs` (1); `crates/dotnet-vm/src/executor.rs` (3); `crates/dotnet-vm/src/executor_collection_tests.rs` (6); `crates/dotnet-vm/src/executor_teardown_tests.rs` (2); `crates/dotnet-vm/src/fault_tests.rs` (5); `crates/dotnet-vm/src/instructions/objects/mod.rs` (2); `crates/dotnet-vm/src/instructions/stack_ops.rs` (2); `crates/dotnet-vm/src/jmp_tests.rs` (1); `crates/dotnet-vm/src/stack/exception_ops_impl.rs` (2); `crates/dotnet-vm/src/stack/native_effects.rs` (4); `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs` (8); `crates/dotnet-vm/src/stack/stack_ops_impl.rs` (3); `crates/dotnet-vm/src/tail_calls_tests.rs` (1); `crates/dotnet-vm/src/threading/basic.rs` (1); `crates/dotnet-vm/src/threading/events.rs` (4); `crates/dotnet-vm/src/threading/workers.rs` (4)

| Entry | Snapshot site and nearby symbol | Decision | Reason / finding |
|---|---|---|---|
| 1 | `crates/dotnet-intrinsics-core/src/math.rs:581` — `intrinsic_math_min_byte` | fix (resolved) | Aggregate group 1 below/above; `let pop_byte = \|ctx: &mut T\| match ctx.pop() {` |
| 2 | `crates/dotnet-intrinsics-core/src/math.rs:632` — `intrinsic_math_max_byte` | fix (resolved) | Aggregate group 1 below/above; `let pop_byte = \|ctx: &mut T\| match ctx.pop() {` |
| 3 | `crates/dotnet-intrinsics-reflection/src/common.rs:100` — `read_reflection_index` | fix (resolved) | Aggregate group 2 below/above; `{` |
| 4 | `crates/dotnet-intrinsics-reflection/src/common.rs:135` — `write_reflection_index` | fix (resolved) | Aggregate group 2 below/above; `{` |
| 5 | `crates/dotnet-intrinsics-reflection/src/invoke_args.rs:45` — `from_base` | keep | Aggregate group 3 below/above; `Some(match base {` |
| 6 | `crates/dotnet-intrinsics-span/src/conversions.rs:598` — `intrinsic_span_get_pinnable_reference` | keep | Aggregate group 4 below/above; `let managed_ref = match read_span_reference_from_ptr(&span, f, ctx.loader().as_ref(), ctx)` |
| 7 | `crates/dotnet-intrinsics-span/src/conversions.rs:602` — `intrinsic_span_get_pinnable_reference` | keep | Aggregate group 5 below/above; `let length = match read_span_length_from_ptr(&span, f, ctx.loader().as_ref(), ctx) {` |
| 8 | `crates/dotnet-intrinsics-span/src/conversions.rs:656` — `intrinsic_memory_marshal_get_reference` | keep | Aggregate group 4 below/above; `let managed =` |
| 9 | `crates/dotnet-intrinsics-span/src/conversions.rs:730` — `intrinsic_span_slice` | keep | Aggregate group 5 below/above; `let length =` |
| 10 | `crates/dotnet-intrinsics-span/src/equality.rs:176` — `intrinsic_memory_extensions_sequence_equal` | keep | Aggregate group 6 below/above; `let a_ptr_info = match read_span_reference(&a, ctx, ctx.loader().as_ref()) {` |
| 11 | `crates/dotnet-intrinsics-span/src/equality.rs:182` — `intrinsic_memory_extensions_sequence_equal` | keep | Aggregate group 6 below/above; `let b_ptr_info = match read_span_reference(&b, ctx, ctx.loader().as_ref()) {` |
| 12 | `crates/dotnet-intrinsics-span/src/equality.rs:318` — `intrinsic_span_get_item` | keep | Aggregate group 5 below/above; `let len = match read_span_length_from_ptr(` |
| 13 | `crates/dotnet-intrinsics-span/src/equality.rs:327` — `intrinsic_span_get_item` | keep | Aggregate group 4 below/above; `let raw_ref_ptr = match read_span_reference_from_ptr(` |
| 14 | `crates/dotnet-intrinsics-string/src/constructors.rs:62` — `readonly_span_to_vec` | keep | Aggregate group 7 below/above; `let len = ctx` |
| 15 | `crates/dotnet-intrinsics-string/src/operations.rs:449` — `intrinsic_string_concat_three_spans` | keep | Aggregate group 7 below/above; `let len = ctx` |
| 16 | `crates/dotnet-intrinsics-string/src/operations.rs:526` — `string_hash_select` | keep | Aggregate group 8 below/above; `if let StackValue::ObjectRef(ObjectRef(Some(handle))) = operands.peek_at(0) {` |
| 17 | `crates/dotnet-intrinsics-string/src/search.rs:708` — `whitespace_select` | keep | Aggregate group 8 below/above; `if let StackValue::ObjectRef(ObjectRef(Some(handle))) = operands.peek_at(0) {` |
| 18 | `crates/dotnet-simd/src/aarch64.rs:67` — `copy_nonoverlapping_neon` | keep | Aggregate group 9 below/above; `if offset < len {` |
| 19 | `crates/dotnet-simd/src/x86.rs:90` — `copy_nonoverlapping_sse2` | keep | Aggregate group 9 below/above; `if offset < len {` |
| 20 | `crates/dotnet-utils/src/gc/cross_arena.rs:1128` — `stress_arena_lifecycle_concurrency` | keep | Aggregate group 10 below/above; `let writer_locks: Arc<Vec<crate::sync::Mutex<()>>> =` |
| 21 | `crates/dotnet-utils/src/gc/cross_arena.rs:1233` — `stress_stw_flag_transitions_with_lease_acquisition` | keep | Aggregate group 10 below/above; `let writer_locks: Arc<Vec<crate::sync::Mutex<()>>> =` |
| 22 | `crates/dotnet-value/src/cts_cli_conversion.rs:58` — `from_enum_underlying` | keep | Aggregate group 3 below/above; `Some(match base {` |
| 23 | `crates/dotnet-vm/src/executor.rs:1065` — `run_inner` | keep | Aggregate group 11 below/above; `#[cfg(test)]` |
| 24 | `crates/dotnet-vm/src/executor.rs:1379` — `unregister` | fix (resolved) | Aggregate group 12 below/above; `let _ = self.with_arena_ref(\|arena\| {` |
| 25 | `crates/dotnet-vm/src/executor.rs:1398` — `drop` | fix (resolved) | Aggregate group 12 below/above; `let _ = self.with_arena_ref(\|arena\| {` |
| 26 | `crates/dotnet-vm/src/executor_collection_tests.rs:118` — `run_vm_collection` | keep | Aggregate group 13 below/above; `let storage = b` |
| 27 | `crates/dotnet-vm/src/executor_collection_tests.rs:150` — `run_vm_collection` | keep | Aggregate group 14 below/above; `a.with_arena(\|arena\| {` |
| 28 | `crates/dotnet-vm/src/executor_collection_tests.rs:322` — `spawn_owner` | keep | Aggregate group 13 below/above; `let storage = owner` |
| 29 | `crates/dotnet-vm/src/executor_collection_tests.rs:398` — `spawn_owner` | keep | Aggregate group 14 below/above; `a.with_arena(\|arena\| {` |
| 30 | `crates/dotnet-vm/src/executor_collection_tests.rs:555` — `arena_lifetime_mark_command_rejects_stale_registration` | keep | Aggregate group 15 below/above; `let replacement_ptr = replacement_arena.mutate(\|gc, _\| {` |
| 31 | `crates/dotnet-vm/src/executor_collection_tests.rs:577` — `arena_lifetime_mark_command_rejects_stale_registration` | keep | Aggregate group 15 below/above; `let foreign_ptr = foreign_arena.mutate(\|gc, _\| {` |
| 32 | `crates/dotnet-vm/src/executor_teardown_tests.rs:141` — `retired_shutdown_waits_for_inflight_collection_and_lease` | keep | Aggregate group 16 below/above; `let completion = state.workers.spawn_owner(move \|running\| {` |
| 33 | `crates/dotnet-vm/src/executor_teardown_tests.rs:1208` — `retired_owner_stays_registered_while_another_owner_finalizes` | keep | Aggregate group 16 below/above; `let owner = state.workers.spawn_owner(move \|running\| {` |
| 34 | `crates/dotnet-vm/src/fault_tests.rs:120` — `run_fault_entrypoint` | keep | Aggregate group 17 below/above; `arena.mutate_root(\|gc, engine\| {` |
| 35 | `crates/dotnet-vm/src/fault_tests.rs:155` — `run_fault_entrypoint` | keep | Aggregate group 18 below/above; `if let StepResult::Return = res {` |
| 36 | `crates/dotnet-vm/src/fault_tests.rs:255` — `test_fault_handler_executed_on_exception` | keep | Aggregate group 19 below/above; `let exc_ctor = res.push_method_reference(ExternalMethodReference {` |
| 37 | `crates/dotnet-vm/src/fault_tests.rs:352` — `test_fault_inside_finally_resumes_outer_handler` | keep | Aggregate group 19 below/above; `let exc_ctor = res.push_method_reference(ExternalMethodReference {` |
| 38 | `crates/dotnet-vm/src/fault_tests.rs:440` — `test_fault_inside_finally_with_pending_exception` | keep | Aggregate group 19 below/above; `let exc_ctor = res.push_method_reference(ExternalMethodReference {` |
| 39 | `crates/dotnet-vm/src/instructions/objects/mod.rs:368` — `stobj` | fix (resolved) | Aggregate group 20 below/above; `if let Err(error) = unsafe { ctx.write_bytes(origin.clone(), offset, bytes) } {` |
| 40 | `crates/dotnet-vm/src/instructions/objects/mod.rs:410` — `initobj` | fix (resolved) | Aggregate group 20 below/above; `if let Err(error) = unsafe { ctx.write_bytes(origin.clone(), offset, zero_bytes) } {` |
| 41 | `crates/dotnet-vm/src/instructions/stack_ops.rs:81` — `ldarga` | fix (resolved) | Aggregate group 21 below/above; `{` |
| 42 | `crates/dotnet-vm/src/instructions/stack_ops.rs:115` — `ldloca` | fix (resolved) | Aggregate group 21 below/above; `{` |
| 43 | `crates/dotnet-vm/src/jmp_tests.rs:178` — `test_jmp_instruction` | keep | Aggregate group 18 below/above; `if let StepResult::Return = res {` |
| 44 | `crates/dotnet-vm/src/stack/exception_ops_impl.rs:67` — `throw_argument_out_of_range` | fix (resolved) | Aggregate group 22 below/above; `exception.as_object_mut(gc, \|obj\| {` |
| 45 | `crates/dotnet-vm/src/stack/exception_ops_impl.rs:95` — `throw_by_name_with_message` | fix (resolved) | Aggregate group 23 below/above; `obj_ref.as_object_mut(gc, \|obj\| {` |
| 46 | `crates/dotnet-vm/src/stack/native_effects.rs:563` — `exception` | fix (resolved) | Aggregate group 23 below/above; `object.as_object_mut(gc, \|obj\| {` |
| 47 | `crates/dotnet-vm/src/stack/native_effects.rs:577` — `exception` | fix (resolved) | Aggregate group 22 below/above; `object.as_object_mut(gc, \|obj\| {` |
| 48 | `crates/dotnet-vm/src/stack/native_effects.rs:1736` — `advance_monitor_wait` | keep | Aggregate group 24 below/above; `if let Some(pulsed) = pending.wait.poll(self.thread_id()) {` |
| 49 | `crates/dotnet-vm/src/stack/native_effects.rs:1748` — `advance_monitor_wait` | keep | Aggregate group 24 below/above; `if let Some(pulsed) = pending.wait.poll(self.thread_id()) {` |
| 50 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:33` — `write_to_inline_storage` | fix (resolved) | Aggregate group 25 below/above; `if offset` |
| 51 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:63` — `read_from_inline_storage` | fix (resolved) | Aggregate group 25 below/above; `if offset` |
| 52 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:86` — `bounds_checked_slice_copy` | fix (resolved) | Aggregate group 26 below/above; `if offset.as_usize() + src.len() > dest.len() {` |
| 53 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:102` — `bounds_checked_slice_read` | fix (resolved) | Aggregate group 26 below/above; `if offset.as_usize() + dest.len() > src.len() {` |
| 54 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:164` — `write_unaligned` | fix (resolved) | Aggregate group 27 below/above; `let layout = if matches!(layout, LayoutManager::Scalar(Scalar::NativeInt))` |
| 55 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:261` — `read_unaligned` | fix (resolved) | Aggregate group 27 below/above; `let layout = if matches!(layout, LayoutManager::Scalar(Scalar::NativeInt))` |
| 56 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:487` — `object_atomic` | fix (resolved) | Aggregate group 28 below/above; `if end.is_none_or(\|end\| end > data.len()) {` |
| 57 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:518` — `object_atomic` | fix (resolved) | Aggregate group 28 below/above; `if end.is_none_or(\|end\| end > data.len()) {` |
| 58 | `crates/dotnet-vm/src/stack/stack_ops_impl.rs:106` — `set_local` | fix (resolved) | Aggregate group 29 below/above; `#[cfg(feature = "multithreading")]` |
| 59 | `crates/dotnet-vm/src/stack/stack_ops_impl.rs:297` — `set_argument` | fix (resolved) | Aggregate group 29 below/above; `#[cfg(feature = "multithreading")]` |
| 60 | `crates/dotnet-vm/src/stack/stack_ops_impl.rs:335` — `set_slot` | fix (resolved) | Aggregate group 29 below/above; `#[cfg(feature = "multithreading")]` |
| 61 | `crates/dotnet-vm/src/tail_calls_tests.rs:178` — `run_tail_chain_and_measure_max_depth` | keep | Aggregate group 17 below/above; `arena.mutate_root(\|gc, engine\| {` |
| 62 | `crates/dotnet-vm/src/threading/basic.rs:323` — `register_thread` | keep | Aggregate group 11 below/above; `#[cfg(test)]` |
| 63 | `crates/dotnet-vm/src/threading/events.rs:65` — `set` | fix (resolved) | Aggregate group 30 below/above; `for waiter in &mut state.waiters {` |
| 64 | `crates/dotnet-vm/src/threading/events.rs:77` — `set` | fix (resolved) | Aggregate group 31 below/above; `if let Some(waiter) = state.waiters.iter_mut().find(\|w\| w.result.is_none()) {` |
| 65 | `crates/dotnet-vm/src/threading/events.rs:197` — `drop` | fix (resolved) | Aggregate group 30 below/above; `for waiter in &mut state.waiters {` |
| 66 | `crates/dotnet-vm/src/threading/events.rs:202` — `drop` | fix (resolved) | Aggregate group 31 below/above; `if let Some(next) = state.waiters.iter_mut().find(\|w\| w.result.is_none()) {` |
| 67 | `crates/dotnet-vm/src/threading/workers.rs:544` — `completed_outcomes_remain_consumable_after_retirement` | keep | Aggregate group 32 below/above; `let state = Arc::new(Mutex::new(RegistryState {` |
| 68 | `crates/dotnet-vm/src/threading/workers.rs:586` — `abort_prevents_retired_worker_from_starting_finalizers` | keep | Aggregate group 32 below/above; `let state = Arc::new(Mutex::new(RegistryState {` |
| 69 | `crates/dotnet-vm/src/threading/workers.rs:614` — `finalizing_preserves_managed_completion` | keep | Aggregate group 32 below/above; `let state = Arc::new(Mutex::new(RegistryState {` |
| 70 | `crates/dotnet-vm/src/threading/workers.rs:648` — `shutdown_requires_retirement_and_records_failed_drain` | keep | Aggregate group 32 below/above; `let state = Arc::new(Mutex::new(RegistryState {` |

### NewCrateVersionAvailable — 12 entries

**Per-file tally:** `Cargo.toml` (3); `crates/dotnet-benchmarks/Cargo.toml` (1); `crates/dotnet-macros-core/Cargo.toml` (1); `crates/dotnet-macros/Cargo.toml` (1); `crates/dotnet-value/fuzz/Cargo.toml` (1); `crates/dotnet-vm/Cargo.toml` (3); `crates/dotnet-vm/fuzz/Cargo.toml` (1); `xtask/Cargo.toml` (1)

| Entry | Snapshot site and nearby symbol | Decision | Reason / finding |
|---|---|---|---|
| 1 | `Cargo.toml:59` — `(module context)` | defer | F-DEP-002; `"0.6.0"` |
| 2 | `Cargo.toml:61` — `(module context)` | defer | F-DEP-002; `"0.12"` |
| 3 | `Cargo.toml:69` — `(module context)` | defer | F-DEP-002; `"0.18"` |
| 4 | `crates/dotnet-benchmarks/Cargo.toml:39` — `(module context)` | defer | F-DEP-002; `"0.5"` |
| 5 | `crates/dotnet-macros/Cargo.toml:13` — `(module context)` | defer | F-DEP-002; `"2.0"` |
| 6 | `crates/dotnet-macros-core/Cargo.toml:7` — `(module context)` | defer | F-DEP-002; `"2.0"` |
| 7 | `crates/dotnet-value/fuzz/Cargo.toml:15` — `(module context)` | defer | F-DEP-002; `"0.6.0"` |
| 8 | `crates/dotnet-vm/Cargo.toml:34` — `(module context)` | defer | F-DEP-002; `"0.13"` |
| 9 | `crates/dotnet-vm/Cargo.toml:97` — `(module context)` | defer | F-DEP-002; `"0.13"` |
| 10 | `crates/dotnet-vm/Cargo.toml:98` — `(module context)` | defer | F-DEP-002; `"2"` |
| 11 | `crates/dotnet-vm/fuzz/Cargo.toml:19` — `(module context)` | defer | F-DEP-002; `"0.6.0"` |
| 12 | `xtask/Cargo.toml:8` — `(module context)` | defer | F-DEP-002; `"2"` |

### RsAssertEqual — 19 entries

**Per-file tally:** `crates/dotnet-value/src/pointer/tests.rs` (17); `crates/dotnet-vm/src/condition_wait_tests.rs` (2)

| Entry | Snapshot site and nearby symbol | Decision | Reason / finding |
|---|---|---|---|
| 1 | `crates/dotnet-value/src/pointer/tests.rs:1627` — `ptr_arith_unmanaged_arithmetic_wraps_to_explicit_null` | fix (resolved) | F-IDIOM-002; `assert!(zero == crate::StackValue::NativeInt(0))` |
| 2 | `crates/dotnet-value/src/pointer/tests.rs:1633` — `ptr_arith_unmanaged_arithmetic_wraps_to_explicit_null` | fix (resolved) | F-IDIOM-002; `assert!(wrapped_from_max == crate::StackValue::NativeInt(0))` |
| 3 | `crates/dotnet-value/src/pointer/tests.rs:1638` — `ptr_arith_unmanaged_arithmetic_wraps_to_explicit_null` | fix (resolved) | F-IDIOM-002; `assert!(wrapped_from_one == crate::StackValue::NativeInt(0))` |
| 4 | `crates/dotnet-value/src/pointer/tests.rs:1696` — `ptr_arith_pointer_comparisons_are_value_bearing` | fix (resolved) | F-IDIOM-002; `assert!( base == base.clone(), "an unmanaged pointer must equal itself" )` |
| 5 | `crates/dotnet-value/src/pointer/tests.rs:1719` — `ptr_arith_pointer_comparisons_are_value_bearing` | fix (resolved) | F-IDIOM-002; `assert!(base == base_integer.clone())` |
| 6 | `crates/dotnet-value/src/pointer/tests.rs:1720` — `ptr_arith_pointer_comparisons_are_value_bearing` | fix (resolved) | F-IDIOM-002; `assert!(base_integer == base)` |
| 7 | `crates/dotnet-value/src/pointer/tests.rs:1771` — `ptr_arith_pointer_comparisons_are_value_bearing` | fix (resolved) | F-IDIOM-002; `assert!(null_managed == zero)` |
| 8 | `crates/dotnet-value/src/pointer/tests.rs:1772` — `ptr_arith_pointer_comparisons_are_value_bearing` | fix (resolved) | F-IDIOM-002; `assert!(zero == null_managed)` |
| 9 | `crates/dotnet-value/src/pointer/tests.rs:1773` — `ptr_arith_pointer_comparisons_are_value_bearing` | fix (resolved) | F-IDIOM-002; `assert!(base != zero)` |
| 10 | `crates/dotnet-value/src/pointer/tests.rs:1774` — `ptr_arith_pointer_comparisons_are_value_bearing` | fix (resolved) | F-IDIOM-002; `assert!(zero != base)` |
| 11 | `crates/dotnet-value/src/pointer/tests.rs:1775` — `ptr_arith_pointer_comparisons_are_value_bearing` | fix (resolved) | F-IDIOM-002; `assert!(managed_value != zero)` |
| 12 | `crates/dotnet-value/src/pointer/tests.rs:1776` — `ptr_arith_pointer_comparisons_are_value_bearing` | fix (resolved) | F-IDIOM-002; `assert!(managed_value == base)` |
| 13 | `crates/dotnet-value/src/pointer/tests.rs:1777` — `ptr_arith_pointer_comparisons_are_value_bearing` | fix (resolved) | F-IDIOM-002; `assert!(base == managed_value)` |
| 14 | `crates/dotnet-value/src/pointer/tests.rs:1778` — `ptr_arith_pointer_comparisons_are_value_bearing` | fix (resolved) | F-IDIOM-002; `assert!(managed_value == base_integer)` |
| 15 | `crates/dotnet-value/src/pointer/tests.rs:1779` — `ptr_arith_pointer_comparisons_are_value_bearing` | fix (resolved) | F-IDIOM-002; `assert!(base_integer == managed_value)` |
| 16 | `crates/dotnet-value/src/pointer/tests.rs:1887` — `ptr_arith_managed_ptr_far_oob_compare_and_difference` | fix (resolved) | F-IDIOM-002; `assert!(far_value == crate::StackValue::NativeInt(observed_far.addr().get() as isize))` |
| 17 | `crates/dotnet-value/src/pointer/tests.rs:1901` — `ptr_arith_managed_ptr_far_oob_compare_and_difference` | fix (resolved) | F-IDIOM-002; `assert!(far_max_value == crate::StackValue::NativeInt(observed_max.addr().get() as isize))` |
| 18 | `crates/dotnet-vm/src/condition_wait_tests.rs:4256` — `static_storage_identity_ignores_generic_argument_resolution` | fix (resolved) | F-IDIOM-002; `assert!(corlib != other)` |
| 19 | `crates/dotnet-vm/src/condition_wait_tests.rs:4272` — `static_storage_identity_ignores_generic_argument_resolution` | fix (resolved) | F-IDIOM-002; `assert!( via_handle != via_field, "raw lookups must ...` |

### RsDeprecation — 4 entries

**Per-file tally:** `crates/dotnet-utils/src/gc/cross_arena.rs` (2); `crates/dotnet-vm/src/threading/basic.rs` (1); `crates/dotnet-vm/src/threading/local.rs` (1)

| Entry | Snapshot site and nearby symbol | Decision | Reason / finding |
|---|---|---|---|
| 1 | `crates/dotnet-utils/src/gc/cross_arena.rs:1667` — `arena_identity_registry_checked_counter_control` | fix (resolved) | F-DEPR-001: std method verified on pin; `fetch_update` |
| 2 | `crates/dotnet-utils/src/gc/cross_arena.rs:1672` — `arena_identity_registry_checked_counter_control` | fix (resolved) | F-DEPR-001: std method verified on pin; `fetch_update` |
| 3 | `crates/dotnet-vm/src/threading/basic.rs:186` — `allocate_managed_thread_id` | keep | F-DEPR-001: Loom lacks try_update; `fetch_update` |
| 4 | `crates/dotnet-vm/src/threading/local.rs:24` — `next_managed_id` | fix (resolved) | F-DEPR-001: std method verified on pin; `fetch_update` |

### RsDetachedFile — 33 entries

**Per-file tally:** `crates/dotnet-value/fuzz/fuzz_targets/fuzz_managed_ptr_offset.rs` (1); `crates/dotnet-value/fuzz/fuzz_targets/fuzz_managed_ptr_roundtrip.rs` (1); `crates/dotnet-value/fuzz/fuzz_targets/fuzz_raw_memory_access.rs` (1); `crates/dotnet-value/fuzz/target/debug/build/thiserror-2d968b5c7ad9a1bf/out/private.rs` (1); `crates/dotnet-value/fuzz/target/debug/build/thiserror/840bc648e8f1962b/out/private.rs` (1); `crates/dotnet-value/fuzz/target/x86_64-unknown-linux-gnu/release/build/thiserror-6da78f8263164a02/out/private.rs` (1); `crates/dotnet-value/fuzz/target/x86_64-unknown-linux-gnu/release/build/thiserror/6da78f8263164a02/out/private.rs` (1); `crates/dotnet-vm/fuzz/fuzz_targets/fuzz_executor.rs` (1); `crates/dotnet-vm/fuzz/target/debug/build/dotnet-vm-ae861fbad373e7cd/out/instruction_dispatch.rs` (1); `crates/dotnet-vm/fuzz/target/debug/build/dotnet-vm-ae861fbad373e7cd/out/intrinsics_dispatch.rs` (1); `crates/dotnet-vm/fuzz/target/debug/build/dotnet-vm-ae861fbad373e7cd/out/intrinsics_phf.rs` (1); `crates/dotnet-vm/fuzz/target/debug/build/dotnet-vm-fb4960a8ef3cc4c0/out/instruction_dispatch.rs` (1); `crates/dotnet-vm/fuzz/target/debug/build/dotnet-vm-fb4960a8ef3cc4c0/out/intrinsics_dispatch.rs` (1); `crates/dotnet-vm/fuzz/target/debug/build/dotnet-vm-fb4960a8ef3cc4c0/out/intrinsics_phf.rs` (1); `crates/dotnet-vm/fuzz/target/debug/build/serde-be62b92d4562b0f4/out/private.rs` (1); `crates/dotnet-vm/fuzz/target/debug/build/serde_core-e9531bdb6193ba06/out/private.rs` (1); `crates/dotnet-vm/fuzz/target/debug/build/thiserror-2d968b5c7ad9a1bf/out/private.rs` (1); `crates/dotnet-vm/fuzz/target/release/build/thiserror-5e70589a9cb01aff/out/private.rs` (1); `crates/dotnet-vm/fuzz/target/release/build/thiserror-92e325efc38a7f72/out/private.rs` (1); `crates/dotnet-vm/fuzz/target/release/build/thiserror-a8c893a0d649ce0b/out/private.rs` (1); `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/dotnet-assemblies-8be02a419a58810b/out/support_slots.rs` (1); `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/dotnet-vm-98554b53b6f8bd9d/out/instruction_dispatch.rs` (1); `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/dotnet-vm-98554b53b6f8bd9d/out/intrinsics_dispatch.rs` (1); `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/dotnet-vm-98554b53b6f8bd9d/out/intrinsics_phf.rs` (1); `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/dotnet-vm-f5f4dd35ccc9d2f4/out/instruction_dispatch.rs` (1); `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/dotnet-vm-f5f4dd35ccc9d2f4/out/intrinsics_dispatch.rs` (1); `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/dotnet-vm-f5f4dd35ccc9d2f4/out/intrinsics_phf.rs` (1); `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/serde-65dabd78520944ae/out/private.rs` (1); `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/serde_core-d34b3bbe0aec4af9/out/private.rs` (1); `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/thiserror-6da78f8263164a02/out/private.rs` (1); `crates/dotnet-vm/fuzz/tools/generate_corpus.rs` (1); `crates/dotnet-vm/fuzz/tools/generate_structured_corpus.rs` (1); `crates/dotnet-vm/src/intrinsics/__rerun_probe.rs` (1)

| Entry | Snapshot site and nearby symbol | Decision | Reason / finding |
|---|---|---|---|
| 1 | `crates/dotnet-value/fuzz/fuzz_targets/fuzz_managed_ptr_offset.rs:1` — `(module context)` | refute | F-INFRA-001; `#![no_main] use arbitrary::Arbitrary; use dotnet_utils::ByteOffset; use dotnet_value::poin` |
| 2 | `crates/dotnet-value/fuzz/fuzz_targets/fuzz_managed_ptr_roundtrip.rs:1` — `(module context)` | refute | F-INFRA-001; `#![no_main] use arbitrary::Arbitrary; use dotnet_utils::{ ArenaId, ByteOffset, gc::{ArenaH` |
| 3 | `crates/dotnet-value/fuzz/fuzz_targets/fuzz_raw_memory_access.rs:1` — `(module context)` | refute | F-INFRA-001; `#![no_main] use dotnet_utils::atomic::StandardAtomicAccess; use libfuzzer_sys::fuzz_target` |
| 4 | `crates/dotnet-value/fuzz/target/debug/build/thiserror/840bc648e8f1962b/out/private.rs:1` — `(module context)` | refute | F-INFRA-001; `#[doc(hidden)] pub mod __private18 { #[doc(hidden)] pub use crate::private::*; }` |
| 5 | `crates/dotnet-value/fuzz/target/debug/build/thiserror-2d968b5c7ad9a1bf/out/private.rs:1` — `(module context)` | refute | F-INFRA-001; `#[doc(hidden)] pub mod __private18 { #[doc(hidden)] pub use crate::private::*; }` |
| 6 | `crates/dotnet-value/fuzz/target/x86_64-unknown-linux-gnu/release/build/thiserror/6da78f8263164a02/out/private.rs:1` — `(module context)` | refute | F-INFRA-001; `#[doc(hidden)] pub mod __private18 { #[doc(hidden)] pub use crate::private::*; }` |
| 7 | `crates/dotnet-value/fuzz/target/x86_64-unknown-linux-gnu/release/build/thiserror-6da78f8263164a02/out/private.rs:1` — `(module context)` | refute | F-INFRA-001; `#[doc(hidden)] pub mod __private18 { #[doc(hidden)] pub use crate::private::*; }` |
| 8 | `crates/dotnet-vm/fuzz/fuzz_targets/fuzz_executor.rs:1` — `(module context)` | refute | F-INFRA-001; `#![no_main] use dotnet_vm::fuzzing::{FuzzProgram, execute_cil_program}; use libfuzzer_sys:` |
| 9 | `crates/dotnet-vm/fuzz/target/debug/build/dotnet-vm-ae861fbad373e7cd/out/instruction_dispatch.rs:1` — `dispatch_monomorphic` | refute | F-INFRA-001; `pub fn dispatch_monomorphic<'gc, T: crate::stack::ops::VesOps<'gc>>( ctx: &mut T, instr: &` |
| 10 | `crates/dotnet-vm/fuzz/target/debug/build/dotnet-vm-ae861fbad373e7cd/out/intrinsics_dispatch.rs:1` — `(module context)` | refute | F-INFRA-001; `#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)] #[allow(non_camel_case_types)] pub enum` |
| 11 | `crates/dotnet-vm/fuzz/target/debug/build/dotnet-vm-ae861fbad373e7cd/out/intrinsics_phf.rs:1` — `(module context)` | refute | F-INFRA-001; `use crate::intrinsics::static_registry::{StaticIntrinsicEntry, StaticIntrinsicHandler, Ran` |
| 12 | `crates/dotnet-vm/fuzz/target/debug/build/dotnet-vm-fb4960a8ef3cc4c0/out/instruction_dispatch.rs:1` — `dispatch_monomorphic` | refute | F-INFRA-001; `pub fn dispatch_monomorphic<'gc, T: crate::stack::ops::VesOps<'gc>>( ctx: &mut T, instr: &` |
| 13 | `crates/dotnet-vm/fuzz/target/debug/build/dotnet-vm-fb4960a8ef3cc4c0/out/intrinsics_dispatch.rs:1` — `(module context)` | refute | F-INFRA-001; `#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)] #[allow(non_camel_case_types)] pub enum` |
| 14 | `crates/dotnet-vm/fuzz/target/debug/build/dotnet-vm-fb4960a8ef3cc4c0/out/intrinsics_phf.rs:1` — `(module context)` | refute | F-INFRA-001; `use crate::intrinsics::static_registry::{StaticIntrinsicEntry, StaticIntrinsicHandler, Ran` |
| 15 | `crates/dotnet-vm/fuzz/target/debug/build/serde-be62b92d4562b0f4/out/private.rs:1` — `(module context)` | refute | F-INFRA-001; `#[doc(hidden)] pub mod __private228 { #[doc(hidden)] pub use crate::private::*; } use serd` |
| 16 | `crates/dotnet-vm/fuzz/target/debug/build/serde_core-e9531bdb6193ba06/out/private.rs:1` — `(module context)` | refute | F-INFRA-001; `#[doc(hidden)] pub mod __private228 { #[doc(hidden)] pub use crate::private::*; }` |
| 17 | `crates/dotnet-vm/fuzz/target/debug/build/thiserror-2d968b5c7ad9a1bf/out/private.rs:1` — `(module context)` | refute | F-INFRA-001; `#[doc(hidden)] pub mod __private18 { #[doc(hidden)] pub use crate::private::*; }` |
| 18 | `crates/dotnet-vm/fuzz/target/release/build/thiserror-5e70589a9cb01aff/out/private.rs:1` — `(module context)` | refute | F-INFRA-001; `#[doc(hidden)] pub mod __private18 { #[doc(hidden)] pub use crate::private::*; }` |
| 19 | `crates/dotnet-vm/fuzz/target/release/build/thiserror-92e325efc38a7f72/out/private.rs:1` — `(module context)` | refute | F-INFRA-001; `#[doc(hidden)] pub mod __private18 { #[doc(hidden)] pub use crate::private::*; }` |
| 20 | `crates/dotnet-vm/fuzz/target/release/build/thiserror-a8c893a0d649ce0b/out/private.rs:1` — `(module context)` | refute | F-INFRA-001; `#[doc(hidden)] pub mod __private18 { #[doc(hidden)] pub use crate::private::*; }` |
| 21 | `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/dotnet-assemblies-8be02a419a58810b/out/support_slots.rs:1` — `(module context)` | refute | F-INFRA-001; `// @generated by crates/dotnet-assemblies/build.rs from support_slots.def. // Do not edit ` |
| 22 | `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/dotnet-vm-98554b53b6f8bd9d/out/instruction_dispatch.rs:1` — `dispatch_monomorphic` | refute | F-INFRA-001; `pub fn dispatch_monomorphic<'gc, T: crate::stack::ops::VesOps<'gc>>( ctx: &mut T, instr: &` |
| 23 | `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/dotnet-vm-98554b53b6f8bd9d/out/intrinsics_dispatch.rs:1` — `(module context)` | refute | F-INFRA-001; `#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)] #[allow(non_camel_case_types)] pub enum` |
| 24 | `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/dotnet-vm-98554b53b6f8bd9d/out/intrinsics_phf.rs:1` — `(module context)` | refute | F-INFRA-001; `use crate::intrinsics::static_registry::{StaticIntrinsicEntry, StaticIntrinsicHandler, Ran` |
| 25 | `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/dotnet-vm-f5f4dd35ccc9d2f4/out/instruction_dispatch.rs:1` — `dispatch_monomorphic` | refute | F-INFRA-001; `pub fn dispatch_monomorphic<'gc, T: crate::stack::ops::VesOps<'gc>>( ctx: &mut T, instr: &` |
| 26 | `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/dotnet-vm-f5f4dd35ccc9d2f4/out/intrinsics_dispatch.rs:1` — `(module context)` | refute | F-INFRA-001; `#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)] #[allow(non_camel_case_types)] pub enum` |
| 27 | `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/dotnet-vm-f5f4dd35ccc9d2f4/out/intrinsics_phf.rs:1` — `(module context)` | refute | F-INFRA-001; `use crate::intrinsics::static_registry::{StaticIntrinsicEntry, StaticIntrinsicHandler, Ran` |
| 28 | `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/serde-65dabd78520944ae/out/private.rs:1` — `(module context)` | refute | F-INFRA-001; `#[doc(hidden)] pub mod __private228 { #[doc(hidden)] pub use crate::private::*; } use serd` |
| 29 | `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/serde_core-d34b3bbe0aec4af9/out/private.rs:1` — `(module context)` | refute | F-INFRA-001; `#[doc(hidden)] pub mod __private228 { #[doc(hidden)] pub use crate::private::*; }` |
| 30 | `crates/dotnet-vm/fuzz/target/x86_64-unknown-linux-gnu/release/build/thiserror-6da78f8263164a02/out/private.rs:1` — `(module context)` | refute | F-INFRA-001; `#[doc(hidden)] pub mod __private18 { #[doc(hidden)] pub use crate::private::*; }` |
| 31 | `crates/dotnet-vm/fuzz/tools/generate_corpus.rs:1` — `(module context)` | refute | F-INFRA-001; `#!/usr/bin/env cargo //! Tool to generate a seed corpus for the CIL bytecode fuzzer from e` |
| 32 | `crates/dotnet-vm/fuzz/tools/generate_structured_corpus.rs:1` — `(module context)` | refute | F-INFRA-001; `#!/usr/bin/env cargo //! Tool to generate structured seed corpus for the CIL bytecode fuzz` |
| 33 | `crates/dotnet-vm/src/intrinsics/__rerun_probe.rs:1` — `(module context)` | refute | F-INFRA-001; `// Tracked fixture in the build script's watched 'intrinsics' directory, kept for // scrip` |

### RsLift — 4 entries

**Per-file tally:** `crates/dotnet-assemblies/src/host.rs` (1); `crates/dotnet-vm-ops/src/effects.rs` (2); `crates/dotnet-vm/src/instructions/objects/boxing.rs` (1)

| Entry | Snapshot site and nearby symbol | Decision | Reason / finding |
|---|---|---|---|
| 1 | `crates/dotnet-assemblies/src/host.rs:526` — `classify_entry_bytes` | fix (resolved) | F-IDIOM-002; `match` |
| 2 | `crates/dotnet-vm/src/instructions/objects/boxing.rs:80` — `unbox_any` | fix (resolved) | F-IDIOM-002; `if` |
| 3 | `crates/dotnet-vm-ops/src/effects.rs:1292` — `call_inner` | fix (resolved) | F-IDIOM-002; `match` |
| 4 | `crates/dotnet-vm-ops/src/effects.rs:1353` — `ensure_initialized` | fix (resolved) | F-IDIOM-002; `match` |

### RsNeedlessLifetimes — 22 entries

**Per-file tally:** `crates/dotnet-intrinsics-reflection/src/fields.rs` (1); `crates/dotnet-intrinsics-reflection/src/invoke_args.rs` (1); `crates/dotnet-intrinsics-reflection/src/types/mod.rs` (1); `crates/dotnet-intrinsics-string/src/lib.rs` (1); `crates/dotnet-intrinsics-unsafe/src/buffer.rs` (1); `crates/dotnet-intrinsics-unsafe/src/marshal.rs` (1); `crates/dotnet-runtime-memory/src/access.rs` (2); `crates/dotnet-runtime-memory/src/heap.rs` (1); `crates/dotnet-value/src/cts_cli_conversion.rs` (5); `crates/dotnet-value/src/object/mod.rs` (1); `crates/dotnet-value/src/pointer/tests.rs` (1); `crates/dotnet-value/src/stack_value.rs` (2); `crates/dotnet-vm/src/intrinsics/object_ops.rs` (2); `crates/dotnet-vm/src/stack/context_ops.rs` (1); `crates/dotnet-vm/src/test_utils/foreign_reference.rs` (1)

| Entry | Snapshot site and nearby symbol | Decision | Reason / finding |
|---|---|---|---|
| 1 | `crates/dotnet-intrinsics-reflection/src/fields.rs:427` — `read_instance_field_bytes` | fix (resolved) | F-IDIOM-002: no informative input/output brand relation; `fn read_instance_field_bytes<'gc>( target_obj: ObjectRef<'gc>, field_parent: TypeDescripti` |
| 2 | `crates/dotnet-intrinsics-reflection/src/invoke_args.rs:390` — `widen_boxed` | keep | F-IDIOM-002: preserve named arena transfer/short-lifetime witness; `fn widen_boxed<'gc>( arg: ObjectRef<'gc>, from: Prim, to: Prim, ) -> Option<StackValue<'..` |
| 3 | `crates/dotnet-intrinsics-reflection/src/types/mod.rs:257` — `string_from_heap_obj` | fix (resolved) | F-IDIOM-002: no informative input/output brand relation; `fn string_from_heap_obj<'gc>(name_obj: ObjectRef<'gc>) -> Result<String, VmError>` |
| 4 | `crates/dotnet-intrinsics-string/src/lib.rs:42` — `string_with_span_data` | fix (resolved) | F-IDIOM-002: no informative input/output brand relation; `fn string_with_span_data` |
| 5 | `crates/dotnet-intrinsics-unsafe/src/buffer.rs:19` — `stack_value_to_byte` | fix (resolved) | F-IDIOM-002: no informative input/output brand relation; `fn stack_value_to_byte<'gc>(value: &StackValue<'gc>) -> Option<u8>` |
| 6 | `crates/dotnet-intrinsics-unsafe/src/marshal.rs:16` — `offset_ptr` | keep | F-IDIOM-002: preserve named arena transfer/short-lifetime witness; `fn offset_ptr<'gc>( val: StackValue<'gc>, byte_offset: isize, ) -> Result<StackValue<'gc>,` |
| 7 | `crates/dotnet-runtime-memory/src/access.rs:2171` — `storage_is_string` | fix (resolved) | F-IDIOM-002: no informative input/output brand relation; `fn storage_is_string<'a>(storage: &HeapStorage<'a>) -> bool` |
| 8 | `crates/dotnet-runtime-memory/src/access.rs:2176` — `cross_arena_with_short_lifetime` | keep | F-IDIOM-002: preserve named arena transfer/short-lifetime witness; `fn cross_arena_with_short_lifetime<'short>( gc: GCHandle<'short>, origin: dotnet_val...` |
| 9 | `crates/dotnet-runtime-memory/src/heap.rs:308` — `object_size` | fix (resolved) | F-IDIOM-002: no informative input/output brand relation; `fn object_size<'gc>(obj: ObjectRef<'gc>) -> Option<usize>` |
| 10 | `crates/dotnet-value/src/cts_cli_conversion.rs:154` — `widen` | keep | F-IDIOM-002: preserve named arena transfer/short-lifetime witness; `fn widen<'gc>(value: CTSValue<'gc>) -> StackValue<'gc>` |
| 11 | `crates/dotnet-value/src/cts_cli_conversion.rs:161` — `widen_value_type` | keep | F-IDIOM-002: preserve named arena transfer/short-lifetime witness; `fn widen_value_type<'gc>(value: ValueType<'gc>) -> StackValue<'gc>` |
| 12 | `crates/dotnet-value/src/cts_cli_conversion.rs:212` — `narrow_scalar_value` | keep | F-IDIOM-002: preserve named arena transfer/short-lifetime witness; `fn narrow_scalar_value<'gc>( data: StackValue<'gc>, target: CtsScalarKind, ) -> ...` |
| 13 | `crates/dotnet-value/src/cts_cli_conversion.rs:237` — `narrow_float32` | keep | F-IDIOM-002: preserve named arena transfer/short-lifetime witness; `fn narrow_float32<'gc>( data: StackValue<'gc>, ) -> Result<ValueType<'gc>, TypeResolutio..` |
| 14 | `crates/dotnet-value/src/cts_cli_conversion.rs:248` — `narrow_float64` | keep | F-IDIOM-002: preserve named arena transfer/short-lifetime witness; `fn narrow_float64<'gc>( data: StackValue<'gc>, ) -> Result<ValueType<'gc>, TypeResolutio..` |
| 15 | `crates/dotnet-value/src/object/mod.rs:235` — `from_handle` | fix (resolved) | F-IDIOM-002: no informative input/output brand relation; `fn from_handle<'gc>(handle: ObjectHandle<'gc>) -> Self` |
| 16 | `crates/dotnet-value/src/pointer/tests.rs:68` — `managed_ptr_to_heap_object_start` | keep | F-IDIOM-002: preserve named arena transfer/short-lifetime witness; `fn managed_ptr_to_heap_object_start<'gc>(obj: ObjectRef<'gc>) -> ManagedPtr<'gc>` |
| 17 | `crates/dotnet-value/src/stack_value.rs:234` — `managed_ptr_to_raw` | fix (resolved) | F-IDIOM-002: no informative input/output brand relation; `fn managed_ptr_to_raw<'gc>(m: &StackManagedPtr<'gc>) -> *mut u8` |
| 18 | `crates/dotnet-value/src/stack_value.rs:982` — `managed_offset` | keep | F-IDIOM-002: preserve named arena transfer/short-lifetime witness; `fn managed_offset<'gc>( m: StackManagedPtr<'gc>, bytes: isize, ) -> Result<StackValue<'gc>` |
| 19 | `crates/dotnet-vm/src/intrinsics/object_ops.rs:339` — `scalar_enum_value_from_stack` | fix (resolved) | F-IDIOM-002: no informative input/output brand relation; `fn scalar_enum_value_from_stack<'gc>(value: StackValue<'gc>) -> Option<(i128, bool)>` |
| 20 | `crates/dotnet-vm/src/intrinsics/object_ops.rs:349` — `enum_type_from_stack_value` | fix (resolved) | F-IDIOM-002: no informative input/output brand relation; `fn enum_type_from_stack_value<'gc>(value: &StackValue<'gc>) -> Option<TypeDescription>` |
| 21 | `crates/dotnet-vm/src/stack/context_ops.rs:369` — `string_with_span_data` | fix (resolved) | F-IDIOM-002: no informative input/output brand relation; `fn string_with_span_data<'span, R, F: FnOnce(&[u8]) -> R>( &self, span: Object<'span...` |
| 22 | `crates/dotnet-vm/src/test_utils/foreign_reference.rs:38` — `decode_reference` | keep | F-IDIOM-002: preserve named arena transfer/short-lifetime witness; `fn decode_reference<'gc>( storage: dotnet_value::storage::FieldStorage, handle: dotnet_uti` |

### RsSortImplTraitMembers — 11 entries

**Per-file tally:** `crates/dotnet-benchmarks/benches/managed_ptr_serde.rs` (1); `crates/dotnet-cli/tests/integration_tests_impl/fixtures.rs` (2); `crates/dotnet-cli/tests/integration_tests_impl/reflection_tail.rs` (1); `crates/dotnet-vm/src/stack/call_ops_impl.rs` (1); `crates/dotnet-vm/src/stack/context_ops.rs` (1); `crates/dotnet-vm/src/stack/exception_ops_impl.rs` (1); `crates/dotnet-vm/src/stack/native_effects.rs` (1); `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs` (2); `xtask/src/main.rs` (1)

| Entry | Snapshot site and nearby symbol | Decision | Reason / finding |
|---|---|---|---|
| 1 | `crates/dotnet-benchmarks/benches/managed_ptr_serde.rs:100` — `non_null_at` | keep | F-IDIOM-003; `impl<'gc> ManagedPtrResolver<'gc> for BenchmarkManagedPtrResolver` |
| 2 | `crates/dotnet-cli/tests/integration_tests_impl/fixtures.rs:89` — `multicast_selection_precedes_operand_commitment` | keep | F-IDIOM-003; `impl Visit for Fields` |
| 3 | `crates/dotnet-cli/tests/integration_tests_impl/fixtures.rs:185` — `reflection_path_selection` | keep | F-IDIOM-003; `impl Visit for Fields` |
| 4 | `crates/dotnet-cli/tests/integration_tests_impl/reflection_tail.rs:95` — `reflection_tail_dispatch_matches_dotnet` | keep | F-IDIOM-003; `impl Visit for Fields` |
| 5 | `crates/dotnet-vm/src/stack/call_ops_impl.rs:292` — `allocate_frame_homes` | keep | F-IDIOM-003; `impl<'a, 'gc> VmCallOps<'gc> for VesContext<'a, 'gc>` |
| 6 | `crates/dotnet-vm/src/stack/context_ops.rs:990` — `initialize_static_storage` | keep | F-IDIOM-003; `impl<'a, 'gc> VesInternals<'gc> for VesContext<'a, 'gc>` |
| 7 | `crates/dotnet-vm/src/stack/exception_ops_impl.rs:55` — `throw_type_initialization_exception` | keep | F-IDIOM-003; `impl<'a, 'gc> ExceptionOps<'gc> for VesContext<'a, 'gc>` |
| 8 | `crates/dotnet-vm/src/stack/native_effects.rs:312` — `event_method` | keep | F-IDIOM-003; `impl<'gc> NativeBackend<'gc> for Adapter<'_, '_, 'gc>` |
| 9 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:133` — `active_gc_scope_depth` | keep | F-IDIOM-003; `impl<'a, 'gc> RawMemoryOps<'gc> for VesContext<'a, 'gc>` |
| 10 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:995` — `check_gc_safe_point` | keep | F-IDIOM-003; `impl<'a, 'gc> VmRawMemoryOps<'gc> for VesContext<'a, 'gc>` |
| 11 | `xtask/src/main.rs:540` — `scan_slot_accessor_calls` | keep | F-IDIOM-003; `impl<'ast> Visit<'ast> for SlotAccessorCallVisitor` |

### RsThreadLocalStableMethodCanBeUsed — 75 entries

**Per-file tally:** `crates/dotnet-benchmarks/benches/managed_ptr_serde.rs` (1); `crates/dotnet-utils/src/raw_block.rs` (2); `crates/dotnet-value/src/object/mod.rs` (1); `crates/dotnet-value/src/pointer/tests.rs` (2); `crates/dotnet-value/src/test_helpers.rs` (1); `crates/dotnet-vm-data/src/frame_alloc.rs` (4); `crates/dotnet-vm/src/executor_teardown_tests.rs` (1); `crates/dotnet-vm/src/gc/coordinator.rs` (1); `crates/dotnet-vm/src/native_return_tests.rs` (4); `crates/dotnet-vm/src/native_substring_tests.rs` (58)

| Entry | Snapshot site and nearby symbol | Decision | Reason / finding |
|---|---|---|---|
| 1 | `crates/dotnet-benchmarks/benches/managed_ptr_serde.rs:62` — `drop` | fix (resolved) | F-IDIOM-001; resolved with LocalKey cell method |
| 2 | `crates/dotnet-utils/src/raw_block.rs:73` — `zeroed_with` | keep | F-IDIOM-001; read-modify-write remains in one TLS access |
| 3 | `crates/dotnet-utils/src/raw_block.rs:118` — `drop` | keep | F-IDIOM-001; read-modify-write remains in one TLS access |
| 4 | `crates/dotnet-value/src/object/mod.rs:1103` — `drop` | fix (resolved) | F-IDIOM-001; resolved with LocalKey cell method |
| 5 | `crates/dotnet-value/src/pointer/tests.rs:2042` — `new` | fix (resolved) | F-IDIOM-001; resolved with LocalKey cell method |
| 6 | `crates/dotnet-value/src/pointer/tests.rs:2101` — `drop_arena` | fix (resolved) | F-IDIOM-001; resolved with LocalKey cell method |
| 7 | `crates/dotnet-value/src/test_helpers.rs:37` — `drop` | fix (resolved) | F-IDIOM-001; resolved with LocalKey cell method |
| 8 | `crates/dotnet-vm/src/executor_teardown_tests.rs:981` — `drop` | fix (resolved) | F-IDIOM-001; resolved with LocalKey cell method |
| 9 | `crates/dotnet-vm/src/gc/coordinator.rs:82` — `debug_mark_collection_lock_acquired` | keep | F-IDIOM-001; debug depth read-modify-write remains in one TLS access |
| 10 | `crates/dotnet-vm/src/native_return_tests.rs:117` — `return_body` | fix (resolved) | F-IDIOM-001; resolved with LocalKey cell method |
| 11 | `crates/dotnet-vm/src/native_return_tests.rs:141` — `run` | fix (resolved) | F-IDIOM-001; resolved with LocalKey cell method |
| 12 | `crates/dotnet-vm/src/native_return_tests.rs:142` — `run` | fix (resolved) | F-IDIOM-001; resolved with LocalKey cell method |
| 13 | `crates/dotnet-vm/src/native_return_tests.rs:173` — `run` | fix (resolved) | F-IDIOM-001; resolved with LocalKey cell method |
| 14 | `crates/dotnet-vm/src/native_substring_tests.rs:2153` — `native_inline_calls_body` | keep | F-IDIOM-001; inline-call read-modify-write remains in one TLS access |
| 15 | `crates/dotnet-vm/src/native_substring_tests.rs:2170` — `native_activation_inline_call_budget` | fix (resolved) | F-IDIOM-001; resolved with LocalKey cell method |
| 16 | `crates/dotnet-vm/src/native_substring_tests.rs:3227` — `drop` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 17 | `crates/dotnet-vm/src/native_substring_tests.rs:3250` — `drop` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 18 | `crates/dotnet-vm/src/native_substring_tests.rs:3335` — `boundary_executor_with_initializer` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 19 | `crates/dotnet-vm/src/native_substring_tests.rs:3336` — `boundary_executor_with_initializer` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 20 | `crates/dotnet-vm/src/native_substring_tests.rs:3337` — `boundary_executor_with_initializer` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 21 | `crates/dotnet-vm/src/native_substring_tests.rs:3539` — `capturing_boundary_body` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 22 | `crates/dotnet-vm/src/native_substring_tests.rs:3588` — `native_capture_filters_and_child_cleanup_precede_outer_search` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 23 | `crates/dotnet-vm/src/native_substring_tests.rs:3604` — `native_capture_filters_and_child_cleanup_precede_outer_search` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 24 | `crates/dotnet-vm/src/native_substring_tests.rs:4489` — `native_capture_initializer_does_not_wrap_before_reply` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 25 | `crates/dotnet-vm/src/native_substring_tests.rs:4517` — `capturing_success_body` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 26 | `crates/dotnet-vm/src/native_substring_tests.rs:4555` — `native_capture_child_handler_returns_normally` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 27 | `crates/dotnet-vm/src/native_substring_tests.rs:4646` — `capturing_synchronous_body` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 28 | `crates/dotnet-vm/src/native_substring_tests.rs:4689` — `native_capture_intrinsic_throw_precedes_outer_filter` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 29 | `crates/dotnet-vm/src/native_substring_tests.rs:4692` — `native_capture_intrinsic_throw_precedes_outer_filter` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 30 | `crates/dotnet-vm/src/native_substring_tests.rs:4724` — `capturing_handled_body` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 31 | `crates/dotnet-vm/src/native_substring_tests.rs:4794` — `native_capture_inside_filter_restores_outer_exception` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 32 | `crates/dotnet-vm/src/native_substring_tests.rs:4808` — `native_capture_inside_filter_restores_outer_exception` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 33 | `crates/dotnet-vm/src/native_substring_tests.rs:4840` — `native_boundary_unhandled_runs_child_cleanup` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 34 | `crates/dotnet-vm/src/native_substring_tests.rs:4856` — `native_boundary_unhandled_runs_child_cleanup` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 35 | `crates/dotnet-vm/src/native_substring_tests.rs:4857` — `native_boundary_unhandled_runs_child_cleanup` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 36 | `crates/dotnet-vm/src/native_substring_tests.rs:4891` — `native_boundary_outer_handler_runs_after_child_cleanup` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 37 | `crates/dotnet-vm/src/native_substring_tests.rs:4906` — `native_boundary_outer_handler_runs_after_child_cleanup` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 38 | `crates/dotnet-vm/src/native_substring_tests.rs:4907` — `native_boundary_outer_handler_runs_after_child_cleanup` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 39 | `crates/dotnet-vm/src/native_substring_tests.rs:4925` — `native_boundary_inner_handler_keeps_parent_alive` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 40 | `crates/dotnet-vm/src/native_substring_tests.rs:4941` — `native_boundary_inner_handler_keeps_parent_alive` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 41 | `crates/dotnet-vm/src/native_substring_tests.rs:4942` — `native_boundary_inner_handler_keeps_parent_alive` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 42 | `crates/dotnet-vm/src/native_substring_tests.rs:4971` — `native_boundary_filter_search_precedes_cleanup` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 43 | `crates/dotnet-vm/src/native_substring_tests.rs:4972` — `native_boundary_filter_search_precedes_cleanup` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 44 | `crates/dotnet-vm/src/native_substring_tests.rs:4978` — `native_boundary_filter_search_precedes_cleanup` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 45 | `crates/dotnet-vm/src/native_substring_tests.rs:4980` — `native_boundary_filter_search_precedes_cleanup` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 46 | `crates/dotnet-vm/src/native_substring_tests.rs:4981` — `native_boundary_filter_search_precedes_cleanup` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 47 | `crates/dotnet-vm/src/native_substring_tests.rs:5021` — `native_boundary_stack_trace_and_cleanup_for_nested_calls` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 48 | `crates/dotnet-vm/src/native_substring_tests.rs:5022` — `native_boundary_stack_trace_and_cleanup_for_nested_calls` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 49 | `crates/dotnet-vm/src/native_substring_tests.rs:5075` — `native_boundary_survives_exception_in_outer_filter` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 50 | `crates/dotnet-vm/src/native_substring_tests.rs:5077` — `native_boundary_survives_exception_in_outer_filter` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 51 | `crates/dotnet-vm/src/native_substring_tests.rs:5078` — `native_boundary_survives_exception_in_outer_filter` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 52 | `crates/dotnet-vm/src/native_substring_tests.rs:5098` — `native_boundary_cleanup_survives_collection` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 53 | `crates/dotnet-vm/src/native_substring_tests.rs:5100` — `native_boundary_cleanup_survives_collection` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 54 | `crates/dotnet-vm/src/native_substring_tests.rs:5101` — `native_boundary_cleanup_survives_collection` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 55 | `crates/dotnet-vm/src/native_substring_tests.rs:5110` — `drop` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 56 | `crates/dotnet-vm/src/native_substring_tests.rs:5130` — `drop` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 57 | `crates/dotnet-vm/src/native_substring_tests.rs:5181` — `native_boundary_synchronous_throw_reaches_caller_handler` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 58 | `crates/dotnet-vm/src/native_substring_tests.rs:5185` — `native_boundary_synchronous_throw_reaches_caller_handler` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 59 | `crates/dotnet-vm/src/native_substring_tests.rs:5186` — `native_boundary_synchronous_throw_reaches_caller_handler` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 60 | `crates/dotnet-vm/src/native_substring_tests.rs:5286` — `native_call_in_filter_does_not_reply_to_suspended_call` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 61 | `crates/dotnet-vm/src/native_substring_tests.rs:5288` — `native_call_in_filter_does_not_reply_to_suspended_call` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 62 | `crates/dotnet-vm/src/native_substring_tests.rs:5633` — `finalizer_preserves_filter_and_suspended_native_context` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 63 | `crates/dotnet-vm/src/native_substring_tests.rs:5689` — `finalizer_exception_cannot_reach_interrupted_handler` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 64 | `crates/dotnet-vm/src/native_substring_tests.rs:5824` — `native_call_in_handler_preserves_ambient_unwind` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 65 | `crates/dotnet-vm/src/native_substring_tests.rs:5925` — `native_activations_follow_nested_filter_segments` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 66 | `crates/dotnet-vm/src/native_substring_tests.rs:5938` — `native_activations_follow_nested_filter_segments` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 67 | `crates/dotnet-vm/src/native_substring_tests.rs:5946` — `native_activations_follow_nested_filter_segments` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 68 | `crates/dotnet-vm/src/native_substring_tests.rs:5947` — `native_activations_follow_nested_filter_segments` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 69 | `crates/dotnet-vm/src/native_substring_tests.rs:5977` — `abort_retires_suspended_native_frames` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 70 | `crates/dotnet-vm/src/native_substring_tests.rs:6097` — `finalizer_exception_cannot_enter_interrupted_filter` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 71 | `crates/dotnet-vm/src/native_substring_tests.rs:6215` — `abort_cleans_finalizer_and_parked_caller` | fix (resolved) | F-IDIOM-001; resolved by boundary event/snapshot protocol |
| 72 | `crates/dotnet-vm-data/src/frame_alloc.rs:114` — `allocate_external` | fix (resolved) | F-IDIOM-001; resolved with LocalKey cell method |
| 73 | `crates/dotnet-vm-data/src/frame_alloc.rs:150` — `bytes` | fix (resolved) | F-IDIOM-001; resolved with LocalKey cell method |
| 74 | `crates/dotnet-vm-data/src/frame_alloc.rs:344` — `overflow_and_invalid_layout_leave_state_unchanged` | fix (resolved) | F-IDIOM-001; resolved with LocalKey cell method |
| 75 | `crates/dotnet-vm-data/src/frame_alloc.rs:349` — `overflow_and_invalid_layout_leave_state_unchanged` | fix (resolved) | F-IDIOM-001; resolved with LocalKey cell method |

### RsUnnecessaryCast — 2 entries

**Per-file tally:** `crates/dotnet-assemblies/src/support.rs` (2)

| Entry | Snapshot site and nearby symbol | Decision | Reason / finding |
|---|---|---|---|
| 1 | `crates/dotnet-assemblies/src/support.rs:244` — `collect_stub_candidates` | fix (resolved) | F-IDIOM-002; `as &AssemblyLoader` |
| 2 | `crates/dotnet-assemblies/src/support.rs:443` — `validate_support_contract` | fix (resolved) | F-IDIOM-002; `as &AssemblyLoader` |

### RsUnnecessaryParentheses — 4 entries

**Per-file tally:** `crates/dotnet-utils/src/gc/cross_arena.rs` (2); `crates/dotnet-vm-ops/src/effects.rs` (2)

| Entry | Snapshot site and nearby symbol | Decision | Reason / finding |
|---|---|---|---|
| 1 | `crates/dotnet-utils/src/gc/cross_arena.rs:476` — `drain_leases` | fix (resolved) | F-IDIOM-002; `(` |
| 2 | `crates/dotnet-utils/src/gc/cross_arena.rs:476` — `drain_leases` | fix (resolved) | F-IDIOM-002; `)` |
| 3 | `crates/dotnet-vm-ops/src/effects.rs:1765` — `copy_chunk` | fix (resolved) | F-IDIOM-002; `(` |
| 4 | `crates/dotnet-vm-ops/src/effects.rs:1765` — `copy_chunk` | fix (resolved) | F-IDIOM-002; `)` |

### RsUnnecessaryQualifications — 296 entries

**Per-file tally:** `crates/dotnet-benchmarks/benches/runtime_primitives.rs` (1); `crates/dotnet-intrinsics-reflection/src/types/type_queries.rs` (2); `crates/dotnet-intrinsics-string/src/constructors.rs` (1); `crates/dotnet-intrinsics-string/src/simd.rs` (1); `crates/dotnet-intrinsics-threading/src/volatile.rs` (2); `crates/dotnet-intrinsics-unsafe/src/lib.rs` (1); `crates/dotnet-intrinsics-unsafe/src/unsafe_ptr.rs` (3); `crates/dotnet-macros-core/src/lib.rs` (11); `crates/dotnet-macros/src/lib.rs` (2); `crates/dotnet-metrics/src/lib.rs` (1); `crates/dotnet-pinvoke/src/call.rs` (4); `crates/dotnet-runtime-memory/src/access.rs` (4); `crates/dotnet-runtime-resolver/src/layout.rs` (1); `crates/dotnet-runtime-resolver/src/types.rs` (2); `crates/dotnet-tracer/src/lib.rs` (2); `crates/dotnet-types/src/generics.rs` (1); `crates/dotnet-types/src/members.rs` (3); `crates/dotnet-utils/src/atomic.rs` (4); `crates/dotnet-utils/src/gc/cross_arena.rs` (3); `crates/dotnet-utils/src/sync/lock_order.rs` (3); `crates/dotnet-value/src/layout.rs` (1); `crates/dotnet-value/src/object_tests.rs` (3); `crates/dotnet-value/src/pointer/cross_arena.rs` (1); `crates/dotnet-value/src/pointer/tests.rs` (16); `crates/dotnet-value/src/stack_value.rs` (2); `crates/dotnet-vm-data/src/lib.rs` (1); `crates/dotnet-vm-ops/src/effects.rs` (7); `crates/dotnet-vm-ops/src/ops.rs` (4); `crates/dotnet-vm/src/condition_wait_tests.rs` (48); `crates/dotnet-vm/src/conversion_regression_tests.rs` (1); `crates/dotnet-vm/src/executor.rs` (3); `crates/dotnet-vm/src/executor_collection_tests.rs` (6); `crates/dotnet-vm/src/intrinsics/object_ops.rs` (12); `crates/dotnet-vm/src/layout.rs` (1); `crates/dotnet-vm/src/native_return_tests.rs` (1); `crates/dotnet-vm/src/native_substring_tests.rs` (38); `crates/dotnet-vm/src/pointer_provenance_tests.rs` (23); `crates/dotnet-vm/src/stack/context.rs` (2); `crates/dotnet-vm/src/stack/context_ops.rs` (6); `crates/dotnet-vm/src/stack/mod.rs` (1); `crates/dotnet-vm/src/stack/native_effects.rs` (5); `crates/dotnet-vm/src/stack/ops.rs` (2); `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs` (54); `crates/dotnet-vm/src/sync/mod.rs` (1); `crates/dotnet-vm/src/sync/threaded.rs` (2); `crates/dotnet-vm/src/threading/basic.rs` (1); `crates/dotnet-vm/src/threading/workers.rs` (2)

| Entry | Snapshot site and nearby symbol | Decision | Reason / finding |
|---|---|---|---|
| 1 | `crates/dotnet-benchmarks/benches/runtime_primitives.rs:55` — `primitive_i32_vector` | fix (resolved) | F-QUAL-001; `std::mem::` |
| 2 | `crates/dotnet-intrinsics-reflection/src/types/type_queries.rs:30` — `resolve_base_runtime_type` | fix (resolved) | F-QUAL-001; `dotnet_types::` |
| 3 | `crates/dotnet-intrinsics-reflection/src/types/type_queries.rs:369` — `intrinsic_type_get_enum_underlying_type` | fix (resolved) | F-QUAL-001; `dotnetdll::prelude::` |
| 4 | `crates/dotnet-intrinsics-string/src/constructors.rs:207` — `intrinsic_string_ctor_char_array` | fix (resolved) | F-QUAL-001; `crate::` |
| 5 | `crates/dotnet-intrinsics-string/src/simd.rs:10` — `probe_sequence_equal_u16` | fix (resolved) | F-QUAL-001; `core::mem::` |
| 6 | `crates/dotnet-intrinsics-threading/src/volatile.rs:42` — `intrinsic_volatile_read` | fix (resolved) | F-QUAL-001; `std::mem::` |
| 7 | `crates/dotnet-intrinsics-threading/src/volatile.rs:241` — `intrinsic_volatile_write` | fix (resolved) | F-QUAL-001; `std::mem::` |
| 8 | `crates/dotnet-intrinsics-unsafe/src/lib.rs:80` — `ptr_info` | fix (resolved) | F-QUAL-001; `dotnet_value::object::` |
| 9 | `crates/dotnet-intrinsics-unsafe/src/unsafe_ptr.rs:104` — `intrinsic_unsafe_null_ref` | fix (resolved) | F-QUAL-001; `std::` |
| 10 | `crates/dotnet-intrinsics-unsafe/src/unsafe_ptr.rs:446` — `intrinsic_unsafe_bitcast` | fix (resolved) | F-QUAL-001; `std::mem::` |
| 11 | `crates/dotnet-intrinsics-unsafe/src/unsafe_ptr.rs:507` — `intrinsic_unsafe_bitcast` | fix (resolved) | F-QUAL-001; `std::mem::` |
| 12 | `crates/dotnet-macros/src/lib.rs:314` — `find_filter_fn` | keep | F-QUAL-001: macro hygiene, sync identity or doc target; `syn::` |
| 13 | `crates/dotnet-macros/src/lib.rs:329` — `assert_filter_tail_is_true` | keep | F-QUAL-001: macro hygiene, sync identity or doc target; `syn::` |
| 14 | `crates/dotnet-macros-core/src/lib.rs:136` — `parse` | keep | F-QUAL-001: macro hygiene, sync identity or doc target; `syn::` |
| 15 | `crates/dotnet-macros-core/src/lib.rs:733` — `optional_async_options_preserve_stacked_equality` | keep | F-QUAL-001: macro hygiene, sync identity or doc target; `syn::` |
| 16 | `crates/dotnet-macros-core/src/lib.rs:735` — `optional_async_options_preserve_stacked_equality` | keep | F-QUAL-001: macro hygiene, sync identity or doc target; `syn::` |
| 17 | `crates/dotnet-macros-core/src/lib.rs:738` — `optional_async_options_preserve_stacked_equality` | keep | F-QUAL-001: macro hygiene, sync identity or doc target; `syn::` |
| 18 | `crates/dotnet-macros-core/src/lib.rs:739` — `optional_async_options_preserve_stacked_equality` | keep | F-QUAL-001: macro hygiene, sync identity or doc target; `syn::` |
| 19 | `crates/dotnet-macros-core/src/lib.rs:757` — `intrinsic_argument_syntax` | keep | F-QUAL-001: macro hygiene, sync identity or doc target; `syn::` |
| 20 | `crates/dotnet-macros-core/src/lib.rs:760` — `intrinsic_argument_syntax` | keep | F-QUAL-001: macro hygiene, sync identity or doc target; `syn::` |
| 21 | `crates/dotnet-macros-core/src/lib.rs:770` — `intrinsic_argument_syntax` | keep | F-QUAL-001: macro hygiene, sync identity or doc target; `syn::` |
| 22 | `crates/dotnet-macros-core/src/lib.rs:931` — `test_parse_trait_alias_generics` | keep | F-QUAL-001: macro hygiene, sync identity or doc target; `syn::` |
| 23 | `crates/dotnet-macros-core/src/lib.rs:942` — `test_struct_mapping_rejects_duplicate_binding_names` | keep | F-QUAL-001: macro hygiene, sync identity or doc target; `syn::` |
| 24 | `crates/dotnet-macros-core/src/lib.rs:955` — `test_struct_mapping_rejects_duplicate_field_names` | keep | F-QUAL-001: macro hygiene, sync identity or doc target; `syn::` |
| 25 | `crates/dotnet-metrics/src/lib.rs:1442` — `snapshot_includes_always_on_gc_pause_percentiles` | fix (resolved) | F-QUAL-001; `std::time::` |
| 26 | `crates/dotnet-pinvoke/src/call.rs:1257` — `typed_reference_input_abi_is_a_two_pointer_temporary` | fix (resolved) | F-QUAL-001; `std::mem::` |
| 27 | `crates/dotnet-pinvoke/src/call.rs:1258` — `typed_reference_input_abi_is_a_two_pointer_temporary` | fix (resolved) | F-QUAL-001; `std::mem::` |
| 28 | `crates/dotnet-pinvoke/src/call.rs:1261` — `typed_reference_input_abi_is_a_two_pointer_temporary` | fix (resolved) | F-QUAL-001; `std::mem::` |
| 29 | `crates/dotnet-pinvoke/src/call.rs:1262` — `typed_reference_input_abi_is_a_two_pointer_temporary` | fix (resolved) | F-QUAL-001; `std::mem::` |
| 30 | `crates/dotnet-runtime-memory/src/access.rs:169` — `object_atomic` | fix (resolved) | F-QUAL-001; `std::mem::` |
| 31 | `crates/dotnet-runtime-memory/src/access.rs:2079` — `dummy_wb_entry` | fix (resolved) | F-QUAL-001; `gc_arena::` |
| 32 | `crates/dotnet-runtime-memory/src/access.rs:2085` — `dummy_wb_entry` | fix (resolved) | F-QUAL-001; `dotnet_utils::gc::` |
| 33 | `crates/dotnet-runtime-memory/src/access.rs:2086` — `dummy_wb_entry` | fix (resolved) | F-QUAL-001; `dotnet_value::` |
| 34 | `crates/dotnet-runtime-resolver/src/layout.rs:34` — `integral_to_usize` | fix (resolved) | F-QUAL-001; `dotnetdll::binary::signature::attribute::` |
| 35 | `crates/dotnet-runtime-resolver/src/types.rs:169` — `has_finalizer` | fix (resolved) | F-QUAL-001; `dotnet_types::generics::` |
| 36 | `crates/dotnet-runtime-resolver/src/types.rs:171` — `has_finalizer` | fix (resolved) | F-QUAL-001; `dotnetdll::prelude::` |
| 37 | `crates/dotnet-tracer/src/lib.rs:70` — `(module context)` | fix (resolved) | F-QUAL-001; `dotnet_metrics::` |
| 38 | `crates/dotnet-tracer/src/lib.rs:710` — `init_tracing` | fix (resolved) | F-QUAL-001; `std::sync::` |
| 39 | `crates/dotnet-types/src/generics.rs:189` — `has_default_constructor` | fix (resolved) | F-QUAL-001; `crate::generics::` |
| 40 | `crates/dotnet-types/src/members.rs:145` — `method_index` | fix (resolved) | F-QUAL-001; `dotnetdll::prelude::` |
| 41 | `crates/dotnet-types/src/members.rs:159` — `body` | fix (resolved) | F-QUAL-001; `dotnetdll::prelude::` |
| 42 | `crates/dotnet-types/src/members.rs:262` — `make_type_index` | fix (resolved) | F-QUAL-001; `std::mem::` |
| 43 | `crates/dotnet-utils/src/atomic.rs:336` — `execute` | fix (resolved) | F-QUAL-001; `std::mem::` |
| 44 | `crates/dotnet-utils/src/atomic.rs:347` — `execute` | fix (resolved) | F-QUAL-001; `std::` |
| 45 | `crates/dotnet-utils/src/atomic.rs:359` — `execute` | fix (resolved) | F-QUAL-001; `std::` |
| 46 | `crates/dotnet-utils/src/atomic.rs:929` — `test_orderings` | fix (resolved) | F-QUAL-001; `std::` |
| 47 | `crates/dotnet-utils/src/gc/cross_arena.rs:872` — `unregister_waits_for_outstanding_lease` | fix (resolved) | F-QUAL-001; `std::` |
| 48 | `crates/dotnet-utils/src/gc/cross_arena.rs:879` — `unregister_waits_for_outstanding_lease` | fix (resolved) | F-QUAL-001; `std::` |
| 49 | `crates/dotnet-utils/src/gc/cross_arena.rs:879` — `unregister_waits_for_outstanding_lease` | fix (resolved) | F-QUAL-001; `std::time::` |
| 50 | `crates/dotnet-utils/src/sync/lock_order.rs:429` — `assert_can_acquire_after` | keep | F-QUAL-001; `super::AcquireAfter` names the parent-owned lock-order trait in the conformance bound; importing it would hide that ownership behind the test module glob |
| 51 | `crates/dotnet-utils/src/sync/lock_order.rs:430` — `assert_can_acquire_after` | keep | F-QUAL-001; `super::LockLevel` names the parent-owned lock-order trait in the conformance bound; importing it would hide that ownership behind the test module glob |
| 52 | `crates/dotnet-utils/src/sync/lock_order.rs:431` — `assert_can_acquire_after` | keep | F-QUAL-001; `super::LockLevel` names the parent-owned lock-order trait in the conformance bound; importing it would hide that ownership behind the test module glob |
| 53 | `crates/dotnet-value/src/layout.rs:657` — `read_from` | fix (resolved) | F-QUAL-001; `dotnet_types::` |
| 54 | `crates/dotnet-value/src/object_tests.rs:28` — `new` | fix (resolved) | F-QUAL-001; imported `Arc` and `AtomicBool` from `dotnet_utils::sync` |
| 55 | `crates/dotnet-value/src/object_tests.rs:178` — `codec_local_invalid_control` | fix (resolved) | F-QUAL-001; `std::sync::` |
| 56 | `crates/dotnet-value/src/object_tests.rs:202` — `publication_drops` | fix (resolved) | F-QUAL-001; `std::sync::` |
| 57 | `crates/dotnet-value/src/pointer/cross_arena.rs:91` — `retained_stack_value_lease` | fix (resolved) | F-QUAL-001; `dotnet_utils::gc::` |
| 58 | `crates/dotnet-value/src/pointer/tests.rs:39` — `static_storage_base` | fix (resolved) | F-QUAL-001; `hashbrown::` |
| 59 | `crates/dotnet-value/src/pointer/tests.rs:127` — `owner_retention_transient_native_int_conversion_is_rejected` | fix (resolved) | F-QUAL-001; `crate::pointer::` |
| 60 | `crates/dotnet-value/src/pointer/tests.rs:986` — `frame_home_access_checks_liveness_and_range` | fix (resolved) | F-QUAL-001; `dotnet_types::error::` |
| 61 | `crates/dotnet-value/src/pointer/tests.rs:996` — `frame_home_access_checks_liveness_and_range` | fix (resolved) | F-QUAL-001; `dotnet_types::error::` |
| 62 | `crates/dotnet-value/src/pointer/tests.rs:1228` — `test_managed_ptr_serialization_bugs_reproduction` | fix (resolved) | F-QUAL-001; `crate::layout::` |
| 63 | `crates/dotnet-value/src/pointer/tests.rs:1229` — `test_managed_ptr_serialization_bugs_reproduction` | fix (resolved) | F-QUAL-001; `hashbrown::` |
| 64 | `crates/dotnet-value/src/pointer/tests.rs:1232` — `test_managed_ptr_serialization_bugs_reproduction` | fix (resolved) | F-QUAL-001; `crate::layout::` |
| 65 | `crates/dotnet-value/src/pointer/tests.rs:1235` — `test_managed_ptr_serialization_bugs_reproduction` | fix (resolved) | F-QUAL-001; `crate::storage::` |
| 66 | `crates/dotnet-value/src/pointer/tests.rs:1388` — `ptr_arith_managed_ptr_checked_offset_reports_errors` | fix (resolved) | F-QUAL-001; `crate::pointer::` |
| 67 | `crates/dotnet-value/src/pointer/tests.rs:1484` — `ptr_arith_managed_ptr_checked_offset_reports_errors` | fix (resolved) | F-QUAL-001; `crate::pointer::` |
| 68 | `crates/dotnet-value/src/pointer/tests.rs:1507` — `ptr_arith_managed_ptr_checked_offset_reports_errors` | fix (resolved) | F-QUAL-001; `crate::pointer::` |
| 69 | `crates/dotnet-value/src/pointer/tests.rs:2532` — `retained_frame_home_range_checks_full_access` | fix (resolved) | F-QUAL-001; `std::ptr::` |
| 70 | `crates/dotnet-value/src/pointer/tests.rs:2541` — `retained_frame_home_range_checks_full_access` | fix (resolved) | F-QUAL-001; `super::` |
| 71 | `crates/dotnet-value/src/pointer/tests.rs:2549` — `retained_frame_home_range_checks_full_access` | fix (resolved) | F-QUAL-001; `dotnet_types::error::` |
| 72 | `crates/dotnet-value/src/pointer/tests.rs:2553` — `retained_frame_home_range_checks_full_access` | fix (resolved) | F-QUAL-001; `dotnet_types::error::` |
| 73 | `crates/dotnet-value/src/pointer/tests.rs:2557` — `retained_frame_home_range_checks_full_access` | fix (resolved) | F-QUAL-001; `dotnet_types::error::` |
| 74 | `crates/dotnet-value/src/stack_value.rs:1189` — `managed_pointer_numeric_operations_use_cached_non_heap_address` | fix (resolved) | F-QUAL-001; `crate::` |
| 75 | `crates/dotnet-value/src/stack_value.rs:1204` — `typed_reference_slot_copy_retains_type_lifetime_and_pointer_origin` | fix (resolved) | F-QUAL-001; `crate::` |
| 76 | `crates/dotnet-vm/src/condition_wait_tests.rs:117` — `condition_wait_release_before_recheck_keeps_ownership` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 77 | `crates/dotnet-vm/src/condition_wait_tests.rs:402` — `finalizer_wait_executor` | fix (resolved) | F-QUAL-001; `crate::executor::` |
| 78 | `crates/dotnet-vm/src/condition_wait_tests.rs:406` — `finalizer_wait_executor` | fix (resolved) | F-QUAL-001; `crate::executor::` |
| 79 | `crates/dotnet-vm/src/condition_wait_tests.rs:452` — `start_condition_monitor` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 80 | `crates/dotnet-vm/src/condition_wait_tests.rs:475` — `condition_wait_registration_schedules_do_not_lose_release` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 81 | `crates/dotnet-vm/src/condition_wait_tests.rs:511` — `condition_wait_stale_release_never_grants_ownership` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 82 | `crates/dotnet-vm/src/condition_wait_tests.rs:544` — `condition_wait_executor_parks_outside_mutation_and_wakes_on_release` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 83 | `crates/dotnet-vm/src/condition_wait_tests.rs:599` — `condition_wait_cancel_does_not_manufacture_finalizer_readiness` | fix (resolved) | F-QUAL-001; `crate::executor::` |
| 84 | `crates/dotnet-vm/src/condition_wait_tests.rs:602` — `condition_wait_cancel_does_not_manufacture_finalizer_readiness` | fix (resolved) | F-QUAL-001; `crate::executor::` |
| 85 | `crates/dotnet-vm/src/condition_wait_tests.rs:629` — `condition_wait_timeout_does_not_acquire_or_release_monitor` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 86 | `crates/dotnet-vm/src/condition_wait_tests.rs:645` — `condition_wait_executor_participates_in_real_gc_stop` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 87 | `crates/dotnet-vm/src/condition_wait_tests.rs:682` — `condition_wait_spurious_and_duplicate_wakes_only_trigger_rechecks` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 88 | `crates/dotnet-vm/src/condition_wait_tests.rs:720` — `condition_wait_cancellation_releases_registration_without_releasing_monitor` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 89 | `crates/dotnet-vm/src/condition_wait_tests.rs:726` — `condition_wait_cancellation_releases_registration_without_releasing_monitor` | fix (resolved) | F-QUAL-001; `crate::executor::` |
| 90 | `crates/dotnet-vm/src/condition_wait_tests.rs:754` — `condition_wait_deadline_survives_real_collection` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 91 | `crates/dotnet-vm/src/condition_wait_tests.rs:806` — `condition_wait_deadlines_and_notifications_are_activation_local` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 92 | `crates/dotnet-vm/src/condition_wait_tests.rs:850` — `condition_error_after_acquire_body` | fix (resolved) | F-QUAL-001; `dotnet_types::error::` |
| 93 | `crates/dotnet-vm/src/condition_wait_tests.rs:851` — `condition_error_after_acquire_body` | fix (resolved) | F-QUAL-001; `dotnet_types::error::` |
| 94 | `crates/dotnet-vm/src/condition_wait_tests.rs:861` — `condition_wait_acquisition_survives_later_native_failure` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 95 | `crates/dotnet-vm/src/condition_wait_tests.rs:949` — `condition_wait_foreign_monitor_owner_survives_collection` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 96 | `crates/dotnet-vm/src/condition_wait_tests.rs:998` — `condition_wait_unprobed_monitor_rechecks_before_parking` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 97 | `crates/dotnet-vm/src/condition_wait_tests.rs:1022` — `condition_wait_unprobed_monitor_rechecks_before_parking` | fix (resolved) | F-QUAL-001; `crate::executor::` |
| 98 | `crates/dotnet-vm/src/condition_wait_tests.rs:1026` — `condition_wait_unprobed_monitor_rechecks_before_parking` | fix (resolved) | F-QUAL-001; `crate::executor::` |
| 99 | `crates/dotnet-vm/src/condition_wait_tests.rs:1139` — `monitor_byref_contended_dispatch_survives_collection` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 100 | `crates/dotnet-vm/src/condition_wait_tests.rs:1309` — `static_descriptor_body` | fix (resolved) | F-QUAL-001; `dotnet_types::error::` |
| 101 | `crates/dotnet-vm/src/condition_wait_tests.rs:1506` — `monitor_static_timed_wait_survives_collection_and_writes_result` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 102 | `crates/dotnet-vm/src/condition_wait_tests.rs:1589` — `monitor_unmanaged_timed_wait_survives_collection_and_writes_result` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 103 | `crates/dotnet-vm/src/condition_wait_tests.rs:1686` — `release_monitor_for_test` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 104 | `crates/dotnet-vm/src/condition_wait_tests.rs:1692` — `hold_monitor_for_timeout` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 105 | `crates/dotnet-vm/src/condition_wait_tests.rs:1711` — `monitor_timed_byref_alias_writes_once_for_timeout_and_acquisition` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 106 | `crates/dotnet-vm/src/condition_wait_tests.rs:1822` — `monitor_timed_byref_writes_result_and_preserves_ownership` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 107 | `crates/dotnet-vm/src/condition_wait_tests.rs:1870` — `monitor_heap_byref_survives_collection_and_writes_owner` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 108 | `crates/dotnet-vm/src/condition_wait_tests.rs:2112` — `monitor_wait_rejects_retired_frame_home_without_unlocking` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 109 | `crates/dotnet-vm/src/condition_wait_tests.rs:2175` — `monitor_timed_dispatch_keeps_deadline_across_collection` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 110 | `crates/dotnet-vm/src/condition_wait_tests.rs:2380` — `drop` | fix (resolved) | F-QUAL-001; `super::super::` |
| 111 | `crates/dotnet-vm/src/condition_wait_tests.rs:2618` — `sleep_park_participates_in_gc_and_abort` | fix (resolved) | F-QUAL-001; `crate::executor::` |
| 112 | `crates/dotnet-vm/src/condition_wait_tests.rs:2685` — `monitor_pulse_schedules_preserve_wake_and_recursion` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 113 | `crates/dotnet-vm/src/condition_wait_tests.rs:2733` — `monitor_timeout_and_pulse_are_serialized_and_cancel_removes_waiter` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 114 | `crates/dotnet-vm/src/condition_wait_tests.rs:2734` — `monitor_timeout_and_pulse_are_serialized_and_cancel_removes_waiter` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 115 | `crates/dotnet-vm/src/condition_wait_tests.rs:2735` — `monitor_timeout_and_pulse_are_serialized_and_cancel_removes_waiter` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 116 | `crates/dotnet-vm/src/condition_wait_tests.rs:2868` — `native_monitor_wait_pulse_restores_recursion_after_suspension` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 117 | `crates/dotnet-vm/src/condition_wait_tests.rs:3619` — `pool_worker_parks_participates_in_gc_reuses_executor_and_shuts_down` | fix (resolved) | F-QUAL-001; `crate::test_utils::` |
| 118 | `crates/dotnet-vm/src/condition_wait_tests.rs:3635` — `pool_worker_parks_participates_in_gc_reuses_executor_and_shuts_down` | fix (resolved) | F-QUAL-001; `crate::test_utils::` |
| 119 | `crates/dotnet-vm/src/condition_wait_tests.rs:3667` — `pool_worker_parks_participates_in_gc_reuses_executor_and_shuts_down` | keep | F-QUAL-001: macro hygiene, sync identity or doc target; `dotnet_utils::sync::` |
| 120 | `crates/dotnet-vm/src/condition_wait_tests.rs:3689` — `pool_worker_parks_participates_in_gc_reuses_executor_and_shuts_down` | keep | F-QUAL-001: macro hygiene, sync identity or doc target; `dotnet_utils::sync::` |
| 121 | `crates/dotnet-vm/src/condition_wait_tests.rs:3924` — `event_wait_parked_gc_and_signal` | fix (resolved) | F-QUAL-001; `crate::executor::` |
| 122 | `crates/dotnet-vm/src/condition_wait_tests.rs:3948` — `event_wait_spurious_wakes_and_abort_remove_registration` | fix (resolved) | F-QUAL-001; `crate::executor::` |
| 123 | `crates/dotnet-vm/src/condition_wait_tests.rs:4215` — `join_timeout_survives_collection` | fix (resolved) | F-QUAL-001; `std::sync::` |
| 124 | `crates/dotnet-vm/src/conversion_regression_tests.rs:120` — `new_static_field` | fix (resolved) | F-QUAL-001; `resolved_mod::` |
| 125 | `crates/dotnet-vm/src/executor.rs:480` — `run_thread_start_inner` | fix (resolved) | F-QUAL-001; `std::sync::atomic::` |
| 126 | `crates/dotnet-vm/src/executor.rs:741` — `install_thread_start_with_args` | fix (resolved) | F-QUAL-001; `dotnet_value::object::` |
| 127 | `crates/dotnet-vm/src/executor.rs:750` — `install_thread_start_with_args` | fix (resolved) | F-QUAL-001; `dotnet_value::object::` |
| 128 | `crates/dotnet-vm/src/executor_collection_tests.rs:109` — `run_vm_collection` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 129 | `crates/dotnet-vm/src/executor_collection_tests.rs:303` — `probe_any` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 130 | `crates/dotnet-vm/src/executor_collection_tests.rs:310` — `spawn_owner` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 131 | `crates/dotnet-vm/src/executor_collection_tests.rs:565` — `arena_lifetime_mark_command_rejects_stale_registration` | fix (resolved) | F-QUAL-001; `dotnet_utils::gc::` |
| 132 | `crates/dotnet-vm/src/executor_collection_tests.rs:573` — `arena_lifetime_mark_command_rejects_stale_registration` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 133 | `crates/dotnet-vm/src/executor_collection_tests.rs:587` — `arena_lifetime_mark_command_rejects_stale_registration` | fix (resolved) | F-QUAL-001; `dotnet_utils::gc::` |
| 134 | `crates/dotnet-vm/src/intrinsics/object_ops.rs:120` — `read_enum_value` | fix (resolved) | F-QUAL-001; `dotnetdll::prelude::` |
| 135 | `crates/dotnet-vm/src/intrinsics/object_ops.rs:128` — `read_enum_value` | fix (resolved) | F-QUAL-001; `dotnetdll::prelude::` |
| 136 | `crates/dotnet-vm/src/intrinsics/object_ops.rs:136` — `read_enum_value` | fix (resolved) | F-QUAL-001; `dotnetdll::prelude::` |
| 137 | `crates/dotnet-vm/src/intrinsics/object_ops.rs:144` — `read_enum_value` | fix (resolved) | F-QUAL-001; `dotnetdll::prelude::` |
| 138 | `crates/dotnet-vm/src/intrinsics/object_ops.rs:152` — `read_enum_value` | fix (resolved) | F-QUAL-001; `dotnetdll::prelude::` |
| 139 | `crates/dotnet-vm/src/intrinsics/object_ops.rs:160` — `read_enum_value` | fix (resolved) | F-QUAL-001; `dotnetdll::prelude::` |
| 140 | `crates/dotnet-vm/src/intrinsics/object_ops.rs:168` — `read_enum_value` | fix (resolved) | F-QUAL-001; `dotnetdll::prelude::` |
| 141 | `crates/dotnet-vm/src/intrinsics/object_ops.rs:176` — `read_enum_value` | fix (resolved) | F-QUAL-001; `dotnetdll::prelude::` |
| 142 | `crates/dotnet-vm/src/intrinsics/object_ops.rs:184` — `read_enum_value` | fix (resolved) | F-QUAL-001; `dotnetdll::prelude::` |
| 143 | `crates/dotnet-vm/src/intrinsics/object_ops.rs:190` — `read_enum_value` | fix (resolved) | F-QUAL-001; `dotnetdll::prelude::` |
| 144 | `crates/dotnet-vm/src/intrinsics/object_ops.rs:366` — `generic_enum_type` | fix (resolved) | F-QUAL-001; `dotnetdll::prelude::` |
| 145 | `crates/dotnet-vm/src/intrinsics/object_ops.rs:444` — `runtime_type_from_desc` | fix (resolved) | F-QUAL-001; `dotnet_types::` |
| 146 | `crates/dotnet-vm/src/layout.rs:166` — `runtime_handle_value_slots_use_object_ref_layout` | fix (resolved) | F-QUAL-001; `std::mem::` |
| 147 | `crates/dotnet-vm/src/native_return_tests.rs:42` — `method` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::` |
| 148 | `crates/dotnet-vm/src/native_substring_tests.rs:404` — `pending_finalizer_runs_in_separate_segment_during_copy` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::signature::` |
| 149 | `crates/dotnet-vm/src/native_substring_tests.rs:786` — `indirect_opcode_refuses_without_consuming_operands` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::signature::` |
| 150 | `crates/dotnet-vm/src/native_substring_tests.rs:799` — `indirect_opcode_refuses_without_consuming_operands` | fix (resolved) | F-QUAL-001; `crate::error::` |
| 151 | `crates/dotnet-vm/src/native_substring_tests.rs:800` — `indirect_opcode_refuses_without_consuming_operands` | fix (resolved) | F-QUAL-001; `dotnet_types::error::` |
| 152 | `crates/dotnet-vm/src/native_substring_tests.rs:1422` — `native_test_select_reject` | fix (resolved) | F-QUAL-001; `crate::error::` |
| 153 | `crates/dotnet-vm/src/native_substring_tests.rs:1431` — `native_test_host_failure` | fix (resolved) | F-QUAL-001; `crate::error::` |
| 154 | `crates/dotnet-vm/src/native_substring_tests.rs:1891` — `native_activation_forgotten_access_is_terminal` | fix (resolved) | F-QUAL-001; `crate::error::` |
| 155 | `crates/dotnet-vm/src/native_substring_tests.rs:1892` — `native_activation_forgotten_access_is_terminal` | fix (resolved) | F-QUAL-001; `crate::error::` |
| 156 | `crates/dotnet-vm/src/native_substring_tests.rs:1906` — `native_activation_unregistered_pending_is_terminal` | fix (resolved) | F-QUAL-001; `crate::error::` |
| 157 | `crates/dotnet-vm/src/native_substring_tests.rs:1907` — `native_activation_unregistered_pending_is_terminal` | fix (resolved) | F-QUAL-001; `crate::error::` |
| 158 | `crates/dotnet-vm/src/native_substring_tests.rs:1977` — `native_activation_rejects_each_changed_caller_component` | fix (resolved) | F-QUAL-001; `crate::error::` |
| 159 | `crates/dotnet-vm/src/native_substring_tests.rs:1978` — `native_activation_rejects_each_changed_caller_component` | fix (resolved) | F-QUAL-001; `crate::error::` |
| 160 | `crates/dotnet-vm/src/native_substring_tests.rs:2974` — `push_new_string` | fix (resolved) | F-QUAL-001; `crate::error::` |
| 161 | `crates/dotnet-vm/src/native_substring_tests.rs:3300` — `boundary_executor_with_initializer` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::members::` |
| 162 | `crates/dotnet-vm/src/native_substring_tests.rs:3316` — `boundary_executor_with_initializer` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::members::` |
| 163 | `crates/dotnet-vm/src/native_substring_tests.rs:3572` — `native_capture_filters_and_child_cleanup_precede_outer_search` | fix (resolved) | F-QUAL-001; `dotnet_vm_data::` |
| 164 | `crates/dotnet-vm/src/native_substring_tests.rs:3614` — `native_capture_filters_and_child_cleanup_precede_outer_search` | fix (resolved) | F-QUAL-001; `dotnet_vm_data::` |
| 165 | `crates/dotnet-vm/src/native_substring_tests.rs:3705` — `static_store_operand_collection` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::members::` |
| 166 | `crates/dotnet-vm/src/native_substring_tests.rs:4091` — `claim_initializer_for` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 167 | `crates/dotnet-vm/src/native_substring_tests.rs:4116` — `native_initializer_wait_rechecks_cycle_after_real_collection` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 168 | `crates/dotnet-vm/src/native_substring_tests.rs:4160` — `native_initializer_wait_replies_before_collection_when_owner_is_busy` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 169 | `crates/dotnet-vm/src/native_substring_tests.rs:4193` — `native_initializer_wait_observes_published_success` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 170 | `crates/dotnet-vm/src/native_substring_tests.rs:4219` — `native_initializer_wait_abort_removes_graph_edge` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 171 | `crates/dotnet-vm/src/native_substring_tests.rs:4263` — `drop` | fix (resolved) | F-QUAL-001; `super::` |
| 172 | `crates/dotnet-vm/src/native_substring_tests.rs:4268` — `drop` | fix (resolved) | F-QUAL-001; `dotnet_utils::` |
| 173 | `crates/dotnet-vm/src/native_substring_tests.rs:4295` — `drop` | fix (resolved) | F-QUAL-001; `super::` |
| 174 | `crates/dotnet-vm/src/native_substring_tests.rs:4698` — `native_capture_intrinsic_throw_precedes_outer_filter` | fix (resolved) | F-QUAL-001; `dotnet_vm_data::` |
| 175 | `crates/dotnet-vm/src/native_substring_tests.rs:4830` — `native_boundary_unhandled_runs_child_cleanup` | fix (resolved) | F-QUAL-001; `dotnet_vm_data::` |
| 176 | `crates/dotnet-vm/src/native_substring_tests.rs:4968` — `native_boundary_filter_search_precedes_cleanup` | fix (resolved) | F-QUAL-001; `dotnet_vm_data::` |
| 177 | `crates/dotnet-vm/src/native_substring_tests.rs:5815` — `native_call_in_handler_preserves_ambient_unwind` | fix (resolved) | F-QUAL-001; `dotnet_vm_data::exceptions::` |
| 178 | `crates/dotnet-vm/src/native_substring_tests.rs:6258` — `managed_join_returns_before_native_worker_exits` | fix (resolved) | F-QUAL-001; `std::sync::` |
| 179 | `crates/dotnet-vm/src/native_substring_tests.rs:6305` — `managed_join_resolves_completion_published_after_start` | fix (resolved) | F-QUAL-001; `std::sync::` |
| 180 | `crates/dotnet-vm/src/native_substring_tests.rs:6375` — `native_error_retires_initializer_frame_before_future_drop` | fix (resolved) | F-QUAL-001; `std::future::` |
| 181 | `crates/dotnet-vm/src/native_substring_tests.rs:6536` — `worker_teardown_cleans_native_before_retiring_caller` | fix (resolved) | F-QUAL-001; `std::future::` |
| 182 | `crates/dotnet-vm/src/native_substring_tests.rs:6642` — `drop` | fix (resolved) | F-QUAL-001; `dotnet_types::error::` |
| 183 | `crates/dotnet-vm/src/native_substring_tests.rs:6741` — `drop` | fix (resolved) | F-QUAL-001; `dotnet_types::error::` |
| 184 | `crates/dotnet-vm/src/native_substring_tests.rs:6806` — `static_fields_complete_without_activation` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::members::` |
| 185 | `crates/dotnet-vm/src/native_substring_tests.rs:7416` — `test_failing_value_constructor` | fix (resolved) | F-QUAL-001; `crate::error::` |
| 186 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:508` — `void_sig` | fix (resolved) | F-QUAL-001; `dotnetdll::binary::signature::kinds::` |
| 187 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:513` — `void_sig` | fix (resolved) | F-QUAL-001; `dotnetdll::binary::signature::kinds::` |
| 188 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:528` — `new_static_field` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::` |
| 189 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:549` — `run_program` | fix (resolved) | F-QUAL-001; `dotnetdll::prelude::` |
| 190 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:559` — `run_program_with_collection` | fix (resolved) | F-QUAL-001; `dotnetdll::prelude::` |
| 191 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:592` — `build_program` | fix (resolved) | F-QUAL-001; `dotnetdll::prelude::` |
| 192 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:807` — `p35_ldflda_on_stack_value_type_then_ldind` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::assembly::` |
| 193 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:810` — `p35_ldflda_on_stack_value_type_then_ldind` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::types::` |
| 194 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:813` — `p35_ldflda_on_stack_value_type_then_ldind` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::types::` |
| 195 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:852` — `push_test_value_type` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::assembly::` |
| 196 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:855` — `push_test_value_type` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::types::` |
| 197 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:858` — `push_test_value_type` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::types::` |
| 198 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:1046` — `p43_enum_home_scalar_roundtrip` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::assembly::` |
| 199 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:1049` — `p43_enum_home_scalar_roundtrip` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::types::` |
| 200 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:1052` — `p43_enum_home_scalar_roundtrip` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::types::` |
| 201 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:3004` — `frame_blocks_exception_lifecycle` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::types::` |
| 202 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:3007` — `frame_blocks_exception_lifecycle` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::types::` |
| 203 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:3010` — `frame_blocks_exception_lifecycle` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::members::` |
| 204 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:3011` — `frame_blocks_exception_lifecycle` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::members::` |
| 205 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:3239` — `frame_home_block` | fix (resolved) | F-QUAL-001; `std::mem::` |
| 206 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:3839` — `p42_typed_reference_address_policy` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::assembly::` |
| 207 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:3842` — `p42_typed_reference_address_policy` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::types::` |
| 208 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:3845` — `p42_typed_reference_address_policy` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::types::` |
| 209 | `crates/dotnet-vm/src/stack/context.rs:88` — `drop` | fix (resolved) | F-QUAL-001; `super::ops::` |
| 210 | `crates/dotnet-vm/src/stack/context.rs:308` — `on_pop_safe` | fix (resolved) | F-QUAL-001; `crate::error::` |
| 211 | `crates/dotnet-vm/src/stack/context_ops.rs:165` — `instance_field_layout` | fix (resolved) | F-QUAL-001; `dotnet_vm_ops::ops::` |
| 212 | `crates/dotnet-vm/src/stack/context_ops.rs:196` — `new_object` | fix (resolved) | F-QUAL-001; `dotnet_value::object::` |
| 213 | `crates/dotnet-vm/src/stack/context_ops.rs:205` — `new_object_with_lookup` | fix (resolved) | F-QUAL-001; `dotnet_value::object::` |
| 214 | `crates/dotnet-vm/src/stack/context_ops.rs:385` — `type_layout` | fix (resolved) | F-QUAL-001; `std::sync::` |
| 215 | `crates/dotnet-vm/src/stack/context_ops.rs:556` — `threading_current_thread` | fix (resolved) | F-QUAL-001; `std::sync::` |
| 216 | `crates/dotnet-vm/src/stack/context_ops.rs:582` — `threading_new_managed_thread` | fix (resolved) | F-QUAL-001; `std::sync::` |
| 217 | `crates/dotnet-vm/src/stack/mod.rs:47` — `(module context)` | keep | F-QUAL-001: macro hygiene, sync identity or doc target; `crate::dispatch::` |
| 218 | `crates/dotnet-vm/src/stack/native_effects.rs:146` — `wait_test_hook` | fix (resolved) | F-QUAL-001; `dotnet_types::` |
| 219 | `crates/dotnet-vm/src/stack/native_effects.rs:433` — `class_constructor_target` | fix (resolved) | F-QUAL-001; `dotnet_types::` |
| 220 | `crates/dotnet-vm/src/stack/native_effects.rs:468` — `probe_initializer` | fix (resolved) | F-QUAL-001; `dotnet_types::` |
| 221 | `crates/dotnet-vm/src/stack/native_effects.rs:525` — `wrap_initializer_failure` | fix (resolved) | F-QUAL-001; `dotnet_types::` |
| 222 | `crates/dotnet-vm/src/stack/native_effects.rs:1142` — `start_native` | fix (resolved) | F-QUAL-001; `dotnet_vm_data::` |
| 223 | `crates/dotnet-vm/src/stack/ops.rs:66` — `local_address` | fix (resolved) | F-QUAL-001; `crate::error::` |
| 224 | `crates/dotnet-vm/src/stack/ops.rs:72` — `argument_address` | fix (resolved) | F-QUAL-001; `crate::error::` |
| 225 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:157` — `write_unaligned` | fix (resolved) | F-QUAL-001; `dotnet_value::layout::` |
| 226 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:173` — `write_unaligned` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 227 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:191` — `write_unaligned` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 228 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:206` — `write_unaligned` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 229 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:217` — `write_unaligned` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 230 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:253` — `read_unaligned` | fix (resolved) | F-QUAL-001; `dotnet_value::layout::` |
| 231 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:270` — `read_unaligned` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 232 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:290` — `read_unaligned` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 233 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:305` — `read_unaligned` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 234 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:313` — `read_unaligned` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 235 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:348` — `write_bytes` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 236 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:361` — `write_bytes` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 237 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:370` — `write_bytes` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 238 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:378` — `write_bytes` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 239 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:407` — `read_bytes` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 240 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:420` — `read_bytes` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 241 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:427` — `read_bytes` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 242 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:435` — `read_bytes` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 243 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:486` — `object_atomic` | fix (resolved) | F-QUAL-001; `std::mem::` |
| 244 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:490` — `object_atomic` | fix (resolved) | F-QUAL-001; `std::mem::` |
| 245 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:507` — `object_atomic` | fix (resolved) | F-QUAL-001; `std::mem::` |
| 246 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:517` — `object_atomic` | fix (resolved) | F-QUAL-001; `std::mem::` |
| 247 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:521` — `object_atomic` | fix (resolved) | F-QUAL-001; `std::mem::` |
| 248 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:553` — `compare_exchange_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 249 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:579` — `compare_exchange_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 250 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:591` — `compare_exchange_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 251 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:605` — `compare_exchange_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 252 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:615` — `compare_exchange_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 253 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:627` — `compare_exchange_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 254 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:659` — `exchange_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 255 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:683` — `exchange_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 256 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:691` — `exchange_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 257 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:703` — `exchange_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 258 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:710` — `exchange_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 259 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:718` — `exchange_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 260 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:748` — `exchange_add_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 261 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:772` — `exchange_add_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 262 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:780` — `exchange_add_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 263 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:792` — `exchange_add_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 264 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:799` — `exchange_add_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 265 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:807` — `exchange_add_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 266 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:836` — `load_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 267 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:850` — `load_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 268 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:858` — `load_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 269 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:870` — `load_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 270 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:877` — `load_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 271 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:885` — `load_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 272 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:913` — `store_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 273 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:934` — `store_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 274 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:942` — `store_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 275 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:953` — `store_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 276 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:959` — `store_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 277 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:967` — `store_atomic` | fix (resolved) | F-QUAL-001; `dotnet_runtime_memory::` |
| 278 | `crates/dotnet-vm/src/stack/raw_memory_ops_impl/mod.rs:1102` — `owned_origin_typed_access_checks_complete_range` | fix (resolved) | F-QUAL-001; `dotnetdll::resolved::` |
| 279 | `crates/dotnet-vm/src/sync/mod.rs:95` — `expire_for_test` | fix (resolved) | F-QUAL-001; `std::time::` |
| 280 | `crates/dotnet-vm/src/sync/threaded.rs:103` — `lock_and_try_enter` | fix (resolved) | F-QUAL-001; `super::` |
| 281 | `crates/dotnet-vm/src/sync/threaded.rs:144` — `enter_with_timeout` | fix (resolved) | F-QUAL-001; `super::` |
| 282 | `crates/dotnet-vm/src/threading/basic.rs:914` — `execute_gc_command_for_current_thread` | fix (resolved) | F-QUAL-001; `std::sync::` |
| 283 | `crates/dotnet-vm/src/threading/workers.rs:37` — `(module context)` | fix (resolved) | F-QUAL-001; `std::sync::` |
| 284 | `crates/dotnet-vm/src/threading/workers.rs:53` — `(module context)` | fix (resolved) | F-QUAL-001; `std::sync::` |
| 285 | `crates/dotnet-vm-data/src/lib.rs:93` — `new` | fix (resolved) | F-QUAL-001; `exceptions::` |
| 286 | `crates/dotnet-vm-ops/src/effects.rs:55` — `from` | fix (resolved) | F-QUAL-001; `std::` |
| 287 | `crates/dotnet-vm-ops/src/effects.rs:182` — `fetch` | fix (resolved) | F-QUAL-001; `std::` |
| 288 | `crates/dotnet-vm-ops/src/effects.rs:386` — `frame_home_block` | fix (resolved) | F-QUAL-001; `std::` |
| 289 | `crates/dotnet-vm-ops/src/effects.rs:2802` — `rooted_frame_home_descriptor_preserves_aliases_and_rejects_retired_home` | fix (resolved) | F-QUAL-001; `std::` |
| 290 | `crates/dotnet-vm-ops/src/effects.rs:2820` — `frame_home_block` | fix (resolved) | F-QUAL-001; `std::` |
| 291 | `crates/dotnet-vm-ops/src/effects.rs:2828` — `frame_home_block` | fix (resolved) | F-QUAL-001; `std::` |
| 292 | `crates/dotnet-vm-ops/src/effects.rs:2829` — `frame_home_block` | fix (resolved) | F-QUAL-001; `std::` |
| 293 | `crates/dotnet-vm-ops/src/ops.rs:134` — `push_value_type` | fix (resolved) | F-QUAL-001; `dotnet_value::object::` |
| 294 | `crates/dotnet-vm-ops/src/ops.rs:181` — `pop_value_type` | fix (resolved) | F-QUAL-001; `dotnet_value::object::` |
| 295 | `crates/dotnet-vm-ops/src/ops.rs:440` — `new_object` | fix (resolved) | F-QUAL-001; `dotnet_value::object::` |
| 296 | `crates/dotnet-vm-ops/src/ops.rs:445` — `new_object_with_lookup` | fix (resolved) | F-QUAL-001; `dotnet_value::object::` |

### RsUnusedMut — 1 entries

**Per-file tally:** `crates/dotnet-vm/src/pointer_provenance_tests.rs` (1)

| Entry | Snapshot site and nearby symbol | Decision | Reason / finding |
|---|---|---|---|
| 1 | `crates/dotnet-vm/src/pointer_provenance_tests.rs:2346` — `p26_native_int_boundary_store_load_roundtrip` | refute | F-IDIOM-002: record_bit! increments bit_index; `mut` |

### UndefinedParamsPresent — 109 entries

**Per-file tally:** `.github/workflows/ci.yml` (73); `.github/workflows/fuzz.yml` (9); `.github/workflows/miri.yml` (23); `.github/workflows/valgrind.yml` (4)

| Entry | Snapshot site and nearby symbol | Decision | Reason / finding |
|---|---|---|---|
| 1 | `.github/workflows/ci.yml:70` — `(module context)` | refute | F-CI-001; `fetch-depth: 0` |
| 2 | `.github/workflows/ci.yml:136` — `(module context)` | refute | F-CI-001; `components: rustfmt` |
| 3 | `.github/workflows/ci.yml:140` — `(module context)` | refute | F-CI-001; `dotnet-version: '10.0.x'` |
| 4 | `.github/workflows/ci.yml:177` — `(module context)` | refute | F-CI-001; `toolchain: nightly-2026-05-27` |
| 5 | `.github/workflows/ci.yml:224` — `(module context)` | refute | F-CI-001; `components: clippy` |
| 6 | `.github/workflows/ci.yml:228` — `(module context)` | refute | F-CI-001; `dotnet-version: '10.0.x'` |
| 7 | `.github/workflows/ci.yml:262` — `(module context)` | refute | F-CI-001; `components: clippy` |
| 8 | `.github/workflows/ci.yml:266` — `(module context)` | refute | F-CI-001; `dotnet-version: '10.0.x'` |
| 9 | `.github/workflows/ci.yml:295` — `(module context)` | refute | F-CI-001; `components: clippy` |
| 10 | `.github/workflows/ci.yml:300` — `(module context)` | refute | F-CI-001; `dotnet-version: '10.0.x'` |
| 11 | `.github/workflows/ci.yml:327` — `(module context)` | refute | F-CI-001; `dotnet-version: '10.0.x'` |
| 12 | `.github/workflows/ci.yml:345` — `(module context)` | refute | F-CI-001; `path: ${{ steps.fixture-output-dir.outputs.path }}` |
| 13 | `.github/workflows/ci.yml:346` — `(module context)` | refute | F-CI-001; `key: ${{ runner.os }}-dotnet-fixtures-${{ steps.fixture-cache-key.outputs.key }}` |
| 14 | `.github/workflows/ci.yml:347` — `(module context)` | refute | F-CI-001; `restore-keys: \| ${{ runner.os }}-dotnet-fixtures-` |
| 15 | `.github/workflows/ci.yml:357` — `(module context)` | refute | F-CI-001; `name: dotnet-fixtures` |
| 16 | `.github/workflows/ci.yml:358` — `(module context)` | refute | F-CI-001; `path: ${{ steps.fixture-output-dir.outputs.path }}` |
| 17 | `.github/workflows/ci.yml:359` — `(module context)` | refute | F-CI-001; `include-hidden-files: true` |
| 18 | `.github/workflows/ci.yml:360` — `(module context)` | refute | F-CI-001; `retention-days: 1` |
| 19 | `.github/workflows/ci.yml:388` — `(module context)` | refute | F-CI-001; `dotnet-version: '10.0.x'` |
| 20 | `.github/workflows/ci.yml:393` — `(module context)` | refute | F-CI-001; `name: dotnet-fixtures` |
| 21 | `.github/workflows/ci.yml:394` — `(module context)` | refute | F-CI-001; `path: ${{ needs.build-fixtures.outputs.fixtures-path }}` |
| 22 | `.github/workflows/ci.yml:512` — `(module context)` | refute | F-CI-001; `toolchain: nightly-2026-05-27` |
| 23 | `.github/workflows/ci.yml:513` — `(module context)` | refute | F-CI-001; `components: miri` |
| 24 | `.github/workflows/ci.yml:561` — `(module context)` | refute | F-CI-001; `name: ci-diagnostics-miri-value-attempt-${{ github.run_attempt }}` |
| 25 | `.github/workflows/ci.yml:562` — `(module context)` | refute | F-CI-001; `path: ${{ runner.temp }}/ci-diagnostics` |
| 26 | `.github/workflows/ci.yml:563` — `(module context)` | refute | F-CI-001; `if-no-files-found: warn` |
| 27 | `.github/workflows/ci.yml:564` — `(module context)` | refute | F-CI-001; `retention-days: 14` |
| 28 | `.github/workflows/ci.yml:582` — `(module context)` | refute | F-CI-001; `toolchain: nightly-2026-05-27` |
| 29 | `.github/workflows/ci.yml:583` — `(module context)` | refute | F-CI-001; `components: miri` |
| 30 | `.github/workflows/ci.yml:637` — `(module context)` | refute | F-CI-001; `name: ci-diagnostics-miri-frame-blocks-attempt-${{ github.run_attempt }}` |
| 31 | `.github/workflows/ci.yml:638` — `(module context)` | refute | F-CI-001; `path: ${{ runner.temp }}/ci-diagnostics` |
| 32 | `.github/workflows/ci.yml:639` — `(module context)` | refute | F-CI-001; `if-no-files-found: warn` |
| 33 | `.github/workflows/ci.yml:640` — `(module context)` | refute | F-CI-001; `retention-days: 14` |
| 34 | `.github/workflows/ci.yml:658` — `(module context)` | refute | F-CI-001; `toolchain: nightly-2026-05-27` |
| 35 | `.github/workflows/ci.yml:659` — `(module context)` | refute | F-CI-001; `components: miri` |
| 36 | `.github/workflows/ci.yml:692` — `(module context)` | refute | F-CI-001; `name: ci-diagnostics-miri-owner-retention-attempt-${{ github.run_attempt }}` |
| 37 | `.github/workflows/ci.yml:693` — `(module context)` | refute | F-CI-001; `path: ${{ runner.temp }}/ci-diagnostics` |
| 38 | `.github/workflows/ci.yml:694` — `(module context)` | refute | F-CI-001; `if-no-files-found: warn` |
| 39 | `.github/workflows/ci.yml:695` — `(module context)` | refute | F-CI-001; `retention-days: 14` |
| 40 | `.github/workflows/ci.yml:832` — `(module context)` | refute | F-CI-001; `toolchain: nightly-2026-05-27` |
| 41 | `.github/workflows/ci.yml:833` — `(module context)` | refute | F-CI-001; `components: miri` |
| 42 | `.github/workflows/ci.yml:839` — `(module context)` | refute | F-CI-001; `key: miri-cross-arena-nightly-2026-05-27-${{ matrix.configuration }}-${{ matrix.group }}-$` |
| 43 | `.github/workflows/ci.yml:845` — `(module context)` | refute | F-CI-001; `dotnet-version: '10.0.x'` |
| 44 | `.github/workflows/ci.yml:950` — `(module context)` | refute | F-CI-001; `name: ci-diagnostics-miri-cross-arena-${{ matrix.configuration }}-${{ matrix.group }}-${{ ` |
| 45 | `.github/workflows/ci.yml:951` — `(module context)` | refute | F-CI-001; `path: ${{ runner.temp }}/ci-diagnostics` |
| 46 | `.github/workflows/ci.yml:952` — `(module context)` | refute | F-CI-001; `if-no-files-found: warn` |
| 47 | `.github/workflows/ci.yml:953` — `(module context)` | refute | F-CI-001; `retention-days: 14` |
| 48 | `.github/workflows/ci.yml:997` — `(module context)` | refute | F-CI-001; `toolchain: nightly-2026-05-27` |
| 49 | `.github/workflows/ci.yml:998` — `(module context)` | refute | F-CI-001; `components: miri` |
| 50 | `.github/workflows/ci.yml:1007` — `(module context)` | refute | F-CI-001; `key: miri-pointer-batch-v1-nightly-2026-05-27-${{ matrix.configuration }}` |
| 51 | `.github/workflows/ci.yml:1014` — `(module context)` | refute | F-CI-001; `dotnet-version: '10.0.x'` |
| 52 | `.github/workflows/ci.yml:1048` — `(module context)` | refute | F-CI-001; `name: ci-diagnostics-pointer-batch-${{ matrix.configuration }}-batch-${{ matrix.batch }}-a` |
| 53 | `.github/workflows/ci.yml:1049` — `(module context)` | refute | F-CI-001; `path: ${{ runner.temp }}/ci-diagnostics` |
| 54 | `.github/workflows/ci.yml:1050` — `(module context)` | refute | F-CI-001; `if-no-files-found: warn` |
| 55 | `.github/workflows/ci.yml:1051` — `(module context)` | refute | F-CI-001; `retention-days: 14` |
| 56 | `.github/workflows/ci.yml:1098` — `(module context)` | refute | F-CI-001; `toolchain: nightly-2026-05-27` |
| 57 | `.github/workflows/ci.yml:1099` — `(module context)` | refute | F-CI-001; `components: miri` |
| 58 | `.github/workflows/ci.yml:1108` — `(module context)` | refute | F-CI-001; `key: miri-pointer-v2-nightly-2026-05-27-${{ matrix.configuration }}-${{ matrix.shard }}` |
| 59 | `.github/workflows/ci.yml:1115` — `(module context)` | refute | F-CI-001; `dotnet-version: '10.0.x'` |
| 60 | `.github/workflows/ci.yml:1149` — `(module context)` | refute | F-CI-001; `name: ci-diagnostics-pointer-${{ matrix.configuration }}-shard-${{ matrix.shard }}-attempt` |
| 61 | `.github/workflows/ci.yml:1150` — `(module context)` | refute | F-CI-001; `path: ${{ runner.temp }}/ci-diagnostics` |
| 62 | `.github/workflows/ci.yml:1151` — `(module context)` | refute | F-CI-001; `if-no-files-found: warn` |
| 63 | `.github/workflows/ci.yml:1152` — `(module context)` | refute | F-CI-001; `retention-days: 14` |
| 64 | `.github/workflows/ci.yml:1180` — `(module context)` | refute | F-CI-001; `toolchain: 1.95.0` |
| 65 | `.github/workflows/ci.yml:1183` — `(module context)` | refute | F-CI-001; `toolchain: nightly-2026-05-27` |
| 66 | `.github/workflows/ci.yml:1187` — `(module context)` | refute | F-CI-001; `dotnet-version: '10.0.x'` |
| 67 | `.github/workflows/ci.yml:1196` — `(module context)` | refute | F-CI-001; `workspaces: \| . ${{ matrix.fuzz-workspace }}` |
| 68 | `.github/workflows/ci.yml:1203` — `(module context)` | refute | F-CI-001; `path: ~/.cargo/bin/cargo-fuzz` |
| 69 | `.github/workflows/ci.yml:1205` — `(module context)` | refute | F-CI-001; `key: ${{ runner.os }}-${{ runner.arch }}-cargo-fuzz-v2-${{ env.CARGO_FUZZ_VERSION }}-build` |
| 70 | `.github/workflows/ci.yml:1247` — `(module context)` | refute | F-CI-001; `name: ci-diagnostics-replay-${{ matrix.target }}-attempt-${{ github.run_attempt }}` |
| 71 | `.github/workflows/ci.yml:1248` — `(module context)` | refute | F-CI-001; `path: ${{ runner.temp }}/ci-diagnostics` |
| 72 | `.github/workflows/ci.yml:1249` — `(module context)` | refute | F-CI-001; `if-no-files-found: warn` |
| 73 | `.github/workflows/ci.yml:1250` — `(module context)` | refute | F-CI-001; `retention-days: 14` |
| 74 | `.github/workflows/fuzz.yml:21` — `(module context)` | refute | F-CI-001; `toolchain: 1.95.0` |
| 75 | `.github/workflows/fuzz.yml:24` — `(module context)` | refute | F-CI-001; `toolchain: nightly-2026-05-27` |
| 76 | `.github/workflows/fuzz.yml:29` — `(module context)` | refute | F-CI-001; `workspaces: \| . crates/dotnet-vm/fuzz` |
| 77 | `.github/workflows/fuzz.yml:36` — `(module context)` | refute | F-CI-001; `dotnet-version: '10.0.x'` |
| 78 | `.github/workflows/fuzz.yml:47` — `(module context)` | refute | F-CI-001; `path: ~/.cargo/bin/cargo-fuzz` |
| 79 | `.github/workflows/fuzz.yml:49` — `(module context)` | refute | F-CI-001; `key: ${{ runner.os }}-${{ runner.arch }}-cargo-fuzz-v2-${{ env.CARGO_FUZZ_VERSION }}-build` |
| 80 | `.github/workflows/fuzz.yml:97` — `(module context)` | refute | F-CI-001; `name: fuzz-artifacts` |
| 81 | `.github/workflows/fuzz.yml:98` — `(module context)` | refute | F-CI-001; `path: crates/dotnet-vm/fuzz/artifacts/fuzz_executor` |
| 82 | `.github/workflows/fuzz.yml:99` — `(module context)` | refute | F-CI-001; `if-no-files-found: ignore` |
| 83 | `.github/workflows/miri.yml:53` — `(module context)` | refute | F-CI-001; `toolchain: nightly-2026-05-27` |
| 84 | `.github/workflows/miri.yml:54` — `(module context)` | refute | F-CI-001; `components: miri` |
| 85 | `.github/workflows/miri.yml:66` — `(module context)` | refute | F-CI-001; `dotnet-version: '10.0.x'` |
| 86 | `.github/workflows/miri.yml:98` — `(module context)` | refute | F-CI-001; `name: ci-diagnostics-miri-${{ matrix.crate }}-${{ matrix.configuration }}-attempt-${{ gith` |
| 87 | `.github/workflows/miri.yml:99` — `(module context)` | refute | F-CI-001; `path: ${{ runner.temp }}/ci-diagnostics` |
| 88 | `.github/workflows/miri.yml:100` — `(module context)` | refute | F-CI-001; `if-no-files-found: warn` |
| 89 | `.github/workflows/miri.yml:101` — `(module context)` | refute | F-CI-001; `retention-days: 14` |
| 90 | `.github/workflows/miri.yml:150` — `(module context)` | refute | F-CI-001; `toolchain: nightly-2026-05-27` |
| 91 | `.github/workflows/miri.yml:151` — `(module context)` | refute | F-CI-001; `components: miri` |
| 92 | `.github/workflows/miri.yml:157` — `(module context)` | refute | F-CI-001; `shared-key: miri-advisory-discovery-v1-nightly-2026-05-27-${{ matrix.configuration }}` |
| 93 | `.github/workflows/miri.yml:164` — `(module context)` | refute | F-CI-001; `dotnet-version: '10.0.x'` |
| 94 | `.github/workflows/miri.yml:187` — `(module context)` | refute | F-CI-001; `name: ci-diagnostics-miri-discovery-${{ matrix.configuration }}-attempt-${{ github.run_att` |
| 95 | `.github/workflows/miri.yml:188` — `(module context)` | refute | F-CI-001; `path: ${{ runner.temp }}/ci-diagnostics` |
| 96 | `.github/workflows/miri.yml:189` — `(module context)` | refute | F-CI-001; `if-no-files-found: warn` |
| 97 | `.github/workflows/miri.yml:190` — `(module context)` | refute | F-CI-001; `retention-days: 14` |
| 98 | `.github/workflows/miri.yml:221` — `(module context)` | refute | F-CI-001; `toolchain: nightly-2026-05-27` |
| 99 | `.github/workflows/miri.yml:222` — `(module context)` | refute | F-CI-001; `components: miri` |
| 100 | `.github/workflows/miri.yml:228` — `(module context)` | refute | F-CI-001; `shared-key: miri-advisory-v2-nightly-2026-05-27-${{ matrix.crate }}-${{ matrix.configurati` |
| 101 | `.github/workflows/miri.yml:235` — `(module context)` | refute | F-CI-001; `dotnet-version: '10.0.x'` |
| 102 | `.github/workflows/miri.yml:265` — `(module context)` | refute | F-CI-001; `name: ci-diagnostics-miri-${{ matrix.crate }}-${{ matrix.configuration }}-${{ matrix.shard` |
| 103 | `.github/workflows/miri.yml:266` — `(module context)` | refute | F-CI-001; `path: ${{ runner.temp }}/ci-diagnostics` |
| 104 | `.github/workflows/miri.yml:267` — `(module context)` | refute | F-CI-001; `if-no-files-found: warn` |
| 105 | `.github/workflows/miri.yml:268` — `(module context)` | refute | F-CI-001; `retention-days: 14` |
| 106 | `.github/workflows/valgrind.yml:33` — `(module context)` | refute | F-CI-001; `dotnet-version: '10.0.x'` |
| 107 | `.github/workflows/valgrind.yml:56` — `(module context)` | refute | F-CI-001; `path: ${{ steps.fixture-output-dir.outputs.path }}` |
| 108 | `.github/workflows/valgrind.yml:57` — `(module context)` | refute | F-CI-001; `key: ${{ runner.os }}-dotnet-fixtures-${{ steps.fixture-cache-key.outputs.key }}` |
| 109 | `.github/workflows/valgrind.yml:58` — `(module context)` | refute | F-CI-001; `restore-keys: \| ${{ runner.os }}-dotnet-fixtures-` |



## Phase Plan

CHECKLIST.md is the ordered execution contract. Register the same slices in the next free durable plan during the first usable change; do not replay completed architecture plans or create dormant support. Every slice includes callers, tests, removals and affected docs.

## Phase 1: Semantic duplication

- 1.1 Unify intrinsic operand and reflection-slot selection [effort: high] — refs REVIEW.md#F-DUP-001

Complete the byte Min/Max operand decoder in dotnet-intrinsics-core and the validated reflection index-slot selector in dotnet-intrinsics-reflection; migrate both callers of each shared concept and remove the duplicated definitions (aggregate groups 1–2). Preserve integer truncation, pop order, canonical names and read-error versus write-contract behavior. Fold touched qualifications/lifetimes into this change. Add enabled edge/type-error and reflection-wrapper tests. Register the refactor as the next free docs/plans plan and queue entry in this same usable slice, with the ordered slices below, not a separate preparatory change. Acceptance: both pairs use one semantic implementation, all four wrapper kinds are tested, crate-scoped clippy/nextest for supported no-feature and MT combinations, slot coverage, fmt and doc drift pass.

- 1.2 Make raw-memory windows and byref layout selection explicit [effort: high] — refs REVIEW.md#F-DUP-002

In dotnet-vm stack/raw_memory_ops_impl, replace groups 25–28 with one checked-range contract used by inline reads/writes, slice copies and pointer-sized accesses, plus one effective frame-home layout selection used by read and write. Migrate every affected caller, retaining separate unsafe operations and local proofs; fold qualifications here rather than churning this file later. Exercise zero length, end boundary, usize overflow, stale homes and managed-byref width through enabled VM paths. Acceptance: no unchecked offset-plus-length in the affected copy helpers, exact BoundsCheck payloads retained, byref provenance preserved; VM clippy/nextest across shared matrix, release clippy, focused pointer Miri, provenance lints and doc drift pass.

- 1.3 Share exception initialization and executor publication contracts [effort: high] — refs REVIEW.md#F-DUP-003

Unify exception message/parameter-name initialization used by ExceptionOps and NativeBackend (groups 22–23), preserving rooting and host-error versus managed-exception boundaries; route both paths through the same field-initialization contract. Consolidate repeated executor finalizer publication (group 12) without merging lifecycle phases. Unify the object-write and address-resolution error translations (groups 20–21) at their semantic boundary; preserve stale-home errors and guest exception messages. Migrate all callers and remove the old branches; retain distinct monitor-poll phases (group 24). Acceptance: enabled sync/effect exception, object init/store, ldarga/ldloca and teardown tests exercise new paths; VM and dependent CLI tests in no-feature, MT and MT+validation-all, clippy, focused Miri for changed GC boundaries, provenance/doc drift and release clippy pass.

- 1.4 Centralize event transitions and value-type allocation accounting [effort: high] — refs REVIEW.md#F-DUP-004

In dotnet-vm threading/events, give locked event state explicit expire-waiters and deliver-auto-reset-signal transitions, used by both signal and cancellation/cleanup paths (groups 30–31). In stack/stack_ops_impl, consolidate value-type write allocation accounting at an item-gated semantic helper used by every existing write (group 29), preserving exactly-once metrics and GC debt. Include callers, enabled timeout/cancel/handoff and value-type-write tests and affected synchronization docs together. Acceptance: timeout ordering, pending-signal retention and no double accounting demonstrated in native tests; VM no-feature/MT/MT+validation-all tests and clippy, bench-instrumentation guards, release clippy, Loom and doc drift pass.

## Phase 2: Instrumentation and Rust readability

- 2.1 Give native-effect tests a coherent boundary-observation API [effort: high] — refs REVIEW.md#F-IDIOM-001

Replace scattered BOUNDARY_DROPS/RESUMES/NEXT reset, increment and observation plumbing in native_substring_tests with a small thread-local boundary-observation abstraction whose reset/snapshot/event methods express the test protocol; migrate all existing tests in this slice, preserving observable ordering and drop timing. Convert other exported trivial LocalKey<Cell> reads/writes to get/set/replace/take in their owning test/production modules, without forcing RefCell hooks or read-modify-write closures into fake helpers. Fold the touched test qualifications in. Acceptance: all 75 exported TLS sites resolved or individually justified, isolation and unwind/drop assertions retained, enabled native substring/return/teardown tests run in no-feature, MT and MT+validation-all, affected value/utils/vm-data crate tests, clippy and doc drift pass.

- 2.2 Finish the feature-aware Rust readability sweep [effort: default] — refs REVIEW.md#F-QUAL-001, REVIEW.md#F-IDIOM-002, REVIEW.md#F-IDIOM-003, REVIEW.md#F-DEPR-001

Resolve remaining exported qualifications and Rust idioms after structural slices: local imports of recurring domain types, assert_eq/assert_ne diagnostics, the two support.rs casts, both parenthesis pairs, simple lift expressions and genuinely uninformative lifetime names. Preserve named arena input/output contracts, expansion hygiene and sync-facade identity. Retain trait impl semantic grouping and macro-mutated bit_index with recorded reasons. Rename only the three standard-atomic fetch_update sites supported by the pin; retain the Loom-facade site. Acceptance: update every touched ledger row, no broad imports/allows or weakened invariants; fmt and full shared-feature clippy/nextest, optional diagnostic/instrumentation guards, release clippy, macro regression probes where token output changes, and doc drift pass. No new project-wide lint without proving a clean full matrix.

## Phase 3: Dependency edges and closure

- 3.1 Remove unused direct dependency edges without upgrading APIs [effort: default] — refs REVIEW.md#F-DEP-001, REVIEW.md#F-DEP-002

Remove the four unused direct edges in dotnet-benchmarks (tracing, tracing-subscriber), dotnet-value (lru), and dotnet-cli (dotnet-value), including feature-forwarding references that exist only to propagate an otherwise unused edge; preserve real dependency feature propagation through actual consumers. Check all targets, build scripts, macro expansion and standalone fuzz manifests. Do not upgrade any dependency. Acceptance: cargo tree -e features confirms surviving feature routes, all affected targets build across shared and optional feature guards, CLI/benchmark smoke and crate tests pass, full matrix clippy/nextest and fmt pass; record all twelve major-version notices as scoped follow-ups in docs/plans/README.md Not yet planned with upstream evidence.

- 3.2 Close the inspection ledger and durable plan with reproducible checks [effort: default] — refs REVIEW.md#F-INFRA-001, REVIEW.md#F-CI-001, REVIEW.md#F-DUP-005

Complete the disposition ledger for every export entry and all 32 aggregate duplicate groups; retain the intentional __rerun_probe build fixture, standalone fuzz crates, generated output and every valid workflow input. Add durable IDE-scope guidance to the appropriate contributor/fuzz docs only where useful (attach fuzz manifests, exclude target/obj; no source suppressions). Record unrelated sync-facade and dependency follow-ups under Not yet planned, reconcile the registered plan exit conditions, and mark its Status complete only on evidence. Acceptance: no missing ledger rows/anchors, all planned fixes have enabled caller/test evidence, all retained entries have reasons; run ./check.sh --full, cargo fmt --check, CI release-clippy configurations, doc/provenance/build-script/slot checks and the relevant Miri/Loom lanes from changed paths. Recheck inspection-equivalent lints and manually audit dependency features if udeps/machete unavailable. Record unavailable lanes honestly; do not call skipped checks passes.


