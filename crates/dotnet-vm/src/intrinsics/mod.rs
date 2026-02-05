//! This module handles .NET intrinsic methods.
//!
//! Intrinsics are methods implemented directly in the VM rather than in CIL.
//! They are used for:
//! 1. Performance (e.g., Math functions)
//! 2. Low-level operations (e.g., Unsafe, Memory manipulation)
//! 3. VM-specific functionality (e.g., Reflection, GC control)
//!
//! ## Architecture Overview
//!
//! The intrinsic system is built on a unified classification and dispatch pipeline:
//!
//! ### Intrinsic Classification
//!
//! All intrinsics are classified into three categories via [`IntrinsicKind`]:
//!
//! - **`Static`**: VM-only implementations with no BCL equivalent.
//!   - Examples: `GC.Collect`, `Monitor.Enter`, reflection internals.
//!   - These methods provide VM-specific functionality.
//!
//! - **`VirtualOverride`**: Runtime type-specific implementations.
//!   - Examples: `DotnetRs.RuntimeType` overriding `System.Type` methods.
//!   - Virtual dispatch selects the correct implementation based on runtime type.
//!   - Integrated with standard CIL virtual method resolution.
//!
//! - **`DirectIntercept`**: Must bypass BCL for correctness.
//!   - Examples: `String.get_Length`, `Unsafe.*`, `Buffer.Memmove`.
//!   - VM's internal representation differs from BCL expectations.
//!   - Must be intercepted even when BCL implementation exists.
//!
//! ### Virtual Dispatch Integration
//!
//! Virtual intrinsics are integrated into the standard virtual method resolution
//! flow in `resolve_virtual_method()` (see `src/stack/mod.rs`).
//!
//! When resolving a virtual call:
//! 1. Check if the target method has a `VirtualOverride` intrinsic for the runtime type.
//! 2. If yes, return the intrinsic override immediately (respects VMT cache).
//! 3. Otherwise, fall back to standard ancestor search.
//!
//! ### Unified Dispatch Pipeline
//!
//! All method calls flow through a unified dispatch pipeline (see `src/dispatch.rs`):
//! 1. **Generic resolution**: Resolve the method from instruction operand.
//! 2. **Virtual resolution**: Resolve the runtime type-specific method (if virtual).
//! 3. **Intrinsic check**: Use [`classify_intrinsic`] to check if the resolved method is an intrinsic.
//! 4. **Execution**: Dispatch to intrinsic handler, P/Invoke, or managed bytecode.
//!
//! Dispatch priority: **Intrinsics (including DirectIntercept) → P/Invoke → Managed**.
//!
//! ## Adding a New Intrinsic
//!
//! To implement a new intrinsic method:
//!
//! 1.  **Define the handler function**:
//!     Create a function with the following signature and apply the `#[dotnet_intrinsic]` attribute:
//!     ```rust,ignore
//!     #[dotnet_intrinsic("static double System.Math::Min(double, double)")]
//!     pub fn math_min_double<'gc, 'm: 'gc>(
//!         ctx: &mut VesContext<'_, 'gc, 'm>,
//!         gc: GCHandle<'gc>,
//!         method: MethodDescription,
//!         generics: &GenericLookup,
//!     ) -> StepResult { ... }
//!     ```
//!     Place this function in an appropriate submodule (e.g., `src/intrinsics/math.rs`).
//!
//! 2.  **Use the `#[dotnet_intrinsic]` attribute**:
//!     The attribute takes a C#-style signature string. This automatically registers the handler
//!     in the static PHF-based registry at build time.
//!
//!     The signature string format:
//!     `[static] <ReturnType> <Namespace>.<Type>::<MethodName>(<ParamTypes>)`
//!
//!     Examples:
//!     - `"static double System.Math::Sqrt(double)"`
//!     - `"int System.String::get_Length()"`
//!     - `"System.Type System.Type::GetTypeFromHandle(System.RuntimeTypeHandle)"`
//!
//! 3.  **Ensure the method is marked as intrinsic**:
//!     The method in the .NET assembly should be marked with `[IntrinsicAttribute]`
//!     or be an `InternalCall`.
//!
//! ## Dispatch Flow
//!
//! The complete dispatch flow for method calls:
//!
//! ```text
//! Call/CallVirtual instruction
//!   ↓
//! unified_dispatch()
//!   ↓
//! Stage 1: find_generic_method()
//!   ↓
//! Stage 2: resolve_virtual_method() [if virtual]
//!   ├─→ Check VirtualOverride intrinsics
//!   └─→ Standard ancestor search
//!   ↓
//! Stage 3: dispatch_method()
//!   ├─→ is_intrinsic_cached() [checks DirectIntercept, Static, VirtualOverride]
//!   ├─→ intrinsic_call() [if intrinsic]
//!   ├─→ external_call() [if P/Invoke]
//!   └─→ call_frame() [managed CIL]
//! ```
use crate::vm_trace_intrinsic;
use dotnet_assemblies::AssemblyLoader;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
};
use dotnet_utils::gc::GCHandle;
use std::sync::Arc;
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
    string::CLRString,
};

