# Library audit cleanup dispositions

This records the implementation disposition for LIB-1 through LIB-25 in
`libraries.md`. It distinguishes source-policy guards from runtime evidence and
lists only validation that was actually run. `dotnet-vm`, `dotnet-cli`, and
`dotnet-benchmarks` changes are owned by the other cleanup agents; shared CI,
benchmark, manifest, and diagnostic-checker wiring is owned by the coordinator.

## Dispositions

| Finding | Disposition, changed symbols, and retained or replacement coverage |
| --- | --- |
| LIB-1 | Removed `dotnet_utils::gc::cross_arena::tests::arena_lifetime_transport`, which scanned production source text. Retained the live transport controls in `dotnet_value::pointer::tests::{arena_lifetime_transport_permissive_control,arena_lifetime_transport_strict_core}` and production registry lease/identity tests. The strict-core control transports and reads a live allocation under strict provenance. It does not claim to exercise VM `MarkObjects` coordination. |
| LIB-2 | Strengthened `fuzz_managed_ptr_offset` to access a byte through the returned in-bounds pointer and verify origin, offset, address, and byte. Added `ptr_arith_live_offset_preserves_provenance_strict_core` over live unmanaged, frame-home, and static origins. The broader arithmetic test still covers permissive exposed-provenance cases. |
| LIB-3 | Replaced generated-code whitespace/token-tail snapshots in `dotnet_macros::tests` with a complete normalized keyword-to-`BaseType` AST mapping check. The VM now executes representative loaded `Array.GetValue(int)`, `GetValue(long)`, and `GetValue(int[])` registry matches and cross-mismatch/rejection cases, plus the loaded `Math.Sqrt(double)` handler. Installed .NET 10 CoreLib lacks the optional two-int overload; the table test covers the full mapping, but runtime execution covers representative loaded shapes rather than every keyword. |
| LIB-4 | Removed literal total/census assertions from support-slot tests. `support_contract_tests` now keeps an independent named legacy ID-to-number baseline, compares generated/loaded IDs and exact shapes to the generated descriptors, and checks the next assignable ID dynamically. Unknown-ID validation uses `i32::MIN`, outside the positive slot-ID domain. Slot discovery, uniqueness, dense numbering, owner/layout checks, and parser coverage remain. |
| LIB-5 | Reworked malformed support-metadata checks to assert the validation error variant, structured owner/field context, and stable diagnostic reason fragments instead of full prose. Distinct missing, duplicate, misplaced, signature, staticness, and annotation failures remain. |
| LIB-6 | Changed concrete type identity, assignability, subclass/interface checks, and variance comparison to return `Result<bool, TypeResolutionError>`. Resolution failures propagate through runtime-resolver and VM/reflection callers to `VmError::TypeResolution`; `Ok(false)` remains a resolved mismatch. Tests cover unresolved left and right operands, equal error payloads on distinct definitions, valid identity, mismatch, and assignability controls. Existing bool compatibility surfaces remain tolerant as described below; fallible comparer paths use `try_is_value_type`. |
| LIB-7 | Made `rust_files` return `io::Result`, propagate directory and entry errors, and added `scanner_fails_when_an_inventory_root_cannot_be_read`. The inventory no longer relies on a file-count threshold; named source inventory, retired-variant scan, and runtime ABA/lease tests remain. |
| LIB-8 | Removed duplicate exact-layout integration snapshots for `StackValue` and VM frame types. Kept the justified 64-bit `StackValue` upper-bound const assertion, gated out under Loom instrumentation, and kept the production layout/type contracts. The guard permits smaller representations. |
| LIB-9 | Replaced constructed `CompareExchangeError` values with `compare_exchange_reports_mismatch_and_owner_bounds_errors`, which invokes `RawMemoryAccess::compare_exchange_atomic` on live string storage. It checks the mismatch payload, owner bounds payload, and unchanged bytes after both outcomes. |
| LIB-10 | Replaced fixed shared version-test directories with per-test unique RAII temporary roots in `version_tests`; cleanup is scoped to each test. Basic binding and redirect/mismatch assertions remain. |
| LIB-11 | Made the direct self-reference case assert its successful result. Merged duplicate cyclic tests into `test_cyclic_generic_constraint_rejects_incompatible_argument_and_terminates`, asserting `GenericConstraintViolation`. A bounded resolver fixture returns a different error on runaway recursion; the test leaves the incidental number of successful resolutions unconstrained. Retained the independent non-cyclic violation test. |
| LIB-12 | Replaced `DefaultHasher` inequality assumptions with a deliberately colliding map test in `method_description_eq_hash_tests`; unequal declaring-type descriptors retrieve independent entries despite the collision, while equal descriptors remain hash-consistent. |
| LIB-13 | Removed the `WellKnown::COUNT == 60` pin. Retained existing discriminants and the contiguous, unique name/discriminant inventory whose length tracks `COUNT`, allowing appended values without renumbering old variants. |
| LIB-14 | Reworked atomic alignment/fallback tests to use explicitly aligned backing plus offset one. Typed-width expectations now compare native-endian bytes. The memory-validation test observes both atomic and non-atomic access-kind records after the production store/validator calls; its claim is limited to tracking and does not assert warning emission. Width, ordering, and valid-storage controls remain. |
| LIB-15 | Annotated the sealed `FieldType` implementation example with `compile_fail,E0277`; the unsafe read example retains `compile_fail,E0133`. The coordinator extended the existing diagnostic checker with positive controls, exact expected codes, and both feature configurations. |
| LIB-16 | Reworked the cross-arena unregister, re-registration, and STW stress controls. Unregister is observed through lease invalidation before lease release and uses bounded completion; re-registration requires the new lease and advanced generation; workers are joined before cleanup/assertions. STW observations are asserted while writer updates are serialized per arena. |
| LIB-17 | Strengthened and renamed `test_managed_ptr_offset_valid` to `ptr_arith_managed_ptr_offset_valid`; it checks `PointerOffset::Managed(4)` and reads the expected in-bounds byte through the result. |
| LIB-18 | Restricted `test_arena_mismatch_panics` to `memory-validation` without `multithreading`, where its single-threaded wrong-arena policy applies. The empty MT body is gone; separate multithreaded dangling-arena and uncoordinated-STW tests remain. |
| LIB-19 | Removed the duplicate `publication_pending_survival` case. Kept one negative old-path case and one root-barrier control, and labeled both as a local raw-slot/root-barrier model rather than VM `MarkObjects` coverage. Production VM lifecycle controls remain with the VM owner. |
| LIB-20 | Renamed the vector trace counter test to `tracing_a_value_type_vector_visits_embedded_object_refs`; it claims that production `Vector::trace` visits embedded refs, not that a collection keeps them alive. |
| LIB-21 | Updated `next_test_arena_id` to skip reserved boundary IDs. Only tests requiring fixed boundary registrations share a mutex, including the ID-40000 cases; other users of test arena contexts can continue concurrently. |
| LIB-22 | Strengthened the unmanaged branch of `fuzz_managed_ptr_roundtrip` to compare decoded offset and `PointerOffset::Unmanaged` address bits with the input, including null and high-bit representations. Other origins, tag/checksum checks, malformed subtags, and live pointer controls remain. |
| LIB-23 | Gated `test_object_ptr_as_heap_storage_corrupt_magic_panics` and its `with_magic_for_tests` constructor on debug assertions or `memory-validation`, matching the production `validate_magic` policy. The validation-enabled corruption assertion remains; release/no-validation discovery omits it. |
| LIB-24 | Removed `dotnet_types::tests::test_null_exists`, which only named sentinel constants without asserting behavior. Public null sentinels remain referenced by production code and the type test suite. |
| LIB-25 | Removed the test-only `WriteBackSource::Raw` variant and copy arm. `by_ref_write_back_sends_managed_bytes_and_offset_to_writer` exercises the production managed write-back path through a recording context and checks origin, offset, bytes, and error propagation; the existing managed-write failure test remains. |

