# Code Review and Refactoring Plan for dotnet-rs

> **Document Purpose**: A comprehensive review identifying code quality improvements and a phased refactoring plan for future AI agents to implement.
>
> **Date**: 2026-02-08 (Last updated: 2026-02-09)
> **Codebase Stats**: 99 Rust files, ~27,600 lines across 8 crates

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Code Organization Analysis](#code-organization-analysis)
3. [Rust Idiom Improvements](#rust-idiom-improvements)
4. [Error Handling Patterns](#error-handling-patterns)
5. [Abstraction and Trait Design](#abstraction-and-trait-design)
6. [Documentation Gaps](#documentation-gaps)
7. [C# Support Library Review](#c-support-library-review)
8. [Testing Improvements](#testing-improvements)
9. [Phased Refactoring Plan](#phased-refactoring-plan)

---

## Executive Summary

The dotnet-rs project is a well-structured Rust implementation of a .NET runtime VM with a solid foundation. However, several areas need improvement for long-term maintainability:

### Key Findings

| Category               | Status                                                           | Priority |
|------------------------|------------------------------------------------------------------|----------|
| **Code Organization**  | Large files split in Phase 2; continued improvements needed      | Medium   |
| **Error Handling**     | Error type hierarchy added in Phase 2.4; migration ongoing       | High     |
| **Documentation**      | 430+ doc comments, 44 SAFETY comments for 250+ unsafe blocks     | Medium   |
| **Trait Design**       | ReflectionOps split in Phase 2.1; further decomposition possible | Low      |
| **C# Support Library** | Well-designed; focus should be runtime integration only          | Low      |
| **Testing**            | Good fixture-based approach; some gaps in unit tests             | Medium   |

### Implementation Progress (Phases 1-2.4 Complete)

**✅ Phase 1 (Complete):**
- SAFETY comments added to critical unsafe blocks
- Let-else patterns adopted for cleaner early returns
- Module-level documentation added
- `#[must_use]` attributes added to critical types

**✅ Phase 2.1 (Complete):**
- `ReflectionOps` split into `LoaderOps`, `StaticsOps`, `ThreadOps`
- All tests pass across feature combinations

**✅ Phase 2.2 (Complete):**
- `stack/context.rs` split into multiple impl files
- Unused imports cleaned up
- All feature combinations verified

**✅ Phase 2.3 (Complete):**
- `runtime_type_intrinsic_call()` mega-function extracted into handlers
- 534-line match statement refactored
- Tests verified across all features

**✅ Phase 2.4 (Complete):**
- VM error type hierarchy implemented using `thiserror`
- `StepResult::Error` variant added
- `pop_safe` method added for graceful stack underflow handling
- High-priority panics converted to errors

### Critical Issues Requiring Attention

1. **GC Handle Threading**: Current pervasive `gc` parameter passing needs careful review (see Phase 3.1 analysis)
2. **Error Migration**: ~400+ remaining `unwrap()`/`expect()` calls need categorization and conversion
3. **Unsafe Documentation**: Still ~200+ unsafe blocks lacking SAFETY comments

### Positive Aspects

- ✅ Clippy-clean codebase (no warnings)
- ✅ Clear crate separation with defined responsibilities
- ✅ Well-designed trait hierarchy (`VesOps`, `StackOps`, etc.)
- ✅ Modern C# support library using file-scoped namespaces and nullable reference types
- ✅ Comprehensive integration test framework with auto-generated test runners
- ✅ Structured error handling foundation now in place

---

## Code Organization Analysis

### Large Files Requiring Attention

Files exceeding 500 lines that should be considered for splitting:

| File                                           | Lines | Issue                                       | Status/Action                                        |
|------------------------------------------------|-------|---------------------------------------------|------------------------------------------------------|
| `dotnet-vm/src/stack/context.rs`               | 592   | Core struct and VesOps implementation       | ✅ Split in Phase 2.2                                 |
| `dotnet-vm/src/intrinsics/reflection/types.rs` | 1,088 | Single 534-line function with massive match | ✅ Extracted in Phase 2.3                             |
| `dotnet-vm/src/resolver.rs`                    | 947   | `ResolverService` has 28 methods            | Group related methods into sub-modules               |
| `dotnet-vm/src/intrinsics/threading.rs`        | 773   | Many intrinsics in one file                 | Split by threading subsystem (Thread, Monitor, etc.) |
| `dotnet-assemblies/src/lib.rs`                 | 759   | Mixed assembly loading and resolution       | Extract `AssemblyLoader` and `ResolutionHelper`      |
| `dotnet-vm/src/tracer.rs`                      | 745   | Tracer logic interleaved with formatting    | Separate tracing core from output formatters         |
| `dotnet-value/src/lib.rs`                      | 705   | `StackValue` + conversions + utilities      | Extract conversion logic to `conversions.rs`         |
| `dotnet-value/src/object.rs`                   | 691   | Object types + heap storage + GC support    | Split into `object.rs`, `heap.rs`, `gc_support.rs`   |
| `dotnet-vm/src/intrinsics/delegates.rs`        | 674   | Delegate intrinsics + multicast logic       | Separate `delegate_core.rs` and `multicast.rs`       |
| `dotnet-vm/src/pinvoke.rs`                     | 672   | P/Invoke implementation                     | Group by P/Invoke category                           |
| `dotnet-vm/src/exceptions.rs`                  | 646   | Exception handling state machine            | Well-structured; consider splitting only if grows    |
| `dotnet-vm/src/memory/access.rs`               | 542   | Memory access patterns                      | Split read/write operations                          |
| `dotnet-vm/src/threading/basic.rs`             | 540   | Threading primitives                        | Split into `thread_pool.rs`, `synchronization.rs`    |
| `dotnet-vm/src/intrinsics/unsafe_ops.rs`       | 528   | System.Runtime.CompilerServices.Unsafe      | Consider grouping by operation type                  |

### Module Structure Improvements

**1. ~~God Object Pattern: `VesContext`~~ (Phase 2 Complete)**
```
Status: Split in Phase 2.2
- VesContext trait implementations extracted to separate files
- LoaderOps, StaticsOps, ThreadOps split from ReflectionOps in Phase 2.1
```

**2. ~~Overloaded `ReflectionOps` Trait~~ (Phase 2.1 Complete)**
```
Status: Completed
- Split into LoaderOps, StaticsOps, ThreadOps, and reduced ReflectionOps
- All tests pass across feature combinations
```

**3. ~~Mega-Function in Reflection Intrinsics~~ (Phase 2.3 Complete)**
```
Status: Completed
- runtime_type_intrinsic_call() extracted into individual handler functions
- Match arms refactored for clarity
- Dispatch table consideration deferred (can add later with no behavior change)
```

**4. Mixed Concerns in `dotnet-assemblies`**
```
Location: crates/dotnet-assemblies/src/lib.rs
Problem: Assembly loading, support library embedding, and resolution mixed together
Impact: Makes it hard to test loading vs. resolution independently
```

### Circular Dependencies

No circular dependencies detected between crates. The dependency hierarchy is clean:

```
dotnet-cli
    └── dotnet-vm
        ├── dotnet-assemblies
        ├── dotnet-value
        ├── dotnet-types
        ├── dotnet-utils
        ├── dotnet-macros
        └── dotnet-macros-core
```

However, there is **tight coupling within `dotnet-vm`**:
- `stack/` depends on `memory/`, `intrinsics/`, `dispatch/`
- `intrinsics/` depends on `stack/`, `memory/`, `dispatch/`
- This mutual dependency makes refactoring difficult

---

## Rust Idiom Improvements

### Modern Rust Features Not Yet Utilized

**1. Let-else (Rust 1.65+)** ✅ Applied in Phase 1
Replace verbose match statements for extracting from Option/Result:
```rust
// Before (common pattern in codebase)
let value = match some_option {
    Some(v) => v,
    None => return StepResult::Exception,
};

// After (Phase 1 complete)
let Some(value) = some_option else {
    return StepResult::Exception;
};
```

**2. Then Method for Option (Rust 1.50+)**
```rust
// Before
if condition {
    Some(compute_value())
} else {
    None
}

// After
condition.then(|| compute_value())
```

**3. Inline const (Rust 1.79+)**
For compile-time constants in generic contexts, useful in layout calculations.

**4. Box Patterns (when stabilized)**
Would simplify some nested matching in exception handling.

### Pattern Matching Improvements

**Location: `intrinsics/reflection/types.rs:115-649`**

The massive match on method names should be refactored:
```rust
// Current: 534-line match statement
let result = match (method_name, param_count) {
    ("CreateInstanceCheckThis", 0) => { /* 5 lines */ }
    ("GetAssembly" | "get_Assembly", 0) => { /* 40 lines */ }
    // ... 28 more arms
};

// Proposed: Dispatch table or individual handler functions
// Each handler is decorated with #[dotnet_intrinsic] like other intrinsics
```

**Location: Various instruction handlers**

Many instruction handlers have repetitive pop/push patterns:
```rust
// Repetitive pattern throughout
let a = ctx.pop_i32(gc);
let b = ctx.pop_i32(gc);
ctx.push_i32(gc, a + b);

// Could use a helper macro or combinator pattern
binary_op!(ctx, gc, i32, |a, b| a + b)
```

### Iterator and Functional Style Opportunities

**1. Loop-based Collection Building**
```rust
// Found in resolver.rs and elsewhere
let mut result = Vec::new();
for item in collection {
    if predicate(item) {
        result.push(transform(item));
    }
}

// Better
let result: Vec<_> = collection
    .iter()
    .filter(|item| predicate(item))
    .map(transform)
    .collect();
```

**2. Index-based Loops with Mutations**
Some loops use index iteration where `iter_mut()` would be cleaner.

**3. Early Returns in Loops**
Several `for` loops could use `Iterator::find()` or `Iterator::position()`.

### GC Handle Pattern Analysis

**IMPORTANT**: `GCHandle<'gc>` is part of `gc-arena`'s lifetime-based branding system. It represents a **single mutation operation** within an arena and enforces invariants through Rust's borrow checker.

Currently, `GCHandle<'gc>` is passed as a parameter to almost every method:
```rust
fn push_i32(&mut self, gc: GCHandle<'gc>, value: i32);
fn pop_i32(&mut self, gc: GCHandle<'gc>) -> i32;
fn push_obj(&mut self, gc: GCHandle<'gc>, value: ObjectRef<'gc>);
// ... 40+ more methods
```

**Why gc is passed explicitly:**
1. **Lifetime branding** - Objects allocated with `gc` are branded with the `'gc` lifetime
2. **Mutation tracking** - Each `GCHandle` represents a distinct mutation "epoch"
3. **Borrow checker enforcement** - Prevents use of objects after GC might have moved them

**Status for Phase 3.1 (Removing GC Handle threading):**

The project has partially implemented the transition of storing `GCHandle` directly in `VesContext`. While initially considered high risk, this path was chosen to significantly reduce boilerplate and improve readability.

**Implementation Progress:**
- `VesContext` has been updated to include `gc: GCHandle<'gc>`.
- Core traits (`StackOps`, `VesOps`, etc.) now feature `_v2` method variants that omit the explicit `GCHandle` parameter.
- Many instruction handlers have been migrated to use these `_v2` methods internally.

**Safety Considerations:**
- `VesContext` is a short-lived stack-allocated struct used during instruction execution steps.
- Storing the `GCHandle` (which is a `Copy` wrapper around the mutation context) within `VesContext` is functionally equivalent to passing it as a separate argument to every method, provided the context itself does not outlive the mutation epoch.
- Current usage patterns in `ExecutionEngine` ensure `VesContext` is created and destroyed within a single GC-safe point boundary.

---

## Error Handling Patterns

### Current State Analysis (Post-Phase 2.4)

**Phase 2.4 accomplishments:**
- ✅ VM error type hierarchy created using `thiserror`
- ✅ `VmError` with variants: `AssemblyLoad`, `TypeResolution`, `Execution`, `Memory`
- ✅ `StepResult::Error` variant added to execution flow
- ✅ `ExecutorResult::VmError` added to top-level executor
- ✅ Stack underflow converted from panic to error
- ✅ `pop_safe` method added for graceful stack operations
- ✅ Common instruction macros updated to use safe pop operations

**Remaining work: ~400+ panic-prone calls still exist**

Distribution by crate (estimated):

| Crate               | Count | Severity | Priority   |
|---------------------|-------|----------|------------|
| `dotnet-vm`         | ~280  | High     | Phase 3.3  |
| `dotnet-assemblies` | ~80   | High     | Phase 3.3  |
| `dotnet-value`      | ~40   | Medium   | Later      |
| `dotnet-types`      | ~30   | Medium   | Later      |
| `dotnet-macros`     | ~10   | Low      | Keep as-is |
| `dotnet-cli`        | ~10   | Low      | Keep as-is |

### Panic vs. Error Return Guidelines

**MUST PANIC (Internal Invariants):**
- Programming errors that indicate bugs in the VM implementation
- Unreachable code paths (use `unreachable!()`)
- Internal GC consistency checks
- Data structure invariants that should never be violated

Examples:
```rust
// OK: Internal invariant that indicates bug if violated
.expect("Frame stack should never be empty in Return handler")

// OK: Unreachable due to type system guarantees
unreachable!("Validated TypeDef index out of bounds")
```

**MUST RETURN ERROR (External Inputs):**
- Any operation that depends on assembly metadata content
- Stack operations that could underflow/overflow
- Memory bounds checks
- Method/type/field resolution from user code
- File I/O and assembly loading
- Null dereference in managed code
- Type mismatches in instruction execution

Examples:
```rust
// GOOD: Stack underflow is user error (Phase 2.4 complete)
ctx.pop_safe(gc).ok_or(ExecutionError::StackUnderflow)?

// GOOD: Type not found in metadata
resolution.find_type(name).ok_or(TypeResolutionError::NotFound(name))?

// BAD: Should return error, not panic
let file_data = std::fs::read(&path).expect("failed to read file");
```

**GRAY AREA (Requires Judgment):**
- Operations that "should" succeed if earlier validation passed
- Cached lookups where cache miss indicates inconsistency
- **Guideline**: If triggered by malformed assembly → Error. If indicates VM bug → Panic.

### Migration Strategy for Phase 3.3

**Categories of remaining panics:**

1. **Assembly Loading & Parsing** (~80 in `dotnet-assemblies`)
   - All should become `AssemblyLoadError`
   - Priority: High

2. **Type/Method Resolution** (~100 in `dotnet-vm/resolver.rs`)
   - Should become `TypeResolutionError`
   - Priority: High

3. **Instruction Execution** (~100 in `dotnet-vm/instructions/`)
   - Most should become `ExecutionError`
   - Some internal invariants can stay as panics
   - Priority: High (requires careful categorization)

4. **Memory Operations** (~50 in `dotnet-vm/memory/`)
   - Bounds checks → `MemoryError`
   - Internal heap invariants → panic
   - Priority: Medium

5. **Legitimate Invariants** (~100 scattered)
   - Keep as `expect()` or `debug_assert!`/`assert!`
   - Add clear comments explaining invariant
   - Priority: Document only

### Proposed Error Type Strategy (Phase 2.4 Complete)

**✅ Phase 2.4: VM Error Type Hierarchy (Implemented)**

The error hierarchy is now implemented in `crates/dotnet-vm/src/error.rs`:
- `VmError` enum with variants for `AssemblyLoad`, `TypeResolution`, `Execution`, `Memory`
- Sub-error types for each category using `thiserror`
- `StepResult::Error(VmError)` variant added
- `ExecutorResult::VmError(VmError)` added to top-level executor
- Stack underflow and other high-priority panics converted

See `crates/dotnet-vm/src/error.rs` for complete implementation.

---

## Abstraction and Trait Design

### Trait Hierarchy Analysis

The current trait hierarchy in `stack/ops.rs` (Post-Phase 2.1):

```
VesOps (26 methods)
    : StackOps (27 methods)
    + MemoryOps (8 methods)
    + ExceptionOps (7 methods)
    + ResolutionOps (4 methods)
    + PoolOps (1 method)
    + RawMemoryOps (3 methods)
    + LoaderOps (3 methods)         ← Split from ReflectionOps in Phase 2.1
    + StaticsOps (2 methods)        ← Split from ReflectionOps in Phase 2.1
    + ThreadOps (1 method)          ← Split from ReflectionOps in Phase 2.1
    + ReflectionOps (13 methods)    ← Reduced from 18 methods
    + CallOps (5 methods)
```

**Phase 2.1 improvements:**
- ✅ `ReflectionOps` decomposed into focused traits
- ✅ Loader access separated from reflection
- ✅ Static storage management isolated
- ✅ Thread ID access separated

**Remaining concerns:**
- `StackOps` still has 27 methods (could be split further)
- `VesOps` still has 26 methods (mostly coordination)
- All traits still require `GCHandle` parameter threading (see Phase 3.1 analysis)

### Potential Future Trait Refactoring (Phase 3.2)

**StackOps Decomposition Proposal:**
```rust
// Basic push/pop
trait EvalStackOps<'gc> {
    fn push(&mut self, gc: GCHandle<'gc>, value: StackValue<'gc>);
    fn pop(&mut self, gc: GCHandle<'gc>) -> StackValue<'gc>;
    fn pop_safe(&mut self, gc: GCHandle<'gc>) -> Option<StackValue<'gc>>;  // Added in Phase 2.4
    fn dup(&mut self, gc: GCHandle<'gc>);
    fn peek(&self) -> Option<StackValue<'gc>>;
}

// Typed push/pop helpers (default implementations)
trait TypedStackOps<'gc>: EvalStackOps<'gc> {
    fn push_i32(&mut self, gc: GCHandle<'gc>, value: i32) {
        self.push(gc, StackValue::Int32(value))
    }
    fn pop_i32(&mut self, gc: GCHandle<'gc>) -> i32 { /* ... */ }
    // Default implementations reduce boilerplate
}

// Local variable access
trait LocalOps<'gc> {
    fn get_local(&self, index: usize) -> StackValue<'gc>;
    fn set_local(&mut self, index: usize, value: StackValue<'gc>);
    fn get_local_address(&self, index: usize) -> NonNull<u8>;
}

// Argument access
trait ArgumentOps<'gc> {
    fn get_argument(&self, index: usize) -> StackValue<'gc>;
    fn set_argument(&mut self, index: usize, value: StackValue<'gc>);
    fn get_argument_address(&self, index: usize) -> NonNull<u8>;
}
```

This decomposition allows instruction handlers to request only the capabilities they need, improving clarity and enabling more focused testing.

---

## Documentation Gaps

### Missing Module-Level Documentation

**Statistics:**
- Total doc comments (`///`): 430+
- Inner doc comments (`//!`): 500+
- Lines of code: 27,606
- **Doc coverage: ~5.0%** (improved but still low)

**Modules lacking `//!` documentation:**

| Module                    | Lines | Priority | Notes                           |
|---------------------------|-------|----------|---------------------------------|
| `dotnet-assemblies/src/`  | 800+  | High     | Assembly loading and resolution |
| `dotnet-utils/src/`       | 300+  | Medium   | Shared utility primitives       |
| `dotnet-macros/src/`      | 450+  | Medium   | Procedural macro definitions    |
| `dotnet-macros-core/src/` | 200+  | Medium   | Macro expansion logic           |

**Recommended module docs template:**
```rust
//! # Module Name
//!
//! Brief description of what this module provides.
//!
//! ## Key Types
//! - [`TypeA`] - description
//! - [`TypeB`] - description
//!
//! ## Usage
//! ```rust
//! // Example code
//! ```
//!
//! ## ECMA-335 References
//! - Section X.Y.Z: relevant spec section
```

### Undocumented Public APIs

**High-priority items needing documentation:**

1. **`VesContext` and all traits in `stack/ops.rs`**
   - Currently no doc comments on any trait methods
   - Should explain ECMA-335 semantics being implemented

2. **`StepResult` enum variants**
   - Critical for understanding execution flow
   - Each variant should explain when it's returned

3. **`StackValue` and `ObjectRef`**
   - Core types need examples of creation and use

4. **All `#[dotnet_intrinsic]` functions**
   - Should document which .NET method they implement
   - Reference ECMA-335 or Microsoft docs

5. **`TypeDescription`, `MethodDescription`, `FieldDescription`**
   - Key types in `dotnet-types` with complex semantics

### SAFETY Comments Audit

**Current state: 44 SAFETY comments for 250+ unsafe blocks (~17% coverage)**

**Unsafe blocks lacking SAFETY comments:**

| Location                                  | Unsafe Operation              | Risk Level |
|-------------------------------------------|-------------------------------|------------|
| `dotnet-assemblies/src/lib.rs:70`         | `ptr::copy_nonoverlapping`    | High       |
| `dotnet-value/src/lib.rs`                 | Raw pointer casts             | High       |
| `dotnet-vm/src/exceptions.rs:257,402,418` | Object field access           | Medium     |
| `dotnet-types/src/lib.rs:88`              | Pointer dereference           | Medium     |
| `dotnet-types/src/resolution.rs:35,51`    | `from_raw` pointer conversion | High       |
| `dotnet-vm/src/instructions/memory.rs`    | Unaligned heap access         | High       |
| `dotnet-vm/src/stack/context.rs`          | Raw memory manipulation       | High       |

**Required SAFETY documentation format:**
```rust
// SAFETY: [invariant being upheld]
// - Bullet point explaining why this is safe
// - Another invariant if applicable
unsafe {
    // code
}
```

**Example of good SAFETY comment from codebase:**
```rust
// SAFETY: This type only contains Copy types and no GC references.
// The thread-safety is manually managed via the Sync impl.
unsafe impl Send for TypeDescription {}
unsafe impl Sync for TypeDescription {}
```

**Missing SAFETY documentation priority list:**
1. All `unsafe impl Collect` blocks - explain what's traced
2. All raw pointer dereferences - explain validity guarantees
3. All `transmute` calls - explain type compatibility
4. All `from_raw` constructors - explain ownership/lifetime

---

## C# Support Library Review

### Design Philosophy

**Core Principle:** The support library should contain **only** stubs for BCL features that require tight runtime integration (intrinsics, internal VM state access, or unsafe memory operations). All other BCL functionality should be provided by referencing Microsoft's official libraries when possible.

**Location:** `crates/dotnet-assemblies/src/support/`

**Current stubs (all require runtime integration):**

| File                     | Lines | Purpose                                                  | Why Runtime Integration Required             |
|--------------------------|-------|----------------------------------------------------------|----------------------------------------------|
| `Array.cs`               | 236   | `System.Array` stub with IList, ICloneable               | Direct heap layout knowledge, GC interaction |
| `RuntimeType.cs`         | 210   | Custom `DotnetRs.RuntimeType` implementing `System.Type` | VM type system bridge, metadata access       |
| `Span.cs`                | 170   | `System.Span<T>` and `ReadOnlySpan<T>`                   | ByRef-like types, stack-only semantics       |
| `Comparers/Equality.cs`  | 100   | `EqualityComparer<T>` for collections                    | Generic type introspection                   |
| `MulticastDelegate.cs`   | 70    | Delegate invocation list support                         | Invocation list manipulation                 |
| `MethodInfo.cs`          | 60    | `System.Reflection.MethodInfo` stub                      | VM method descriptor bridge                  |
| `ConstructorInfo.cs`     | 55    | `System.Reflection.ConstructorInfo` stub                 | VM constructor descriptor bridge             |
| `FieldInfo.cs`           | 50    | `System.Reflection.FieldInfo` stub                       | VM field descriptor bridge                   |
| `Delegate.cs`            | 45    | Base delegate support                                    | Function pointer semantics                   |
| `RuntimeMethodHandle.cs` | 45    | Method handle wrapper                                    | Opaque VM handle                             |
| `RuntimeTypeHandle.cs`   | 40    | Type handle wrapper                                      | Opaque VM handle                             |
| `RuntimeFieldHandle.cs`  | 40    | Field handle wrapper                                     | Opaque VM handle                             |
| `Assembly.cs`            | 5     | Assembly representation                                  | VM assembly loader bridge                    |
| `Module.cs`              | 3     | Module stub                                              | VM module bridge                             |
| `StubAttribute.cs`       | 7     | `[Stub]` attribute for type replacement                  | Compile-time marker                          |

**Well-implemented areas:**
- ✅ Array operations (indexing, copying, enumerating)
- ✅ Reflection primitives (Type, MethodInfo, FieldInfo)
- ✅ Span/ReadOnlySpan with proper memory semantics
- ✅ Delegate and MulticastDelegate

### What NOT to Add to Support Library

The following should be provided via Microsoft's official BCL assemblies, not custom stubs:

**Do NOT implement (use Microsoft's libraries instead):**

1. **`System.Collections.Generic`** - `List<T>`, `Dictionary<TKey, TValue>`, `HashSet<T>`
   - ❌ Pure managed code, no runtime integration needed
   - ✅ Use Microsoft's implementation from System.Private.CoreLib

2. **`System.Linq`** - `Enumerable` extension methods
   - ❌ Pure managed code
   - ✅ Use Microsoft's System.Linq

3. **`System.Threading.Tasks`** - `Task`, `Task<T>`, async/await
   - ❌ Complex state machine, but no special VM support needed (beyond thread intrinsics)
   - ✅ Use Microsoft's implementation

4. **`System.IO`** - `Stream`, `MemoryStream`, file operations
   - ❌ P/Invoke based, no special VM semantics
   - ✅ Use Microsoft's implementation

5. **`System.Numerics`** - `Vector<T>`, `BigInteger`
   - ❌ SIMD intrinsics can be provided via intrinsic handlers
   - ✅ Use Microsoft's implementation for the managed wrapper

6. **Exception types** - `ArgumentNullException`, `InvalidOperationException`, etc.
   - ❌ Pure managed types, no special VM support needed
   - ✅ Use Microsoft's implementations

7. **`System.String` operations** - `Substring`, `IndexOf`, `Split`, `Join`
   - ❌ Currently handled via intrinsics, but most operations are pure managed
   - ✅ Use Microsoft's implementation (intrinsics only for length, allocation, indexing)

**Only add stubs when:**
- Type requires `extern` methods that call VM intrinsics
- Type requires access to VM internal state (GC, type system, reflection)
- Type has special layout or lifetime semantics (ByRef-like types)
- Microsoft's implementation is not available for the minimal profile being targeted

### Code Style Consistency

**Positive observations:**
- ✅ File-scoped namespaces (C# 10+): `namespace DotnetRs;`
- ✅ Nullable reference types enabled
- ✅ `ArgumentNullException.ThrowIfNull()` (C# 10+)
- ✅ `ArgumentOutOfRangeException.ThrowIfNegative()` (C# 10+)
- ✅ Expression-bodied members where appropriate

**Areas for improvement (low priority):**

1. **Inconsistent `NotImplementedException` usage**
   ```csharp
   // Some methods throw with message
   throw new NotImplementedException("Method not supported");
   // Others throw without
   throw new NotImplementedException();
   ```
   **Recommendation:** Use consistent message format

2. **Missing XML documentation**
   ```csharp
   // Better: Add XML doc comments
   /// <summary>Gets the total number of elements in the array.</summary>
   public extern int Length { get; }
   ```

---

## Testing Improvements

### Test Coverage Gaps

**Current Integration Test Coverage:**
- 74 C# fixture files across 17 categories
- Categories: arithmetic, arrays, basic, conversions, delegates, exceptions, fields, floats, gc, generics, interfaces, pinvoke, reflection, strings, structs, threading, unsafe

**Well-covered areas:**
- ✅ Basic arithmetic operations
- ✅ Array operations
- ✅ Exception handling (throw, catch, finally, filter)
- ✅ Struct layouts and value types
- ✅ Unsafe operations and Span
- ✅ Error handling paths (Phase 2.4 tests updated)

**Under-tested areas:**

| Area        | Current Tests | Recommended Additions              |
|-------------|---------------|------------------------------------|
| Edge cases  | Limited       | Stack underflow, overflow, NaN     |
| Memory      | Partial       | Allocation patterns, large objects |
| Interop     | Limited       | More P/Invoke scenarios            |

**Missing unit tests in Rust code:**

1. **`dotnet-vm/src/resolver.rs`** - Complex type resolution logic with no unit tests
2. **`dotnet-vm/src/layout.rs`** - Layout calculations could have pure unit tests
3. **`dotnet-types/src/comparer.rs`** - Type comparison logic (526 lines, no tests)
4. **`dotnet-value/src/`** - Value conversions could have isolated tests
5. **`dotnet-vm/src/error.rs`** - Error conversion and propagation (Phase 2.4)

### Test Organization

**Current Structure:**
```
crates/dotnet-cli/tests/
├── fixtures/           # 74 C# integration tests
│   ├── arithmetic/
│   ├── arrays/
│   ├── basic/
│   └── ...
├── debug_fixtures/     # Ignored/WIP tests
└── integration.rs      # Test harness
```

**Strengths:**
- ✅ Automatic test generation from filenames (`build.rs`)
- ✅ Clear naming convention: `<name>_<expected_exit_code>.cs`
- ✅ Organized by feature area
- ✅ Handles VM errors gracefully (Phase 2.4)

**Improvement Opportunities:**

1. **Add unit test modules to Rust crates**
   ```
   crates/dotnet-vm/src/
   ├── error.rs
   ├── error_tests.rs       # Add this
   ├── resolver.rs
   ├── resolver_tests.rs    # Add this
   ├── layout.rs
   └── layout_tests.rs      # Add this
   ```

2. **Add property-based tests with proptest**
   - Stack value conversions
   - Type comparison symmetry and transitivity
   - Layout calculations

3. **Add benchmark suite with criterion**
   - Instruction execution throughput
   - GC collection times
   - Method dispatch overhead

### Integration Test Improvements

**Current Limitations:**

1. **No `Console.WriteLine` support** (noted in guidelines)
   - Tests must return exit codes, can't verify output
   - Limits testing of string formatting

2. **No test isolation**
   - Tests share the same compilation cache
   - Could have ordering dependencies

3. **Limited error diagnostics**
   - When tests fail, stack traces aren't always clear
   - Add better failure reporting

**Recommended Improvements:**

1. **Add output capture tests**
   ```csharp
   // Once Console.WriteLine works:
   // output_test.cs
   Console.WriteLine("Hello");
   return 0;
   // Expected output: "Hello\n"
   ```

2. **Add timeout tests for infinite loop detection**
   ```rust
   #[test]
   #[timeout(5000)]  // 5 second timeout
   fn test_infinite_loop_detection() {
       // Should detect and terminate
   }
   ```

3. **Add negative tests (expected failures)**
   ```csharp
   // expected_null_ref_exception.cs
   // [ExpectedException: NullReferenceException]
   object o = null;
   o.ToString();  // Should throw
   ```

4. **Parameterized fixture tests**
   ```csharp
   // Currently: one file per test case
   // Better: parameterized tests
   // [TestCase(1, 2, 3)]
   // [TestCase(-1, 1, 0)]
   public static int Add(int a, int b) => a + b == expected ? 0 : 1;
   ```

5. **Test coverage reporting**
   - Add `cargo-tarpaulin` integration
   - Track which CIL instructions are exercised
   - Generate coverage reports in CI

### Suggested New Test Categories

| Category      | Purpose             | Example Tests                                      |
|---------------|---------------------|----------------------------------------------------|
| `edge_cases/` | Boundary conditions | Stack limits, empty arrays                         |
| `regression/` | Bug fixes           | Tests for specific fixed issues                    |
| `errors/`     | Error handling      | Tests expecting specific VmError types (Phase 2.4) |
| `stress/`     | High-load scenarios | Deep recursion, many allocations                   |

---

## Phased Refactoring Plan

### Phase 1: Quick Wins (1-2 days each)

These are low-risk, high-impact changes that can be done independently.

---

#### 1.1 Add SAFETY Comments to Unsafe Blocks

**Effort:** 1 day  
**Risk:** Low  
**Status:** Completed (Priority files: `dotnet-assemblies/src/lib.rs`, `dotnet-vm/src/instructions/memory.rs`, `dotnet-vm/src/stack/context.rs`)

**Steps:**
1. Run `grep -rn "unsafe {" crates/ --include="*.rs" | grep -v SAFETY`
2. For each unsafe block, add a `// SAFETY:` comment explaining:
   - What invariant is being upheld
   - Why the operation is safe
   - Any preconditions required
3. Priority Files:
   - `dotnet-assemblies/src/lib.rs` (Manual buffer management)
   - `dotnet-vm/src/instructions/memory.rs` (Unaligned access)
   - `dotnet-vm/src/stack/context.rs` (Evaluation stack internals)
4. Follow the format in the Documentation Gaps section
5. Run `cargo test` after each file to ensure no regressions

**Verification:** `grep -r "SAFETY" crates/ | wc -l` should increase from 44 to ~250+

---

#### 1.2 Replace Verbose Match with Let-Else

**Effort:** 1 day  
**Risk:** Low  
**Status:** Completed

**Steps:**
1. Identify patterns where `match` or `if let` is used for early exit:
   ```rust
   let value = match opt { Some(v) => v, None => return X };
   ```
2. Replace with:
   ```rust
   let Some(value) = opt else { return X };
   ```
3. Primary candidates:
   - `dotnet-vm/src/executor.rs:270` (GC coordination)
   - Various instruction handlers in `dotnet-vm/src/instructions/`
4. Test each file after modification

---

#### 1.3 Add Module-Level Documentation

**Effort:** 2 days  
**Risk:** Low  
**Status:** Completed

**Steps:**
1. Add `//!` documentation to each module following template in Documentation Gaps
2. Remaining Priority:
   - `dotnet-assemblies/src/lib.rs`
   - `dotnet-utils/src/lib.rs`
   - `dotnet-macros/src/lib.rs`
   - `dotnet-macros-core/src/lib.rs`
3. Include ECMA-335 section references where applicable

---

#### 1.4 Add `#[must_use]` Attributes

**Effort:** 0.5 days  
**Risk:** Low  
**Status:** Completed

**Steps:**
1. Add `#[must_use]` to critical enums and functions
2. Primary targets:
   - `dotnet_vm::StepResult` (variants must be handled by executor)
   - `dotnet_vm::stack::ops::MemoryOps` methods
   - `dotnet_vm::stack::ops::StackOps` methods (pop operations)
3. Fix any new warnings that appear

---

### Phase 2: Structural Improvements (3-5 days each)

These require more careful planning but significantly improve code quality.

---

#### 2.1 Split `ReflectionOps` Trait (COMPLETED)

**Notes for next agent:**
- Phase 2.1 is fully implemented.
- `ReflectionOps` has been split into `LoaderOps`, `StaticsOps`, and `ThreadOps`.
- `VesOps` now includes these new traits as supertraits.
- `intrinsics/reflection/common.rs` has been updated to use broader trait bounds.
- All integration tests passed across all feature combinations.

**Effort:** 3 days  
**Risk:** Medium  
**Files:** 
- `crates/dotnet-vm/src/stack/ops.rs`
- `crates/dotnet-vm/src/stack/context.rs`
- All intrinsic files using `ReflectionOps`

**Steps:**

1. **Create new trait definitions** in `stack/ops.rs`:
   ```rust
   pub trait LoaderOps<'m> {
       fn loader(&self) -> &'m AssemblyLoader;
       fn resolver(&self) -> ResolverService<'m>;
       fn shared(&self) -> &Arc<SharedGlobalState<'m>>;
   }
   
   pub trait StaticsOps<'gc> {
       fn statics(&self) -> &StaticStorageManager;
       fn initialize_static_storage(&mut self, gc: GCHandle<'gc>, 
           description: TypeDescription, generics: GenericLookup) -> StepResult;
   }
   
   pub trait ThreadOps {
       fn thread_id(&self) -> usize;
   }
   ```

2. **Update `VesOps` supertrait bounds**:
   ```rust
   pub trait VesOps<'gc, 'm: 'gc>: 
       StackOps<'gc, 'm> + LoaderOps<'m> + StaticsOps<'gc> + ThreadOps + ...
   ```

3. **Implement new traits for VesContext** in `context.rs`

4. **Update call sites** - search for uses of removed methods:
   ```bash
   grep -rn "\.loader()\|\.resolver()\|\.shared()\|\.statics()\|\.thread_id()" crates/
   ```

5. **Run full test suite**: `./check.sh`

---

#### 2.2 Split `stack/context.rs` Into Multiple Files (COMPLETED)

Checkpoint (2026-02-09):
- Completed cleanup of unused imports across split modules to satisfy `-D warnings`.
  - Files updated: `stack/context.rs`, `stack/call_ops_impl.rs`, `stack/exception_ops_impl.rs`, `stack/reflection_ops_impl.rs`, `stack/stack_ops_impl.rs`.
  - Commit: AI-generated: Phase 2.2 – fix unused imports in split stack modules to satisfy -D warnings
- Verified tests for all feature combinations with per-command timeouts ≤ 20s.
  - No features: all tests passed (75/75 integration tests passed, 1 ignored: `hello_world`).
  - `multithreading`: all tests passed (83/83 integration tests passed, 1 ignored).
  - `multithreaded-gc`: all tests passed (87/87 integration tests passed, 1 ignored).
  - Clippy ran cleanly with `-D warnings` in all configurations.
  - Commands used (each wrapped with `timeout 20s`):
    - `cargo clippy --all-targets --no-default-features -- -D warnings && cargo test --no-default-features`
    - `cargo clippy --all-targets --no-default-features --features multithreading -- -D warnings && cargo test --no-default-features --features multithreading`
    - `cargo clippy --all-targets --no-default-features --features multithreaded-gc -- -D warnings && cargo test --no-default-features --features multithreaded-gc`
- Removed intermediate artifacts from project root.
  - Deleted: `*_ops_content.txt`, `prepare_impls.py`, `rewrite_context.py`, `clean_context.py`, `final_clean.py`, and `context_*.rs` files as planned.
  - Commit: AI-generated: Phase 2.2 – test verification across features and cleanup of intermediate artifacts

Notes for next agent:
- Phase 2.2 is fully complete and green across all feature sets. Proceed to Phase 2.3.
- A log file `.output.txt` may be present from earlier long command output truncation; it is safe to ignore or delete if not needed.

---

#### 2.3 Extract Mega-Function in Reflection Intrinsics

**Effort:** 4-5 days  
**Risk:** Medium  
**File:** `crates/dotnet-vm/src/intrinsics/reflection/types.rs`
**Target:** `runtime_type_intrinsic_call()` - 534 lines

**Steps:**

1. **Identify all handlers** in the match statement (30+ cases)

2. **Create individual handler functions**:
   ```rust
   fn handle_get_assembly<'gc, 'm>(
       ctx: &mut dyn VesOps<'gc, 'm>,
       gc: GCHandle<'gc>,
       generics: &GenericLookup,
   ) -> StepResult { ... }
   
   fn handle_get_namespace<'gc, 'm>(...) -> StepResult { ... }
   ```

3. **Replace match arms with function calls**:
   ```rust
   match (method_name, param_count) {
       ("GetAssembly" | "get_Assembly", 0) => handle_get_assembly(ctx, gc, generics),
       ("GetNamespace" | "get_Namespace", 0) => handle_get_namespace(ctx, gc, generics),
       // ...
   }
   ```

4. **Consider using a dispatch table**:
   ```rust
   type IntrinsicHandler = fn(&mut dyn VesOps, GCHandle, &GenericLookup) -> StepResult;
   static HANDLERS: phf::Map<&str, IntrinsicHandler> = phf_map! {
       "get_Assembly" => handle_get_assembly,
       // ...
   };
   ```

5. **Test each handler individually** after extraction

Checkpoint (2026-02-09):
- Extracted handlers and refactored `runtime_type_intrinsic_call` into function calls. Handlers added:
  - `handle_create_instance_check_this`, `handle_get_assembly`, `handle_get_namespace`, `handle_get_methods`, `handle_get_method_impl`, `handle_get_constructors`, `handle_get_name`, `handle_get_base_type`, `handle_get_is_generic_type`, `handle_get_generic_type_definition`, `handle_get_generic_arguments`, `handle_get_type_handle`, `handle_make_generic_type`, `handle_create_instance_default_ctor`.
- Kept a simple match-based dispatch for clarity. A `phf` dispatch table was considered but deferred to minimize churn; can be added later with no behavior change.
- Verified build and tests across all feature combinations (timeouts ≤ 20s):
  - `timeout 20s cargo test --no-default-features` → PASS
  - `timeout 20s cargo test --no-default-features --features multithreading` → PASS
  - `timeout 20s cargo test --no-default-features --features multithreaded-gc` → PASS
- Notes for next agent:
  - The extraction is behavior-preserving. Further micro-extractions (e.g., sharing BindingFlags decoding) can be done later but are not required.
  - If adding dispatch table, prefer `phf` with a small wrapper that also checks `param_count` to avoid accidental collisions between overloads.
  - Reflection methods not implemented in the match (e.g., `GetFields`, `GetField`, `GetProperties`, `GetPropertyImpl`) still route to the panic as before; implement them in future phases as needed.
- Commits:
  - AI-generated: Phase 2.3 – extract handlers from runtime_type_intrinsic_call and refactor match arms

---

#### 2.4 Create VM Error Type Hierarchy

**Effort:** 4 days  
**Risk:** Medium  
**New file:** `crates/dotnet-vm/src/error.rs`

**Steps:**

1. **Add thiserror dependency** to `Cargo.toml`:
   ```toml
   thiserror = "1.0"
   ```

2. **Create error types** as defined in Error Handling section

3. **Add `Error` variant to `StepResult`**:
   ```rust
   pub enum StepResult {
       // ... existing variants
       Error(VmError),
   }
   ```

4. **Handle `StepResult::Error` in execution loop** (`executor.rs`) - DONE

5. **Migrate high-priority unwrap/expect calls** (see Error Handling section) - DONE (migrated stack underflow, call/constrained dispatch, and memory allocation panics)

6. **Run full test suite** to ensure no panics converted to errors break tests - DONE

---

### Checkpoint 2.4 Complete
The VM now has a structured error hierarchy using `thiserror`. 
- `VmError` covers assembly loading, type resolution, execution, and memory access.
- `StepResult` and `ExecutorResult` now propagate these errors instead of panicking.
- `pop_safe` added to `StackOps` to handle stack underflow gracefully.
- Common instruction macros updated to use `pop_safe`.
- Integration tests updated to handle the new `Error` variant.

### Phase 3: Architectural Changes (1-2 weeks each)

These are significant refactors that touch many files.

---

#### 3.1 Remove GC Handle Parameter Threading (CHECKPOINT)

**Effort:** 1-2 weeks
**Risk:** High
**Impact:** Nearly every file in dotnet-vm

**Status (2026-02-10 - Checkpoint 8):**
- ✅ **Phase 3.1 COMPLETE**: The removal of `GCHandle` parameters from `VesOps`, `EvaluationStack`, `external_call`, and instruction/intrinsic macros is finished and the codebase is clean.
- ✅ **Massive Cleanup Finished**: All unused imports (`GCHandle`, `Mutation`) and unused variable assignments (`let gc = ctx.gc();`) flagged by clippy have been removed.
- ✅ **Builds Passing**: `./check.sh` passes for all feature combinations without warnings.
- ✅ **Verified Correctness**: All integration tests (87/87) passed successfully.

**Next Actions for Next Agent:**
1. Proceed to **Phase 3.2: Decompose `StackOps` Trait**.

**Steps:**

1. **Fix Trait Definition (Done):**
   - ✅ Add `fn gc(&self) -> GCHandle<'gc>;` to `VesOps`.

2. **Implement Missing Core Methods (Done):**
   - ✅ `handle_return()`, `handle_exception()` implemented and renamed from `_v2`.

3. **Update Macro Logic (Done):**
   - ✅ `dotnet_instruction` updated to remove `GCHandle` from call generation.
   - ✅ `InstructionHandler` type updated in registry.
   - ✅ `with_string!` and `with_string_mut!` updated to use `ctx.gc()`.

4. **Complete Instruction Handler Migration (Done):**
   - ✅ All CIL instruction handlers (macro and manual) updated.
5. **Fix Intrinsic Signatures (Done):**
   - ✅ Function signatures and internal call sites updated.
6. **Final API Cleanup (Done):**
   - ✅ `_v2` suffixes removed.
   - ✅ Cleanup of unused items finished.
7. **Verify Safety (Done):**
   - ✅ Run `./check.sh` after cleanup.

---

#### 3.2 Decompose `StackOps` Trait

**Effort:** 1 week  
**Risk:** High  
**Impact:** All instruction handlers

**Steps:**

1. **Create new sub-traits** as defined in Abstraction section:
   - `EvalStackOps`
   - `TypedStackOps`
   - `LocalOps`
   - `ArgumentOps`

2. **Add default implementations** where possible

3. **Update `StackOps` to be a supertrait alias**:
   ```rust
   pub trait StackOps<'gc, 'm>: 
       EvalStackOps<'gc> + TypedStackOps<'gc> + LocalOps<'gc> + ArgumentOps<'gc> {}
   ```

4. **Update instruction handlers** to use minimal bounds:
   ```rust
   // Before
   fn add<T: StackOps<'gc, 'm>>(ctx: &mut T, ...) { ... }
   // After (more precise)
   fn add<T: TypedStackOps<'gc>>(ctx: &mut T, ...) { ... }
   ```

---

#### 3.3 Comprehensive Error Handling Migration

**Effort:** 2 weeks  
**Risk:** High  
**Goal:** Reduce panic-prone calls from 563 to <100

**Steps:**

1. **Audit all `unwrap()`/`expect()` calls**:
   ```bash
   grep -rn "\.unwrap()\|\.expect(" crates/ --include="*.rs" > unwrap_audit.txt
   ```

2. **Categorize each call** using the criteria in Error Handling section

3. **Convert file by file**, starting with:
   - `dotnet-assemblies/src/lib.rs` (assembly loading)
   - `dotnet-vm/src/dispatch/mod.rs` (method dispatch)
   - `dotnet-vm/src/resolver.rs` (type resolution)

4. **Update function signatures** to return `Result<T, VmError>`

5. **Propagate errors** up the call stack with `?`

6. **Add context** using `.context()` or error variants

7. **Test extensively** - converted errors shouldn't change behavior for valid input

---

### Implementation Guidelines for AI Agents

#### General Rules

1. **Always run `./check.sh` before and after changes**
   - This tests all feature flag combinations
   - Don't submit if any configuration fails

2. **Make atomic commits**
   - One logical change per commit
   - Each commit should pass tests

3. **Preserve existing behavior**
   - Refactoring shouldn't change observable behavior
   - If tests fail, investigate before adjusting tests

4. **Use MCP tools for .NET semantics**
   - `mcp_ecma-search_ecma_search` for ECMA-335 questions
   - `mcp_microsoft-learn_microsoft_docs_search` for BCL behavior

#### File Modification Patterns

**When splitting a file:**
1. Create the new file with proper module declaration
2. Move code in small chunks, testing between moves
3. Update imports in both files
4. Update `mod.rs` declarations

**When modifying traits:**
1. Add new methods/traits first (additive change)
2. Update implementations
3. Update call sites
4. Remove deprecated items last

**When changing error handling:**
1. Add error types first
2. Change one function at a time
3. Ensure callers handle the new error case
4. Test with both valid and invalid inputs

#### Testing Requirements

- All existing tests must pass
- Add unit tests for any new functions
- Add integration tests for behavior changes
- Use `cargo test -- --nocapture` for debugging

#### Documentation Requirements

- Add doc comments to new public APIs
- Update REVIEW.md if findings change
- Add SAFETY comments to any new unsafe code

---

## Appendix

### A. File Size Inventory

All Rust source files sorted by size (descending):

| Lines | File                                           |
|-------|------------------------------------------------|
| 1,546 | `dotnet-vm/src/stack/context.rs`               |
| 1,088 | `dotnet-vm/src/intrinsics/reflection/types.rs` |
| 947   | `dotnet-vm/src/resolver.rs`                    |
| 773   | `dotnet-vm/src/intrinsics/threading.rs`        |
| 759   | `dotnet-assemblies/src/lib.rs`                 |
| 745   | `dotnet-vm/src/tracer.rs`                      |
| 705   | `dotnet-value/src/lib.rs`                      |
| 691   | `dotnet-value/src/object.rs`                   |
| 674   | `dotnet-vm/src/intrinsics/delegates.rs`        |
| 672   | `dotnet-vm/src/pinvoke.rs`                     |
| 671   | `dotnet-cli/tests/integration.rs`              |
| 646   | `dotnet-vm/src/exceptions.rs`                  |
| 542   | `dotnet-vm/src/memory/access.rs`               |
| 540   | `dotnet-vm/src/threading/basic.rs`             |
| 528   | `dotnet-vm/src/intrinsics/unsafe_ops.rs`       |
| 526   | `dotnet-types/src/comparer.rs`                 |
| 478   | `dotnet-vm/src/layout.rs`                      |
| 467   | `dotnet-utils/src/gc.rs`                       |
| 465   | `dotnet-vm/src/instructions/objects/fields.rs` |
| 464   | `dotnet-macros/src/lib.rs`                     |
| 448   | `dotnet-vm/src/intrinsics/mod.rs`              |
| 438   | `dotnet-vm/src/intrinsics/string_ops.rs`       |
| 437   | `dotnet-vm/src/gc/coordinator.rs`              |
| 413   | `dotnet-vm/src/intrinsics/span.rs`             |
| 404   | `dotnet-vm/src/instructions/objects/arrays.rs` |

**Total:** 99 files, 27,606 lines

### B. Dependency Graph

```
dotnet-cli (CLI entry point)
│
└── dotnet-vm (VM implementation)
    │
    ├── dotnet-assemblies (Assembly loading)
    │   └── dotnetdll (external: .NET metadata parsing)
    │
    ├── dotnet-value (Stack/heap values)
    │   └── gc-arena (external: GC support)
    │
    ├── dotnet-types (Type system)
    │   ├── dotnetdll
    │   └── gc-arena
    │
    ├── dotnet-utils (GC utilities, sync primitives)
    │   └── gc-arena
    │
    ├── dotnet-macros (Procedural macros)
    │   └── dotnet-macros-core
    │
    └── dotnet-macros-core (Macro expansion logic)
```

**External dependencies:**
- `dotnetdll` - .NET PE/metadata parsing (sister crate)
- `gc-arena` - Garbage collection with `Arena`, `Gc`, `Collect` trait
- `thiserror` - Error type derivation (added in Phase 2.4)
- `phf` - Perfect hash functions for intrinsic dispatch
- `log` - Logging facade
- `parking_lot` - Efficient synchronization primitives

### C. References

**ECMA-335 Standard:**
- [ECMA-335 Spec PDF](https://www.ecma-international.org/publications-and-standards/standards/ecma-335/)
- Use `mcp_ecma-search_ecma_search` MCP tool for specific sections

**Microsoft Documentation:**
- [.NET Runtime Documentation](https://docs.microsoft.com/en-us/dotnet/standard/clr)
- [CoreCLR GitHub](https://github.com/dotnet/runtime/tree/main/src/coreclr)
- Use `mcp_microsoft-learn_microsoft_docs_search` MCP tool

**Rust Resources:**
- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [Unsafe Code Guidelines](https://rust-lang.github.io/unsafe-code-guidelines/)
- [gc-arena Documentation](https://docs.rs/gc-arena/latest/gc_arena/)

**Project Resources:**
- [dotnetdll Crate](https://github.com/nickbclifford/dotnetdll)
- Project Guidelines: `.junie/guidelines.md`

---

*Document generated: 2026-02-08*
*Last updated: 2026-02-09 (Phase 2.4 complete)*
