# CLR call-site signature extensions and dotnetdll 0.5.1

These are CLR-accepted `SignatureHelper` inputs, not established requirements of a surveyed
library. ECMA-335 II.23.2.3 and II.23.2.10 do not include a `pinned` constraint in a method
parameter; II.23.2.3 does not include a sentinel after the final parameter. The recorder retains
these bytes for `GetSignature()` and normalizes the call-site signature for execution. Exact
inspection of the synthetic method's internal metadata is outside plan 25.

## Pinned method argument

The [fixture](../../../crates/dotnet-cli/tests/fixtures/reflection/dynamic_method_pinned_signature_42.cs)
constructs a standard method signature returning `int`, adds an `int` argument with
`SignatureHelper.AddArgument(typeof(int), pinned: true)`, prints its signature, emits it as a
`calli` operand, and calls a managed method. .NET 10.0.14 prints:

```text
0001084508
42
```

In dotnetdll 0.5.1, `StandAloneMethodSig` parsing of `[00, 01, 08, 45, 08]` returns
`Parse error: invalid element type tag 0x45`; its resolved `Parameter` also has no pinned flag.
If exact representation of this CLR extension is required, the binary and resolved models,
reader, and writer need an extension. On .NET 10.0.14, the supplementary
[compacting-GC probe](25-signature-pinned-gc-probe.cs) observed the array argument move in all
10 calls both with and without a pinned parameter marker. That is evidence for normalization in
the tested case, not proof of every runtime and signature shape.

## Trailing sentinels

The [edge fixture](../../../crates/dotnet-cli/tests/fixtures/reflection/dynamic_method_signature_edge_42.cs)
shows .NET 10.0.14 accepts either one or two trailing sentinels on a standard zero-argument
signature, and the resulting `calli` returns 42:

```text
trailing:00000841:42
twice:0000084141:42
```

The resolved signature model cannot retain the number of trailing sentinel bytes. The recorder
keeps them in `GetSignature()` but omits them when building a standard managed call signature.
The output demonstrates CLR acceptance, not a need to preserve invalid internal metadata.
