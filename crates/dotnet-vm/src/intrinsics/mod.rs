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
//! flow in `resolve_virtual_method()` (see `src/vm/stack.rs`).
//!
//! When resolving a virtual call:
//! 1. Check if the target method has a `VirtualOverride` intrinsic for the runtime type.
//! 2. If yes, return the intrinsic override immediately (respects VMT cache).
//! 3. Otherwise, fall back to standard ancestor search.
//!
//! ### Unified Dispatch Pipeline
//!
//! All method calls flow through a unified dispatch pipeline (see `src/vm/instructions.rs`):
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
//!     Create a function with the following signature:
//!     ```rust,ignore
//!     pub fn my_intrinsic_handler<'gc, 'm>(
//!         gc: GCHandle<'gc>,
//!         stack: &mut CallStack<'gc, 'm>,
//!         method: MethodDescription,
//!         generics: &GenericLookup,
//!     ) -> StepResult { ... }
//!     ```
//!     Place this function in an appropriate submodule (e.g., `src/vm/intrinsics/math.rs`).
//!
//! 2.  **Determine the intrinsic category**:
//!     - **Static**: No BCL equivalent (e.g., new VM functionality).
//!     - **VirtualOverride**: Overrides base class method for specific runtime type.
//!     - **DirectIntercept**: Must bypass BCL due to internal representation differences.
//!
//! 3.  **Register the handler**:
//!     Use `register_metadata()` in `IntrinsicRegistry::initialize()` (preferred):
//!     ```rust,ignore
//!     registry.register_metadata(
//!         method,
//!         IntrinsicMetadata::direct_intercept(
//!             my_intrinsic_handler,
//!             "Reason why this must be intrinsic"
//!         )
//!     );
//!     ```
//!     For static registration without a full method description, use `register_raw_metadata()`.
//!
//! 4.  **Ensure the method is marked as intrinsic**:
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
use crate::{vm_pop, vm_push, vm_trace_intrinsic};
use dotnet_assemblies::AssemblyLoader;
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
use dotnetdll::prelude::{BaseType, MethodType, ParameterType};
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

use super::{context::ResolutionContext, sync::Arc, tracer::Tracer, CallStack, StepResult};

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
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult;

