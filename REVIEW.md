# Code Review and Refactoring Plan for dotnet-rs

> **Document Purpose**: A comprehensive review identifying code quality improvements and a phased refactoring plan for future AI agents to implement.
>
> **Date**: 2026-02-08
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

| Category               | Status                                                     | Priority |
|------------------------|------------------------------------------------------------|----------|
| **Code Organization**  | 14 files exceed 500 LOC; largest is 1,088 lines            | High     |
| **Error Handling**     | 563 `unwrap()`/`expect()` calls; no unified error type     | High     |
| **Documentation**      | 430+ doc comments, 44 SAFETY comments for 250+ unsafe blocks | Medium   |
| **Trait Design**       | ReflectionOps has 18 methods; traits could be decomposed   | Medium   |
| **C# Support Library** | Well-written with modern C# idioms                         | Low      |
| **Testing**            | Good fixture-based approach; some gaps in unit tests       | Medium   |

### Critical Issues Requiring Immediate Attention

1. **`stack/context.rs` (592 lines)**: God Object has been mostly split; needs final cleanup and build fix
2. **`intrinsics/reflection/types.rs`**: Contains a 534-line match statement that's hard to maintain
3. **Panic-heavy code**: Many `expect()` calls could crash the VM instead of returning proper errors
4. **Unsafe blocks lacking documentation**: Only 17% of unsafe blocks have SAFETY comments

### Positive Aspects

- ✅ Clippy-clean codebase (no warnings)
- ✅ Clear crate separation with defined responsibilities  
- ✅ Well-designed trait hierarchy (`VesOps`, `StackOps`, etc.)
- ✅ Modern C# support library using file-scoped namespaces and nullable reference types
- ✅ Comprehensive integration test framework with auto-generated test runners

---

## Code Organization Analysis

### Large Files Requiring Attention

Files exceeding 500 lines that should be considered for splitting:

| File                                           | Lines | Issue                                          | Recommended Action                                   |
|------------------------------------------------|-------|------------------------------------------------|------------------------------------------------------|
| `dotnet-vm/src/stack/context.rs`               | 592   | Core struct and VesOps implementation          | Further split VesOps if necessary                    |
| `dotnet-vm/src/intrinsics/reflection/types.rs` | 1,088 | Single 534-line function with massive match    | Extract to dispatch table or individual handlers     |
| `dotnet-vm/src/resolver.rs`                    | 947   | `ResolverService` has 28 methods               | Group related methods into sub-modules               |
| `dotnet-vm/src/intrinsics/threading.rs`        | 773   | Many intrinsics in one file                    | Split by threading subsystem (Thread, Monitor, etc.) |
| `dotnet-assemblies/src/lib.rs`                 | 759   | Mixed assembly loading and resolution          | Extract `AssemblyLoader` and `ResolutionHelper`      |
| `dotnet-vm/src/tracer.rs`                      | 745   | Tracer logic interleaved with formatting       | Separate tracing core from output formatters         |
| `dotnet-value/src/lib.rs`                      | 705   | `StackValue` + conversions + utilities         | Extract conversion logic to `conversions.rs`         |
| `dotnet-value/src/object.rs`                   | 691   | Object types + heap storage + GC support       | Split into `object.rs`, `heap.rs`, `gc_support.rs`   |
| `dotnet-vm/src/intrinsics/delegates.rs`        | 674   | Delegate intrinsics + multicast logic          | Separate `delegate_core.rs` and `multicast.rs`       |
| `dotnet-vm/src/pinvoke.rs`                     | 672   | P/Invoke implementation                        | Group by P/Invoke category                           |
| `dotnet-vm/src/exceptions.rs`                  | 646   | Exception handling state machine               | Well-structured; consider splitting only if grows    |
| `dotnet-vm/src/memory/access.rs`               | 542   | Memory access patterns                         | Split read/write operations                          |
| `dotnet-vm/src/threading/basic.rs`             | 540   | Threading primitives                           | Split into `thread_pool.rs`, `synchronization.rs`    |
| `dotnet-vm/src/intrinsics/unsafe_ops.rs`       | 528   | System.Runtime.CompilerServices.Unsafe         | Consider grouping by operation type                  |

