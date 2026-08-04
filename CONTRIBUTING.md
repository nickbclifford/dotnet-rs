# Contributing to dotnet-rs

Thanks for contributing to `dotnet-rs`.

## Prerequisites

Before running local builds or tests, install:

- **Rust 1.95.0**, selected by the repository's `rust-toolchain.toml` and
  including the `clippy` and `rustfmt` components. Rustup activates this pin
  automatically when commands are run from the repository.
- **.NET SDK** (required for fixture compilation and build-script paths that invoke dotnet/MSBuild)

## Unsafe code policy

Unsafe code must make the invariant that justifies it reviewable at the point
where it is relied on. Follow these rules when adding or changing unsafe code:

- Put a `// SAFETY:` comment immediately above every `unsafe { ... }` block.
  The comment must explain the invariant that makes the operation valid; it
  must not merely restate the operation.
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

The longer-term plan for this policy — naming the invariants these comments
cite, and the falsifiers that check them — is
[`docs/ASSURANCE_ROADMAP.md`](docs/ASSURANCE_ROADMAP.md).

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

For the standard local validation pass used by contributors, run:

```bash
bash check.sh
```

`check.sh` includes the ratcheted multithreading cfg-occurrence budget. When changing feature gates, run its focused check directly as well:

```bash
bash scripts/check_mt_cfg_ceiling.sh
```

## Build test fixtures

Some test paths rely on managed fixtures. Build them with:

```bash
cargo run -p xtask -- fixtures build
```

## Miri policy

The pinned-nightly `dotnet-value` suite is a blocking CI gate. Run it locally when changing its unsafe or atomic-memory code:

```bash
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
cargo +nightly-2026-05-27 miri test -p dotnet-value -- --test-threads=1
```

### `dotnet-vm` unsafe-gate advisory signoff

For `dotnet-vm` changes that add or modify `unsafe`, the accepted local advisory invocations
cover both the no-feature and `multithreading` configurations:

```bash
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
cargo +nightly-2026-05-27 miri test -p dotnet-vm --no-default-features -- --test-threads=1 jmp_tests tail_calls fault_tests
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks" \
cargo +nightly-2026-05-27 miri test -p dotnet-vm --no-default-features --features multithreading -- --test-threads=1 jmp_tests tail_calls fault_tests
```

Why this is the project-standard invocation right now:
- `-Zmiri-strict-provenance` currently fails before VM execution due dependency-level integer-to-pointer casts reached during assembly parsing (`bitvec`/`dotnetdll`, plus rayon worker-thread paths via `crossbeam-epoch`).
- `-Zmiri-disable-isolation` is needed for host filesystem syscalls used by the test harness.
- `-Zmiri-ignore-leaks` is needed for non-joined background worker threads that are not VM-unsafe regressions.

Until upstream dependency behavior changes, strict-provenance is treated as infeasible for `dotnet-vm` unsafe-gate signoff.
The Miri-only nightly is pinned separately because `rust-toolchain.toml` selects
stable by default. As recorded in [`docs/CI.md`](docs/CI.md), neither targeted
VM command completed within the local validation time box on this nightly, so
the pin records the attempted toolchain rather than known-passing Miri results.

## Blocking fuzz corpus replay

Changes to atomic-memory code should also replay the tracked `fuzz_raw_memory_access` corpus with
the CI-pinned toolchain:

```bash
cd crates/dotnet-value
cargo +nightly-2026-05-27 fuzz run fuzz_raw_memory_access -- -runs=0
```

The other fuzz targets remain advisory; their known failures are documented in
[`docs/FUZZING.md`](docs/FUZZING.md).

## Documentation drift check

If your change touches docs, run the doc drift gate and rustdoc link check:

```bash
bash scripts/check_doc_drift.sh
DOTNET_SKIP_BUILD=1 cargo doc --no-deps --no-default-features
```

## Feature flag matrix reference

Feature combinations and validation ownership are documented in:

- [`docs/VALIDATION_FEATURES.md`](docs/VALIDATION_FEATURES.md)

For a subsystem overview, see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). For broader CI/local validation guidance, see [`docs/CI.md`](docs/CI.md). The project overview and quick-start flow live in [`README.md`](README.md).
