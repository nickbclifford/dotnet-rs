# Contributing to dotnet-rs

Thanks for contributing to `dotnet-rs`.

## Prerequisites

Before running local builds or tests, install:

- **Rust 1.95.0**, selected by the repository's `rust-toolchain.toml` and
  including the `clippy` and `rustfmt` components. Rustup activates this pin
  automatically when commands are run from the repository.
- **.NET SDK** (required for fixture compilation and build-script paths that invoke dotnet/MSBuild)

## Engineering decisions and plan authority

The current user task and accepted decisions define the outcome and scope. The
[project queue](docs/plans/README.md) sets priorities; a numbered plan owns its runtime contract;
[CI policy](docs/CI.md#test-hierarchy-and-migration) owns verification selection. Reviews,
checklists, historical proposals and memory are fallible supporting records. Check claims
against the code and distinguish implemented behavior from a proposed change.

Agents may resolve routine engineering details, correct stale prose, reorder or combine
interdependent steps, and include supporting edits needed for a complete behavior. Explain
consequential adaptations in the review. A changed file list, module location, benchmark command
or checklist boundary does not by itself require maintainer approval.

Ask when a choice materially changes supported behavior, public compatibility or agreed scope,
weakens a safety requirement, or accepts an unresolved safety tradeoff. Present a concrete
recommendation and its consequences. Reuse prior decisions; do not ask again because a handoff
or historical document repeats the old question. An approval cannot prove an unsafe invariant.
If one part needs a decision, continue independent authorized work instead of blocking the whole
outcome. A prerequisite should describe a real missing contract or defect, not a deviation from prose.

Read the current plan and relevant decision/evidence entries. Do not require every session to
reread an append-only history. Keep the current contract concise and link to historical evidence.
New findings receive an explicit disposition: fix within the outcome, record a separate defect,
refute, or ask about a material change. Do not automatically expand the plan for every finding.
Do not mark a larger safety claim complete because a scoped step or workflow is green.
Generated research/step prompts must follow these rules too. Correct a generated prompt that
reintroduces superseded approval or repetition requirements during setup; this is routine
instruction maintenance, not another maintainer decision. The sibling orchestrator's generator
has not been changed by this documentation revision.

Lexical cfg/sync ceilings remain enforced by the existing scripts. Prefer module boundaries and
repository synchronization types. Necessary bounded growth can be justified, measured and
updated in the same reviewed implementation change without a separate permission request.
Do not obfuscate tokens or remove unrelated code to fit a count. The semantic synchronization
and unsafe contracts remain unchanged.

## Unsafe code policy

Unsafe code must make the invariant that justifies it reviewable at the point
where it is relied on. Follow these rules when adding or changing unsafe code:

- Put a `// SAFETY:` comment immediately above every `unsafe { ... }` block.
  The comment must explain the invariant that makes the operation valid; it
  must not merely restate the operation. It must also cite at least one named
  predicate from [`docs/INVARIANT_REGISTRY.md`](docs/INVARIANT_REGISTRY.md),
  for example `F4.WidthAligned`; retain the local prose explaining why that
  predicate holds at this site. Adding a predicate requires adding its registry
  row in the same commit.
- Do not use a `where` clause as proof for `unsafe impl Send` or `unsafe impl
  Sync`: a bound only makes the implementation conditional. Use an
  unconditional implementation when it is valid (retaining any required
  feature `cfg`), document its SAFETY invariant, and add an
  `assert_impl_all!` guard in a `#[cfg(test)]` block to mechanically check the
  intended traits.
- An `unsafe impl Collect` must trace every contained `Gc<'gc, _>` handle. For
  types that are `'static` and contain no GC handles, use `static_collect!`.
  For a lifetime-parameterized type, implement `Collect` only for the
  instantiations whose borrows are valid in GC-rooted state (for example,
  `MethodInfo<'static>`); do not leave the lifetime unconstrained merely
  because the type has no GC handles. A `NEEDS_TRACE = false` proof must state
  both the admitted lifetime and why no field requires tracing.
- A layout transmute must have a co-located size assertion and a `SAFETY:`
  comment that explicitly explains the representation and validity reasoning.

The workspace enforces this policy through `[workspace.lints.clippy]` in the
root `Cargo.toml`: `undocumented_unsafe_blocks = "deny"` and
`multiple_unsafe_ops_per_block = "deny"`. Member crates inherit these lints;
keep unsafe operations individually proved rather than weakening the workspace
configuration.

The registry and its drift check enforce the naming part of this policy.

## Panic-vs-Result policy

When adding or changing runtime behavior, classify failure paths into two categories:

1. **VM invariant violation (bug / impossible state):** use `panic!`, `unreachable!`, `debug_assert!`, or `expect()` with a specific message.
   - These are conditions that should never be reachable if VM internals are correct.
   - Do **not** route these through `VmError`.
2. **Runtime/metadata failure (input/environment driven):** return `VmError` via `Result` / `StepResult::Error`.
   - This includes metadata resolution failures, invalid CIL, null/memory access errors, P/Invoke load/symbol failures, and other recoverable host-side execution failures.

### Host errors vs. managed exceptions

`VmError` and related enums are **host-side Rust errors**.

Managed `.NET` exceptions (`ManagedException`, `ExceptionState`, SEH search/unwind flow) are a separate mechanism described in [`docs/EXCEPTION_HANDLING.md`](docs/EXCEPTION_HANDLING.md).

Keep this distinction intact: host errors are not managed exceptions. The `.cctor` wrapping path (host error -> managed `System.TypeInitializationException`) is a bridge between layers, not a reason to merge the two models.

## Runtime offset/index newtype policy

The offset and index wrappers exported by `dotnet-utils` have private fields for encapsulation and
API evolution; they do not claim that every representable value is valid in every runtime context.
Their `new` constructors and existing infallible `From` implementations intentionally perform no
range validation; `Default`, where implemented, is only a zero-valued convenience. Validate bounds
against the relevant layout, stack, or arena at the use site; `ManagedByteOffset::try_from(usize)`
is the narrowing exception that enforces its `u32` range.
Under the `fuzzing` feature, module-local `Arbitrary` derives may bypass the public construction
surface to generate unvalidated values deliberately (see [`docs/FUZZING.md`](docs/FUZZING.md)).

Do not add raw `Sub` or `SubAssign` implementations to `ByteOffset`, `LocalIndex`,
`ArgumentIndex`, or `StackSlotIndex`. Use an available checked/saturating helper, or apply checked
arithmetic to the accessor value and reconstruct the wrapper, then classify underflow under the
panic-vs-Result policy above. Compiler rejection of raw subtraction is the regression guard;
release overflow checks remain enabled as a backstop for other arithmetic.

Keep the heterogeneous newtype implementations explicit rather than adding general newtype code
generation to `dotnet-macros-core`, whose charter is build-time .NET metadata parsing. Consider a
local `macro_rules!` only if the surviving behavior becomes genuinely uniform.

## Run the project checks

Follow the [test hierarchy](docs/CI.md#test-hierarchy-and-migration): fast core checks for every
PR, selected deep checks before accepting affected changes, full sweeps on a schedule and for
broad architectural changes, and investigations for named uncertainties. Advisory Miri, fuzz-campaign and Valgrind
workflows no longer run on push/PR (nightly and manual dispatch only), and `ci.yml` starts its
deep Miri and corpus-replay jobs only after the `fast-core` aggregate succeeds. Test selection
is still pending.

During iteration use focused native tests/lints and relevant unsafe/lifecycle controls. At the
completion of a coherent code outcome, run the current integration command once:

```bash
./check.sh --fast
```

It builds/verifies fixtures and runs Clippy/nextest in no-features, multithreading and
multithreading,validation-all configurations, plus source ceilings. `./check.sh --full` uses the
full xtask matrix and extra guards. Use full affected suites before accepting shared storage,
access or dispatch changes. Carry unchanged evidence forward with source bindings; rerun when
new edits/findings invalidate it. Do not repeat a full pass for every small step or docs edit.

Select isolated SIMD/fuzzing/validation features when affected. If dependency impact is unclear,
select broader coverage and explain it. A zero-match filter, unexpected ignore, timeout or failed
setup is not a passed check. Report native, Miri, Loom, ABI and performance evidence separately.
Nextest does not run doctests; run relevant compile-fail/doctests separately.

## Build test fixtures

Some test paths rely on managed fixtures. Build them with:

```bash
cargo run -p xtask -- fixtures build
```

## Miri policy

Current CI runs a pinned-nightly `dotnet-value` gate. Use affected library tests during
iteration and the relevant complete library selection when integrating unsafe/atomic changes.
This existing broad command is a reference, not a requirement for unrelated edits:

```bash
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
cargo +nightly-2026-05-27 miri test -p dotnet-value -- --test-threads=1
```

The separate blocking cross-arena singleton lane and its exact feature-specific selectors are
documented in [`docs/CI.md`](docs/CI.md); its isolated strict-core test is not whole-VM strict signoff.

### VM unsafe checks

Changes to unsafe or its callers require tests that exercise the changed invariant, including
applicable feature modes. Add focused regressions with the repair, use bounded real bridge tests,
and inspect discovery/counts. Run the existing broader advisory selection below when the changed
interpreter/exception path warrants it, or for a full sweep. It is not an automatic per-file or
per-step tax and does not substitute for a regression of the new unsafe boundary:

```bash
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
cargo +nightly-2026-05-27 miri test -p dotnet-vm --no-default-features -- --test-threads=1 jmp_tests tail_calls fault_tests
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
cargo +nightly-2026-05-27 miri test -p dotnet-vm --no-default-features --features multithreading -- --test-threads=1 jmp_tests tail_calls fault_tests
```

Limits of these existing invocations:
- `-Zmiri-strict-provenance` currently fails before VM execution due dependency-level integer-to-pointer casts reached during assembly parsing (`bitvec`/`dotnetdll`, plus rayon worker-thread paths via `crossbeam-epoch`).
- `-Zmiri-disable-isolation` is needed for host filesystem syscalls used by the test harness.
- `-Zmiri-ignore-leaks` is needed for non-joined background worker threads that are not VM-unsafe regressions.

These historical dependency blockers limit the whole-VM strict claim. Investigate exact current
blockers when a change needs that evidence; do not require dependency research to close an
independent runtime repair. Strict-core results must identify their isolated selection.
The nightly used for Miri and fuzz targets is pinned separately because `rust-toolchain.toml`
selects stable by default. Hosted advisory VM selections completed as guarded singleton shards
in both configurations; see [`docs/CI.md`](docs/CI.md) for the tested SHA, timings and limits.
This does not imply strict-provenance coverage or a soundness proof.

## Blocking fuzz corpus replay

Changes to fuzzed code should replay the tracked corpora with the CI-pinned toolchains. Build
cargo-fuzz 0.13.1 with stable Rust 1.95.0 and `--locked`; target compilation and execution use
`nightly-2026-05-27`. Current CI runs all four replays as pipeline-failing gates. Under the
agreed hierarchy, select affected corpora before acceptance and keep all four in full sweeps.
Changes to shared pointer/executor infrastructure select broadly. These direct commands are
references; use the guarded runner for acceptance:

```bash
cargo +1.95.0 install cargo-fuzz --version 0.13.1 --locked
cd crates/dotnet-value
ASAN_OPTIONS=detect_leaks=0 cargo +nightly-2026-05-27 fuzz run fuzz_managed_ptr_roundtrip fuzz/corpus/fuzz_managed_ptr_roundtrip -- -runs=0
ASAN_OPTIONS=detect_leaks=0 cargo +nightly-2026-05-27 fuzz run fuzz_managed_ptr_offset fuzz/corpus/fuzz_managed_ptr_offset -- -runs=0
ASAN_OPTIONS=detect_leaks=0 cargo +nightly-2026-05-27 fuzz run fuzz_raw_memory_access fuzz/corpus/fuzz_raw_memory_access -- -runs=0

cd ../dotnet-vm/fuzz
ASAN_OPTIONS=detect_leaks=0 cargo +nightly-2026-05-27 fuzz run fuzz_executor corpus/fuzz_executor -- -runs=0
```

These bare commands do not implement CI's inventory and execution-count guard; see
[`docs/FUZZING.md`](docs/FUZZING.md#blocking-corpus-regression). `ASAN_OPTIONS=detect_leaks=0`
makes replay usable on sandboxed runners; exploratory duration-based fuzzing remains advisory.

## Documentation drift check

Documentation-only changes need the drift/queue checks and a link check; they do not need a
runtime build/test matrix, Miri or benchmarks. Rustdoc is the existing intra-doc-link check:

```bash
bash scripts/check_doc_drift.sh
DOTNET_SKIP_BUILD=1 RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps --no-default-features
```

## Feature flag matrix reference

Feature combinations and validation ownership are documented in:

- [`docs/VALIDATION_FEATURES.md`](docs/VALIDATION_FEATURES.md)

For a subsystem overview, see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). For broader CI/local validation guidance, see [`docs/CI.md`](docs/CI.md). The project overview and quick-start flow live in [`README.md`](README.md).