### Module Structure Issues

**1. God Object Pattern: `VesContext`**
```
Location: crates/dotnet-vm/src/stack/context.rs
Problem: VesContext implements StackOps, MemoryOps, ExceptionOps, ResolutionOps, 
         PoolOps, RawMemoryOps, ReflectionOps, CallOps, and VesOps
Impact: Changes to any trait require modifying this single 1,546-line file
```

**2. Overloaded `ReflectionOps` Trait**
```
Location: crates/dotnet-vm/src/stack/ops.rs (lines 147-192)
Problem: 18 methods mixing reflection, loader access, static storage, and thread info
Impact: Violates Single Responsibility; makes testing difficult
Recommendation: Split into ReflectionOps, LoaderOps, StaticsOps
```

**3. Mega-Function in Reflection Intrinsics**
```
Location: crates/dotnet-vm/src/intrinsics/reflection/types.rs
Function: runtime_type_intrinsic_call() - lines 115-649 (534 lines)
Problem: Handles 30+ intrinsic method signatures in one giant match
Impact: Difficult to test individual methods; merge conflicts likely
Recommendation: Use dispatch table pattern matching the `#[dotnet_intrinsic]` system
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

**1. Let-else (Rust 1.65+)**
Replace verbose match statements for extracting from Option/Result:
```rust
// Before (common pattern in codebase)
let value = match some_option {
    Some(v) => v,
    None => return StepResult::Exception,
};

// After
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

### GC Handle Pattern Issue

Currently, `GCHandle<'gc>` is passed as the first parameter to almost every method:
```rust
fn push_i32(&mut self, gc: GCHandle<'gc>, value: i32);
fn pop_i32(&mut self, gc: GCHandle<'gc>) -> i32;
fn push_obj(&mut self, gc: GCHandle<'gc>, value: ObjectRef<'gc>);
// ... 40+ more methods
```

**Recommended Improvement**: Store `gc` in the context:
```rust
// Before: 2 params everywhere
ctx.push_i32(gc, value);
ctx.pop_i32(gc);

// After: gc stored in context
ctx.push_i32(value);
ctx.pop_i32();
// Or provide gc() accessor when needed for allocations
```

This would reduce parameter noise significantly but requires careful analysis of lifetime implications.

---

## Error Handling Patterns

### Current State Analysis

**Total panic-prone calls: 563** (includes `unwrap()`, `expect()`, `panic!`)

Distribution by crate:

| Crate               | Count | Severity | Notes                     |
|---------------------|-------|----------|---------------------------|
| `dotnet-vm`         | ~350  | High     | Core execution paths      |
| `dotnet-assemblies` | ~80   | High     | Assembly loading can fail |
| `dotnet-value`      | ~50   | Medium   | Value conversions         |
| `dotnet-types`      | ~40   | Medium   | Type resolution           |
| `dotnet-macros`     | ~30   | Low      | Compile-time only         |
| `dotnet-cli`        | ~10   | Low      | Entry point               |

**Categories of panics found:**

1. **Legitimate Invariant Assertions** (~100)
   ```rust
   // OK: Internal invariant that indicates bug if violated
   .expect("Method should always have a body at this point")
   ```

2. **Should Be User-Facing Errors** (~200)
   ```rust
   // BAD: Could fail with malformed assembly
   Resolution::parse(byte_slice, ReadOptions::default()).unwrap()
   // BAD: Could fail if type not found
   res.definition().type_definition_index(index).unwrap()
   ```

3. **Missing Error Propagation** (~150)
   ```rust
   // BAD: File operations should return Result
   let file_data = std::fs::read(&path).expect("failed to read file");
   ```

