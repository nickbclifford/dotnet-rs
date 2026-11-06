# Repair enum and named scalar frame homes

**Status:** In progress (completion audit found remaining acceptance gaps)

The [completion audit](../ci-evidence/2026-11-05-plans03-04-completion-audit.md)
records fresh validation and remaining acceptance gaps. The implementation record
below is historical; its claim of an explicit byte-buffer-length check was not true
of the audited implementation. The subsequent guard is recorded below.

### Scalar-buffer repair (2026-11-05)

Scalar home stores now reject a byte buffer whose actual length differs from the
scalar layout size, inside the shared borrow and before the raw read. A regression
tests all shorter Int32 buffers and verifies the destination remains unchanged; the
identity and declared-layout rejection test remains in place. Both tests pass with
default features and with `--no-default-features`. VM clippy for no-default-features
tests passes with warnings denied. Explicit opcode and nested-growth mapping and the
fresh historical failure and identical-cache EF reproduction remain outstanding;
this repair alone does not complete the plan.

Pinned nightly-2026-05-27 Tree Borrows Miri passed p43 in both no-feature and
multithreading configurations and the short-payload test without features. The runs
emitted exposed-provenance warnings, which limit Miri's ability to detect pointer bugs.

## Evidence

With the identical prebuilt EF fixture from the slice-5 baseline, candidate `c743e4d0`
fails entering
`Microsoft.EntityFrameworkCore.Diagnostics.CoreEventId.MakeUpdateId(CoreEventId/Id)`.
A diagnostic build of the candidate in `/tmp` logged argument 0 as `Int32(10000)`;
its declared type is the enum `CoreEventId/Id`, with an Int32 `value__` field. The
method contains `ldarga.0` followed by a constrained virtual call. `variable_home`
constructs a struct field layout for every named value type, so `install_frame_homes`
passes the scalar to a struct writer, which rejects it before the method runs. The
existing resolver instead uses an enum's underlying scalar for type layouts.

A temporary exploratory build used the enum's scalar layout and normalized named
primitives before choosing a home layout. The same cached EF fixture completed in
Criterion test mode. This supports the diagnosis; it is not a validated repair. It did
not preserve declared enum identity on home pointers or test the cases below. No claim
is made about which earlier slice introduced the defect.

The affected paths include homed enum arguments on entry and homed enum locals or
arguments on assignment. Even when a value reaches a home as an enum `ValueType`,
the current struct representation disagrees with scalar enum operations. In
particular, scalar storage must support both the primitive stack representation and
an enum `ValueType` produced by generic code. A scalar home also needs its declared
identity independently of its byte layout: `home_address` otherwise derives `Int32`
from an Int32 load, while virtual dispatch and `Object.GetType` inspect a managed
pointer's `inner_type`. The failing constrained call supplies its own enum type, so
its success alone does not establish that pointer identity is correct.

Named `System` primitive types are another risk: `variable_home` branches on the
un-normalized concrete type, whereas the resolver normalizes named primitives to
scalar layouts. Generic local initialization can create a `ValueType` for a named
value type. These observations identify a plausible related mismatch; the EF trace
above establishes the enum case only.

## Implementation

1. In `frame_homes::variable_home`, resolve the declared concrete type and its
   descriptor before normalizing it. Give enums the resolver's underlying scalar
   layout, and give named primitive types their normalized scalar layout. Keep
   ordinary structs, including reference-bearing and byref-like structs, on their
   existing field-layout path; leave `TypedReference` and byref policies intact.
2. Store declared value-type identity separately from `HomeValueType`'s struct
   reconstruction metadata. Use the enum or named primitive descriptor when creating
   its home address; do not infer that descriptor from the widened stack value.
   Keep current behavior for reference and byref homes. Preserve frame-home serial,
   bounds, alignment, tracing, and the 24-byte managed-pointer encoding.
3. Make home writes use the full `VariableHome`, not only its layout. For an enum
   scalar home, accept the matching scalar and matching enum `ValueType`, converting
   the latter with the existing enum-to-underlying conversion. For a named primitive
   scalar, accept its scalar stack value and, when presented with a matching primitive
   `ValueType`, decode its bytes with the resolved scalar layout after verifying type
   identity and storage size. Apply existing implicit numeric conversions and
   truncation; reject mismatched values. Scalar home reads continue to use the
   layout-driven reader, which sign- or zero-extends narrow values. This same path
   must handle initial argument copies, `starg`, and `stloc`.