pub mod array_ops;
pub mod diagnostics;
pub mod gc;
pub mod math;
pub mod metadata;
pub mod reflection;
pub mod span;
pub mod static_registry;
pub mod string_ops;

include!(concat!(env!("OUT_DIR"), "/intrinsics_phf.rs"));
pub mod text_ops;
pub mod threading;
pub mod unsafe_ops;

pub use metadata::{IntrinsicKind, IntrinsicMetadata, classify_intrinsic};
pub use reflection::ReflectionExtensions;

use super::{StepResult, context::ResolutionContext, tracer::Tracer};

pub const INTRINSIC_ATTR: &str = "System.Runtime.CompilerServices.IntrinsicAttribute";

// ============================================================================
// Intrinsic Registry Infrastructure
// ============================================================================

/// Type alias for intrinsic handler functions.
///
/// An intrinsic handler is responsible for:
/// 1. Popping its arguments from the call stack
/// 2. Performing the intrinsic operation
/// 3. Pushing any return value onto the stack
/// 4. Returning a StepResult indicating the outcome
///
/// Note: The actual implementations should use `'m: 'gc` bound, but we can't
/// express this in higher-ranked trait bounds. The registry uses transmute
/// to work around this limitation safely.
///
/// GenericLookup is passed by reference to avoid cloning on every intrinsic call.
pub type IntrinsicHandler = for<'gc, 'm> fn(
    ctx: &mut crate::stack::VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult;

pub type IntrinsicFieldHandler = for<'gc, 'm> fn(
    ctx: &mut crate::stack::VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    field: FieldDescription,
    type_generics: Arc<[ConcreteType]>,
    is_address: bool,
) -> StepResult;

// Note on transmute safety:
// Throughout this file, we use `unsafe { std::mem::transmute::<_, IntrinsicHandler>(...) }`
// to convert concrete function pointers to the IntrinsicHandler type. This is safe because:
// 1. Both function types have identical memory layouts and ABI
// 2. We're converting between function pointers with the same parameter/return types
// 3. The only difference is the lifetime representation ('m: 'gc vs higher-ranked 'for')
// 4. The transmute is from a concrete function pointer to a higher-ranked one, which
//    is valid since the concrete function can be called with any valid lifetime combination
// 5. The Rust type system cannot express 'm: 'gc in higher-ranked trait bounds, but the
//    actual runtime behavior is identical

/// Registry for intrinsic method implementations.
///
/// This provides O(1) lookup of intrinsic handlers by MethodDescription,
/// replacing the older O(N) macro-based matching approach.
///
/// The registry is lazily initialized on first use via OnceLock.
pub struct IntrinsicRegistry;

impl IntrinsicRegistry {
    /// Initializes a new registry with intrinsic handlers.
    pub fn initialize(_loader: &AssemblyLoader, _tracer: Option<&mut Tracer>) -> Self {
        Self
    }

    /// Looks up an intrinsic handler for the given method.
    pub fn get(&self, method: &MethodDescription) -> Option<IntrinsicHandler> {
        self.get_metadata(method).map(|m| m.handler)
    }

    /// Looks up an intrinsic handler for the given field.
    pub fn get_field(&self, field: &FieldDescription) -> Option<IntrinsicFieldHandler> {
        let mut buf = [0u8; 512];
        let key = self.build_field_key(field, &mut buf)?;
        let range = INTRINSIC_LOOKUP.get(key)?;
        for entry in &INTRINSIC_ENTRIES[range.start..range.start + range.len] {
            if let StaticIntrinsicHandler::Field(h) = entry.handler {
                return Some(h);
            }
        }
        None
    }

    /// Looks up intrinsic metadata for the given method.
    /// Returns full metadata including kind and documentation.
    pub fn get_metadata(&self, method: &MethodDescription) -> Option<IntrinsicMetadata> {
        let mut buf = [0u8; 512];
        let key = self.build_method_key(method, &mut buf)?;
        let range = INTRINSIC_LOOKUP.get(key)?;
        for entry in &INTRINSIC_ENTRIES[range.start..range.start + range.len] {
            if let StaticIntrinsicHandler::Method(h) = entry.handler
                && entry.filter.is_none_or(|f| f(method))
            {
                // Map to IntrinsicMetadata
                let kind = if entry.is_static {
                    IntrinsicKind::Static
                } else {
                    IntrinsicKind::VirtualOverride
                };
                return Some(IntrinsicMetadata {
                    kind,
                    handler: h,
                    reason: "Registered via PHF",
                    signature_filter: entry.filter,
                });
            }
        }
        None
    }

    fn build_method_key<'a>(
        &self,
        method: &MethodDescription,
        buf: &'a mut [u8],
    ) -> Option<&'a str> {
        use std::fmt::Write;
        let mut w = StackWrite { buf, pos: 0 };
        let arity = if method.method.signature.instance {
            method.method.signature.parameters.len() + 1
        } else {
            method.method.signature.parameters.len()
        };
        write!(
            w,
            "M:{}::{}#{}",
            method.parent.type_name(),
            &*method.method.name,
            arity
        )
        .ok()?;
        let pos = w.pos;
        std::str::from_utf8(&buf[..pos]).ok()
    }

    fn build_field_key<'a>(&self, field: &FieldDescription, buf: &'a mut [u8]) -> Option<&'a str> {
        use std::fmt::Write;
        let mut w = StackWrite { buf, pos: 0 };
        write!(w, "F:{}::{}", field.parent.type_name(), &*field.field.name).ok()?;
        let pos = w.pos;
        std::str::from_utf8(&buf[..pos]).ok()
    }
}

