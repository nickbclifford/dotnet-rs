# Plan 07 — Fixture exit-code oracle

**Gate:** harness-level outcomes (setup failure, unhandled managed exception,
executor error) occupy a reserved high exit-code band distinct from the
0–200 range fixture bodies use for their own assertions, so `1` and `255` stop
being overloaded; and an opt-in differential *exit-code-only* comparison
against real `dotnet` exists for fixtures that cannot use the stdout-diff
`diff_test!` path.

**Status:** not started. Independent of every other plan in this queue —
lowest coupling, no dependency in either direction. Do this when the appetite
is for test infrastructure rather than runtime code.

## Goal

The managed fixture suite (176 `.cs` files under
`crates/dotnet-cli/tests/fixtures`) intentionally asserts only a `u8` exit
code encoded in the filename — assertions live in the C# body, and comparing
stdout for every fixture would couple all of them to the console/IO stack.
That design choice is correct and is not being revisited here.

The real gap is that exit code `1` is overloaded.
`crates/dotnet-cli/tests/integration_tests_impl/harness.rs:264` maps
`ExecutorResult::Threw` to exit code `1`, which is also the dominant C#
first-failure-branch idiom (`Environment.Exit(1)` on the "something went
wrong" path). The three `exceptions/stack_trace_{no_params,params,generic}_1.cs`
fixtures therefore cannot distinguish "threw with the expected trace" from
"threw at all" — both produce the same observable exit code. Separately,
`Error` (executor-level failure) and setup failure both map to `255`
(`harness.rs:254,276`), which the harness already reserves as a high code, so
fixing this extends an existing convention instead of inventing a new one.

There is also a confirmed, unrecorded compatibility divergence: real `dotnet`
exits `134` (`SIGABRT`) on an unhandled exception, while `dotnet-rs` exits `1`
— the suite currently treats this as correct because no fixture checks for it.

## Current state (verified 2026-08-10)

- 176 `.cs` fixtures under `crates/dotnet-cli/tests/fixtures`, generated into
  `tests.rs` at build time by `crates/dotnet-cli/build.rs`.
- Exit-code assignment in `harness.rs`: setup error → `255` (`:254`),
  `ExecutorResult::Threw` → `1` (`:264`), `ExecutorResult::Error` → `255`
  (`:276`). `ExecutorResult::Exited(code)` passes the fixture-authored code
  through unchanged.
- 7 of 475 total fixtures (`.cs` files across the whole tree, not just this
  suite) are compared differentially against real `dotnet` via the
  `diff_test!` macro in
  `crates/dotnet-cli/tests/integration_tests_impl/diff_harness.rs`, which
  requires a fixture whose expected exit code is `42` and diffs both exit code
  and stdout.
- `diff_harness.rs` already carries a rejected-candidates comment block naming
  exactly the divergence class this plan addresses:
  `structs/interlocked_misaligned_1` (intentional ECMA alignment divergence),
  `exceptions/exception_filter_5` (VM correctness mismatch),
  `exceptions/unhandled_exception_1` and the `stack_trace_*` fixtures (real
  `.NET` exits 134 for unhandled exceptions while `dotnet-rs` exits 1), and
  `exceptions/intrinsic_trace_42` (GC.Collect timing mismatch). That list is
  itself an assurance artifact — the record of known, intentional divergences
  — and should be preserved, not replaced, by this plan.

## Steps

1. Reserve a high exit-code band (e.g. `240`–`254`, keeping `255` for the
   existing catch-all setup-error case) for harness-level outcomes, and give
   `ExecutorResult::Threw` its own code in that band distinct from any
   fixture-authored `1`.
2. Migrate the three `stack_trace_*` fixtures (and any other fixture relying
   on the overloaded `1`) to assert against the new dedicated code, so they
   can distinguish "threw with the right trace" from "threw at all" — today
   they cannot.
3. Add an opt-in, exit-code-only differential mode alongside `diff_test!` for
   fixtures whose stdout is known to diverge for a documented, intentional
   reason (timing-sensitive GC output, the alignment divergence) but whose
   exit code should still match. `diff_harness.rs` already has the subprocess
   and comparison machinery to generalize — this is a new macro arm, not new
   infrastructure.
4. Record the confirmed `134` vs `1` unhandled-exception divergence as a
   deviation entry once [plan 06](06-trust-register.md)'s trust register
   exists; until then, note it in `diff_harness.rs`'s rejected-candidates
   block (already partially done) rather than leaving it implicit.

## Not in scope

- Comparing stdout for the full fixture suite. Deliberately rejected —
  correctly, per prior review — because it would couple every fixture to the
  console/IO stack.
- Resolving the `134` vs `1` divergence itself (i.e. making `dotnet-rs` match
  `SIGABRT` semantics). This plan records and makes it testable; whether to
  change runtime behavior is a separate decision.

## Related

- [`docs/plans/README.md`](README.md)
- [`docs/plans/02-falsifier-portfolio.md`](02-falsifier-portfolio.md),
  instrument 5 — the differential-fixture-count ratchet this plan's new
  exit-code-only mode feeds
- [`docs/plans/06-trust-register.md`](06-trust-register.md) — destination for
  the `134`/`1` deviation entry