### LIB-6 policy boundary

The fallible behavior is limited to concrete comparison/assignability and the
dependent variance, array, runtime-resolver, VM, and reflection paths migrated
for LIB-6. Existing tolerant APIs remain explicit compatibility surfaces:

- `TypeComparer::{type_slices_equal,types_equal,signatures_equal}` return `bool`
  and treat unresolved signature references as nonmatches.
- `ConcreteType::{is_class,is_value_type,is_interface,is_nullable,has_default_constructor}`
  return `bool`; lookup errors are indistinguishable from ordinary Boolean
  results (and `is_class` may still classify an unresolved type reference as a
  class after `is_value_type` falls back to `false`). `try_is_value_type` is the
  fallible sibling used by the new comparer.
- `GenericLookup::validate_constraints` propagates errors from fallible
  assignability, but its legacy reference/value/default-constructor special
  constraint checks still call the tolerant boolean classifiers above.
- `runtime_type_from_concrete` and `runtime_type_from_method_type` return
  `Option<RuntimeType>` and retain their existing optional-probe behavior.

The remaining tolerant classifier and optional-probe policies are a separate
follow-up tracked in the [plan follow-up queue](../README.md); this cleanup does
not convert unrelated public query APIs.

## Validation run

Commands and results below are the exact focused or matrix checks run for this
cleanup. Root-owned combined checks and diagnostics are included only after
their result was reported.

### Focused library and feature checks

