//! This module handles .NET intrinsic methods.
//!
//! Intrinsics are methods implemented directly in the VM rather than in CIL.
//! They are used for:
//! 1. Performance (e.g., Math functions)
//! 2. Low-level operations (e.g., Unsafe, Memory manipulation)
//! 3. VM-specific functionality (e.g., Reflection, GC control)
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
//! 2.  **Register the handler**:
//!     In `src/vm/intrinsics/mod.rs`, within the `initialize_global` function, add your
//!     handler to the `define_intrinsics!` macro call:
//!     ```rust,ignore
//!     define_intrinsics!(registry, loader, {
//!         "Namespace.TypeName" {
//!             "MethodName" => submodule::my_intrinsic_handler,
//!         }
//!     });
//!     ```
//!     If the method is overloaded, specify the parameter count: `"MethodName"(1) => ...`.
//!
//! 3.  **Ensure the method is marked as intrinsic**:
//!     The method in the .NET assembly should be marked with `[IntrinsicAttribute]`
//!     or be an `InternalCall`.
use crate::{
    assemblies::AssemblyLoader,
    types::{
        generics::{ConcreteType, GenericLookup},
        members::{FieldDescription, MethodDescription},
    },
    value::{object::ObjectRef, StackValue},
    vm::{context::ResolutionContext, CallStack, GCHandle, StepResult},
    vm_msg,
};
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock, RwLock},
};

pub mod gc;
pub mod math;
pub mod reflection;
pub mod span;
pub mod string_ops;
pub mod threading;
pub mod unsafe_ops;

use reflection::{
    runtime_field_info_intrinsic_call, runtime_method_info_intrinsic_call,
    runtime_type_intrinsic_call,
};

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
);

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
macro_rules! define_intrinsics {
    ($registry:ident, $loader:ident, {
        $( $type_name:literal {
            $( $method_name:literal $( ( $params:expr ) )? => $handler:path ),* $(,)?
        } )*
    }) => {
        $(
            {
                let type_def = $loader.corlib_type($type_name);
                $(
                    {
                        let method_name = $method_name;
                        #[allow(unused_mut, unused_assignments)]
                        let mut param_count: Option<usize> = None;
                        $( param_count = Some($params); )?

                        let found_method = type_def.definition().methods.iter().find(|m| {
                            m.name == method_name && param_count.map_or(true, |p| m.signature.parameters.len() == p)
                        });

                        let handler: IntrinsicHandler = unsafe {
                            std::mem::transmute::<_, IntrinsicHandler>(
                                $handler as fn(_, _, _, _) -> _
                            )
                        };

                        if let Some(method) = found_method {
                            let method_desc = MethodDescription {
                                parent: type_def,
                                method,
                            };
                            $registry.register(method_desc, handler);
                        } else {
                            $registry.register_raw($type_name, method_name, param_count.unwrap_or(0), handler);
                        }
                    }
                )*
            }
        )*
    };
}

macro_rules! define_field_intrinsics {
    ($registry:ident, $loader:ident, {
        $( $type_name:literal {
            $( $field_name:literal => $handler:path ),* $(,)?
        } )*
    }) => {
        $(
            {
                let type_def = $loader.corlib_type($type_name);
                $(
                    {
                        let field_name = $field_name;
                        if let Some(field) = type_def.definition().fields.iter().find(|f| f.name == field_name) {
                            let field_desc = FieldDescription {
                                parent: type_def,
                                field,
                            };
                            let handler: IntrinsicFieldHandler = unsafe {
                                std::mem::transmute::<_, IntrinsicFieldHandler>(
                                    $handler as fn(_, _, _, _) -> _
                                )
                            };
                            $registry.register_field(field_desc, handler);
                        }
                    }
                )*
            }
        )*
    };
}

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
        let type_name = Arc::from(method.parent.definition().type_name());
        let member_name = Arc::from(&*method.method.name);
        let param_count = method.method.signature.parameters.len();
        Self {
            type_name,
            member_name,
            param_count: Some(param_count),
        }
    }

    /// Creates an IntrinsicKey from a FieldDescription.
    fn from_field(field: FieldDescription) -> Self {
        let type_name = Arc::from(field.parent.definition().type_name());
        let member_name = Arc::from(&*field.field.name);
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
}

