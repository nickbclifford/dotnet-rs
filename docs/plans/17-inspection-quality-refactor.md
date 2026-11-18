# Inspection-driven code quality refactor

**Status:** Complete
**Outcome:** Shared semantic contracts replace the identified duplication, and remaining inspection
entries have verified fixes or documented reasons to retain them without weakening runtime safety.

The inspection ledger was removed after completion; read it with `git show 0f85ff25^:REVIEW.md`.

## What changed

- **Intrinsic operand and reflection-slot selection.** One byte `Min`/`Max` operand decoder in
  `dotnet-intrinsics-core` and one validated reflection index-slot selector in
  `dotnet-intrinsics-reflection`, used by every caller.
- **Raw-memory windows.** One checked-range contract (`checked_window`) for inline typed reads and
  writes, slice copies and pointer-sized accesses in `stack/raw_memory_ops_impl`, plus one
  effective frame-home layout selection for read and write; `BoundsCheck` payloads are exact.
- **Exception initialization and runtime transitions.** Synchronous throws and the native-effects
  adapter share `VesContext::initialize_exception_string`; executor finalizer publication and the
  object-write and address-resolution error translations each have one implementation.
- **Event transitions and accounting.** Locked event state has explicit
  `State::expire_waiters` and `State::deliver_auto_reset_signal` transitions; value-type stack
  writes charge allocation pressure through one helper (`account_value_type_write`).
- **Dependencies.** Four unused direct dependency edges were removed without upgrading APIs.
- A feature-aware readability sweep and a thread-local boundary-observation API for the native
  effect tests.

Guest behavior, public APIs, feature gating and the invariant-registry predicates were unchanged.
Numeric domain unification and new project-wide lints were out of scope; the dependency
major-version upgrades were done separately later.
