# Investigate Reflection.Emit and `DynamicMethod`

**Status:** Not started
**Outcome:** This plan records a decision, backed by a working spike or a measured blocker:
either a follow-up plan to implement `DynamicMethod` (and, separately, whether to do
`AssemblyBuilder`/`TypeBuilder`), or a single switch-level answer that replaces the per-call-site
patches. The `RuntimeType.GetNativeTypeHandle` access violation is either fixed or reproduced
with its cause recorded.

Closes these README "Not yet planned" entries once the decision is made: **Dynamic code without
Reflection.Emit** (replaced by the follow-up plan, or closed by the switch-level answer) and
**Access violation in `RuntimeType.GetNativeTypeHandle`**.

## Problem

- The VM reports `RuntimeFeature.IsDynamicCodeSupported` true (EF Core treats false as NativeAOT)
  and `IsDynamicCodeCompiled` false (`crates/dotnet-vm/src/intrinsics/app_context.rs`, plan 18
  slice 7). Code that checks only `IsDynamicCodeSupported` before emitting IL fails.
- Two such paths are patched per call site in `crates/dotnet-vm/src/stack/call_ops_impl.rs`:
  `try_redirect_expression_compile_to_interpreter` (redirects `LambdaExpression.Compile()` to the
  `preferInterpretation` overload) and `try_no_body_runtime_stub` (stubs
  `DelegateHelpers.<CreateObjectArrayDelegateRefEmit>g__ForceAllowDynamicCode`). Each new library
  that emits IL would need another patch.
- When Newtonsoft used Reflection.Emit, `DynamicMethod.Init` → `get_IsActualInterface` →
  `RuntimeType.GetNativeTypeHandle` raised an `AccessViolationException`. No rung reaches it now
  ([evidence](evidence/18-array-casts-host-rungs.txt)), but managed code should never produce
  one.
- The VM's method model may not fit runtime-created methods. `MethodDescription`
  (`crates/dotnet-types/src/members.rs`) is a (`ResolutionS`, `MethodMemberIndex`) pair into
  loaded dotnetdll metadata, and bodies are decoded by dotnetdll with tokens resolved against that
  module's tables. A `DynamicMethod` body's tokens instead index into the `DynamicResolver`'s
  object list (`DynamicILGenerator` / `DynamicScope`), and it has no owning metadata row.

## Questions to answer

1. **Demand.** Which real code paths reach Emit today or would with the patches removed? Check
   Newtonsoft, EF Core InMemory, Microsoft.Extensions.DependencyInjection, System.Text.Json,
   `System.Linq.Expressions` and EF Core SQLite (the next candidate rung). For each, record:
   whether it checks `IsDynamicCodeSupported` or `IsDynamicCodeCompiled`, and whether a
   non-Emit fallback exists.
2. **Switch-level alternative.** Is there one AppContext switch, feature value or CoreLib
   substitution (for example making `DynamicMethod`'s constructor throw `PlatformNotSupportedException`
   the way NativeAOT's CoreLib does, which libraries catch) that makes every path in (1) take its
   fallback without breaking EF's NativeAOT check? Test it by removing both call-site patches.
3. **`DynamicMethod` feasibility.** What is the smallest VM change that runs a `DynamicMethod`
   body?
   - **(a) Token-resolver body source:** a new `MethodDescription` variant or side table keyed by
     a VM-allocated handle, with a body decoder that resolves tokens through the managed
     `DynamicResolver` (the CLR's `DynamicResolver.ResolveToken`/`GetCodeInfo` contract).
   - **(b) In-memory assembly:** build a one-method module with dotnetdll's writer, rewriting
     tokens to MemberRefs into the referenced assemblies, and load it as an ordinary `ResolutionS`.
   For each option, estimate the touched modules (resolver, type caches, call dispatch, stack
   traces, exception handling regions, `RuntimeMethodHandle` and reflection objects, GC of
   collectible methods) and note whether the existing intrinsic dispatch, effects and frame homes
   work unchanged.
4. **`AssemblyBuilder`/`TypeBuilder`.** Does any rung in (1) need full type emission, or only
   `DynamicMethod`? Size it only as far as needed to decide whether it belongs in the follow-up.
5. **Access violation.** Reproduce `DynamicMethod.Init` → `get_IsActualInterface` → `GetNativeTypeHandle`
   with a direct fixture (`new DynamicMethod("m", typeof(int), Type.EmptyTypes)`). Find which
   handle or pointer is invalid. Fix it if the fix is local to type handles (it may affect other
   reflection paths), otherwise record the cause.

## Scope

In: answering the five questions, one spike, the AV repro, and the decision.

Out: a production `DynamicMethod` implementation. The spike may stay on a branch, and only the
fixtures, the AV fix and the decision record land on main. Changing either feature value without
rerunning the host rungs is also out, because the values were chosen on rung evidence.

## Slices

1. **Demand survey.** Answer question 1, with evidence (call stacks or source references) per
   library. Note it here.
2. **AV repro.** Add the `DynamicMethod` constructor fixture (expected to differ from `dotnet`
   for now; keep it in `debug_fixtures` until it passes), then find the cause and fix it or record
   it.
3. **Switch-level experiment.** Answer question 2 on a branch with both patches removed. Rerun
   Newtonsoft, EF InMemory and the expression fixtures, and record the results.
4. **`DynamicMethod` spike.** Pick option (a) or (b) from question 3 and get a fixture that emits
   `ldarg.0; ldarg.1; add; ret` and calls it through both `Invoke` and `CreateDelegate`. Then add
   one that calls an existing method by token and one that throws and catches. Record what broke
   and how much code each step needed.
5. **Decision.** Write the conclusion in this plan, answering question 4 in it. Then either
   draft the follow-up implementation plan under the next free ID, or land the switch-level answer
   with the call-site patches removed. Update the README entries and `docs/COMPATIBILITY.md`.

## Done when

- Questions 1–5 each have a recorded answer with evidence in this plan.
- The `DynamicMethod` constructor no longer raises `AccessViolationException`, or the cause is
  recorded with a reason it can't be fixed locally.
- Either a follow-up plan exists and the README points to it, or the switch-level answer is on
  main, `call_ops_impl.rs` has no Emit call-site patches, and the host rungs still match `dotnet`.

## Ask the maintainer only if

- The demand survey finds no real-application path that needs Emit once the switch-level answer
  works. In that case confirm before dropping `DynamicMethod` entirely.
- The spike needs a dotnetdll change (a writer or body-decoder API) that must go upstream.
- Option (a) needs a new `MethodDescription` representation that touches every resolver cache.
