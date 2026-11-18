# Repair enum and named scalar frame homes

**Status:** Complete
**Outcome:** Address-taken enum and named primitive locals and arguments retain scalar storage
semantics and declared identity across assignment and stack growth.

## What was fixed

Frame-home selection treated enum arguments, which the evaluation stack holds as scalars, as
struct homes and rejected argument installation (seen in EF Core's
`CoreEventId.MakeUpdateId`). Selection now resolves declared identity before normalization:
enums and named primitives use resolver scalar layouts and keep their declared type for
address-taking; ordinary structs keep field-based reconstruction. Argument installation, `starg`
and `stloc` share one scalar-home conversion, and value-type decoding checks type, layout size and
buffer length before reading. The 24-byte managed-pointer encoding is unchanged.

The same work fixed catch entry: the logical stack height was reset while backing operands
remained, so catch entry now truncates them before pushing the exception (ECMA-335 §I.12.4.2.5).

Specification basis: §I.12.1.2 (extension on narrow loads, truncation on stores), §II.14.3
(enum/underlying interchange preserving identity), §III.2.1 (constrained calls), §III.3.44
(managed local addresses). Acceptance covers the eight C# integer underlying types.

## Checks

- `enum_frame_homes_42` is a CLR differential fixture: constrained enum calls, declared identity,
  saved references, generic defaults and copies, narrow signed/unsigned values and a
  reference-containing struct across collection.
- p43 (`p43_enum_home_scalar_roundtrip`) exercises scalar and enum argument homes across
  `ldarga`/`starg`, `ldloca`/`stloc`, nested stack growth and `ldobj`/`stobj`; p37/p38 remain the
  struct-home controls.

```bash
cargo test -p dotnet-cli --test integration_tests fields_enum_frame_homes_42_matches_dotnet
cargo test -p dotnet-vm --lib scalar_home_rejects            # and --no-default-features
bash scripts/pointer_provenance.sh controls --configuration CONFIG --shard 43   # p37=35, p38=37
```

For the EF smoke test, build `end_to_end` with
`cargo bench --profile bench-fat -p dotnet-benchmarks --bench end_to_end --no-run` and run its
executable with `ef_inmemory --test`.
