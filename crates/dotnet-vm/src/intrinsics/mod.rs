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
//!     in the `inventory`-based registry at startup.
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
use dotnet_value::{
    object::{HeapStorage, ObjectRef},
    string::CLRString,
    StackValue,
};
use std::collections::HashMap;

pub mod array_ops;
pub mod diagnostics;
pub mod gc;
pub mod math;
pub mod metadata;
pub mod reflection;
pub mod span;
pub mod string_ops;
pub mod text_ops;
pub mod threading;
pub mod unsafe_ops;

pub use metadata::{classify_intrinsic, IntrinsicKind, IntrinsicMetadata};
pub use reflection::ReflectionExtensions;

use super::{context::ResolutionContext, sync::Arc, tracer::Tracer, StepResult};

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
    type_generics: Vec<ConcreteType>,
    is_address: bool,
) -> StepResult;

pub struct IntrinsicEntry {
    pub class_name: &'static str,
    pub method_name: &'static str,
    #[allow(dead_code)]
    pub signature: &'static str,
    pub handler: IntrinsicHandler,
    pub is_static: bool,
    pub param_count: usize,
    pub signature_filter: Option<fn(&MethodDescription) -> bool>,
}

inventory::collect!(IntrinsicEntry);

pub struct IntrinsicFieldEntry {
    pub class_name: &'static str,
    pub field_name: &'static str,
    pub handler: IntrinsicFieldHandler,
}

inventory::collect!(IntrinsicFieldEntry);

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
/// A key for looking up intrinsics that works across different AssemblyLoader instances.
/// Uses type name + method name + parameter count instead of pointer equality.
///
/// We use `Arc<str>` instead of `Box<str>` or `String` to optimize lookups:
/// - Cloning Arc<str> is cheap (just incrementing a reference count, no heap allocation)
/// - This allows us to construct lookup keys without new heap allocations
/// - Arc<str> can be created from &str via Arc::from() which reuses existing string data
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct IntrinsicKey {
    type_name: Arc<str>,
    member_name: Arc<str>,
    param_count: Option<usize>,
}

impl IntrinsicKey {
    /// Creates an IntrinsicKey from a MethodDescription.
    ///
    /// Uses Arc::from to potentially reuse string data. While this still allocates
    /// the Arc control block, it's much cheaper than allocating new strings.
    fn from_method(method: MethodDescription) -> Self {
        let type_name: Arc<str> = method.parent.type_name().into();
        let member_name: Arc<str> = (&*method.method.name).into();
        let param_count = if method.method.signature.instance {
            method.method.signature.parameters.len() + 1
        } else {
            method.method.signature.parameters.len()
        };

        Self {
            type_name,
            member_name,
            param_count: Some(param_count),
        }
    }

    /// Creates an IntrinsicKey from a FieldDescription.
    fn from_field(field: FieldDescription) -> Self {
        let type_name: Arc<str> = field.parent.type_name().into();
        let member_name: Arc<str> = (&*field.field.name).into();
        Self {
            type_name,
            member_name,
            param_count: None,
        }
    }
}

pub struct IntrinsicRegistry {
    method_handlers: HashMap<IntrinsicKey, IntrinsicHandler>,
    field_handlers: HashMap<IntrinsicKey, IntrinsicFieldHandler>,
    /// Metadata storage for intrinsics with full classification.
    method_metadata: HashMap<IntrinsicKey, Vec<IntrinsicMetadata>>,
}

impl IntrinsicRegistry {
    /// Creates a new empty registry.
    fn new() -> Self {
        Self {
            method_handlers: HashMap::new(),
            field_handlers: HashMap::new(),
            method_metadata: HashMap::new(),
        }
    }

    /// Registers an intrinsic handler for the given method.
    pub fn register(
        &mut self,
        method: MethodDescription,
        handler: IntrinsicHandler,
        tracer: Option<&mut Tracer>,
    ) {
        let key = IntrinsicKey::from_method(method);
        if let Some(tracer) = tracer {
            tracer.trace_intrinsic(
                0,
                "REGISTER",
                &format!(
                    "{}.{}({:?} params)",
                    key.type_name,
                    key.member_name,
                    key.param_count.unwrap_or(0)
                ),
            );
        }
        self.method_handlers.insert(key, handler);
    }

    /// Registers an intrinsic handler by name and parameter count.
    /// Used for methods that might not be in the metadata.
    pub fn register_raw(
        &mut self,
        type_name: &str,
        method_name: &str,
        param_count: usize,
        handler: IntrinsicHandler,
        tracer: Option<&mut Tracer>,
    ) {
        let key = IntrinsicKey {
            type_name: Arc::from(type_name),
            member_name: Arc::from(method_name),
            param_count: Some(param_count),
        };
        if let Some(tracer) = tracer {
            tracer.trace_intrinsic(
                0,
                "REGISTER-RAW",
                &format!(
                    "{}.{}({} params)",
                    key.type_name, key.member_name, param_count
                ),
            );
        }
        self.method_handlers.insert(key, handler);
    }

    /// Registers an intrinsic handler for the given field.
    pub fn register_field(
        &mut self,
        field: FieldDescription,
        handler: IntrinsicFieldHandler,
        tracer: Option<&mut Tracer>,
    ) {
        let key = IntrinsicKey::from_field(field);
        self.register_field_with_key(key, handler, tracer);
    }

