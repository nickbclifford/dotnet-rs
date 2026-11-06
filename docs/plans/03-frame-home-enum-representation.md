# Repair enum and named scalar frame homes

**Status:** Complete

**Outcome:** Address-taken enum and named primitive locals and arguments retain
scalar storage semantics and declared identity across assignment and stack growth.

## Problem and implementation

The pre-repair runtime selected struct homes for enum arguments represented on the
evaluation stack as scalars. It rejected argument installation before executing a
constrained enum call, including EF Core's `CoreEventId.MakeUpdateId`.

Frame-home selection now resolves declared identity before normalization. Enums and
named primitives use resolver scalar layouts and retain separate declared type
metadata for address-taking. Ordinary structs retain field-based reconstruction;
reference, byref, TypedReference, and special pointer policies are unchanged.

Argument installation, `starg`, and `stloc` share scalar-home conversion. Matching
scalar or matching value-type inputs are accepted; value-type decoding checks type,
layout size, and actual borrowed buffer length before reading. Numeric stores retain
existing conversion/truncation and layout-driven reads retain extension behavior.
No public API or 24-byte managed-pointer encoding change is required.

## Specification and scope

- ECMA-335 I.12.1.2 requires signed/unsigned extension on narrow loads and truncation
  on stores and argument assignment.
- II.14.3 permits enum/underlying scalar interchange while preserving identity for
  binding and boxing. This repair's acceptance covers the eight C# integer underlying
  types; additional CLI enum underlying types are outside this repair.
- III.2.1 defines the managed receiver and boxing behavior of constrained calls;
  III.3.44 defines aligned managed local addresses.
- II.14.3 prohibits direct enum `initobj`. Generic `Default<T>` instantiated with an
  enum is tested against CLR as compatibility evidence, not a direct-enum initialization
  requirement.

## Regression and acceptance

`enum_frame_homes_42` has an automated CLR oracle covering constrained enum argument
calls, declared identity, saved references, generic defaults/copies, narrow signed and
unsigned values, 32/64-bit patterns, primitive substitutions, and a reference-containing
struct across collection.

p43 explicitly exercises initial scalar and enum `ValueType` argument copies,
`ldarga`/`starg`, `ldloca`/`stloc`, saved local and argument addresses across observed
nested evaluation-stack growth, stable home addresses/serials, native access, and
`ldobj`/`stobj`. Both invocation results are independently checked. Existing p37/p38
remain struct-home assignment controls. Scalar rejection tests cover wrong identity,
wrong layout size, and shorter/longer payloads without changing destination bytes.

The [completion follow-up](../ci-evidence/2026-11-05-plan03-completion-follow-up.md)
contains actual generated-IL mapping, fresh historical reproduction, identical-cache
EF hashes/results, and validation commands/results. Earlier audit findings remain
in their historical records.

Focused/default/no-feature tests, pinned Tree Borrows p37/p38/p43 and scalar
rejection tests in both configurations, runner/batch/doc checks, and the full
`./check.sh --fast` passed. The enabled Substring delegate regression exposed a catch-entry
stack cleanup defect during that gate; the minimal exception repair is included and
all three fast configurations now pass.

Plan02's completed benchmark matrix and accepted primitive cost satisfy the separate
performance dependency. EF test-mode smoke results here claim no performance timings.