4. **Lazy Placeholders** (~100)
   ```rust
   // TODO markers that panic
   throw new NotImplementedException();  // in C# stubs
   unimplemented!()  // in Rust
   ```

### Proposed Error Type Strategy

**Phase 1: Create VM Error Type Hierarchy**
```rust
// In dotnet-vm/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum VmError {
    #[error("Assembly loading failed: {0}")]
    AssemblyLoad(#[from] AssemblyLoadError),
    
    #[error("Type resolution failed: {0}")]
    TypeResolution(#[from] TypeResolutionError),
    
    #[error("Method execution failed: {0}")]
    Execution(#[from] ExecutionError),
    
    #[error("Memory access violation: {0}")]
    Memory(#[from] MemoryError),
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
    #[error("Stack underflow")]
    StackUnderflow,
    
    #[error("Invalid instruction pointer: {0}")]
    InvalidIP(usize),
    
    #[error("Type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },
    
    #[error("Null reference")]
    NullReference,
}
```

**Phase 2: Extend StepResult**
```rust
pub enum StepResult {
    Continue,
    Jump(usize),
    FramePushed,
    Return,
    Exception,
    MethodThrew,
    Yield,
    // NEW: For internal VM errors that aren't .NET exceptions
    Error(VmError),
}
```

**Phase 3: Gradual Migration**
1. Add `#[must_use]` attributes to prevent ignored Results
2. Replace `unwrap()` with `?` in functions returning `Result`
3. Use `ok_or_else()` for Option-to-Result conversions
4. Add context with `.context()` (from anyhow) or custom error variants

### Critical vs. Non-Critical Panics

**Keep as Panics (Invariants):**
- Internal consistency checks in GC code
- Unreachable code paths (use `unreachable!()`)
- Programming errors that indicate bugs, not user input issues

**Convert to Errors:**
- File I/O operations
- Assembly parsing
- Type resolution from metadata
- Method dispatch failures
- Field/array access bounds
- Stack operations (underflow/overflow)

**Specific High-Priority Conversions:**

| Location                 | Current                              | Should Be                                   |
|--------------------------|--------------------------------------|---------------------------------------------|
| `assemblies/lib.rs:82`   | `Resolution::parse().unwrap()`       | `Result<Resolution, ParseError>`            |
| `assemblies/lib.rs:700`  | `File::read().unwrap_or_else(panic)` | `Result<Vec<u8>, IoError>`                  |
| `vm/resolver.rs` various | `.expect("type not found")`          | `Option<T>` or `Result<T, ResolutionError>` |
| `vm/dispatch/mod.rs`     | `.expect("method not found")`        | Return `StepResult::Error`                  |

---

## Abstraction and Trait Design

### Trait Hierarchy Analysis

The current trait hierarchy in `stack/ops.rs`:

```
VesOps (26 methods) 
    : StackOps (27 methods)
    + MemoryOps (8 methods) 
    + ExceptionOps (7 methods)
    + ResolutionOps (4 methods)
    + PoolOps (1 method)
    + RawMemoryOps (3 methods)
    + ReflectionOps (18 methods)
    + CallOps (5 methods)
```

**Strengths:**
- Clear separation of concerns by operation type
- Enables trait-based testing with mock implementations
- Allows instruction handlers to specify minimal bounds

**Weaknesses:**
- `VesOps` has 26 methods, making implementations verbose
- `ReflectionOps` mixes too many concerns (see below)
- All traits require `GCHandle` parameter threading

### Recommended Trait Refactoring

**Split `ReflectionOps` (18 methods) into focused traits:**

