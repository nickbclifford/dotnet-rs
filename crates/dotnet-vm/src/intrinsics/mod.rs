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
//!     pub fn math_min_double<'gc, 'm: 'gc, T: VesOps<'gc, 'm>>(
//!         ctx: &mut T,
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
//! ### Support Library Conventions
//!
//! When implementing intrinsics for types in the support library (`crates/dotnet-assemblies/src/support/`):
//!
//! - **Stubbed Types**: If the C# type has a `[Stub(InPlaceOf = "System.X")]` attribute,
//!   always use the canonical `System.X` name in the `#[dotnet_intrinsic]` signature.
//!   The VM automatically normalizes `DotnetRs.X` to `System.X` during dispatch.
//!   - *Example*: Use `System.Delegate` instead of `DotnetRs.Delegate`.
//!
//! - **Internal Types**: If the C# type is purely internal (no `[Stub]` attribute),
//!   use its actual `DotnetRs.X` name in the `#[dotnet_intrinsic]` signature.
//!   - *Example*: Use `DotnetRs.Assembly` (which extends `System.Reflection.Assembly`).
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
use crate::stack::ops::{
    EvalStackOps, ExceptionOps, LoaderOps, MemoryOps, ReflectionOps, TypedStackOps, VesOps,
};
use dotnet_assemblies::AssemblyLoader;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    runtime::RuntimeType,
};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
    string::CLRString,
};
use std::sync::Arc;

pub mod array_ops;
pub mod cpu_intrinsics;
pub mod delegates;
pub mod diagnostics;
pub mod exceptions;
pub mod gc;
pub mod math;
pub mod metadata;
pub mod object_ops;
pub mod reflection;
pub mod span;
pub mod static_registry;
pub mod string_ops;

include!(concat!(env!("OUT_DIR"), "/intrinsics_dispatch.rs"));
include!(concat!(env!("OUT_DIR"), "/intrinsics_phf.rs"));

pub mod text_ops;
pub mod threading;
pub mod unsafe_ops;

pub use metadata::{IntrinsicKind, IntrinsicMetadata, classify_intrinsic};

use super::{StepResult, context::ResolutionContext, tracer::Tracer};

pub const INTRINSIC_ATTR: &str = "System.Runtime.CompilerServices.IntrinsicAttribute";

const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";

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
/// GenericLookup is passed by reference to avoid cloning on every intrinsic call.
pub type IntrinsicHandler = MethodIntrinsicId;

pub type IntrinsicFieldHandler = FieldIntrinsicId;

pub fn missing_intrinsic_handler<'gc, 'm: 'gc, T: VesOps<'gc, 'm>>(
    _ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    StepResult::Error(
        crate::error::ExecutionError::NotImplemented(format!(
            "Missing intrinsic implementation for method: {:?}",
            method
        ))
        .into(),
    )
}

pub fn get_missing_intrinsic_handler() -> IntrinsicHandler {
    MethodIntrinsicId::Missing
}

/// Registry for intrinsic method implementations.
///
/// This provides O(1) lookup of intrinsic handlers by MethodDescription,
/// replacing the older O(N) macro-based matching approach.
///
/// The registry is lazily initialized on first use via OnceLock.
pub struct IntrinsicRegistry;

impl IntrinsicRegistry {
    /// Initializes a new registry with intrinsic handlers.
    pub fn initialize(_loader: &AssemblyLoader, _tracer: Option<&Tracer>) -> Self {
        Self
    }

    /// Looks up an intrinsic handler for the given method.
    pub fn get(
        &self,
        method: &MethodDescription,
        loader: &AssemblyLoader,
    ) -> Option<IntrinsicHandler> {
        self.get_metadata(method, loader).map(|m| m.handler)
    }

    /// Looks up an intrinsic handler for the given field.
    pub fn get_field(
        &self,
        field: &FieldDescription,
        loader: &AssemblyLoader,
    ) -> Option<IntrinsicFieldHandler> {
        let mut buf = [0u8; 512];
        let key = self.build_field_key(field, loader, &mut buf)?;
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
    pub fn get_metadata(
        &self,
        method: &MethodDescription,
        loader: &AssemblyLoader,
    ) -> Option<IntrinsicMetadata> {
        let mut buf = [0u8; 512];
        let key = self.build_method_key(method, loader, &mut buf)?;
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
        loader: &AssemblyLoader,
        buf: &'a mut [u8],
    ) -> Option<&'a str> {
        use std::fmt::Write;
        let mut w = StackWrite { buf, pos: 0 };
        let arity = if method.method.signature.instance {
            method.method.signature.parameters.len() + 1
        } else {
            method.method.signature.parameters.len()
        };
        let raw_type_name = method.parent.type_name();
        let canonical_name = loader.canonical_type_name(&raw_type_name);
        let normalized_name = canonical_name.replace('/', "+");
        write!(
            w,
            "M:{}::{}#{}",
            normalized_name, &*method.method.name, arity
        )
        .ok()?;
        let pos = w.pos;
        std::str::from_utf8(&buf[..pos]).ok()
    }

