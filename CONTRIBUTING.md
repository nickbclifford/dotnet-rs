# Contributing to dotnet-rs

Thanks for contributing to `dotnet-rs`.

## Prerequisites

Before running local builds or tests, install:

- **Rust**, at the version pinned in `rust-toolchain.toml` (rustup selects it automatically),
  with `clippy` and `rustfmt`.
- **.NET SDK** (required for fixture compilation and build-script paths that invoke dotnet/MSBuild)

## Where things are decided

- The current task and the maintainer's decisions set the outcome and scope.
- [`docs/plans/`](docs/plans/README.md) holds the queue and each plan's scope, slices and exit
  conditions. A plan's `Status:` line is the only record of its status.
- [`docs/CI.md`](docs/CI.md) says which checks to run.
- Subsystem docs (`docs/*.md`) describe how the code works now. They contain no work items.
- [`docs/INVARIANT_REGISTRY.md`](docs/INVARIANT_REGISTRY.md) defines the predicates that
  `// SAFETY:` comments cite.
- Evidence and history live in `docs/pointer-provenance/`, `docs/ci-evidence/` and git. They
  are reference material, not instructions.

When a document and the code disagree, the code describes current behavior; fix the document.

## Making decisions while working

Decide routine engineering details yourself: file layout, internal APIs, combining dependent
steps, fixing stale prose. Explain consequential choices in the commit or review.

Ask the maintainer, with a recommendation, only when a choice changes supported guest behavior
or public compatibility, changes the agreed scope, or weakens a safety requirement. Ask once and
reuse the answer.

Keep work in enabled increments. Each change should leave the VM using what it adds, with tests
enabled. If an increment needs support for cases the plan scoped out, or turns into several new
modules of preparatory code, stop and propose a plan revision instead of continuing.

Handle each new finding one of four ways: fix it within the current slice if it is small and
on the path, record it under "Not yet planned" in the plans README, refute it, or ask.

## Writing documentation and comments

- **State each fact once and link to it.** Do not copy rules, counts, toolchain pins, statuses
  or test lists into other files. If something has a source of truth in code or a script
  (a manifest, `ci.yml`, `rust-toolchain.toml`), link to it instead of restating it.
- **Describe the present.** No dated status paragraphs, run timings, commit or run IDs, step
  numbers or "previously/now" narration in normative docs or comments. Git has the history;
  dated evidence goes in an evidence file.
- **Define IDs where you use them, or don't use them.** Finding codes (p01, R02) are defined
  in the pointer-provenance inventory. Outside it, link to the inventory or write the
  test name instead.
- **Positive, checkable rules.** Say what must hold. Avoid piles of "this is not a proof of X"
  caveats; state the guarantee once, with its limit, in the owning document.
- **Cite symbols, not line numbers**, in docs.
- **Comments explain why the code is the way it is**: the invariant, the non-obvious
  constraint, the spec rule (cite the ECMA-335 section). They do not record which plan or step
  produced the code or reference planning files.

## Feature gating and synchronization

Gate `multithreading`-only code at module or item boundaries rather than scattering `#[cfg]`
through function bodies. Use the lock and atomic types from `dotnet_utils::sync` instead of
`std::sync`/`parking_lot` equivalents; they switch to Loom under `cfg(loom)`, to single-threaded
stand-ins without `multithreading`, and participate in lock ordering. `Arc`, `OnceLock`,
`LazyLock` and test-only `std::sync` types are fine.

## Invariants and unsafe code

[`docs/INVARIANT_REGISTRY.md`](docs/INVARIANT_REGISTRY.md) is the inventory of every
non-obvious fact the code relies on for soundness. Each fact is a named predicate such as
`F4.WidthAligned`. Code cites it at the point of reliance with one of two markers:

- `// SAFETY: F<n>.<Name> — <why it holds here>` immediately above an `unsafe` operation or
  `unsafe impl`.
- `// INVARIANT: F<n>.<Name> — <why it holds here>` where safe code relies on or maintains a
  registered invariant (for example, the code that retains an arena until its drain completes,
  or the layout arm that grants the runtime-handle override). Never use `SAFETY:` on safe code.

`scripts/check_doc_drift.sh` requires both directions: every marker cites an existing predicate,
and every predicate is cited somewhere. When you rely on a new kind of fact, add its registry
row in the same commit.

Rules for unsafe code:

- Put a `// SAFETY:` comment immediately above every `unsafe { ... }` block.
  The comment must explain the invariant that makes the operation valid; it
  must not merely restate the operation, and it cites the matching registry
  predicate.
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
configuration. Clippy lints the debug build, so unsafe code inside
`#[cfg(not(debug_assertions))]` needs the same care without the lint's help.

A SAFETY comment states the local premise: what was checked and where, or which type or guard
guarantees it. "The caller guarantees" belongs only inside an `unsafe fn` whose `# Safety`
section lists that guarantee. Cite the predicate that actually matches. Do not paste the same
justification at many sites; put the contract in the function's `# Safety` section and have
call sites state only what is local.

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

While iterating, run focused tests for what you changed. When a coherent change is done, run
once:

```bash
./check.sh --fast
```

CI selects deeper Miri and fuzz lanes from the changed paths; see
[`docs/CI.md`](docs/CI.md) for what runs where and how to reproduce a lane locally. Run a deep
lane locally when you are changing unsafe code it covers or debugging its failure.

A zero-match test filter, an unexpected ignore, a timeout or a failed setup is not a pass.
Nextest does not run doctests; run relevant compile-fail doctests separately.

## Build test fixtures

Some test paths rely on managed fixtures. Build them with:

```bash
cargo run -p xtask -- fixtures build
```

## Miri, Loom and fuzzing

The pinned nightly, Miri flags, selectors and corpus-replay commands are in
[`docs/CI.md`](docs/CI.md) and [`docs/FUZZING.md`](docs/FUZZING.md). When you change unsafe
code, add a focused regression for the invariant you changed and run it under Miri locally.
Broad Miri selections run in CI.

## Documentation-only changes

Run `bash scripts/check_doc_drift.sh` and the intra-doc-link check:

```bash
DOTNET_SKIP_BUILD=1 RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps --no-default-features
```

No runtime build, Miri or benchmarks are needed.

## Feature flag matrix reference

Feature combinations and validation ownership are documented in:

- [`docs/VALIDATION_FEATURES.md`](docs/VALIDATION_FEATURES.md)

For a subsystem overview, see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). For broader CI/local validation guidance, see [`docs/CI.md`](docs/CI.md). The project overview and quick-start flow live in [`README.md`](README.md).