- `cargo test -p dotnet-types` — 31 passed.
- `cargo test -p dotnet-vm-ops array_assignability::tests` — 2 passed.
- `cargo test -p dotnet-macros` — 12 passed.
- `cargo test -p dotnet-utils --no-default-features --features multithreading cross_arena` — 26 passed.
- `cargo test -p dotnet-runtime-memory --no-default-features --features multithreading compare_exchange_reports_mismatch_and_owner_bounds_errors -- --test-threads=1` — 1 passed.
- `cargo test -p dotnet-runtime-memory --lib --no-default-features --features memory-validation compare_exchange_reports_mismatch_and_owner_bounds_errors -- --test-threads=1` — 1 passed.
- `cargo test -p dotnet-utils --lib --no-default-features --features memory-validation test_mixed_access_validation_records_both_access_kinds -- --test-threads=1` — 1 passed.
- `cargo test -p dotnet-value --lib --no-default-features --features memory-validation test_arena_mismatch_panics -- --test-threads=1` — 1 passed.
- `cargo test -p dotnet-value --lib --no-default-features --features memory-validation test_object_ptr_as_heap_storage_corrupt_magic_panics -- --test-threads=1` — 1 passed.
- `cargo test -p dotnet-value --lib --release --no-default-features -- --list` — 79 tests listed; the corrupt-magic selector is absent, with no warning after gating its test constructor.
- `cargo test -p dotnet-assemblies support_contract` — 19 passed against the built embedded support assembly.
- `cargo run -p xtask -- verify-slots` — verified 21 primary support-slot accessors.
- Two simultaneous `cargo test -p dotnet-assemblies version_tests` invocations — each 4 passed, including binding and redirect cases.
- `cargo test -p dotnet-value --test stack_value_cross_arena_retired --no-default-features` and `cargo test -p dotnet-value --test stack_value_cross_arena_retired` — each 3 passed.
- `cargo test -p dotnet-value --lib --features multithreading codec_` — 7 passed at normal parallel test scheduling.
- `cargo test -p dotnet-pinvoke by_ref_write_back` — 2 passed.
- `cargo test -p dotnet-types --lib --features generic-constraint-validation direct_type_generic -- --test-threads=1` and `cargo test -p dotnet-types --lib --features generic-constraint-validation cyclic_generic_constraint -- --test-threads=1` — each 1 passed.
- `cargo check -p dotnet-types --no-default-features`, `cargo test -p dotnet-types --no-run`, `cargo check -p dotnet-vm-ops --no-default-features`, `cargo check -p dotnet-runtime-resolver --no-default-features`, and `cargo check -p dotnet-assemblies --no-default-features` — all passed during the comparer API migration.

### Miri and fuzz

- `MIRIFLAGS='-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks -Zmiri-strict-provenance' cargo +nightly-2026-05-27 miri test -p dotnet-utils --lib --no-default-features atomic::pointer_atomic_tests::unaligned_pointer_access_is_limited_to_synchronized_load_and_store -- --exact --test-threads=1` — 1 passed.
- `MIRIFLAGS='-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks -Zmiri-strict-provenance' cargo +nightly-2026-05-27 miri test -p dotnet-utils --lib --no-default-features atomic::tests::test_load_field_misaligned_falls_back_non_mt -- --exact --test-threads=1` — 1 passed.
- `MIRIFLAGS='-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks -Zmiri-strict-provenance' cargo +nightly-2026-05-27 miri test -p dotnet-runtime-memory --lib --no-default-features access::tests::compare_exchange_reports_mismatch_and_owner_bounds_errors -- --exact --test-threads=1` — 1 passed.
- `MIRIFLAGS='-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks -Zmiri-strict-provenance' cargo +nightly-2026-05-27 miri test -p dotnet-value --lib --no-default-features --features multithreading pointer::tests::arena_lifetime_transport_strict_core -- --exact --test-threads=1` — 1 passed.
- `ASAN_OPTIONS=detect_leaks=0 python3 scripts/replay_fuzz_corpus.py --workdir crates/dotnet-value --corpus fuzz/corpus/fuzz_managed_ptr_roundtrip --target fuzz_managed_ptr_roundtrip --output-dir /tmp/dotnet-rs-27b-fuzz-libraries` — 129 tracked inputs verified; 184 executions completed.
- `ASAN_OPTIONS=detect_leaks=0 python3 scripts/replay_fuzz_corpus.py --workdir crates/dotnet-value --corpus fuzz/corpus/fuzz_managed_ptr_offset --target fuzz_managed_ptr_offset --output-dir /tmp/dotnet-rs-27b-fuzz-libraries` — 171 tracked inputs verified; 172 executions completed.

The fuzz replays disable LeakSanitizer because this sandbox runs under ptrace;
address instrumentation remains enabled. This is an execution-environment
constraint, not a source or CI suppression.

## Remaining coordination-owned validation

The coordinator owns the combined `check.sh` matrix, diagnostic checker/CI
wiring, Miri selector manifests, advisory discovery accounting, and benchmark
smoke. The VM and CLI agents own their crate callers and fixture lanes. Their
final combined results are recorded in the [shared cleanup verification
ledger](../27-test-audit-ledger.md#combined-cleanup-verification); this file
does not duplicate root-owned commands or claim those results on their behalf.