pub type IntrinsicFieldHandler = for<'gc, 'm> fn(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    field: FieldDescription,
    type_generics: Vec<ConcreteType>,
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
    method_metadata: HashMap<IntrinsicKey, IntrinsicMetadata>,
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
        self.method_metadata.insert(key, metadata);
    }

    /// Looks up intrinsic metadata for the given method.
    /// Returns full metadata including kind and documentation.
    pub fn get_metadata(&self, method: &MethodDescription) -> Option<IntrinsicMetadata> {
        // Check metadata map
        let key = IntrinsicKey::from_method(*method);
        if let Some(metadata) = self.method_metadata.get(&key) {
            // Check signature filter if present
            if let Some(filter) = metadata.signature_filter {
                if !filter(method) {
                    return None;
                }
            }
            return Some(metadata.clone());
        }
        None
    }

    /// Initializes a new registry with intrinsic handlers.
    #[allow(unused_variables)] // loader is used in the register_intrinsic! macro
    #[allow(clippy::missing_transmute_annotations)] // Transmute safety documented at file level
    pub fn initialize(loader: &AssemblyLoader, mut tracer: Option<&mut Tracer>) -> Self {
        let mut registry = Self::new();

        macro_rules! register_static {
            ($type:expr, $name:expr, $params:expr, $handler:path, $reason:expr) => {
                registry.register_raw_metadata(
                    $type,
                    $name,
                    $params,
                    IntrinsicMetadata::static_intrinsic(
                        unsafe {
                            std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                                $handler as fn(_, _, _, _) -> _,
                            )
                        },
                        $reason,
                    ),
                    tracer.as_deref_mut(),
                );
            };
        }

        macro_rules! register_static_filtered {
            ($type:expr, $name:expr, $params:expr, $handler:path, $filter:path, $reason:expr) => {
                registry.register_raw_metadata(
                    $type,
                    $name,
                    $params,
                    IntrinsicMetadata::with_filter(
                        IntrinsicKind::Static,
                        unsafe {
                            std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                                $handler as fn(_, _, _, _) -> _,
                            )
                        },
                        $reason,
                        $filter,
                    ),
                    tracer.as_deref_mut(),
                );
            };
        }

        macro_rules! register_virtual {
            ($type:expr, $name:expr, $params:expr, $handler:path, $reason:expr) => {
                registry.register_raw_metadata(
                    $type,
                    $name,
                    $params + 1,
                    IntrinsicMetadata::virtual_override(
                        unsafe {
                            std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                                $handler as fn(_, _, _, _) -> _,
                            )
                        },
                        $reason,
                    ),
                    tracer.as_deref_mut(),
                );
            };
        }

        macro_rules! register_intercept {
            ($type:expr, $name:expr, $params:expr, $handler:path, $reason:expr) => {
                registry.register_raw_metadata(
                    $type,
                    $name,
                    $params,
                    IntrinsicMetadata::direct_intercept(
                        unsafe {
                            std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                                $handler as fn(_, _, _, _) -> _,
                            )
                        },
                        $reason,
                    ),
                    tracer.as_deref_mut(),
                );
            };
        }

        macro_rules! register_field {
            ($type:expr, $name:expr, $handler:path) => {
                registry.register_raw_field(
                    $type,
                    $name,
                    unsafe { std::mem::transmute::<fn(_, _, _, _, _) -> StepResult, IntrinsicFieldHandler>($handler as fn(_, _, _, _, _) -> StepResult) },
                    tracer.as_deref_mut()
                );
            };
        }

        // Reflection - RuntimeTypeHandle
        register_static!(
            "DotnetRs.RuntimeTypeHandle",
            "GetActivationInfo",
            5,
            reflection::runtime_type_handle_intrinsic_call,
            "Reflection internal activation info"
        );
        register_static!(
            "System.RuntimeTypeHandle",
            "GetActivationInfo",
            5,
            reflection::runtime_type_handle_intrinsic_call,
            "Reflection internal activation info"
        );
        register_static!(
            "DotnetRs.RuntimeTypeHandle",
            "ToIntPtr",
            1,
            reflection::intrinsic_type_handle_to_int_ptr,
            "Internal handle conversion"
        );
        register_static!(
            "System.RuntimeTypeHandle",
            "ToIntPtr",
            1,
            reflection::intrinsic_type_handle_to_int_ptr,
            "Internal handle conversion"
        );

        // Reflection - RuntimeType Virtual Overrides
        register_virtual!(
            "System.Type",
            "get_Name",
            0,
            reflection::runtime_type_intrinsic_call,
            "Reflection internal"
        );
        register_virtual!(
            "System.Type",
            "get_Namespace",
            0,
            reflection::runtime_type_intrinsic_call,
            "Reflection internal"
        );
        register_virtual!(
            "System.Type",
            "get_Assembly",
            0,
            reflection::runtime_type_intrinsic_call,
            "Reflection internal"
        );
        register_virtual!(
            "System.Type",
            "get_BaseType",
            0,
            reflection::runtime_type_intrinsic_call,
            "Reflection internal"
        );
        register_virtual!(
            "System.Type",
            "get_IsGenericType",
            0,
            reflection::runtime_type_intrinsic_call,
            "Reflection internal"
        );
        register_virtual!(
            "System.Type",
            "get_GenericTypeDefinition",
            0,
            reflection::runtime_type_intrinsic_call,
            "Reflection internal"
        );
        register_virtual!(
            "System.Type",
            "GetGenericArguments",
            0,
            reflection::runtime_type_intrinsic_call,
            "Reflection internal"
        );
        register_virtual!(
            "System.Type",
            "get_TypeHandle",
            0,
            reflection::runtime_type_intrinsic_call,
            "Reflection internal"
        );
        register_virtual!(
            "System.Type",
            "MakeGenericType",
            1,
            reflection::runtime_type_intrinsic_call,
            "Reflection internal"
        );

        register_virtual!(
            "DotnetRs.RuntimeType",
            "get_Name",
            0,
            reflection::runtime_type_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.RuntimeType",
            "GetName",
            0,
            reflection::runtime_type_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.RuntimeType",
            "get_Namespace",
            0,
            reflection::runtime_type_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.RuntimeType",
            "GetNamespace",
            0,
            reflection::runtime_type_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.RuntimeType",
            "get_Assembly",
            0,
            reflection::runtime_type_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.RuntimeType",
            "GetAssembly",
            0,
            reflection::runtime_type_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.RuntimeType",
            "get_BaseType",
            0,
            reflection::runtime_type_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.RuntimeType",
            "GetBaseType",
            0,
            reflection::runtime_type_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.RuntimeType",
            "get_IsGenericType",
            0,
            reflection::runtime_type_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.RuntimeType",
            "GetIsGenericType",
            0,
            reflection::runtime_type_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.RuntimeType",
            "get_GenericTypeDefinition",
            0,
            reflection::runtime_type_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.RuntimeType",
            "GetGenericTypeDefinition",
            0,
            reflection::runtime_type_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.RuntimeType",
            "get_GenericArguments",
            0,
            reflection::runtime_type_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.RuntimeType",
            "GetGenericArguments",
            0,
            reflection::runtime_type_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.RuntimeType",
            "get_TypeHandle",
            0,
            reflection::runtime_type_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.RuntimeType",
            "GetTypeHandle",
            0,
            reflection::runtime_type_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.RuntimeType",
            "MakeGenericType",
            1,
            reflection::runtime_type_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.RuntimeType",
            "CreateInstanceDefaultCtor",
            2,
            reflection::runtime_type_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.RuntimeType",
            "CreateInstanceCheckThis",
            0,
            reflection::runtime_type_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );

        register_static!(
            "System.RuntimeType",
            "CreateInstanceDefaultCtor",
            2,
            reflection::runtime_type_intrinsic_call,
            "Reflection internal activation"
        );
        register_static!(
            "System.RuntimeType",
            "CreateInstanceCheckThis",
            0,
            reflection::runtime_type_intrinsic_call,
            "Reflection internal activation"
        );

        // Reflection - Other Virtual Overrides
        register_virtual!(
            "System.Reflection.MethodInfo",
            "get_Name",
            0,
            reflection::runtime_method_info_intrinsic_call,
            "Reflection internal"
        );
        register_virtual!(
            "System.Reflection.MethodInfo",
            "get_DeclaringType",
            0,
            reflection::runtime_method_info_intrinsic_call,
            "Reflection internal"
        );
        register_virtual!(
            "System.Reflection.MethodInfo",
            "get_MethodHandle",
            0,
            reflection::runtime_method_info_intrinsic_call,
            "Reflection internal"
        );
        register_virtual!(
            "System.Reflection.MethodInfo",
            "Invoke",
            5,
            reflection::runtime_method_info_intrinsic_call,
            "Reflection internal"
        );

        register_virtual!(
            "DotnetRs.MethodInfo",
            "get_Name",
            0,
            reflection::runtime_method_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.MethodInfo",
            "GetName",
            0,
            reflection::runtime_method_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.MethodInfo",
            "get_DeclaringType",
            0,
            reflection::runtime_method_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.MethodInfo",
            "GetDeclaringType",
            0,
            reflection::runtime_method_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.MethodInfo",
            "get_MethodHandle",
            0,
            reflection::runtime_method_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.MethodInfo",
            "GetMethodHandle",
            0,
            reflection::runtime_method_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.MethodInfo",
            "Invoke",
            5,
            reflection::runtime_method_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );

        register_virtual!(
            "System.Reflection.ConstructorInfo",
            "get_Name",
            0,
            reflection::runtime_method_info_intrinsic_call,
            "Reflection internal"
        );
        register_virtual!(
            "System.Reflection.ConstructorInfo",
            "get_DeclaringType",
            0,
            reflection::runtime_method_info_intrinsic_call,
            "Reflection internal"
        );
        register_virtual!(
            "System.Reflection.ConstructorInfo",
            "get_MethodHandle",
            0,
            reflection::runtime_method_info_intrinsic_call,
            "Reflection internal"
        );
        register_virtual!(
            "System.Reflection.ConstructorInfo",
            "Invoke",
            5,
            reflection::runtime_method_info_intrinsic_call,
            "Reflection internal"
        );
        register_virtual!(
            "System.Reflection.ConstructorInfo",
            "Invoke",
            4,
            reflection::runtime_method_info_intrinsic_call,
            "Reflection internal"
        );

        register_virtual!(
            "DotnetRs.ConstructorInfo",
            "get_Name",
            0,
            reflection::runtime_method_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.ConstructorInfo",
            "GetName",
            0,
            reflection::runtime_method_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.ConstructorInfo",
            "get_DeclaringType",
            0,
            reflection::runtime_method_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.ConstructorInfo",
            "GetDeclaringType",
            0,
            reflection::runtime_method_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.ConstructorInfo",
            "get_MethodHandle",
            0,
            reflection::runtime_method_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.ConstructorInfo",
            "GetMethodHandle",
            0,
            reflection::runtime_method_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.ConstructorInfo",
            "Invoke",
            5,
            reflection::runtime_method_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.ConstructorInfo",
            "Invoke",
            4,
            reflection::runtime_method_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );

        register_virtual!(
            "System.Reflection.FieldInfo",
            "get_Name",
            0,
            reflection::runtime_field_info_intrinsic_call,
            "Reflection internal"
        );
        register_virtual!(
            "System.Reflection.FieldInfo",
            "get_DeclaringType",
            0,
            reflection::runtime_field_info_intrinsic_call,
            "Reflection internal"
        );
        register_virtual!(
            "System.Reflection.FieldInfo",
            "get_FieldHandle",
            0,
            reflection::runtime_field_info_intrinsic_call,
            "Reflection internal"
        );
        register_virtual!(
            "System.Reflection.FieldInfo",
            "GetValue",
            1,
            reflection::runtime_field_info_intrinsic_call,
            "Reflection internal"
        );
        register_virtual!(
            "System.Reflection.FieldInfo",
            "SetValue",
            2,
            reflection::runtime_field_info_intrinsic_call,
            "Reflection internal"
        );

        register_virtual!(
            "DotnetRs.FieldInfo",
            "get_Name",
            0,
            reflection::runtime_field_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.FieldInfo",
            "GetName",
            0,
            reflection::runtime_field_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.FieldInfo",
            "get_DeclaringType",
            0,
            reflection::runtime_field_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.FieldInfo",
            "GetDeclaringType",
            0,
            reflection::runtime_field_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.FieldInfo",
            "get_FieldHandle",
            0,
            reflection::runtime_field_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.FieldInfo",
            "GetFieldHandle",
            0,
            reflection::runtime_field_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.FieldInfo",
            "GetValue",
            1,
            reflection::runtime_field_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );
        register_virtual!(
            "DotnetRs.FieldInfo",
            "SetValue",
            2,
            reflection::runtime_field_info_intrinsic_call,
            "VM-specific implementation overriding BCL reflection types"
        );

        // System.GC
        register_static!(
            "System.GC",
            "KeepAlive",
            1,
            gc::intrinsic_gc_keep_alive,
            "Managed GC control"
        );
        register_static!(
            "System.GC",
            "SuppressFinalize",
            1,
            gc::intrinsic_gc_suppress_finalize,
            "Managed GC control"
        );
        register_static!(
            "System.GC",
            "ReRegisterForFinalize",
            1,
            gc::intrinsic_gc_reregister_for_finalize,
            "Managed GC control"
        );
        register_static!(
            "System.GC",
            "Collect",
            0,
            gc::intrinsic_gc_collect_0,
            "Managed GC control"
        );
        register_static!(
            "System.GC",
            "Collect",
            1,
            gc::intrinsic_gc_collect_1,
            "Managed GC control"
        );
        register_static!(
            "System.GC",
            "Collect",
            2,
            gc::intrinsic_gc_collect_2,
            "Managed GC control"
        );
        register_static!(
            "System.GC",
            "WaitForPendingFinalizers",
            0,
            gc::intrinsic_gc_wait_for_pending_finalizers,
            "Managed GC control"
        );

        // GCHandle
        register_static!(
            "System.Runtime.InteropServices.GCHandle",
            "InternalAlloc",
            2,
            gc::intrinsic_gchandle_internal_alloc,
            "Direct GC handle manipulation"
        );
        register_static!(
            "System.Runtime.InteropServices.GCHandle",
            "InternalFree",
            1,
            gc::intrinsic_gchandle_internal_free,
            "Direct GC handle manipulation"
        );
        register_static!(
            "System.Runtime.InteropServices.GCHandle",
            "InternalGet",
            1,
            gc::intrinsic_gchandle_internal_get,
            "Direct GC handle manipulation"
        );
        register_static!(
            "System.Runtime.InteropServices.GCHandle",
            "InternalSet",
            2,
            gc::intrinsic_gchandle_internal_set,
            "Direct GC handle manipulation"
        );
        register_static!(
            "System.Runtime.InteropServices.GCHandle",
            "InternalAddrOfPinnedObject",
            1,
            gc::intrinsic_gchandle_internal_addr_of_pinned_object,
            "Direct GC handle manipulation"
        );

        // Environment & Exceptions
        register_static!(
            "System.Environment",
            "GetEnvironmentVariableCore",
            1,
            gc::intrinsic_environment_get_variable_core,
            "VM environment integration"
        );
        register_static!(
            "System.ArgumentNullException",
            "ThrowIfNull",
            1,
            gc::intrinsic_argument_null_exception_throw_if_null,
            "Fast-path null check"
        );

        // Threading
        register_static!(
            "System.Threading.Monitor",
            "Exit",
            1,
            threading::intrinsic_monitor_exit,
            "VM-managed object locking"
        );
        register_static!(
            "System.Threading.Monitor",
            "ReliableEnter",
            2,
            threading::intrinsic_monitor_reliable_enter,
            "VM-managed object locking"
        );
        register_static!(
            "System.Threading.Monitor",
            "TryEnter_FastPath",
            2,
            threading::intrinsic_monitor_try_enter_fast_path,
            "VM-managed object locking"
        );
        register_static!(
            "System.Threading.Monitor",
            "TryEnter",
            2,
            threading::intrinsic_monitor_try_enter_timeout,
            "VM-managed object locking"
        );
        register_static!(
            "System.Threading.Monitor",
            "TryEnter",
            3,
            threading::intrinsic_monitor_try_enter_timeout_ref,
            "VM-managed object locking"
        );
        register_static!(
            "System.Threading.Interlocked",
            "CompareExchange",
            3,
            threading::intrinsic_interlocked_compare_exchange,
            "Atomic operations"
        );
        register_static!(
            "System.Threading.Volatile",
            "Read",
            1,
            threading::intrinsic_volatile_read,
            "Memory barrier operations"
        );
        register_static!(
            "System.Threading.Volatile",
            "Write",
            2,
            threading::intrinsic_volatile_write,
            "Memory barrier operations"
        );

        // String - Direct Intercepts
        register_intercept!(
            "System.String",
            "get_Length",
            1,
            string_ops::intrinsic_string_get_length,
            "VM internal string representation differs from BCL UTF-16 layout"
        );
        register_intercept!(
            "System.String",
            "get_Chars",
            2,
            string_ops::intrinsic_string_get_chars,
            "VM internal string representation differs from BCL UTF-16 layout"
        );
        register_intercept!(
            "System.String",
            "FastAllocateString",
            1,
            string_ops::intrinsic_string_fast_allocate_string,
            "VM internal string representation differs from BCL UTF-16 layout"
        );
        register_intercept!(
            "System.String",
            "FastAllocateString",
            2,
            string_ops::intrinsic_string_fast_allocate_string,
            "VM internal string representation differs from BCL UTF-16 layout"
        );
        register_intercept!(
            "System.String",
            "GetRawStringData",
            1,
            string_ops::intrinsic_string_get_raw_data,
            "VM internal string representation differs from BCL UTF-16 layout"
        );
        register_intercept!(
            "System.String",
            "GetPinnableReference",
            1,
            string_ops::intrinsic_string_get_raw_data,
            "VM internal string representation differs from BCL UTF-16 layout"
        );

        // String - Performance Optimizations
        register_static!(
            "System.String",
            "Equals",
            2,
            string_ops::intrinsic_string_equals,
            "Performance-optimized string operation"
        );
        register_static!(
            "System.String",
            "Equals",
            2,
            string_ops::intrinsic_string_equals,
            "Performance-optimized string operation"
        );
        register_static!(
            "System.String",
            "CopyStringContent",
            3,
            string_ops::intrinsic_string_copy_string_content,
            "Performance-optimized string operation"
        );
        register_static!(
            "System.String",
            "Concat",
            3,
            string_ops::intrinsic_string_concat_three_spans,
            "Performance-optimized string operation"
        );
        register_static!(
            "System.String",
            "GetHashCodeOrdinalIgnoreCase",
            1,
            string_ops::intrinsic_string_get_hash_code_ordinal_ignore_case,
            "Performance-optimized string operation"
        );
        register_static!(
            "System.String",
            "IsNullOrWhiteSpace",
            1,
            string_ops::intrinsic_string_is_null_or_white_space,
            "Performance-optimized string operation"
        );
        register_static!(
            "System.String",
            "IndexOf",
            2,
            string_ops::intrinsic_string_index_of,
            "Performance-optimized string operation"
        );
        register_static!(
            "System.String",
            "IndexOf",
            3,
            string_ops::intrinsic_string_index_of,
            "Performance-optimized string operation"
        );
        register_static!(
            "System.String",
            "Substring",
            2,
            string_ops::intrinsic_string_substring,
            "Performance-optimized string operation"
        );
        register_static!(
            "System.String",
            "Substring",
            3,
            string_ops::intrinsic_string_substring,
            "Performance-optimized string operation"
        );
        register_static!(
            "System.String",
            "InternalSubString",
            3,
            string_ops::intrinsic_string_substring,
            "Performance-optimized string operation"
        );
        register_static!(
            "System.String",
            "IsNullOrEmpty",
            1,
            string_ops::intrinsic_string_is_null_or_empty,
            "Performance-optimized string operation"
        );
        register_static!(
            "System.String",
            "op_Implicit",
            1,
            span::intrinsic_as_span,
            "Performance-optimized string operation"
        );

        // Marshal & Buffer
        register_static!(
            "System.Runtime.InteropServices.Marshal",
            "GetLastPInvokeError",
            0,
            unsafe_ops::intrinsic_marshal_get_last_pinvoke_error,
            "P/Invoke error handling"
        );
        register_static!(
            "System.Runtime.InteropServices.Marshal",
            "SetLastPInvokeError",
            1,
            unsafe_ops::intrinsic_marshal_set_last_pinvoke_error,
            "P/Invoke error handling"
        );
        register_static!(
            "System.Runtime.InteropServices.Marshal",
            "SizeOf",
            0,
            unsafe_ops::intrinsic_marshal_size_of,
            "VM-managed layout info"
        );
        register_static!(
            "System.Runtime.InteropServices.Marshal",
            "SizeOf",
            1,
            unsafe_ops::intrinsic_marshal_size_of,
            "VM-managed layout info"
        );
        register_static!(
            "System.Runtime.InteropServices.Marshal",
            "OffsetOf",
            1,
            unsafe_ops::intrinsic_marshal_offset_of,
            "VM-managed layout info"
        );
        register_static!(
            "System.Runtime.InteropServices.Marshal",
            "OffsetOf",
            2,
            unsafe_ops::intrinsic_marshal_offset_of,
            "VM-managed layout info"
        );
        register_intercept!(
            "System.Buffer",
            "Memmove",
            3,
            unsafe_ops::intrinsic_buffer_memmove,
            "Memory operations must be GC-safe and respect VM memory layout"
        );
        register_intercept!(
            "System.Runtime.InteropServices.MemoryMarshal",
            "GetArrayDataReference",
            1,
            unsafe_ops::intrinsic_memory_marshal_get_array_data_reference,
            "Array internal representation differs from BCL, VM-specific layout"
        );

        // RuntimeHelpers
        register_static!(
            "System.Runtime.CompilerServices.RuntimeHelpers",
            "GetMethodTable",
            1,
            reflection::intrinsic_runtime_helpers_get_method_table,
            "Internal reflection helper"
        );
        register_static!(
            "System.Runtime.CompilerServices.RuntimeHelpers",
            "CreateSpan",
            2,
            span::intrinsic_runtime_helpers_create_span,
            "Internal span factory"
        );
        register_static!(
            "System.Runtime.CompilerServices.RuntimeHelpers",
            "IsBitwiseEquatable",
            0,
            reflection::intrinsic_runtime_helpers_is_bitwise_equatable,
            "Internal reflection helper"
        );
        register_static!(
            "System.Runtime.CompilerServices.RuntimeHelpers",
            "IsReferenceOrContainsReferences",
            0,
            reflection::intrinsic_runtime_helpers_is_reference_or_contains_references,
            "Internal reflection helper"
        );
        register_static!(
            "System.Runtime.CompilerServices.RuntimeHelpers",
            "RunClassConstructor",
            1,
            reflection::intrinsic_runtime_helpers_run_class_constructor,
            "Internal reflection helper"
        );
        register_static!(
            "System.Runtime.CompilerServices.RuntimeHelpers",
            "GetSpanDataFrom",
            3,
            span::intrinsic_runtime_helpers_get_span_data_from,
            "Internal span support"
        );

        // MemoryExtensions & Span
        register_static!(
            "System.MemoryExtensions",
            "Equals",
            3,
            span::intrinsic_memory_extensions_equals_span_char,
            "Performance-optimized span comparison"
        );
        register_static!(
            "System.MemoryExtensions",
            "AsSpan",
            1,
            span::intrinsic_as_span,
            "Performance-optimized span conversion"
        );
        register_static!(
            "System.MemoryExtensions",
            "AsSpan",
            2,
            span::intrinsic_as_span,
            "Performance-optimized span conversion"
        );
        register_static!(
            "System.MemoryExtensions",
            "AsSpan",
            3,
            span::intrinsic_as_span,
            "Performance-optimized span conversion"
        );
        register_static!(
            "DotnetRs.Internal",
            "GetArrayData",
            1,
            span::intrinsic_internal_get_array_data,
            "Internal array support"
        );

        // Unsafe - Direct Intercepts
        register_intercept!(
            "System.Runtime.CompilerServices.Unsafe",
            "AsPointer",
            1,
            unsafe_ops::intrinsic_unsafe_as_pointer,
            "Direct memory access must respect VM memory management and GC"
        );
        register_intercept!(
            "System.Runtime.CompilerServices.Unsafe",
            "Add",
            2,
            unsafe_ops::intrinsic_unsafe_add,
            "Direct memory access must respect VM memory management and GC"
        );
        register_intercept!(
            "System.Runtime.CompilerServices.Unsafe",
            "AreSame",
            2,
            unsafe_ops::intrinsic_unsafe_are_same,
            "Direct memory access must respect VM memory management and GC"
        );
        register_intercept!(
            "System.Runtime.CompilerServices.Unsafe",
            "As",
            1,
            unsafe_ops::intrinsic_unsafe_as,
            "Direct memory access must respect VM memory management and GC"
        );
        register_intercept!(
            "System.Runtime.CompilerServices.Unsafe",
            "As",
            2,
            unsafe_ops::intrinsic_unsafe_as_generic,
            "Direct memory access must respect VM memory management and GC"
        );
        register_intercept!(
            "System.Runtime.CompilerServices.Unsafe",
            "AsRef",
            1,
            unsafe_ops::intrinsic_unsafe_as_ref_any,
            "Direct memory access must respect VM memory management and GC"
        );
        register_intercept!(
            "System.Runtime.CompilerServices.Unsafe",
            "SizeOf",
            0,
            unsafe_ops::intrinsic_unsafe_size_of,
            "Direct memory access must respect VM memory management and GC"
        );
        register_intercept!(
            "System.Runtime.CompilerServices.Unsafe",
            "ByteOffset",
            2,
            unsafe_ops::intrinsic_unsafe_byte_offset,
            "Direct memory access must respect VM memory management and GC"
        );
        register_intercept!(
            "System.Runtime.CompilerServices.Unsafe",
            "AddByteOffset",
            2,
            unsafe_ops::intrinsic_unsafe_add_byte_offset,
            "Direct memory access must respect VM memory management and GC"
        );
        register_intercept!(
            "System.Runtime.CompilerServices.Unsafe",
            "ReadUnaligned",
            1,
            unsafe_ops::intrinsic_unsafe_read_unaligned,
            "Direct memory access must respect VM memory management and GC"
        );
        register_intercept!(
            "System.Runtime.CompilerServices.Unsafe",
            "WriteUnaligned",
            2,
            unsafe_ops::intrinsic_unsafe_write_unaligned,
            "Direct memory access must respect VM memory management and GC"
        );

        // Vectors & Numeric
        register_static!(
            "System.Runtime.Intrinsics.Vector128",
            "get_IsHardwareAccelerated",
            0,
            math::intrinsic_vector_is_hardware_accelerated,
            "Performance optimization"
        );
        register_static!(
            "System.Runtime.Intrinsics.Vector256",
            "get_IsHardwareAccelerated",
            0,
            math::intrinsic_vector_is_hardware_accelerated,
            "Performance optimization"
        );
        register_static!(
            "System.Runtime.Intrinsics.Vector512",
            "get_IsHardwareAccelerated",
            0,
            math::intrinsic_vector_is_hardware_accelerated,
            "Performance optimization"
        );
        register_static!(
            "System.Numerics.Vector",
            "get_IsHardwareAccelerated",
            0,
            math::intrinsic_vector_is_hardware_accelerated,
            "Performance optimization"
        );

        // X86 Intrinsics Support Checks
        let x86_intrinsics = [
            "System.Runtime.Intrinsics.X86.Lzcnt",
            "System.Runtime.Intrinsics.X86.Popcnt",
            "System.Runtime.Intrinsics.X86.Bmi1",
            "System.Runtime.Intrinsics.X86.Bmi2",
            "System.Runtime.Intrinsics.X86.Pclmulqdq",
            "System.Runtime.Intrinsics.X86.Aes",
            "System.Runtime.Intrinsics.X86.Avx",
            "System.Runtime.Intrinsics.X86.Avx2",
            "System.Runtime.Intrinsics.X86.Sse",
            "System.Runtime.Intrinsics.X86.Sse2",
            "System.Runtime.Intrinsics.X86.Sse3",
            "System.Runtime.Intrinsics.X86.Sse41",
            "System.Runtime.Intrinsics.X86.Sse42",
            "System.Runtime.Intrinsics.X86.Ssse3",
            "System.Runtime.Intrinsics.X86.X86Base",
        ];

        for t in x86_intrinsics {
            register_static!(
                t,
                "get_IsSupported",
                0,
                math::intrinsic_vector_is_hardware_accelerated,
                "Hardware support check"
            );
        }

        let numeric_types = [
            "System.Byte",
            "System.SByte",
            "System.UInt16",
            "System.Int16",
            "System.UInt32",
            "System.Int32",
            "System.UInt64",
            "System.Int64",
            "System.UIntPtr",
            "System.IntPtr",
        ];
        for t in numeric_types {
            register_static!(
                t,
                "CreateTruncating",
                1,
                math::intrinsic_numeric_create_truncating,
                "Performance optimization"
            );
        }

        // Activator & EqualityComparer
        register_static!(
            "System.Activator",
            "CreateInstance",
            0,
            reflection::intrinsic_activator_create_instance,
            "Reflection-based creation"
        );
        register_static!(
            "System.Collections.Generic.EqualityComparer`1",
            "get_Default",
            0,
            math::intrinsic_equality_comparer_get_default,
            "Generic comparer integration"
        );

        // Type & Reflection
        register_static!(
            "System.Type",
            "GetTypeFromHandle",
            1,
            reflection::intrinsic_get_from_handle,
            "Reflection internal"
        );
        register_static!(
            "System.Type",
            "get_IsValueType",
            0,
            reflection::intrinsic_type_get_is_value_type,
            "Reflection internal"
        );
        register_static!(
            "System.Type",
            "get_IsEnum",
            0,
            reflection::intrinsic_type_get_is_enum,
            "Reflection internal"
        );
        register_static!(
            "System.Type",
            "get_IsInterface",
            0,
            reflection::intrinsic_type_get_is_interface,
            "Reflection internal"
        );
        register_static!(
            "System.Type",
            "op_Equality",
            2,
            reflection::intrinsic_type_op_equality,
            "Reflection internal"
        );
        register_static!(
            "System.Type",
            "op_Inequality",
            2,
            reflection::intrinsic_type_op_inequality,
            "Reflection internal"
        );
        register_static!(
            "System.Type",
            "get_TypeHandle",
            0,
            reflection::intrinsic_type_get_type_handle,
            "Reflection internal"
        );
        register_static!(
            "System.Reflection.Assembly",
            "GetCustomAttributes",
            2,
            reflection::intrinsic_assembly_get_custom_attributes,
            "Reflection internal"
        );
        register_static!(
            "System.Attribute",
            "GetCustomAttributes",
            2,
            reflection::intrinsic_attribute_get_custom_attributes,
            "Reflection internal"
        );
        register_static!(
            "System.Reflection.MethodBase",
            "GetMethodFromHandle",
            1,
            reflection::intrinsic_get_from_handle,
            "Reflection internal"
        );
        register_static!(
            "System.Reflection.FieldInfo",
            "GetFieldFromHandle",
            1,
            reflection::intrinsic_get_from_handle,
            "Reflection internal"
        );
        register_static!(
            "System.RuntimeMethodHandle",
            "GetFunctionPointer",
            1,
            reflection::intrinsic_method_handle_get_function_pointer,
            "Internal method handle conversion"
        );

        // UnicodeUtility
        register_static!(
            "System.Text.UnicodeUtility",
            "IsAsciiCodePoint",
            1,
            text_ops::intrinsic_unicode_utility_is_ascii_code_point,
            "Performance-optimized text operation"
        );
        register_static!(
            "System.Text.UnicodeUtility",
            "IsInRangeInclusive",
            3,
            text_ops::intrinsic_unicode_utility_is_in_range_inclusive,
            "Performance-optimized text operation"
        );

        // Array - Direct Intercepts
        register_intercept!(
            "System.Array",
            "GetLength",
            2,
            array_ops::intrinsic_array_get_length,
            "Array internal representation differs from BCL"
        );
        register_intercept!(
            "System.Array",
            "get_Length",
            1,
            array_ops::intrinsic_array_get_length,
            "Array internal representation differs from BCL"
        );
        register_intercept!(
            "System.Array",
            "get_Rank",
            1,
            array_ops::intrinsic_array_get_rank,
            "Array internal representation differs from BCL"
        );
        register_intercept!(
            "System.Array",
            "GetValue",
            2,
            array_ops::intrinsic_array_get_value,
            "Array internal representation differs from BCL"
        );
        register_intercept!(
            "System.Array",
            "SetValue",
            3,
            array_ops::intrinsic_array_set_value,
            "Array internal representation differs from BCL"
        );
        register_intercept!(
            "System.Array",
            "get_Count",
            1,
            array_ops::intrinsic_array_get_length,
            "Array internal representation differs from BCL"
        );
        register_intercept!(
            "DotnetRs.Array",
            "GetLength",
            2,
            array_ops::intrinsic_array_get_length,
            "Array internal representation differs from BCL"
        );
        register_intercept!(
            "DotnetRs.Array",
            "get_Length",
            1,
            array_ops::intrinsic_array_get_length,
            "Array internal representation differs from BCL"
        );
        register_intercept!(
            "DotnetRs.Array",
            "get_Rank",
            1,
            array_ops::intrinsic_array_get_rank,
            "Array internal representation differs from BCL"
        );
        register_intercept!(
            "DotnetRs.Array",
            "GetValue",
            2,
            array_ops::intrinsic_array_get_value,
            "Array internal representation differs from BCL"
        );
        register_intercept!(
            "DotnetRs.Array",
            "SetValue",
            3,
            array_ops::intrinsic_array_set_value,
            "Array internal representation differs from BCL"
        );

        // Math
        register_static_filtered!(
            "System.Math",
            "Min",
            2,
            math::intrinsic_math_min_double,
            math_min_double_filter,
            "Performance optimization using native math operations"
        );
        register_static!(
            "System.Math",
            "Sqrt",
            1,
            math::intrinsic_math_sqrt,
            "Performance optimization using native math operations"
        );

        // Fields
        register_field!(
            "System.IntPtr",
            "Zero",
            unsafe_ops::intrinsic_field_intptr_zero
        );
        register_field!(
            "System.String",
            "Empty",
            string_ops::intrinsic_field_string_empty
        );
        register_field!(
            "System.BitConverter",
            "IsLittleEndian",
            math::intrinsic_bitconverter_is_little_endian
        );

        // System.Object
        register_virtual!(
            "System.Object",
            "ToString",
            0,
            object_to_string,
            "Basic ToString implementation"
        );

        // Diagnostics
        register_intercept!(
            "System.Diagnostics.Tracing.XplatEventLogger",
            "IsEventSourceLoggingEnabled",
            0,
            diagnostics::intrinsic_is_event_source_logging_enabled,
            "Prevent QCall lookup"
        );

        if let Some(tracer) = tracer {
            tracer.trace_intrinsic(
                0,
                "INIT",
                "Intrinsic registry initialized with all static handlers",
            );
        }

        registry
    }
}

