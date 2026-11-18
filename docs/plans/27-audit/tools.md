# Tools and benchmark test review

Baseline: `8431c1ab`. Scope/cases are enumerated in [the inventory](tools-inventory.md).
Source inspection covered every listed Python test body and its synthetic inputs, xtask unit test,
shell/compiler probe, benchmark source and managed benchmark fixture. Test-selection/result
boundaries were inspected through their owning scripts and workflow consumers; this is not a CI
redesign or an assertion that every workflow action was executed.

## Findings

### TOOLS-1 — fixed reviewed pointer-control quota (medium; rewrite)

`scripts/pointer_provenance.sh` sets `reviewed_count=44` independently of `expected_controls` and
`catalog`. `PointerScriptTests.test_simulated_promotion_and_stale_table` explicitly requires even
a promotion updating both catalogs to fail, and `test_singleton_partition_is_one_control_per_shard`
pins the final shard and total. `test_batches_cover_each_control_exactly_once` additionally pins
four batches and a 10–12 membership range. The intended contracts are named, enabled, exactly-once
execution and consistent partitioning, not this cardinality or balance. A fully registered new
control is rejected even before discovery.

Observed experiment: a temporary script copy added `p99_audit_additive_control` to both tables
at shard 45. `list` failed with exit 2, "expected control count differs from reviewed count".
Changing only the temporary copy to derive the total from `expected_controls` made `list` succeed
and include the new control. No checkout script was modified and no probe ran.

Remove independent totals and arbitrary batch-size constraints; derive partition identities and
execution expectations from the authoritative named selection. Preserve checks against actual
native/nextest discovery, missing and duplicate cases, unexpected ignores, unknown probes,
diagnostic-only exclusions, and incomplete batches. Replace the promotion test with successful
fully registered growth and failing partial registrations. Verify with
`python3 scripts/tests/test_pointer_provenance.py` and static `verify-batches`, including additive,
missing, duplicate, ignored and incomplete-execution cases.

### TOOLS-2 — advisory manifest freezes suite cardinality (medium; rewrite)

`scripts/miri_advisory.py::manifest` requires precisely A01..A38 and V01..V08; `CONFIGS` repeats
their totals, and `AdvisoryTests.test_manifest_matrix_exactly_once` pins a third aggregate total.
`test_manifest_rejects_missing_extra_duplicate_and_invalid` requires a new A39 case to be rejected.
The intended contract is that the original full/filter discovery partitions into reviewed named
singletons exactly once, with errors/ignores preserved. A legitimate extra test and manifest row
cannot be admitted without editing independent counters. An observed temporary-manifest experiment
adding a unique valid A39 selector failed with the fixed-range diagnostic.

Derive totals/shard rows from the authoritative reviewed manifest; keep independent comparison
with full native/Miri discovery so deriving totals does not make the runner certify its own
coverage. Keep selector validation, uniqueness, zero/missing-case rejection and exact result
verification. Replace the extra-row rejection with successful additive growth while retaining
missing-discovery and unexpected-discovery controls. Verify `python3 scripts/tests/test_miri_advisory.py`
and the actual discovery lane before considering coverage preserved. Ancillary targets gaining
tests should produce an actionable coverage failure, not be silently omitted.

### TOOLS-3 — redundant census snapshots in workflow tests (low; consolidate)

`CrossArenaManifestTests.test_matrix_teardown_is_exact_and_mt_only` fixes 98 rows and
`test_each_row_executes_guarded_exact_selectors` fixes 33/81 unique selections after checking
specific named sets. Related pointer/advisory tests duplicate totals and numeric error examples.
These are maintenance traps rather than evidence of a bloat-reduction policy. The named required
cases, configurations, uniqueness and actual shell invocation checks carry the useful contract;
a valid additive case/shard requires unrelated numeric edits today.

Remove independently repeated census totals where named-set/partition checks already establish
the intended guarantee. Retain independently justified required selectors; do not simply compare
the manifest with a list generated from itself. Choose out-of-range rejection examples from the
actual shard domain. Verify the cross-arena, pointer and advisory script tests, including a
temporary additive named case and removal of an existing required case.

### TOOLS-4 — policy pin rejects correctly handled Markdown input (low; remove assertion)

