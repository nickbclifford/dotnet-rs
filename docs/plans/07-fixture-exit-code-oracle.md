# Plan 07 — Fixture exit-code oracle

**Gate:** harness-level outcomes reserve the high `240`–`254` exit-code band
distinct from fixture-authored codes, with unhandled managed exceptions using
`254`; setup failures and executor errors intentionally retain the existing
`255` catch-all. An opt-in differential *exit-code-only* comparison against
real `dotnet` exists for fixtures that cannot use the stdout-diff `diff_test!`
path.

**Status:** complete (2026-09-17). This independent, low-coupling test
infrastructure plan is complete.

## Goal

The managed fixture suite (178 `.cs` files under
`crates/dotnet-cli/tests/fixtures`) intentionally asserts only a `u8` exit
code encoded in the filename — assertions live in the C# body, and comparing
stdout for every fixture would couple all of them to the console/IO stack.
That design choice is correct and is not being revisited here.

The gap this plan closed was that exit code `1` was overloaded. The harness
formerly mapped `ExecutorResult::Threw` to exit code `1`, which is also the
dominant C# first-failure-branch idiom (`Environment.Exit(1)` on the
"something went wrong" path). The four unconditional-throw exception fixtures
therefore could not distinguish "threw with the expected trace" from "threw at
all" — both produced the same observable exit code. Separately, `Error`
(executor-level failure) and setup failure both mapped to `255` (`harness.rs`),
which the harness already reserved as a high code, so the completed change
extended that convention instead of inventing a new one.

There is also a confirmed, documented compatibility divergence: real `dotnet`
exits `134` (`SIGABRT`) on an unhandled exception, while `dotnet-rs` exits `1`.
The rejected-candidates record preserves it until plan 06's trust register
exists, because no fixture checks that process-level behavior directly.

## Implemented state (2026-09-17)

- 178 `.cs` fixtures under `crates/dotnet-cli/tests/fixtures`, generated into
  `tests.rs` at build time by `crates/dotnet-cli/build.rs`.
- Exit-code assignment in `harness.rs`: setup error → `255`,
  `ExecutorResult::Threw` → the named in-process
  `MANAGED_EXCEPTION_EXIT_CODE` (`254`), and `ExecutorResult::Error` → `255`.
  `ExecutorResult::Exited(code)` passes the fixture-authored code through
  unchanged. Thus `240`–`254` is reserved for harness-level outcomes while
  `255` remains the intentional setup/executor-error catch-all.
- The four unconditional-throw fixture sources are
  `exceptions/stack_trace_{no_params,params,generic}_254.cs` and
  `exceptions/unhandled_exception_254.cs`, so their filename oracle now
  distinguishes an in-process managed exception from fixture-authored `1`.
- Seven fixtures use the stdout-comparing `diff_test!` path, and the new
  `diff_test_exit_code!` test covers the qualified
  `exceptions/intrinsic_trace_42.cs` fixture at expected exit code `42`
  without comparing stdout.
- `diff_harness.rs` carries a rejected-candidates comment block naming the
  divergence classes this plan addresses:
  `structs/interlocked_misaligned_1` (intentional ECMA alignment divergence),
  `exceptions/exception_filter_5` (VM correctness mismatch),
  `exceptions/unhandled_exception_254` and the `stack_trace_*_254` fixtures
  (in-process oracle `254`; unchanged real-.NET `134` versus dotnet-rs CLI
  `1` subprocess behavior), and `exceptions/intrinsic_trace_42` (covered only
  by the exit-code differential test because of its GC.Collect stdout
  mismatch). That list remains an assurance artifact — the record of known,
  intentional divergences.

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