// ============================================================================
// End Intrinsic Registry Infrastructure
// ============================================================================

/// Checks if a method is an intrinsic.
///
/// This uses the unified classification system to determine if a method should
/// be handled by the VM.
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
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );

    vm_trace_intrinsic!(
        stack,
        "CALL",
        &format!("{}.{}", method.parent.type_name(), method.method.name)
    );

    if let Some(metadata) = classify_intrinsic(
        method,
        stack.loader(),
        Some(&stack.shared.caches.intrinsic_registry),
    ) {
        return (metadata.handler)(gc, stack, method, generics);
    }

    panic!("unsupported intrinsic {:?}", method);
}

pub fn intrinsic_field<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    field: FieldDescription,
    type_generics: Vec<ConcreteType>,
    is_address: bool,
) -> StepResult {
    vm_trace_intrinsic!(
        stack,
        "FIELD-LOAD",
        &format!("{}.{}", field.parent.type_name(), field.field.name)
    );
    if let Some(handler) = stack.shared.caches.intrinsic_registry.get_field(&field) {
        handler(gc, stack, field, type_generics, is_address)
    } else {
        panic!("unsupported load from intrinsic field: {:?}", field);
    }
}

fn math_min_double_filter(method: &MethodDescription) -> bool {
    let sig = &method.method.signature;
    if sig.parameters.len() != 2 {
        return false;
    }
    match (&sig.parameters[0].1, &sig.parameters[1].1) {
        (
            ParameterType::Value(MethodType::Base(b1)),
            ParameterType::Value(MethodType::Base(b2)),
        ) => matches!(**b1, BaseType::Float64) && matches!(**b2, BaseType::Float64),
        _ => false,
    }
}

fn object_to_string<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let this = vm_pop!(stack, gc);

    let type_name = if let StackValue::ObjectRef(obj_ref) = this {
        if obj_ref.0.is_some() {
            obj_ref.as_heap_storage(|storage| match storage {
                HeapStorage::Obj(o) => o.description.type_name(),
                HeapStorage::Str(_) => "System.String".to_string(),
                HeapStorage::Vec(_) => "System.Array".to_string(),
                HeapStorage::Boxed(_) => "System.ValueType".to_string(),
            })
        } else {
            return stack.throw_by_name(gc, "System.NullReferenceException");
        }
    } else {
        "System.Object".to_string()
    };

    let str_val = CLRString::from(type_name);
    let storage = HeapStorage::Str(str_val);
    let obj_ref = ObjectRef::new(gc, storage);
    vm_push!(stack, gc, StackValue::ObjectRef(obj_ref));
    StepResult::InstructionStepped
}