```rust
// Current ReflectionOps methods that should be separate:

// 1. New trait: LoaderOps (accessing assembly loader)
trait LoaderOps<'m> {
    fn loader(&self) -> &'m AssemblyLoader;
    fn resolver(&self) -> ResolverService<'m>;
    fn shared(&self) -> &Arc<SharedGlobalState<'m>>;
}

// 2. New trait: StaticsOps (static storage management)
trait StaticsOps<'gc> {
    fn statics(&self) -> &StaticStorageManager;
    fn initialize_static_storage(&mut self, gc: GCHandle<'gc>, 
        description: TypeDescription, generics: GenericLookup) -> StepResult;
}

// 3. New trait: ThreadOps (thread-specific operations)
trait ThreadOps {
    fn thread_id(&self) -> usize;
}

// 4. Reduced ReflectionOps (pure reflection)
trait ReflectionOps<'gc, 'm> {
    fn pre_initialize_reflection(&mut self, gc: GCHandle<'gc>);
    fn get_runtime_type(&mut self, gc: GCHandle<'gc>, target: RuntimeType) -> ObjectRef<'gc>;
    fn get_runtime_method_obj(&mut self, gc: GCHandle<'gc>, ...) -> ObjectRef<'gc>;
    fn get_runtime_field_obj(&mut self, gc: GCHandle<'gc>, ...) -> ObjectRef<'gc>;
    fn resolve_runtime_type(&self, obj: ObjectRef<'gc>) -> RuntimeType;
    fn resolve_runtime_method(&self, obj: ObjectRef<'gc>) -> (MethodDescription, GenericLookup);
    fn resolve_runtime_field(&self, obj: ObjectRef<'gc>) -> (FieldDescription, GenericLookup);
    fn reflection(&self) -> ReflectionRegistry<'_, 'gc>;
}
```

### GCHandle Parameter Passing

**Current Pattern (pervasive throughout codebase):**
```rust
pub trait StackOps<'gc, 'm: 'gc> {
    fn push(&mut self, gc: GCHandle<'gc>, value: StackValue<'gc>);
    fn push_i32(&mut self, gc: GCHandle<'gc>, value: i32);
    fn push_i64(&mut self, gc: GCHandle<'gc>, value: i64);
    fn pop(&mut self, gc: GCHandle<'gc>) -> StackValue<'gc>;
    fn pop_i32(&mut self, gc: GCHandle<'gc>) -> i32;
    // ... 22 more methods all taking gc
}
```

**Problem:** The `gc` handle is passed to ~90% of all methods, creating visual noise and making signatures harder to read.

**Analysis of why `gc` is passed everywhere:**
1. `ObjectRef::new(gc, ...)` - needs gc to brand objects
2. Tracing/debugging that logs with gc context
3. GC-safe point checks during operations

**Proposed Solutions:**

**Option A: Store gc in context (Recommended)**
```rust
pub struct VesContext<'a, 'gc, 'm> {
    gc: GCHandle<'gc>,  // Add field
    // ... existing fields
}

impl<'a, 'gc, 'm> VesContext<'a, 'gc, 'm> {
    pub fn gc(&self) -> GCHandle<'gc> { self.gc }
}

// Then simplify traits:
pub trait StackOps<'gc, 'm: 'gc> {
    fn push(&mut self, value: StackValue<'gc>);
    fn push_i32(&mut self, value: i32);
    // gc accessed via self.gc() when needed internally
}
```

**Option B: Use a context wrapper for GC operations**
```rust
// Keep gc separate but provide a scoped API
ctx.with_gc(gc, |gctx| {
    gctx.push_i32(value);
    gctx.pop_i32()
});
```

**Migration Strategy:**
1. Add `gc` field to `VesContext`
2. Create new method signatures without `gc` parameter
3. Deprecate old signatures
4. Update all call sites (can be automated with sed/ripgrep)
5. Remove deprecated methods

### Large Trait Problem

**Traits exceeding 10 methods:**

| Trait           | Methods | Recommendation                                        |
|-----------------|---------|-------------------------------------------------------|
| `StackOps`      | 27      | Split into `PushOps`, `PopOps`, `LocalOps`, `SlotOps` |
| `VesOps`        | 26      | Consider making some methods default implementations  |
| `ReflectionOps` | 18      | Split as shown above                                  |