impl IntrinsicRegistry {
    /// Creates a new empty registry.
    fn new() -> Self {
        Self {
            method_handlers: HashMap::new(),
            field_handlers: HashMap::new(),
        }
    }

    /// Registers an intrinsic handler for the given method.
    pub fn register(&mut self, method: MethodDescription, handler: IntrinsicHandler) {
        let key = IntrinsicKey::from_method(method);
        #[cfg(feature = "intrinsic-trace")]
        eprintln!(
            "Registering intrinsic method: {}.{}({:?} params)",
            key.type_name,
            key.member_name,
            key.param_count.unwrap_or(0)
        );
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
    ) {
        let key = IntrinsicKey {
            type_name: Arc::from(type_name),
            member_name: Arc::from(method_name),
            param_count: Some(param_count),
        };
        #[cfg(feature = "intrinsic-trace")]
        eprintln!(
            "Registering raw intrinsic method: {}.{}({} params)",
            key.type_name, key.member_name, param_count
        );
        self.method_handlers.insert(key, handler);
    }

    /// Registers an intrinsic handler for the given field.
    pub fn register_field(&mut self, field: FieldDescription, handler: IntrinsicFieldHandler) {
        let key = IntrinsicKey::from_field(field);
        #[cfg(feature = "intrinsic-trace")]
        eprintln!(
            "Registering intrinsic field: {}.{}",
            key.type_name, key.member_name
        );
        self.field_handlers.insert(key, handler);
    }

    /// Looks up an intrinsic handler for the given method.
    pub fn get(&self, method: &MethodDescription) -> Option<&IntrinsicHandler> {
        let key = IntrinsicKey::from_method(*method);
        self.method_handlers.get(&key)
    }

    /// Looks up an intrinsic handler for the given field.
    pub fn get_field(&self, field: &FieldDescription) -> Option<&IntrinsicFieldHandler> {
        let key = IntrinsicKey::from_field(*field);
        self.field_handlers.get(&key)
    }