`SelectionTests.test_repo_has_no_unexpected_md_includes` requires `sel.md_references(REPO) == {}`
even though `test_md_pulled_in_by_include_str_is_code` already verifies correct handling of
included Markdown with independent expected lane results. Adding a legitimate `include_str!`
documentation source fails the empty-map assertion while the selector handles it correctly.
Keep the positive/negative selector cases; remove the empty-repository snapshot rather than
updating it whenever a valid include is introduced. Verify `python3 scripts/tests/test_ci_select.py`.

### TOOLS-5 — no-invocation probe observes log spelling (medium; rewrite)

`check_build_script_regressions.sh::assert_clippy_skips_dotnet` promises that clean clippy does not
invoke dotnet/MSBuild, but only searches output for `dotnet (build|restore)|msbuild|csproj`.
A quiet unintended invocation can pass; an unrelated diagnostic mentioning a project can fail.
The production build scripts invoke `Command::new("dotnet")`, so a PATH shim recording/rejecting
actual invocations is a direct witness. Use that bounded probe instead of a broad log-text oracle;
keep clean-build behavior, skip-toggle artifact checks and genuine scanner diagnostics. Verify the
build-script probe with a quiet invocation negative control. It was inspected, not executed during
this audit, because it runs `cargo clean` and temporarily edits shared sources.

### TOOLS-6 — benchmark decode preflight misses expected rejection/success paths (low; strengthen)

`benches/managed_ptr_serde.rs::bench_managed_ptr_serde` documents successful decoding for the
ordinary origins and `UnknownSubtag` rejection for Transient. Its timed reads only black-box the
`Result`; only the Heap cache preflight asserts success. An ordinary-origin regression returning
an error, or accepting Transient, can still produce a misleading performance result. This is a
measurement-validity issue, not a demand that a benchmark replace pointer correctness tests.

Before timing each case, validate its intended success/rejection and essential decoded origin
against the independently known fixture, keeping these assertions outside timed loops. Preserve
the deliberately simplified resolver's limited measurement scope. Verify bench compilation in its
supported features and a focused benchmark smoke with success and rejection paths.

### TOOLS-7 — benchmark assumes a stronger overlap contract than cpblk provides (medium; rewrite workload)

`fixtures/memory/MemoryBulkOps_0.cs` and `fixtures/unsafe_buffer/UnsafeBufferOps_0.cs` use overlapping
`Unsafe.CopyBlock` calls with comments demanding memmove-compatible behavior. Their checksums
can turn the result into acceptance evidence for that unsupported premise. ECMA-335 III.3.30
specifies overlapping cpblk behavior as unspecified; the section was verified through the
connected ECMA-335 text search. Production registers these Unsafe.CopyBlock overloads in
`dotnet-intrinsics-unsafe/src/unsafe_ptr.rs`.

Use an API with an overlap guarantee (for example span CopyTo) for the overlapping workload,
while preserving disjoint CopyBlock coverage. Establish predictable results for each workload
outside timing. Do not constrain a conforming implementation's cpblk overlap result or change the
runtime to satisfy this benchmark. Verify the revised fixture on CLR and VM and its benchmark smoke.

## Retained contracts and review coverage

All individual symbols are in the inventory; the following rationales group their setup/assertions.