**StackOps Decomposition Proposal:**
```rust
// Basic push/pop
trait EvalStackOps<'gc> {
    fn push(&mut self, value: StackValue<'gc>);
    fn pop(&mut self) -> StackValue<'gc>;
    fn dup(&mut self);
    fn peek(&self) -> Option<StackValue<'gc>>;
}

// Typed push/pop helpers
trait TypedStackOps<'gc>: EvalStackOps<'gc> {
    fn push_i32(&mut self, value: i32) { self.push(StackValue::Int32(value)) }
    fn pop_i32(&mut self) -> i32 { /* ... */ }
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

| Module                        | Lines  | Priority | Notes                         |
|-------------------------------|--------|----------|-------------------------------|
| `dotnet-assemblies/src/`      | 800+   | High     | Assembly loading and resolution |
| `dotnet-utils/src/`           | 300+   | Medium   | Shared utility primitives     |
| `dotnet-macros/src/`          | 450+   | Medium   | Procedural macro definitions  |
| `dotnet-macros-core/src/`     | 200+   | Medium   | Macro expansion logic         |

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

### Current BCL Coverage

**Location:** `crates/dotnet-assemblies/src/support/`

**Files and their purpose:**

| File                     | Lines | Purpose                                                  |
|--------------------------|-------|----------------------------------------------------------|
| `Array.cs`               | 236   | `System.Array` stub with IList, ICloneable               |
| `RuntimeType.cs`         | 210   | Custom `DotnetRs.RuntimeType` implementing `System.Type` |
| `Span.cs`                | 170   | `System.Span<T>` and `ReadOnlySpan<T>`                   |
| `Comparers/Equality.cs`  | 100   | `EqualityComparer<T>` for collections                    |
| `MulticastDelegate.cs`   | 70    | Delegate invocation list support                         |
| `MethodInfo.cs`          | 60    | `System.Reflection.MethodInfo` stub                      |
| `ConstructorInfo.cs`     | 55    | `System.Reflection.ConstructorInfo` stub                 |
| `FieldInfo.cs`           | 50    | `System.Reflection.FieldInfo` stub                       |
| `Delegate.cs`            | 45    | Base delegate support                                    |
| `RuntimeMethodHandle.cs` | 45    | Method handle wrapper                                    |
| `RuntimeTypeHandle.cs`   | 40    | Type handle wrapper                                      |
| `RuntimeFieldHandle.cs`  | 40    | Field handle wrapper                                     |
| `Assembly.cs`            | 5     | Assembly representation                                  |
| `Module.cs`              | 3     | Module stub                                              |
| `StubAttribute.cs`       | 7     | `[Stub]` attribute for type replacement                  |

**Well-implemented areas:**
- ✅ Array operations (indexing, copying, enumerating)
- ✅ Reflection basics (Type, MethodInfo, FieldInfo)
- ✅ Span/ReadOnlySpan with proper memory semantics
- ✅ Delegate and MulticastDelegate

### Missing Critical APIs

**High Priority (frequently used in .NET code):**

1. **`System.String` operations**
   - Currently handled via intrinsics, but no C# stub
   - Missing: `Substring`, `IndexOf`, `Split`, `Join`

2. **`System.Collections.Generic`**
   - `List<T>` - very commonly used
   - `Dictionary<TKey, TValue>` - hash table
   - `HashSet<T>`

3. **`System.Linq`**
   - `Enumerable` extension methods
   - Critical for modern C# code

4. **`System.Threading.Tasks`**
   - `Task` and `Task<T>`
   - `async/await` machinery

5. **`System.IO`**
   - `Stream`, `MemoryStream`
   - File operations (if supporting filesystem access)

**Medium Priority:**

6. **Exception types beyond base**
   - `ArgumentNullException`, `InvalidOperationException`, etc.
   - Currently many throw `NotImplementedException`

7. **`System.Numerics`**
   - `Vector<T>` for SIMD
   - `BigInteger`

### Code Style Consistency

**Positive observations:**
- ✅ File-scoped namespaces (C# 10+): `namespace DotnetRs;`
- ✅ Nullable reference types enabled
- ✅ `ArgumentNullException.ThrowIfNull()` (C# 10+)
- ✅ `ArgumentOutOfRangeException.ThrowIfNegative()` (C# 10+)
- ✅ Primary constructors in some places
- ✅ Expression-bodied members where appropriate

**Areas for improvement:**

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
   // Current
   public extern int Length { get; }
   
   // Better
   /// <summary>Gets the total number of elements in the array.</summary>
   public extern int Length { get; }
   ```