    /// Returns the global intrinsic registry instance.
    ///
    /// Note: The registry uses RwLock to allow concurrent read access since
    /// the registry is read-only after initialization.
    pub fn global() -> &'static RwLock<IntrinsicRegistry> {
        static REGISTRY: OnceLock<RwLock<IntrinsicRegistry>> = OnceLock::new();
        REGISTRY.get_or_init(|| RwLock::new(IntrinsicRegistry::new()))
    }

    /// Initializes the registry with intrinsic handlers.
    ///
    /// This is called dynamically when we have access to the AssemblyLoader,
    /// allowing us to look up MethodDescriptions. This should be called once
    /// during VM initialization.
    #[allow(unused_variables)] // loader is used in the register_intrinsic! macro
    #[allow(clippy::missing_transmute_annotations)] // Transmute safety documented at file level
    pub fn initialize_global(loader: &AssemblyLoader) {
        let mut registry = Self::global().write().unwrap();

        // Skip if already initialized
        if !registry.method_handlers.is_empty() {
            return;
        }

        define_intrinsics!(registry, loader, {
            "System.GC" {
                "KeepAlive" => gc::intrinsic_gc_keep_alive,
                "SuppressFinalize" => gc::intrinsic_gc_suppress_finalize,
                "ReRegisterForFinalize" => gc::intrinsic_gc_reregister_for_finalize,
                "Collect"(0) => gc::intrinsic_gc_collect_0,
                "Collect"(1) => gc::intrinsic_gc_collect_1,
                "Collect"(2) => gc::intrinsic_gc_collect_2,
                "WaitForPendingFinalizers" => gc::intrinsic_gc_wait_for_pending_finalizers,
            }
            "System.Runtime.InteropServices.GCHandle" {
                "InternalAlloc" => gc::intrinsic_gchandle_internal_alloc,
                "InternalFree" => gc::intrinsic_gchandle_internal_free,
                "InternalGet" => gc::intrinsic_gchandle_internal_get,
                "InternalSet" => gc::intrinsic_gchandle_internal_set,
                "InternalAddrOfPinnedObject" => gc::intrinsic_gchandle_internal_addr_of_pinned_object,
            }
            "System.Environment" {
                "GetEnvironmentVariableCore" => gc::intrinsic_environment_get_variable_core,
            }
            "System.ArgumentNullException" {
                "ThrowIfNull" => gc::intrinsic_argument_null_exception_throw_if_null,
            }
            "System.Threading.Monitor" {
                "Exit" => threading::intrinsic_monitor_exit,
                "ReliableEnter" => threading::intrinsic_monitor_reliable_enter,
                "TryEnter_FastPath" => threading::intrinsic_monitor_try_enter_fast_path,
                "TryEnter"(3) => threading::intrinsic_monitor_try_enter_timeout_ref,
                "TryEnter"(2) => threading::intrinsic_monitor_try_enter_timeout,
            }
            "System.Threading.Interlocked" {
                "CompareExchange"(3) => threading::intrinsic_interlocked_compare_exchange,
            }
            "System.Threading.Volatile" {
                "Read" => threading::intrinsic_volatile_read,
                "Write"(2) => threading::intrinsic_volatile_write_bool,
            }
            "System.String" {
                "Equals"(1) => string_ops::intrinsic_string_equals,
                "Equals"(2) => string_ops::intrinsic_string_equals,
                "FastAllocateString"(1) => string_ops::intrinsic_string_fast_allocate_string,
                "FastAllocateString"(2) => string_ops::intrinsic_string_fast_allocate_string,
                "get_Chars"(1) => string_ops::intrinsic_string_get_chars,
                "get_Length"(0) => string_ops::intrinsic_string_get_length,
                "Concat"(3) => string_ops::intrinsic_string_concat_three_spans,
                "GetHashCodeOrdinalIgnoreCase" => string_ops::intrinsic_string_get_hash_code_ordinal_ignore_case,
                "GetPinnableReference" => string_ops::intrinsic_string_get_raw_data,
                "GetRawStringData" => string_ops::intrinsic_string_get_raw_data,
                "IndexOf"(1) => string_ops::intrinsic_string_index_of,
                "IndexOf"(2) => string_ops::intrinsic_string_index_of,
                "Substring" => string_ops::intrinsic_string_substring,
                "IsNullOrEmpty" => string_ops::intrinsic_string_is_null_or_empty,
                "op_Implicit" => span::intrinsic_string_as_span,
            }
            "System.Runtime.InteropServices.Marshal" {
                "GetLastPInvokeError" => unsafe_ops::intrinsic_marshal_get_last_pinvoke_error,
                "SetLastPInvokeError" => unsafe_ops::intrinsic_marshal_set_last_pinvoke_error,
                "SizeOf"(0) => unsafe_ops::intrinsic_marshal_size_of,
                "SizeOf"(1) => unsafe_ops::intrinsic_marshal_size_of,
                "OffsetOf"(1) => unsafe_ops::intrinsic_marshal_offset_of,
                "OffsetOf"(2) => unsafe_ops::intrinsic_marshal_offset_of,
            }
            "System.Buffer" {
                "Memmove" => unsafe_ops::intrinsic_buffer_memmove,
            }
            "System.Runtime.InteropServices.MemoryMarshal" {
                "GetArrayDataReference" => unsafe_ops::intrinsic_memory_marshal_get_array_data_reference,
            }
            "System.Runtime.CompilerServices.RuntimeHelpers" {
                "GetMethodTable" => reflection::intrinsic_runtime_helpers_get_method_table,
                "CreateSpan" => span::intrinsic_runtime_helpers_create_span,
                "IsBitwiseEquatable" => reflection::intrinsic_runtime_helpers_is_bitwise_equatable,
                "IsReferenceOrContainsReferences" => reflection::intrinsic_runtime_helpers_is_reference_or_contains_references,
                "RunClassConstructor" => reflection::intrinsic_runtime_helpers_run_class_constructor,
            }
            "System.MemoryExtensions" {
                "Equals"(3) => span::intrinsic_memory_extensions_equals_span_char,
                "AsSpan" => span::intrinsic_string_as_span,
            }
            "System.ReadOnlySpan`1" {
                "get_Item" => span::intrinsic_span_get_item,
                "get_Length" => span::intrinsic_span_get_length,
            }
            "System.Span`1" {
                "get_Length" => span::intrinsic_span_get_length,
            }
            "System.Runtime.CompilerServices.Unsafe" {
                "Add" => unsafe_ops::intrinsic_unsafe_add,
                "AreSame" => unsafe_ops::intrinsic_unsafe_are_same,
                "As"(1) => unsafe_ops::intrinsic_unsafe_as,
                "As"(2) => unsafe_ops::intrinsic_unsafe_as_generic,
                "AsRef"(1) => unsafe_ops::intrinsic_unsafe_as_ref_any,
                "SizeOf" => unsafe_ops::intrinsic_unsafe_size_of,
                "ByteOffset" => unsafe_ops::intrinsic_unsafe_byte_offset,
                "ReadUnaligned" => unsafe_ops::intrinsic_unsafe_read_unaligned,
                "WriteUnaligned" => unsafe_ops::intrinsic_unsafe_write_unaligned,
            }
            "System.Runtime.Intrinsics.Vector128" {
                "get_IsHardwareAccelerated" => math::intrinsic_vector_is_hardware_accelerated,
            }
            "System.Runtime.Intrinsics.Vector256" {
                "get_IsHardwareAccelerated" => math::intrinsic_vector_is_hardware_accelerated,
            }
            "System.Runtime.Intrinsics.Vector512" {
                "get_IsHardwareAccelerated" => math::intrinsic_vector_is_hardware_accelerated,
            }
            "System.Byte" { "CreateTruncating" => math::intrinsic_numeric_create_truncating }
            "System.SByte" { "CreateTruncating" => math::intrinsic_numeric_create_truncating }
            "System.UInt16" { "CreateTruncating" => math::intrinsic_numeric_create_truncating }
            "System.Int16" { "CreateTruncating" => math::intrinsic_numeric_create_truncating }
            "System.UInt32" { "CreateTruncating" => math::intrinsic_numeric_create_truncating }
            "System.Int32" { "CreateTruncating" => math::intrinsic_numeric_create_truncating }
            "System.UInt64" { "CreateTruncating" => math::intrinsic_numeric_create_truncating }
            "System.Int64" { "CreateTruncating" => math::intrinsic_numeric_create_truncating }
            "System.UIntPtr" { "CreateTruncating" => math::intrinsic_numeric_create_truncating }
            "System.IntPtr" { "CreateTruncating" => math::intrinsic_numeric_create_truncating }
            "System.Activator" { "CreateInstance"(0) => reflection::intrinsic_activator_create_instance }
            "System.Collections.Generic.EqualityComparer`1" { "get_Default" => math::intrinsic_equality_comparer_get_default }
            "System.Type" {
                "GetTypeFromHandle" => reflection::intrinsic_get_from_handle,
                "get_IsValueType" => reflection::intrinsic_type_get_is_value_type,
                "get_IsEnum" => reflection::intrinsic_type_get_is_enum,
                "get_IsInterface" => reflection::intrinsic_type_get_is_interface,
                "op_Equality" => reflection::intrinsic_type_op_equality,
                "op_Inequality" => reflection::intrinsic_type_op_inequality,
                "get_TypeHandle" => reflection::intrinsic_type_get_type_handle,
            }
            "System.Reflection.MethodBase" {
                "GetMethodFromHandle" => reflection::intrinsic_get_from_handle,
            }
            "System.Reflection.FieldInfo" {
                "GetFieldFromHandle" => reflection::intrinsic_get_from_handle,
            }
            "System.RuntimeTypeHandle" {
                "ToIntPtr" => reflection::intrinsic_type_handle_to_int_ptr,
            }
            "System.RuntimeMethodHandle" {
                "GetFunctionPointer" => reflection::intrinsic_method_handle_get_function_pointer,
            }
        });

        define_field_intrinsics!(registry, loader, {
            "System.IntPtr" {
                "Zero" => unsafe_ops::intrinsic_field_intptr_zero,
            }
            "System.String" {
                "Empty" => string_ops::intrinsic_field_string_empty,
            }
        });

        #[cfg(feature = "intrinsic-trace")]
        eprintln!(
            "Intrinsic registry initialized with {} methods and {} fields",
            registry.method_handlers.len(),
            registry.field_handlers.len()
        );
    }
}