struct StackWrite<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> std::fmt::Write for StackWrite<'a> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let bytes = s.as_bytes();
        if self.pos + bytes.len() > self.buf.len() {
            return Err(std::fmt::Error);
        }
        self.buf[self.pos..self.pos + bytes.len()].copy_from_slice(bytes);
        self.pos += bytes.len();
        Ok(())
    }
}

/// Checks if a method is an intrinsic that should be handled by the VM.
pub fn is_intrinsic(
    method: MethodDescription,
    loader: &AssemblyLoader,
    registry: &IntrinsicRegistry,
) -> bool {
    classify_intrinsic(method, loader, Some(registry)).is_some()
}

pub fn is_intrinsic_field(
    field: FieldDescription,
    loader: &AssemblyLoader,
    registry: &IntrinsicRegistry,
) -> bool {
    // Check registry first
    if registry.get_field(&field).is_some() {
        return true;
    }

    // Check for IntrinsicAttribute
    for a in &field.field.attributes {
        let ctor = loader.locate_attribute(field.parent.resolution, a);
        if ctor.parent.type_name() == INTRINSIC_ATTR {
            return true;
        }
    }

    false
}

pub fn intrinsic_call<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    ctx: &mut crate::stack::VesContext<'_, 'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _res_ctx =
        ResolutionContext::for_method(method, ctx.loader(), generics, ctx.shared.caches.clone(), Some(ctx.shared.clone()));

    vm_trace_intrinsic!(
        ctx,
        "CALL",
        &format!("{}.{}", method.parent.type_name(), method.method.name)
    );

    if let Some(metadata) = classify_intrinsic(
        method,
        ctx.loader(),
        Some(&ctx.shared.caches.intrinsic_registry),
    ) {
        return (metadata.handler)(ctx, gc, method, generics);
    }

    panic!("unsupported intrinsic {:?}", method);
}

pub fn intrinsic_field<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    ctx: &mut crate::stack::VesContext<'_, 'gc, 'm>,
    field: FieldDescription,
    type_generics: Arc<[ConcreteType]>,
    is_address: bool,
) -> StepResult {
    vm_trace_intrinsic!(
        ctx,
        "FIELD-LOAD",
        &format!("{}.{}", field.parent.type_name(), field.field.name)
    );
    if let Some(handler) = ctx.shared.caches.intrinsic_registry.get_field(&field) {
        handler(ctx, gc, field, type_generics, is_address)
    } else {
        panic!("unsupported load from intrinsic field: {:?}", field);
    }
}

#[dotnet_intrinsic("string System.Object::ToString()")]
fn object_to_string<'gc, 'm: 'gc>(
    ctx: &mut crate::stack::VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let this = ctx.pop(gc);

    let type_name = if let StackValue::ObjectRef(obj_ref) = this {
        if obj_ref.0.is_some() {
            obj_ref.as_heap_storage(|storage| match storage {
                HeapStorage::Obj(o) => o.description.type_name(),
                HeapStorage::Str(_) => "System.String".to_string(),
                HeapStorage::Vec(_) => "System.Array".to_string(),
                HeapStorage::Boxed(_) => "System.ValueType".to_string(),
            })
        } else {
            return ctx.throw_by_name(gc, "System.NullReferenceException");
        }
    } else {
        "System.Object".to_string()
    };

    let str_val = CLRString::from(type_name);
    let storage = HeapStorage::Str(str_val);
    let obj_ref = ObjectRef::new(gc, storage);
    ctx.push_obj(gc, obj_ref);
    StepResult::Continue
}