    fn build_field_key<'a>(
        &self,
        field: &FieldDescription,
        loader: &AssemblyLoader,
        buf: &'a mut [u8],
    ) -> Option<&'a str> {
        use std::fmt::Write;
        let mut w = StackWrite { buf, pos: 0 };
        let raw_type_name = field.parent.type_name();
        let canonical_name = loader.canonical_type_name(&raw_type_name);
        let normalized_name = canonical_name.replace('/', "+");
        write!(w, "F:{}::{}", normalized_name, &*field.field.name).ok()?;
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
    if registry.get_field(&field, loader).is_some() {
        return true;
    }

    // Check for IntrinsicAttribute
    for a in &field.field.attributes {
        if let Ok(ctor) = loader.locate_attribute(field.parent.resolution, a)
            && ctor.parent.type_name() == INTRINSIC_ATTR
        {
            return true;
        }
    }

    false
}

pub fn intrinsic_call<'gc, 'm: 'gc, T: VesOps<'gc, 'm>>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _res_ctx = ResolutionContext::for_method(
        method,
        ctx.loader(),
        generics,
        ctx.shared().caches.clone(),
        Some(ctx.shared().clone()),
    );

    vm_trace_intrinsic!(
        ctx,
        "CALL",
        &format!("{}.{}", method.parent.type_name(), method.method.name)
    );

    if let Some(metadata) = classify_intrinsic(
        method,
        ctx.loader(),
        Some(&ctx.shared().caches.intrinsic_registry),
    ) {
        ctx.set_current_intrinsic(Some(method));
        let res = dispatch_method_intrinsic(metadata.handler, ctx, method, generics);
        ctx.set_current_intrinsic(None);
        return res;
    }

    panic!("unsupported intrinsic {:?}", method);
}

pub fn intrinsic_field<'gc, 'm: 'gc, T: VesOps<'gc, 'm>>(
    ctx: &mut T,
    field: FieldDescription,
    type_generics: Arc<[ConcreteType]>,
    is_address: bool,
) -> StepResult {
    vm_trace_intrinsic!(
        ctx,
        "FIELD-LOAD",
        &format!("{}.{}", field.parent.type_name(), field.field.name)
    );
    if let Some(handler) = ctx
        .shared()
        .caches
        .intrinsic_registry
        .get_field(&field, ctx.loader())
    {
        dispatch_field_intrinsic(handler, ctx, field, type_generics, is_address)
    } else {
        panic!("unsupported load from intrinsic field: {:?}", field);
    }
}

#[dotnet_intrinsic("string System.Object::ToString()")]
fn object_to_string<
    'gc,
    'm: 'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + MemoryOps<'gc> + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let this = ctx.pop();

    let type_name = if let StackValue::ObjectRef(obj_ref) = this {
        if obj_ref.0.is_some() {
            obj_ref.as_heap_storage(|storage| match storage {
                HeapStorage::Obj(o) => o.description.type_name(),
                HeapStorage::Str(_) => "System.String".to_string(),
                HeapStorage::Vec(_) => "System.Array".to_string(),
                HeapStorage::Boxed(_) => "System.ValueType".to_string(),
            })
        } else {
            return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
        }
    } else {
        "System.Object".to_string()
    };

    let str_val = CLRString::from(type_name);
    let storage = HeapStorage::Str(str_val);
    let obj_ref = ObjectRef::new(
        ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new()),
        storage,
    );
    ctx.push_obj(obj_ref);
    StepResult::Continue
}

#[dotnet_intrinsic("System.Type System.Object::GetType()")]
fn object_get_type<
    'gc,
    'm: 'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + ExceptionOps<'gc>
        + ReflectionOps<'gc, 'm>
        + LoaderOps<'m>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let this = ctx.pop_obj();
    if this.0.is_none() {
        return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
    }

    let rt: RuntimeType = this.as_heap_storage(|storage| match storage {
        HeapStorage::Obj(o) => RuntimeType::Type(o.description),
        HeapStorage::Str(_) => RuntimeType::String,
        // For arrays, return System.Array as a conservative fallback.
        // TODO: encode exact element type and rank into RuntimeType (Vector/Array)
        HeapStorage::Vec(_v) => {
            let arr_td = ctx
                .loader()
                .corlib_type("System.Array")
                .expect("System.Array must exist");
            RuntimeType::Type(arr_td)
        }
        HeapStorage::Boxed(o) => {
            let name = o.description.type_name();
            match name.as_str() {
                "System.Boolean" => RuntimeType::Boolean,
                "System.Char" => RuntimeType::Char,
                "System.SByte" => RuntimeType::Int8,
                "System.Byte" => RuntimeType::UInt8,
                "System.Int16" => RuntimeType::Int16,
                "System.UInt16" => RuntimeType::UInt16,
                "System.Int32" => RuntimeType::Int32,
                "System.UInt32" => RuntimeType::UInt32,
                "System.Int64" => RuntimeType::Int64,
                "System.UInt64" => RuntimeType::UInt64,
                "System.Single" => RuntimeType::Float32,
                "System.Double" => RuntimeType::Float64,
                "System.IntPtr" => RuntimeType::IntPtr,
                "System.UIntPtr" => RuntimeType::UIntPtr,
                _ => RuntimeType::Type(o.description),
            }
        }
    });

    let typ_obj = ctx.get_runtime_type(rt);
    ctx.push_obj(typ_obj);
    StepResult::Continue
}