/// Dispatches an intrinsic call via the registry.
///
/// This is the preferred dispatch mechanism. Returns None if no handler is found in the registry,
/// allowing fallback to macro-based matching.
pub fn dispatch_intrinsic<'gc, 'm>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> Option<StepResult> {
    let registry = IntrinsicRegistry::global().read().unwrap();
    let result = registry.get(&method);
    result.map(|handler| handler(gc, stack, method, generics))
}

// ============================================================================
// End Intrinsic Registry Infrastructure
// ============================================================================

pub fn is_intrinsic(method: MethodDescription, loader: &AssemblyLoader) -> bool {
    if method.method.internal_call {
        return true;
    }

    for a in &method.method.attributes {
        let ctor = loader.locate_attribute(method.resolution(), a);
        if ctor.parent.type_name() == INTRINSIC_ATTR {
            return true;
        }
    }

    // Check the registry for registered intrinsics
    // Note: Registry is initialized during VM startup, so no need to initialize here
    IntrinsicRegistry::global()
        .read()
        .unwrap()
        .get(&method)
        .is_some()
}

pub fn is_intrinsic_field(field: FieldDescription, loader: &AssemblyLoader) -> bool {
    for a in &field.field.attributes {
        let ctor = loader.locate_attribute(field.parent.resolution, a);
        if ctor.parent.type_name() == INTRINSIC_ATTR {
            return true;
        }
    }

    let registry = IntrinsicRegistry::global().read().unwrap();
    registry.get_field(&field).is_some()
}