4. Avoid changing general struct writes or all value-type layouts: the runtime has
   special pointer representations such as `System.ByReference`, and this diagnosis
   does not establish their frame-home behavior. No unchecked pointer origin or heap
   storage of a home pointer is needed.

## Regression and validation

- Add a small managed or generated-IL test that passes an Int32-backed enum by value
  to a method which takes its argument's address and performs a constrained call;
  assert the returned value/text, and show it fails on the candidate before the fix.
- Exercise address-taken enum locals and arguments, including initial copies,
  `ldloc`/`stloc` and `ldarg`/`starg`, default initialization, mutation through a
  saved address after reassignment and nested stack growth, and `ldobj`/`stobj` or
  `initobj`. Cover signed and unsigned 8- and 16-bit extension, 32- and 64-bit bit
  patterns, and a generic enum `ValueType` input as well as a scalar input.
- Check declared enum pointer identity through a type-observing or virtual-dispatch
  path, and cover named primitives and generic substitutions. Include ordinary
  structs and reference-containing homes as controls, including collection, so the
  change does not degrade field copying or tracing. Compare guest outputs with CLR.
- Run focused tests in both default and no-default-features configurations, then the
  same EF test with the identical baseline fixture and the full `./check.sh --fast`.
  Run the relevant pinned Tree Borrows pointer Miri controls in both configurations,
  adding any new pointer control to the CI manifest. Record actual results; a passing
  EF smoke test does not substitute for these checks.
- Once correctness passes, resume plan 02 slice 5's full benchmark matrix and its
  address-free regression review. Do not report performance from the diagnostic build.

## Implementation record (2026-11-05)

Enum and named primitive homes now use resolver scalar layouts. `VariableHome`
retains a separate scalar type descriptor for address-taking; field-based struct
reconstruction is unchanged. Home stores check the identity and size of scalar
`ValueType` inputs and decode them within a shared byte borrow before using the
existing numeric store path. All three store sites use this path; no managed-pointer
encoding, unchecked origin, or heap-container behavior changed.

`enum_frame_homes_42` covers constrained enum argument calls, declared enum identity,
saved references across reassignment, generic default/value copies, narrow signed and
unsigned enums, 32/64-bit patterns, primitive generic substitutions and a struct
containing an object reference across collection. It passes in the VM in both
configurations and returns 42 under CLR. The new p43 control covers scalar argument
initialization and local/argument stores with native-address access; it is registered
as singleton shard 43 and in batch 2, including the hosted CI matrix.

The original EF benchmark in Criterion test mode passes with the identical baseline
fixture cache in both configurations. No performance measurement is claimed.
Local repair validation logs are under `target/plan01-slice5/logs/repair-*`.

### Validation results

- `./check.sh --fast` passed clippy and tests across all three configurations:
  614 passed / 6 skipped without features, 734 / 5 with multithreading,
  749 / 7 with multithreading and validation-all. The build used
  `CARGO_TARGET_DIR=target/plan01-slice5/candidate-build` and
  `DOTNET_FIXTURES_BASE=/home/nick/Desktop/dotnet-rs/target/debug/dotnet-fixtures`
  so fixture generation and consumer validation shared the same cache.
- The expanded final p43 additionally exercises an enum `ValueType` argument from
  an unhomed local, as well as the scalar argument case. Its focused native test passes.
  Scalar `ValueType` decoding checks both the layout size and actual byte-buffer length.
- The expanded p43 passed pinned Tree Borrows Miri in both configurations, including
  the enum `ValueType` storage-read path.
- Pinned Tree Borrows Miri p37 and p38 passed in both configurations, preserving
  the existing struct-home assignment controls.
- Pointer-runner tests passed (21 tests); the batch drift guard confirms 43 controls
  occur exactly once in four batches. Formatting and whitespace checks passed.

Plan 02 subsequently completed its benchmark matrix and accepted the documented
primitive cost; see its performance closure. This repair itself claims no timings.