| Family | Disposition and evidence |
|---|---|
| CI diagnostics | Keep synthetic child-process tests: independent exit/status/message cases, actual signals/timeouts and files exercise the production wrapper. They distinguish UB, setup failure, guard failure, cancellation and timeout, preserve partial logs and avoid inventing success. The direct group-timeout case asserts classification but not descendant death; the separate process-group case actually checks liveness. Timing bounds are generous test deadlines, not quotas. |
| CI aggregates | Keep execution of extracted real aggregate shell with explicit success/failure/cancelled/missing combinations. Fallback output-expression spelling is a narrow configuration guard, not runtime evidence; tolerate spelling refactors when these predicates are expressed another way. |
| CI selector | Keep table-driven expected lanes, real git branch/range cases, error/full-sweep fallback, output files and Cargo-derived dependency closure checks. Using live selector closures for broad crate loops is supplemented by independent representative paths and an independently derived Cargo closure. TOOLS-4 removes the unrelated repository-content pin. |
| Cross-arena workflow | Keep extracting/executing actual job shell with a fake Python command at the external boundary. Named selector sets, crate/feature flags, exact execution, strict-provenance isolation and no-ignore guards have real coverage meaning. The fake runner establishes invocation wiring, not Miri correctness. Consolidate totals under TOOLS-3. |
| Pointer runner | Keep fake-Cargo controls for all successful shards/configurations, discovery mismatches, ignores, incomplete/failed/UB/timed-out runs, durable logs, dry-run behavior and argument validation. Mutating temporary runner/source copies meaningfully tests drift rejection. Preserve named batch order if it remains the declared execution protocol; remove cardinality/balance policy under TOOLS-1. |
| Advisory Miri | Keep synthetic listings, diagnostic interleaving, duplicate/truncated/bad summary rejection and fake-Cargo child failure propagation. These test actual parser/runner behavior without claiming UB absence. TOOLS-2 addresses fixed membership limits. |
| Fuzz replay wrapper | Keep independently constructed malformed logs, tracked/untracked/changed/empty/symlink corpus cases, known SHA-256 example, real temporary git index and fake cargo exit/log checks. They establish replay invocation and summary accounting; target semantic execution remains a separate VM/library finding. |
| perf-symbol resolver | Keep known perf-script input/percentages, foreign thread reporting, unresolved symbol and file/stdin shape cases. Formatting assertions protect the produced report, the actual output of this utility. |
| VM-only differential guard | Keep early rejection test with a known VM-only fixture; asserting exit 2 and the denial message protects a meaningful external boundary. |
| xtask | Keep all 12 units: independently constructed path/option expectations, format selection and syntax-tree scanner positive/negative inputs, real temporary source files for missing accessor rejection. They exercise production functions, not replicas. Exact output paths and names are utility contracts. |
| Build/codegen probes | Keep actual source mutation, compilation, async dispatch execution and expected scanner/compiler diagnostic checks. Generated dispatch text/count assertions are representation evidence and can be consolidated with equivalent execution checks when refactoring; they count providers for a fixed synthetic input, not other tests. Existing-arm byte comparison overconstrains generation representation but deliberately detects collateral edits; replace only with an equivalent justified registration check. TOOLS-5 fixes the no-dotnet oracle. |
| Compile-fail/doctest facade | Keep positive compilation and expected error-code/lifetime filtering, rather than accepting any compiler failure. The example bodies are covered by the libraries partition. |
| Policy checks | Keep doc-drift negative injection: the real gate must emit all four independent missing/unknown citation diagnostics, with nonzero status and cleaned temporary input. Retain provenance compiler lints as structural checks; neither claims to execute a memory-safety proof. |
| Benchmark harness | Keep repeated static-state test: two successful runs followed by a specific guest exception witnesses retention, and repeated warm-resource runs protect the void-entrypoint reuse premise. Actual `ExecutorResult` errors fail. This is a justified internal harness contract. |
| Managed benchmarks | Keep smoke-level workload acceptance, not full correctness claims: arithmetic/dispatch/generics/GC/JSON/LINQ/reflection use nonzero checksums; exception/resource probes require nonempty messages. Heap/static-field, handle/weak/dependent identity, suspended await totals, span equality with inserted mismatches and runtime type identity have stronger independent checks. Substring probes check lengths, not contents. Similar span fixtures cover different offsets/lengths. Do not delete them just for overlapping operations. TOOLS-7 corrects the overlap premise. |
| Rust microbenchmarks | Keep metadata parse-success and stack non-underflow checks as measurement setup; primitive trace loops are timing workloads. End-to-end, cold-start, focused type/static/substring loops check guest exit results. Instrumented GC-pause upper bounds are performance requirements, not suite-size constraints; they alone do not establish that collections occurred. TOOLS-6 adds decode-path preflight. |

## Observed verification

- `python3 -m unittest discover -s scripts/tests -p 'test_*.py'`: 95 tests passed.
- `cargo test -p xtask --offline`: 12 tests passed.
- Temporary-copy pointer quota and advisory-manifest growth experiments: rejections reproduced as above.
- Benchmark, full clean-build probe, compile-fail, Miri, Loom, fuzz and managed reference runs were
  not executed by this partition. Their findings are source/spec inspection, not claimed run results.

No production/test code changed. All findings hand off to 27b; none requires changing supported
runtime behavior to perform the recommended test cleanup.
