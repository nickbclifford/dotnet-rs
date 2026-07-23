# Compatibility Notes

`dotnet-rs` runs framework-dependent applications by reading their
`runtimeconfig.json` and `deps.json` files. The explicit `--assemblies` path
remains available for direct assembly-root selection.

The host path has been validated at three levels:

- the repository's managed fixture suite;
- a Newtonsoft.Json application with matching output and exit status under
  stock `dotnet` and `dotnet-rs`;
- an EF Core InMemory smoke application that creates, saves, and queries an
  entity with matching output and exit status under both runtimes.

The EF Core work exercised generic method context propagation, reflection and
nullability metadata, by-reference value-type marshalling, and method identity
during query translation. Focused regression fixtures for those runtime fixes
live under `crates/dotnet-cli/tests/fixtures/reflection/`.

This is compatibility evidence, not a claim of general EF Core or .NET runtime
compatibility. EF Core SQLite, broad framework coverage, Reflection.Emit,
networking, and complete asynchronous scheduling remain outside the validated
surface.
