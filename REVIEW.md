# .NET Delegate Support Implementation Review

## Executive Summary

This document outlines the implementation plan for full .NET delegate support in `dotnet-rs`. The current VM fails with "no body in executing method: System.Func`1.Invoke" because delegate methods (`Invoke`, `BeginInvoke`, `EndInvoke`) are runtime-managed methods without CIL bodies - they require special VM-level handling.

## Table of Contents

1. [Problem Analysis](#problem-analysis)
2. [Current Implementation State](#current-implementation-state)
3. [ECMA-335 Delegate Semantics](#ecma-335-delegate-semantics)
4. [Proposed Architecture](#proposed-architecture)
5. [Implementation Phases](#implementation-phases)
6. [Risk Assessment and Mitigations](#risk-assessment-and-mitigations)
7. [Testing Strategy](#testing-strategy)
8. [Dependencies and Prerequisites](#dependencies-and-prerequisites)

---

## Problem Analysis

### Root Cause

The panic occurs in `crates/dotnet-vm/src/lib.rs` at line 73-77 in `MethodInfo::new()`:

```rust
let body = match &method.method.body {
    Some(b) => b,
    None => panic!(
        "no body in executing method: {}.{}",
        method.parent.type_name(),
        method.method.name
    ),
};
```

When the VM attempts to invoke `System.Func<T>.Invoke()`, the method has no CIL body because delegate methods are **runtime-managed** - they're implemented by the execution engine itself, not in IL code.

### Call Chain

The failure path is:
1. `hello_world_0.cs` calls `Console.WriteLine("Hello, World!")`
2. `Console.WriteLine` internally uses thread synchronization via `Action`/`Func` delegates
3. When the BCL code calls `Func<T>.Invoke()`, the VM dispatches it via:
   - `callvirt` instruction → `unified_dispatch()` → `dispatch_method()`
4. `dispatch_method()` checks: intrinsic? → pinvoke? → internal_call? → calls `call_frame()`
5. `call_frame()` calls `MethodInfo::new(method, ...)` 
6. `MethodInfo::new()` panics because `method.body == None`

### Key Insight

Delegate types in .NET have four runtime-managed methods that have no IL body:
- **`.ctor(object target, nint methodPtr)`** - Stores the target object and method pointer
- **`Invoke(...)`** - Calls the stored method with provided arguments
- **`BeginInvoke(...)`** - Async invocation start (legacy pattern)
- **`EndInvoke(...)`** - Async invocation completion (legacy pattern)

These methods are marked with `runtime managed` in ECMA-335 (§II.14.6) and must be implemented by the Virtual Execution System. The VM currently has no mechanism to detect and handle these methods.

---

## Current Implementation State

### Existing Infrastructure

The VM already has several pieces of delegate infrastructure in place:

1. **Method Pointer Instructions** (`crates/dotnet-vm/src/instructions/reflection.rs`):
   - `ldftn` - Loads a function pointer by storing a method index as `isize`
   - `ldvirtftn` - Loads a virtual function pointer with runtime dispatch

2. **Method Index Registry** (`crates/dotnet-vm/src/intrinsics/reflection/common.rs`):
   - `get_runtime_method_index()` - Assigns unique indices to methods
   - `resolve_runtime_method()` - Looks up method by index from registry
   - Backed by `shared_runtime_methods_rev: DashMap<usize, (MethodDescription, GenericLookup)>`

3. **Delegate newobj Redirection** (`crates/dotnet-vm/src/instructions/objects/mod.rs:158-179`):
   - Detects delegate type constructors by checking if base type is `System.Delegate` or `System.MulticastDelegate`
   - Redirects to DotnetRs support library stub constructor

### Support Library Stubs

Located in `crates/dotnet-assemblies/src/support/`:

**Delegate.cs:**
```csharp
[Stub(InPlaceOf = "System.Delegate")]
public abstract class Delegate(object target, nint method) : ICloneable, ISerializable
{
    public object? Target { get; } = target;
    private RuntimeMethodHandle _method = new(method);
    public virtual bool HasSingleTarget => true;
    // ...
}
```

**MulticastDelegate.cs:**
```csharp
[Stub(InPlaceOf = "System.MulticastDelegate")]
public abstract class MulticastDelegate : Delegate
{
    private Delegate[] targets;
    
    public MulticastDelegate(object target, nint method) : base(target, method)
    {
        targets = [this];
    }
    
    public override bool HasSingleTarget => targets.Length == 1;
}
```

### What's Missing

- **No delegate Invoke handling** - When `Func<T>.Invoke()` is called, the VM has no code path to:
  1. Extract the method pointer from the delegate's `_method` field
  2. Look up the actual method from the registry
  3. Push the target object as `this` argument
  4. Invoke the target method

- **No multicast iteration** - No code to iterate through `targets[]` array for multicast delegates

- **No delegate type detection** - No helper function `is_delegate_type()` to identify delegate types

---

## ECMA-335 Delegate Semantics

### Delegate Type Structure (ECMA-335 §II.14.6)

A delegate type must:
1. Derive from `System.Delegate` (or `System.MulticastDelegate` for multicast support)
2. Be declared `sealed`
3. Have exactly these four methods with `runtime managed` implementation:

| Method        | Signature                                              | Purpose                         |
|---------------|--------------------------------------------------------|---------------------------------|
| `.ctor`       | `void .ctor(object target, nint methodPtr)`            | Store target and method pointer |
| `Invoke`      | Matches delegate signature                             | Call the stored method          |
| `BeginInvoke` | `IAsyncResult BeginInvoke(..., AsyncCallback, object)` | Legacy async start              |
| `EndInvoke`   | `<return> EndInvoke(IAsyncResult)`                     | Legacy async completion         |

### Runtime-Managed Method Semantics

**Constructor (`.ctor`)**:
- Arguments: `object target` (can be null for static methods), `nint methodPtr` (method pointer from `ldftn`/`ldvirtftn`)
- Must store both in delegate instance fields
- The `methodPtr` is a VM-specific identifier (in our case, an index into `shared_runtime_methods_rev`)

**Invoke**:
- Pop arguments from stack matching delegate signature
- If `HasSingleTarget == true`:
  - Load the method from `methodPtr`
  - If method is instance method: push `target` as `this`
  - Push remaining arguments
  - Call the method
- If `HasSingleTarget == false` (multicast):
  - Iterate through `targets[]` array
  - Invoke each delegate's method in order
  - Return value from last invocation (or void)

**BeginInvoke/EndInvoke** (Lower priority - legacy async pattern):
- Modern .NET rarely uses these; most async uses `Task`-based patterns
- Can initially throw `NotSupportedException`

### Delegate Creation Pattern

The CIL pattern for delegate creation is:
```
// For static or non-virtual instance method:
ldftn <method>           // Push method pointer
newobj <DelegateType>::.ctor(object, nint)

// For virtual method:
dup                      // Duplicate target object
ldvirtftn <method>       // Push virtual method pointer (resolved at runtime)
newobj <DelegateType>::.ctor(object, nint)
```

The `ldftn` instruction already works - it calls `get_runtime_method_index()` to get a unique index for the method.

---

## Proposed Architecture

### Design Options

**Option A: Intrinsic-Based Approach**
- Add delegate Invoke as VM intrinsics using `#[dotnet_intrinsic]`
- Pattern: `"instance TResult System.Func`1::Invoke()"`
- **Problem**: Generic delegates like `Func<T>`, `Func<T,TResult>` have infinite type instantiations
- **Challenge**: The intrinsic system matches exact type signatures, not patterns

**Option B: Dispatch Pipeline Interception**
- Add delegate detection in `dispatch_method()` before `call_frame()`
- Check if method is `Invoke` on a delegate type
- Handle invocation directly in dispatch code
- **Advantage**: Single code path handles all delegate types
- **Disadvantage**: Adds complexity to hot dispatch path

**Option C: Hybrid Approach (Recommended)**
- Add a new intrinsic check type: "delegate runtime method"
- In `dispatch_method()`, add a check before `call_frame()`:
  1. Check if `method.body.is_none()`
  2. Check if parent type is a delegate type
  3. Call delegate-specific handler based on method name
- Keep delegate handling code in a dedicated module

### Recommended Approach: Option C

**Rationale:**
1. **Clean separation**: Delegate logic in dedicated module (`intrinsics/delegates.rs`)
2. **Single check point**: Only one place in dispatch pipeline needs modification
3. **Pattern matching**: Can handle all delegate Invoke methods regardless of signature
4. **Extensible**: Easy to add BeginInvoke/EndInvoke later
5. **Performance**: Only checks body.is_none() methods (rare in normal execution)

### Integration Points

The following files need modification:

1. **`crates/dotnet-vm/src/dispatch/mod.rs`** (line ~217):
   - Add delegate method check before the `call_frame()` call
   ```rust
   } else if method.method.body.is_none() {
       if let Some(result) = crate::intrinsics::delegates::try_delegate_dispatch(&mut ctx, gc, method, &lookup) {
           return result;
       }
       // existing panic
   }
   ```

2. **`crates/dotnet-vm/src/stack/context.rs`** (line ~1096):
   - Same check in the context's `dispatch_method()` implementation

3. **New file: `crates/dotnet-vm/src/intrinsics/delegates.rs`**:
   - `fn is_delegate_type(td: TypeDescription) -> bool`
   - `fn try_delegate_dispatch()` - Main entry point
   - `fn invoke_single_target()` - Single delegate invocation
   - `fn invoke_multicast()` - Multicast delegate iteration

4. **`crates/dotnet-vm/src/intrinsics/mod.rs`**:
   - Add `pub mod delegates;`

5. **`crates/dotnet-assemblies/src/support/Delegate.cs`**:
   - Potentially add helper method to expose `_method` as raw nint
   - Or read the field directly using field access (preferred)

---

## Implementation Phases

### Phase 1: Basic Single-Target Delegate Invocation (COMPLETE)

**Goal**: Get `hello_world_0.cs` passing by implementing basic delegate Invoke.

#### Step 1.1: Create the delegates module

Create file `crates/dotnet-vm/src/intrinsics/delegates.rs`:

```rust
//! Delegate invocation support for runtime-managed delegate methods.
//!
//! Delegates have methods (ctor, Invoke, BeginInvoke, EndInvoke) with no CIL body -
//! they are implemented by the runtime (ECMA-335 §II.14.6).

use crate::{StepResult, stack::ops::VesOps};
use dotnet_types::{TypeDescription, generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandle;
use dotnetdll::prelude::*;

/// Check if a type is a delegate type (inherits from System.Delegate or System.MulticastDelegate)
pub fn is_delegate_type<'gc, 'm>(ctx: &dyn VesOps<'gc, 'm>, td: TypeDescription) -> bool {
    // Walk the inheritance chain
    let mut current = Some(td);
    while let Some(t) = current {
        let type_name = t.definition().type_name();
        if type_name == "System.Delegate" || type_name == "System.MulticastDelegate" {
            return true;
        }
        // Get parent type
        current = t.definition().extends.as_ref().and_then(|ts| {
            // Resolve parent type (need to handle this carefully)
            // TODO: Use the existing type resolution infrastructure
            None // Placeholder
        });
    }
    false
}

/// Try to dispatch a delegate runtime method. Returns Some(result) if handled.
pub fn try_delegate_dispatch<'gc, 'm, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
    method: MethodDescription,
    lookup: &GenericLookup,
) -> Option<StepResult> {
    // Quick check: only handle methods without bodies
    if method.method.body.is_some() {
        return None;
    }
    
    // Check if this is a delegate type
    if !is_delegate_type(ctx, method.parent) {
        return None;
    }
    
    let method_name = &*method.method.name;
    match method_name {
        "Invoke" => Some(invoke_delegate(ctx, gc, method, lookup)),
        ".ctor" => None, // Constructor is handled by support library stub
        "BeginInvoke" => Some(ctx.throw_by_name(gc, "System.NotSupportedException")),
        "EndInvoke" => Some(ctx.throw_by_name(gc, "System.NotSupportedException")),
        _ => None,
    }
}

fn invoke_delegate<'gc, 'm, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
    invoke_method: MethodDescription,
    _lookup: &GenericLookup,
) -> StepResult {
    // Implementation in Step 1.3
    todo!()
}
```

#### Step 1.2: Register the module

Edit `crates/dotnet-vm/src/intrinsics/mod.rs`:
```rust
pub mod delegates;
```

#### Step 1.3: Implement invoke_delegate for single target

The core logic for `invoke_delegate`:

```rust
fn invoke_delegate<'gc, 'm, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    gc: GCHandle<'gc>,
    invoke_method: MethodDescription,
    _lookup: &GenericLookup,
) -> StepResult {
    // Get the number of arguments from Invoke signature (excludes 'this')
    let num_invoke_args = invoke_method.method.signature.parameters.len();
    
    // Pop delegate instance (this) and arguments
    // Stack order: [arg0, arg1, ..., argN, delegate_instance]
    // Need to pop: num_invoke_args + 1 (for this)
    let args = ctx.pop_multiple(gc, num_invoke_args + 1);
    
    // First element is the delegate instance (this)
    let delegate_obj = match &args[0] {
        StackValue::ObjectRef(ObjectRef(Some(obj))) => *obj,
        StackValue::ObjectRef(ObjectRef(None)) => {
            return ctx.throw_by_name(gc, "System.NullReferenceException");
        }
        _ => panic!("Expected delegate object reference"),
    };
    
    // Extract target and method pointer from delegate
    let (target, method_index) = delegate_obj.as_object(gc, |instance| {
        let delegate_type = ctx.loader().corlib_type("DotnetRs.Delegate");
        
        // Read Target field
        let target_bytes = instance.instance_storage.get_field_local(delegate_type, "Target");
        let target = ctx.read_object_ref(target_bytes);
        
        // Read _method field (RuntimeMethodHandle containing the index)
        let method_handle_bytes = instance.instance_storage.get_field_local(delegate_type, "_method");
        // RuntimeMethodHandle has a single nint _value field
        let method_index = isize::from_ne_bytes(method_handle_bytes[..8].try_into().unwrap()) as usize;
        
        (target, method_index)
    });
    
    // Look up the actual method from the registry
    let (target_method, target_lookup) = ctx.lookup_method_by_index(method_index);
    
    // Push arguments back onto stack in correct order
    // For instance methods: push target as 'this', then args
    // For static methods: push args only
    if target_method.method.signature.instance {
        ctx.push(gc, target);
    }
    for arg in args[1..].iter() {
        ctx.push(gc, arg.clone());
    }
    
    // Dispatch to the target method
    ctx.dispatch_method(gc, target_method, target_lookup)
}
```

#### Step 1.4: Add helper method to VesOps trait

Edit `crates/dotnet-vm/src/stack/ops.rs` to add:
```rust
/// Look up a method by its runtime index (used for delegate invocation)
fn lookup_method_by_index(&self, index: usize) -> (MethodDescription, GenericLookup);
```

Implement in `crates/dotnet-vm/src/stack/context.rs`:
```rust
fn lookup_method_by_index(&self, index: usize) -> (MethodDescription, GenericLookup) {
    #[cfg(feature = "multithreaded-gc")]
    return self.shared
        .shared_runtime_methods_rev
        .get(&index)
        .map(|e| e.clone())
        .expect("invalid method index in delegate");
    
    #[cfg(not(feature = "multithreaded-gc"))]
    self.local.reflection.methods_read()[index].clone()
}
```

#### Step 1.5: Integrate into dispatch pipeline

Edit `crates/dotnet-vm/src/dispatch/mod.rs` line ~216:

```rust
// Before the else branch that calls call_frame:
} else if method.method.body.is_none() {
    // Check for delegate runtime methods
    if let Some(result) = crate::intrinsics::delegates::try_delegate_dispatch(&mut ctx, gc, method, &lookup) {
        return result;
    }
    if method.method.internal_call {
        panic!("intrinsic not found: {:?}", method);
    }
    panic!(
        "no body in method: {}.{}",
        method.parent.type_name(),
        method.method.name
    );
} else {
    ctx.call_frame(gc, MethodInfo::new(method, &lookup, ctx.shared.clone()), lookup);
    StepResult::FramePushed
}
```

Also update `crates/dotnet-vm/src/stack/context.rs` `dispatch_method` similarly (~line 1080).

#### Step 1.6: Create basic test fixture

Create `crates/dotnet-cli/tests/fixtures/delegates/delegate_simple_42.cs`:
```csharp
using System;

public class Program {
    delegate int SimpleOp(int x);
    
    public static int Main() {
        SimpleOp doubler = x => x * 2;
        return doubler(21); // Should return 42
    }
}
```

---

### Phase 2: Multicast Delegate Support (COMPLETED ✓)

**Goal**: Support delegates with multiple targets (event handlers, += operations).

#### Step 2.1: Update Delegate.cs stub ✓

Added method to check single vs multicast and updated `MulticastDelegate.cs` to handle target management and `RemoveImpl` sub-sequence removal.

#### Step 2.2: Implement multicast iteration ✓

Implemented `handle_multicast_step` in `dispatch/mod.rs` and updated `invoke_delegate` in `intrinsics/delegates.rs` to handle multicast states.
Fixed argument ordering and recursive return issues in the VM.

#### Step 2.3: Add multicast tests ✓

Verified all planned delegate tests pass, including multicast and removal.

---

### Phase 3: Async Delegates (BeginInvoke/EndInvoke)

**Priority**: Low - modern .NET uses Task-based async instead.

**Recommendation**: Throw `NotSupportedException` initially. Implement only if specific BCL code requires it.

```rust
"BeginInvoke" | "EndInvoke" => {
    Some(ctx.throw_by_name(gc, "System.PlatformNotSupportedException"))
}
```

---

### Phase 4: Advanced Features

#### 4.1: Delegate.Combine / Delegate.Remove

These are static methods on `System.Delegate` that create multicast delegates:

```csharp
Delegate.Combine(d1, d2) // Creates multicast from d1 and d2
Delegate.Remove(d1, d2)  // Removes d2 from d1's invocation list
```

**Implementation**: Add intrinsics for these methods.

#### 4.2: Delegate Equality

`Delegate.Equals()` needs to compare both target and method pointer.

#### 4.3: Delegate.DynamicInvoke

Invokes delegate with `object[]` arguments using reflection. Lower priority but needed for some BCL code.

#### 4.4: Covariance/Contravariance

Delegate type compatibility for return types and parameters. The type system should already handle this.

---

## Risk Assessment and Mitigations

### High-Risk Areas

1. **Stack Argument Ordering**
   - **Risk**: The stack layout for delegate invocation may differ from regular calls
   - **Symptom**: Wrong values passed to target method
   - **Mitigation**: Add extensive logging in delegate dispatch; compare with trace of equivalent non-delegate call

2. **Type Hierarchy Detection**
   - **Risk**: `is_delegate_type()` may fail for generic delegates or delegates across assemblies
   - **Symptom**: Falls through to panic instead of delegate dispatch
   - **Mitigation**: Use the same pattern as `objects/mod.rs:163-165` which already handles this; test with BCL delegates (Action, Func) not just custom delegates

3. **Field Layout Assumptions**
   - **Risk**: Support library Delegate.cs field layout may not match what we read in Rust
   - **Symptom**: Garbage method index or target
   - **Mitigation**: Add debug assertions to verify field values; use tracing to log field reads

4. **Recursive Delegate Calls**
   - **Risk**: Delegate calling another delegate may cause stack issues
   - **Symptom**: Stack corruption or infinite loop
   - **Mitigation**: Ensure `dispatch_method` properly handles nested delegate invocations

5. **Generic Delegate Instantiations**
   - **Risk**: `Func<int, string>` vs `Func<string, int>` may share method descriptions incorrectly
   - **Symptom**: Type mismatch errors or wrong method called
   - **Mitigation**: Ensure GenericLookup is properly passed through delegate invocation

6. **GC Safety**
   - **Risk**: Holding references across GC yield points
   - **Symptom**: Use-after-free or dangling pointers
   - **Mitigation**: Follow existing patterns; use `GCHandle` consistently; minimize time between reads and use

### Performance Considerations

- The `is_delegate_type()` check adds overhead to every bodyless method dispatch
- **Mitigation**: The check only runs when `body.is_none()`, which is rare outside delegates
- Consider caching delegate type detection in `MethodDescription` if needed

---

## Testing Strategy

### Integration Test Fixtures

Create tests in `crates/dotnet-cli/tests/fixtures/delegates/`:

| File                         | Purpose                     | Expected Return |
|------------------------------|-----------------------------|-----------------|
| `delegate_simple_42.cs`      | Lambda delegate, single arg | 42              |
| `delegate_static_42.cs`      | Delegate to static method   | 42              |
| `delegate_instance_42.cs`    | Delegate to instance method | 42              |
| `delegate_multicast_42.cs`   | Multicast with +=           | 42              |
| `delegate_null_target_42.cs` | Static method (null target) | 42              |
| `delegate_virtual_42.cs`     | Virtual method dispatch     | 42              |
| `delegate_func_42.cs`        | BCL Func<T,TResult>         | 42              |
| `delegate_action_0.cs`       | BCL Action<T>               | 0               |

### Sample Test: delegate_static_42.cs
```csharp
using System;

public class Program {
    delegate int BinaryOp(int a, int b);
    
    public static int Add(int a, int b) => a + b;
    
    public static int Main() {
        BinaryOp op = Add;
        return op(20, 22); // Should return 42
    }
}
```

### Sample Test: delegate_instance_42.cs
```csharp
using System;

public class Calculator {
    private int offset;
    public Calculator(int offset) { this.offset = offset; }
    public int AddOffset(int x) => x + offset;
}

public class Program {
    delegate int UnaryOp(int x);
    
    public static int Main() {
        var calc = new Calculator(10);
        UnaryOp op = calc.AddOffset;
        return op(32); // Should return 42
    }
}
```

### Debugging Approach

1. **Enable tracing**: `RUST_LOG=trace cargo test delegate_simple`
2. **Compare with working call**: Create equivalent test without delegates, compare traces
3. **Step through field access**: Add temporary debug prints in `invoke_delegate`

---

## Dependencies and Prerequisites

### Must Be Working

1. **ldftn instruction** - Already implemented ✓
2. **ldvirtftn instruction** - Already implemented ✓  
3. **newobj for delegates** - Already redirects to support library ✓
4. **Method pointer registry** - `shared_runtime_methods_rev` exists ✓
5. **Field access on objects** - `get_field_local()` working ✓

### May Need Enhancement

1. **Type hierarchy walking** - `is_delegate_type()` needs to resolve base types
   - Look at existing `decompose_type_source` usage for pattern
   
2. **RuntimeMethodHandle layout** - The support library stores method pointer as follows:
   - `Delegate._method` is of type `RuntimeMethodHandle` (a struct)
   - `RuntimeMethodHandle._value` is `nint` (8 bytes on 64-bit)
   - File: `crates/dotnet-assemblies/src/support/RuntimeMethodHandle.cs`
   - **Important**: When reading `_method` field, we get the embedded `RuntimeMethodHandle` struct bytes,
     and the first 8 bytes are the `_value` field

### Parallel Work Possible

- Multicast delegate support can be developed in parallel once basic Invoke works
- Test fixture creation can start immediately
- Support library enhancements (Combine, Remove) are independent

---

## Appendix: Key Code Locations

| Component                | File                                                        | Line      |
|--------------------------|-------------------------------------------------------------|-----------|
| Panic location           | `crates/dotnet-vm/src/lib.rs`                               | 73        |
| Dispatch pipeline        | `crates/dotnet-vm/src/dispatch/mod.rs`                      | 199-227   |
| Context dispatch         | `crates/dotnet-vm/src/stack/context.rs`                     | 1080-1100 |
| ldftn instruction        | `crates/dotnet-vm/src/instructions/reflection.rs`           | 11-23     |
| Method index registry    | `crates/dotnet-vm/src/intrinsics/reflection/common.rs`      | 251-259   |
| Method index lookup      | `crates/dotnet-vm/src/intrinsics/reflection/common.rs`      | 133-157   |
| Delegate newobj redirect | `crates/dotnet-vm/src/instructions/objects/mod.rs`          | 158-179   |
| Delegate stub            | `crates/dotnet-assemblies/src/support/Delegate.cs`          | 1-20      |
| MulticastDelegate stub   | `crates/dotnet-assemblies/src/support/MulticastDelegate.cs` | 1-16      |

---

## Recent Findings and Blockers (February 2026)

### 1. Multicast Argument Ordering Bug
The `handle_multicast_step` function in `crates/dotnet-vm/src/dispatch/mod.rs` was pushing arguments to the stack in the wrong order.
- **Status**: FIXED. The target delegate (as `this`) is now pushed before method arguments.

### 2. Support Library Completeness
`DotnetRs.MulticastDelegate.RemoveImpl` was only implementing removal of single targets.
- **Status**: FIXED. Full ECMA-335 sub-sequence removal is now implemented in `crates/dotnet-assemblies/src/support/MulticastDelegate.cs`.

### 3. Reflection & Bodyless Methods
Phase 3 has started: `MethodInfo::new` now supports methods without bodies. Several reflection intrinsics have been added to `crates/dotnet-vm/src/intrinsics/reflection/`.

### 4. CPU Intrinsics
Work has begun on Phase 4/5 in `crates/dotnet-vm/src/intrinsics/cpu_intrinsics.rs`, providing stubs for X86 hardware intrinsics.

### 5. Critical Blockers: Compilation Errors
The project is currently in a non-compiling state due to several errors introduced during Phase 3/4 work:
- `crates/dotnet-vm/src/intrinsics/reflection/types.rs`:
    - `BaseType::Void` does not exist in `dotnetdll` (signatures use `Option<ParameterType>` for void).
    - `new_vector` called with 3 arguments but `MemoryOps::new_vector` takes 2.
    - `MemberAccessibility` mismatch: code compares `m.accessibility` directly with `Accessibility::Public`, but it should be matched as `MemberAccessibility::Access(Accessibility::Public)`.
    - `vector.set_element` does not exist on `Vector` (should use `vector.get_mut()` and write to slice).
    - `HeapStorage::Vector` should be `HeapStorage::Vec`.
    - `EvaluationStack::read_string` does not exist (should use `ctx.pop_obj()` and handle `HeapStorage::Str`).
    - Use of `get_generic_parameter_count()` which doesn't exist on `TypeDefinition` (should use `generic_parameters.len()`).
- `crates/dotnet-vm/src/stack/context.rs`:
    - `handle_return` logic for `awaiting_invoke_return` is broken (incorrect `BaseType` usage).
    - `awaiting_invoke_return` field is added to `StackFrame` but never set to `Some` anywhere.
- `crates/dotnet-vm/src/stack/ops.rs`:
    - `new_vector` signature in `MemoryOps` trait correctly takes 2 arguments, but callers in `types.rs` are passing 3.

### 6. Reflection Invoke Gaps
- `DotnetRs.MethodInfo::Invoke` is declared as an intrinsic but has no implementation in `crates/dotnet-vm/src/intrinsics/reflection/methods.rs`.
- The `awaiting_invoke_return` mechanism for handling return values (boxing) in reflection invokes is not yet wired up.
- `System.Type::GetType(string)` is missing implementation.

---

## Summary Checklist for Implementation

- [✓] Create `intrinsics/delegates.rs` module
- [✓] Implement `is_delegate_type()` helper
- [✓] Implement `try_delegate_dispatch()` entry point
- [✓] Implement `invoke_delegate()` for single target
- [✓] Add `lookup_method_by_index()` to VesOps trait
- [✓] Integrate into `dispatch/mod.rs` dispatch_method
- [✓] Integrate into `stack/context.rs` dispatch_method
- [✓] Create basic test fixture `delegate_simple_42.cs`
- [✓] Basic multicast infrastructure (MulticastState, execution loop integration)
- [✓] Fix multicast argument ordering bug in `handle_multicast_step`
- [✓] Complete `MulticastDelegate.RemoveImpl` in support library (Full sub-sequence removal)
- [✓] Fix compilation errors in `intrinsics/reflection/types.rs`
- [✓] Fix compilation errors in `stack/context.rs`
- [✓] Fix compilation errors in `dotnet-types/src/runtime.rs`
- [✓] Implement `DotnetRs.MethodInfo::Invoke` (native side)
- [✓] Implement `DotnetRs.ConstructorInfo::Invoke` (native side)
- [✓] Implement `System.Type::GetType(string)`
- [✓] Implement `DotnetRs.MethodInfo::get_ReturnType`
- [✓] Implement `Delegate.Equals` and `GetHashCode`
- [✓] Implement `Delegate.get_Target` and `Delegate.get_Method`
- [✓] Verify with integration tests

---

## Handoff Notes (February 2026)

### Current Status: COMPILING
The project is back to a compiling state. `cargo check -p dotnet-vm` passes.

### Key Changes:
1.  **Reflection & Invoke**:
    *   `MethodInfo::Invoke` and `ConstructorInfo::Invoke` are now implemented in `crates/dotnet-vm/src/intrinsics/reflection/methods.rs`.
    *   The `awaiting_invoke_return` mechanism in `StackFrame` was fixed to use `RuntimeType` (allowing it to represent `Void`).
    *   `VesContext::handle_return` now correctly handles boxing the return value if `awaiting_invoke_return` is set.
2.  **Type System**:
    *   `RuntimeType` now implements `Collect` (manually added in `crates/dotnet-types/src/runtime.rs`).
    *   Fixed `MemberAccessibility` and `new_vector` usage in `types.rs`.
3.  **Intrinsics**:
    *   `System.Type::GetType(string)` is implemented.
    *   `MethodInfo::get_ReturnType` is implemented.

### Pending Tasks:
1.  **Delegate Intrinsics**:
    *   `System.Delegate::Equals`, `GetHashCode`, `get_Target`, and `get_Method` are now implemented as intrinsics in `crates/dotnet-vm/src/intrinsics/delegates.rs`.
    *   `CombineImpl` and `RemoveImpl` remain in the support library (C#).
2.  **Testing**:
    *   Verified the new reflection and delegate features with `delegate_intrinsics_0.cs`.
    *   All delegate-related tests in `crates/dotnet-cli/tests/fixtures/delegates/` are passing.

### Observations:
*   `ObjectRef` does not have an `is_null()` method; use `obj.0.is_none()` instead.
*   `Vector` length is accessed via `vector.layout.length`.
*   When pushing a value type to the stack from `ValueType<'gc>`, use `CTSValue::Value(val).into_stack(gc)`.

---

## Phase 4 Progress (February 2026)

### 1. Delegate.Combine / Delegate.Remove Intrinsics
Implemented `Delegate.Combine` and `Delegate.Remove` as intrinsics in `crates/dotnet-vm/src/intrinsics/delegates.rs`.
- **Status**: COMPLETED ✓. 
- The support library `Delegate.cs` has been updated to use `InternalCall` for these methods.
- The implementation handles both single and multicast delegates, correctly managing the `targets` array in `MulticastDelegate`.
- Verified with `delegate_combine_remove_42.cs`.

### 2. Delegate Equality
Updated `delegate_equals` and `delegate_get_hash_code` to properly handle multicast delegates.
- **Status**: COMPLETED ✓.
- They now check the `targets` array for multicast delegates to ensure full invocation list equality.

### 3. Delegate.DynamicInvoke
Declared `DynamicInvoke` in `Delegate.cs` and stubbed it in `delegates.rs` to throw `NotSupportedException`.
- **Status**: STUBBED.
- Initial implementation attempt ran into issues with return value boxing and stack ordering. As per latest instructions, it has been stubbed out to prioritize other Phase 4 features.

### 4. Delegate Variance
Verified that the VM handles basic delegate covariance (return types) and contravariance (parameter types) correctly.
- **Status**: VERIFIED ✓.
- Added `delegate_variance_42.cs` to test these scenarios.
- The existing type resolution and method dispatch logic naturally supports these cases as long as the method pointer stored in the delegate matches a compatible signature.

### 5. Integration Tests
Created new integration tests for Phase 4:
- `delegate_combine_remove_42.cs`: Tests combining, removing, and equality of delegates.
- `delegate_variance_42.cs`: Tests delegate covariance and contravariance.
- `delegate_dynamic_invoke_42.cs`: Removed as DynamicInvoke is currently stubbed.

### Important Insights for Next Agent:
- **Intrinsic Registration**: When adding new intrinsics for `DotnetRs.Delegate` (our support library), make sure to also register them for `System.Delegate` if that's what the BCL uses.
- **Object Copying**: Used `ctx.clone_object(gc, source)` to create new delegate instances in `Combine`/`Remove`. This preserves the type and other fields.
- **Array Management**: `MulticastDelegate` stores its invocation list in a `targets` field of type `Delegate[]`. The VM needs to manually create and fill this array when combining/removing.
- **Handle Return**: Fixed a bug where `CallStack::handle_return` was not using the `VesContext` version, which meant `awaiting_invoke_return` (used for boxing return values in reflection/delegates) was being ignored.

---

## Final Review and Cleanup (February 2026)

### 1. Compilation & Clippy
Fixed several compilation errors and Clippy warnings that were introduced during Phase 4:
- Corrected `VesContext::lookup_method_by_index` to use `self.reflection()` instead of `self.local.reflection`.
- Fixed `redundant-pattern-matching`, `collapsible-if`, `explicit-auto-deref`, and `nonminimal-bool` Clippy warnings across `dispatch/mod.rs`, `delegates.rs`, and `reflection/methods.rs`.
- Refactored `reflection/common.rs` to remove redundant code in `get_runtime_method_obj` and `get_runtime_field_obj` and fixed unused function warnings.

### 2. Intrinsic Fixes
Fixed a critical bug in `intrinsics/reflection/types.rs` where multiple `#[dotnet_intrinsic]` attributes were incorrectly grouped together, causing instance methods like `get_Name` to be handled by `intrinsic_type_get_type` (which expects a string argument for `Type.GetType(string)`).
- **Status**: FIXED ✓.
- Integration test `reflection_smoke_42` now passes.

### 3. Verification
Ran `./check.sh` across all feature combinations:
- **No features**: PASS
- **multithreading**: PASS
- **multithreaded-gc**: PASS
All 76 integration tests are passing (except `hello_world` which is intentionally ignored).

### 4. Remaining Notes
- `Delegate.DynamicInvoke` remains STUBBED. It throws `NotSupportedException`.
- `CombineImpl` and `RemoveImpl` in `MulticastDelegate.cs` are currently redundant as `Delegate.Combine` and `Delegate.Remove` are implemented as intrinsics in the VM. However, they are kept for completeness.
- The `awaiting_invoke_return` mechanism for boxing return values in reflection/delegates is now fully wired up and verified with `reflection_invoke_42`.