pub fn intrinsic_call<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    // Note: Registry is initialized during VM startup, so no need to initialize here

    macro_rules! msg {
        ($($args:tt)*) => {
            vm_msg!(stack, $($args)*)
        }
    }
    let ctx = ResolutionContext::for_method(method, stack.loader(), generics);

    msg!("-- method marked as runtime intrinsic --");

    // Check the registry-based dispatch first.
    // This provides O(1) lookup instead of O(N) macro-based matching.
    if let Some(result) = dispatch_intrinsic(gc, stack, method, generics) {
        return result;
    }

    // For virtual intrinsic calls on base types, check the actual runtime type of 'this' to support derived types
    // This allows DotnetRs.RuntimeType to override System.Type methods
    let static_type_name = method.parent.definition().type_name();
    let should_check_runtime_type = method.method.signature.instance
        && (static_type_name == "System.Type"
            || static_type_name == "System.Reflection.MethodBase"
            || static_type_name == "System.Reflection.MethodInfo"
            || static_type_name == "System.Reflection.ConstructorInfo"
            || static_type_name == "System.Reflection.FieldInfo");

    if should_check_runtime_type && !stack.execution.stack.is_empty() {
        // Peek at the 'this' argument (it's at the bottom of the arguments on the stack)
        let num_args = method.method.signature.parameters.len() + 1; // +1 for 'this'
        let this_index = stack.execution.stack.len() - num_args;
        if let Some(this_handle) = stack.execution.stack.get(this_index) {
            let this_value = stack.get_slot(this_handle);
            if let StackValue::ObjectRef(ObjectRef(Some(obj_handle))) = this_value {
                let actual_type = ctx.get_heap_description(obj_handle);
                let actual_type_name = actual_type.definition().type_name();

                // Check if the actual runtime type is one of our specialized types
                if actual_type_name == "DotnetRs.RuntimeType" {
                    return runtime_type_intrinsic_call(gc, stack, method, generics);
                }
                if actual_type_name == "DotnetRs.MethodInfo"
                    || actual_type_name == "DotnetRs.ConstructorInfo"
                {
                    return runtime_method_info_intrinsic_call(gc, stack, method, generics);
                }
                if actual_type_name == "DotnetRs.FieldInfo" {
                    return runtime_field_info_intrinsic_call(gc, stack, method, generics);
                }
            }
        }
    }

    // Fall back to checking the static type for static methods or when runtime type check doesn't apply
    if method.parent.definition().type_name() == "DotnetRs.RuntimeType" {
        return runtime_type_intrinsic_call(gc, stack, method, generics);
    }

    if method.parent.definition().type_name() == "DotnetRs.MethodInfo"
        || method.parent.definition().type_name() == "DotnetRs.ConstructorInfo"
    {
        return runtime_method_info_intrinsic_call(gc, stack, method, generics);
    }

    if method.parent.definition().type_name() == "DotnetRs.FieldInfo" {
        return runtime_field_info_intrinsic_call(gc, stack, method, generics);
    }

    panic!("unsupported intrinsic {:?}", method);
}

pub fn intrinsic_field<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    field: FieldDescription,
    type_generics: Vec<ConcreteType>,
) {
    let registry = IntrinsicRegistry::global().read().unwrap();
    if let Some(handler) = registry.get_field(&field) {
        handler(gc, stack, field, type_generics);
    } else {
        panic!("unsupported load from intrinsic field: {:?}", field);
    }
}