    /// Registers an intrinsic field by name.
    pub fn register_raw_field(
        &mut self,
        type_name: &str,
        field_name: &str,
        handler: IntrinsicFieldHandler,
        tracer: Option<&mut Tracer>,
    ) {
        let key = IntrinsicKey {
            type_name: Arc::from(type_name),
            member_name: Arc::from(field_name),
            param_count: None,
        };
        self.register_field_with_key(key, handler, tracer);
    }

    fn register_field_with_key(
        &mut self,
        key: IntrinsicKey,
        handler: IntrinsicFieldHandler,
        tracer: Option<&mut Tracer>,
    ) {
        if let Some(tracer) = tracer {
            tracer.trace_intrinsic(
                0,
                "REGISTER-FIELD",
                &format!("{}.{}", key.type_name, key.member_name),
            );
        }
        self.field_handlers.insert(key, handler);
    }

    /// Looks up an intrinsic handler for the given method.
    pub fn get(&self, method: &MethodDescription) -> Option<IntrinsicHandler> {
        if let Some(metadata) = self.get_metadata(method) {
            return Some(metadata.handler);
        }
        let key = IntrinsicKey::from_method(*method);
        self.method_handlers.get(&key).copied()
    }

    /// Looks up an intrinsic handler for the given field.
    pub fn get_field(&self, field: &FieldDescription) -> Option<IntrinsicFieldHandler> {
        let key = IntrinsicKey::from_field(*field);
        self.field_handlers.get(&key).copied()
    }

    /// Registers an intrinsic with full metadata.
    /// This is the preferred registration method.
    pub fn register_metadata(
        &mut self,
        method: MethodDescription,
        metadata: IntrinsicMetadata,
        tracer: Option<&mut Tracer>,
    ) {
        let key = IntrinsicKey::from_method(method);
        self.register_metadata_with_key(key, metadata, tracer);
    }

    /// Registers an intrinsic using raw names and parameter count with full metadata.
    pub fn register_raw_metadata(
        &mut self,
        type_name: &str,
        method_name: &str,
        param_count: usize,
        metadata: IntrinsicMetadata,
        tracer: Option<&mut Tracer>,
    ) {
        let key = IntrinsicKey {
            type_name: Arc::from(type_name),
            member_name: Arc::from(method_name),
            param_count: Some(param_count),
        };
        self.register_metadata_with_key(key, metadata, tracer);
    }

    fn register_metadata_with_key(
        &mut self,
        key: IntrinsicKey,
        metadata: IntrinsicMetadata,
        tracer: Option<&mut Tracer>,
    ) {
        if let Some(tracer) = tracer {
            tracer.trace_intrinsic(
                0,
                "REGISTER-META",
                &format!(
                    "{}.{}({:?} params) [{:?}] - {}",
                    key.type_name,
                    key.member_name,
                    key.param_count.unwrap_or(0),
                    metadata.kind,
                    metadata.reason
                ),
            );
        }
        // Register both in metadata map and handler map for backward compatibility
        self.method_handlers.insert(key.clone(), metadata.handler);
        self.method_metadata.entry(key).or_default().push(metadata);
    }

    /// Looks up intrinsic metadata for the given method.
    /// Returns full metadata including kind and documentation.
    pub fn get_metadata(&self, method: &MethodDescription) -> Option<IntrinsicMetadata> {
        // Check metadata map
        let key = IntrinsicKey::from_method(*method);
        if let Some(candidates) = self.method_metadata.get(&key) {
            for metadata in candidates {
                if let Some(filter) = metadata.signature_filter {
                    if filter(method) {
                        return Some(metadata.clone());
                    }
                } else {
                    return Some(metadata.clone());
                }
            }
        }
        None
    }

    /// Initializes a new registry with intrinsic handlers.
    pub fn initialize(_loader: &AssemblyLoader, mut tracer: Option<&mut Tracer>) -> Self {
        let mut registry = Self::new();

        for entry in inventory::iter::<IntrinsicEntry> {
            let metadata = if entry.is_static {
                if let Some(filter) = entry.signature_filter {
                    IntrinsicMetadata::with_filter(
                        IntrinsicKind::Static,
                        entry.handler,
                        "Registered via inventory",
                        filter,
                    )
                } else {
                    IntrinsicMetadata::static_intrinsic(entry.handler, "Registered via inventory")
                }
            } else if let Some(filter) = entry.signature_filter {
                IntrinsicMetadata::with_filter(
                    IntrinsicKind::VirtualOverride,
                    entry.handler,
                    "Registered via inventory",
                    filter,
                )
            } else {
                IntrinsicMetadata::virtual_override(entry.handler, "Registered via inventory")
            };

            registry.register_raw_metadata(
                entry.class_name,
                entry.method_name,
                entry.param_count,
                metadata,
                tracer.as_deref_mut(),
            );
        }

        for entry in inventory::iter::<IntrinsicFieldEntry> {
            registry.register_raw_field(
                entry.class_name,
                entry.field_name,
                entry.handler,
                tracer.as_deref_mut(),
            );

            // Alias registration for base types
            let alias = match entry.class_name {
                "String" => Some("System.String"),
                "Object" => Some("System.Object"),
                _ => None,
            };

            if let Some(alias_name) = alias {
                registry.register_raw_field(
                    alias_name,
                    entry.field_name,
                    entry.handler,
                    tracer.as_deref_mut(),
                );
            }
        }

        registry
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
        ResolutionContext::for_method(method, ctx.loader(), generics, ctx.shared.caches.clone());

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
    type_generics: Vec<ConcreteType>,
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