3. **Consider using `required` modifier (C# 11)**
   For initialization-required properties in stubs

4. **Consider `init` accessors** for immutable-after-construction properties

### C# Modernization Opportunities

The support library could benefit from:

1. **Static abstract interface members (C# 11)** for numeric interfaces
2. **Collection expressions (C# 12)** when implementing collection methods
3. **Primary constructors (C# 12)** for simpler type definitions

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

**Under-tested areas:**

| Area        | Current Tests | Recommended Additions              |
|-------------|---------------|------------------------------------|
| Collections | 0             | List, Dictionary operations        |
| LINQ        | 0             | Basic query operations             |
| Async/Await | 0             | Task-based async patterns          |
| Memory      | Partial       | Allocation patterns, large objects |
| Interop     | Limited       | More P/Invoke scenarios            |
| Edge cases  | Limited       | Overflow, NaN, infinity            |

**Missing unit tests in Rust code:**

1. **`dotnet-vm/src/resolver.rs`** - Complex type resolution logic with no unit tests
2. **`dotnet-vm/src/layout.rs`** - Layout calculations could have pure unit tests
3. **`dotnet-types/src/comparer.rs`** - Type comparison logic (526 lines, no tests)
4. **`dotnet-value/src/`** - Value conversions could have isolated tests

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

**Improvement Opportunities:**

1. **Add unit test modules to Rust crates**
   ```
   crates/dotnet-vm/src/
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

| Category       | Purpose             | Example Tests                            |
|----------------|---------------------|------------------------------------------|
| `edge_cases/`  | Boundary conditions | Max/min integers, empty arrays           |
| `regression/`  | Bug fixes           | Tests for specific fixed issues          |
| `performance/` | Timing-sensitive    | Tests with expected perf characteristics |
| `stress/`      | High-load scenarios | Deep recursion, many allocations         |
| `compat/`      | .NET compatibility  | Behavior matching CoreCLR                |

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

4. **Handle `StepResult::Error` in execution loop** (`executor.rs`)

5. **Migrate high-priority unwrap/expect calls** (see Error Handling section)

6. **Run full test suite** to ensure no panics converted to errors break tests

---

### Phase 3: Architectural Changes (1-2 weeks each)

These are significant refactors that touch many files.

---

#### 3.1 Remove GC Handle Parameter Threading

**Effort:** 1-2 weeks  
**Risk:** High  
**Impact:** Nearly every file in dotnet-vm

**Steps:**

1. **Add `gc` field to `VesContext`**:
   ```rust
   pub struct VesContext<'a, 'gc, 'm> {
       gc: GCHandle<'gc>,  // NEW
       // ... existing fields
   }
   ```

2. **Add `gc()` accessor**:
   ```rust
   impl VesContext {
       pub fn gc(&self) -> GCHandle<'gc> { self.gc }
   }
   ```

3. **Create transitional API** - add new methods without `gc` param:
   ```rust
   // Old (keep for now)
   fn push(&mut self, gc: GCHandle<'gc>, value: StackValue<'gc>);
   // New
   fn push_v2(&mut self, value: StackValue<'gc>);
   ```

4. **Migrate call sites incrementally** by file/module

5. **Update trait definitions** to remove `gc` parameter

6. **Remove old methods** after all callers updated

7. **Run full test suite** at each step

**Alternative approach:** Use a feature flag to toggle between old/new API during migration.

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
*Last updated: 2026-02-08*
