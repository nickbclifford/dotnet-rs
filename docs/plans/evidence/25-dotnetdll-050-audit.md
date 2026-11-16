# dotnetdll 0.5.0 audit for DynamicMethod

Investigated against the published `dotnetdll` 0.5.0 crate and .NET 10.0.14. The checks below are
reproductions, not a comprehensive audit. The VM registers an in-memory `Resolution`, so not every
serialized-PE bug here blocks the current dynamic-method path; they are included so one upstream
release can fix the related resolved writer and reader issues together.

## Reproduce

From the dotnet-rs repository root, make a temporary standalone Cargo project:

```sh
audit_dir=$(mktemp -d)
mkdir -p "$audit_dir/src"
cp docs/plans/evidence/25-dotnetdll-050-repro.rs "$audit_dir/src/main.rs"
cat > "$audit_dir/Cargo.toml" <<'TOML'
[package]
name = "dotnetdll-audit"
version = "0.1.0"
edition = "2024"
[dependencies]
dotnetdll = "=0.5.0"
scroll = "0.13"
TOML
cargo run --manifest-path "$audit_dir/Cargo.toml" --offline --quiet
```

Omit `--offline` if the dependencies are not cached. The program prints the observed 0.5.0
results; it catches the two reproduced reader panics so the rest of the audit can run. The
[static constrained-tail probe](../../../crates/dotnet-cli/tests/fixtures/reflection/dynamic_method_static_constrained_tail_42.cs)
returns 42 on .NET but the VM rejects the prefix combination. Two other .NET controls,
[21 catch handlers](25-dotnetdll-050-many-handlers.cs) and
[mixed handler lengths](25-dotnetdll-050-mixed-handlers.cs), both return 42 on .NET 10.0.14.

## Confirmed gaps

1. **Static constrained tail call cannot be represented.** `Instruction::CallConstrained` has no
   tail flag or companion variant. The binary prefix declarations compose `constrained.` and
   `tail.` only for `callvirt`. The .NET probe uses `constrained. Value; tail.; call IValue.Get`,
   where `IValue.Get` is a static abstract interface method. Add a resolved representation and
   binary read/write support for this combination. Preserve `CallConstrained` for existing users
   if practical; adding any enum variant can still break exhaustive matches. Test resolved and
   binary round trips, branch targets across the instruction, and execution on .NET.

2. **Unmanaged C vararg standalone signatures are truncated.** With Cdecl, one fixed `int`, one
   optional `int`, and `void` return, the writer emits `01 02 01 08` instead of
   `01 02 01 08 41 08`: it counts the optional parameter but writes the sentinel and optional
   types only for the managed `Vararg` convention. The reader accepts the complete encoding. This
   is permitted by [ECMA-335](https://ecma-international.org/publications-and-standards/standards/ecma-335/)
   II.23.2.3. Test Cdecl and managed vararg with zero and nonzero optional arguments, and reject
   optional tails for conventions that cannot encode them rather than silently dropping them.
   A truncated input also panics in `build_params_with_varargs`, which indexes beyond the buffer;
   it should return a parse error.

3. **An empty managed vararg method reference gets a stray sentinel.** `MethodRefSig` writes
   `05 01 01 08 41` for `vararg void(int)` with no optional arguments; the expected bytes are
   `05 01 01 08`. `ref_sig_impl` emits the sentinel for every `Vararg` reference. ECMA-335
   II.23.2.2 excludes a sentinel when no extra arguments are supplied. Retain the sentinel when
   there are optional arguments.

4. **Nested function-pointer signatures lose varargs when read.** A method parameter whose type
   is `FunctionPointer` with a managed vararg signature and one optional `int` writes successfully,
   but the parsed function pointer has `varargs: None`. `convert::read::maybe_unmanaged_method`
   discards the tail; the `calli` instruction reader has a separate reconstruction path. Test
   nested function-pointer signatures independently of `calli` for managed and C varargs.

5. **Branch relocation can write invalid targets.** `resolution/write.rs` predicts which branches
   shrink by testing whether the *target instruction index* fits in `i8`, rather than the resulting
   signed byte displacement. The audit demonstrates both a far target with a small index and a
   nearby backward target with a large index; both written assemblies fail the reader with
   `invalid instruction branch target offset`. Compute sizes and byte offsets consistently; always
   emitting long branches is a correct baseline if short-branch relaxation is deferred. Cover
   forward/backward distances around -128 and 127, interacting branches, switches, and exception
   boundaries.

6. **Exception-section boundaries and encoding have three defects.**
   - When a handler ends at the method's end after a multi-byte final instruction (the probe uses
     `rethrow`), `resolution/lazy.rs` compares the endpoint with the last instruction's *start + 1*
     rather than the method's byte length. The reader rejects the writer's output.
   - A small EH section with 21 clauses needs 256 bytes. `binary/method.rs` chooses the small
     format based only on individual clause sizes and offsets, truncates the section length to
     zero, and the reader panics. Select fat format when the whole section length does not fit.
   - In a fat section containing both small and large clauses, each clause is encoded using its
     own size heuristic. The small clause is therefore written in a compact form inside a fat
     section; the reader reports invalid clause flags. Every clause in a fat section must use the
     fat format. Test both cases, including filters and faults and handler ends at code size.

7. **`callvirt` loses its `no.` null-check flag.** Writing
   `CallVirtual { skip_null_check: true, ... }` uses `check_mask(false, true, false)` (0x02,
   range-check) instead of the null-check bit 0x04. Reading the result yields
   `skip_null_check: false`. This is an upstream round-trip bug; .NET 10's public `OpCodes` API
   does not expose `no.`, so it is not evidence that an ordinary `ILGenerator` caller needs it.

## Boundaries and controls

- The 0.5.0 virtual constrained-tail case and ordinary filter/fault clauses round-trip in the
  tested shapes. `unaligned.` + `volatile.` has explicit representations for both orders.
- `SignatureHelper` construction, unmanaged `calli` execution, and the remaining opcode work in
  dotnet-rs are separate VM/support-library work; these findings do not establish that dotnetdll
  must implement those APIs.
- The probes do not establish which other multi-prefix combinations are accepted by the CLR.
  Avoid claiming complete multi-prefix coverage without separate tests.
