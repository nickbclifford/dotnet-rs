use crate::{
    any_match_field,
    assemblies::AssemblyLoader,
    match_field,
    types::{
        generics::{ConcreteType, GenericLookup},
        members::{FieldDescription, MethodDescription},
    },
    value::{object::ObjectRef, string::CLRString, StackValue},
    vm::{context::ResolutionContext, CallStack, GCHandle, StepResult},
    vm_msg, vm_push,
};
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock, RwLock},
};

pub mod gc;
pub mod matcher;
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
// Phase 1: Intrinsic Registry Infrastructure
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
/// replacing the previous O(N) string-based matching approach.
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
    method_name: Arc<str>,
    param_count: usize,
}

impl IntrinsicKey {
    /// Creates an IntrinsicKey from a MethodDescription.
    ///
    /// Uses Arc::from to potentially reuse string data. While this still allocates
    /// the Arc control block, it's much cheaper than allocating new strings.
    fn from_method(method: MethodDescription) -> Self {
        let type_name = Arc::from(method.parent.definition().type_name());
        let method_name = Arc::from(&*method.method.name);
        let param_count = method.method.signature.parameters.len();
        Self {
            type_name,
            method_name,
            param_count,
        }
    }
}

pub struct IntrinsicRegistry {
    handlers: HashMap<IntrinsicKey, IntrinsicHandler>,
}

impl IntrinsicRegistry {
    /// Creates a new empty registry.
    fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Registers an intrinsic handler for the given method.
    #[allow(dead_code)] // Will be used in Phase 2+
    pub fn register(&mut self, method: MethodDescription, handler: IntrinsicHandler) {
        let key = IntrinsicKey::from_method(method);
        #[cfg(feature = "intrinsic-trace")]
        eprintln!(
            "Registering intrinsic: {}.{}({} params)",
            key.type_name, key.method_name, key.param_count
        );
        self.handlers.insert(key, handler);
    }

    /// Looks up an intrinsic handler for the given method.
    /// Returns None if no handler is registered.
    ///
    /// Creates an Arc-based key for lookup. While this increments reference counts,
    /// it avoids full string allocations and is much faster than Box<str>.
    pub fn get(&self, method: &MethodDescription) -> Option<&IntrinsicHandler> {
        let key = IntrinsicKey::from_method(*method);
        self.handlers.get(&key)
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
        if !registry.handlers.is_empty() {
            return;
        }

        // Helper macro to register a handler
        macro_rules! register_intrinsic {
            ($type_name:expr, $method_name:expr, $handler:expr) => {
                let type_def = loader.corlib_type($type_name);
                if let Some(method) = type_def
                    .definition()
                    .methods
                    .iter()
                    .find(|m| m.name == $method_name)
                {
                    let method_desc = MethodDescription {
                        parent: type_def,
                        method,
                    };
                    registry.register(method_desc, $handler);
                }
            };
            ($type_name:expr, $method_name:expr, params = $param_count:expr, $handler:expr) => {
                let type_def = loader.corlib_type($type_name);
                if let Some(method) = type_def.definition().methods.iter().find(|m| {
                    m.name == $method_name && m.signature.parameters.len() == $param_count
                }) {
                    let method_desc = MethodDescription {
                        parent: type_def,
                        method,
                    };
                    registry.register(method_desc, $handler);
                }
            };
        }

        // Register GC intrinsics
        register_intrinsic!("System.GC", "KeepAlive", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                gc::intrinsic_gc_keep_alive as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.GC", "SuppressFinalize", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                gc::intrinsic_gc_suppress_finalize as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.GC", "ReRegisterForFinalize", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                gc::intrinsic_gc_reregister_for_finalize as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.GC", "Collect", params = 0, unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                gc::intrinsic_gc_collect_0 as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.GC", "Collect", params = 1, unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                gc::intrinsic_gc_collect_1 as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.GC", "Collect", params = 2, unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                gc::intrinsic_gc_collect_2 as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.GC", "WaitForPendingFinalizers", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                gc::intrinsic_gc_wait_for_pending_finalizers as fn(_, _, _, _) -> _,
            )
        });

        // Register GCHandle intrinsics
        register_intrinsic!(
            "System.Runtime.InteropServices.GCHandle",
            "InternalAlloc",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    gc::intrinsic_gchandle_internal_alloc as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!(
            "System.Runtime.InteropServices.GCHandle",
            "InternalFree",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    gc::intrinsic_gchandle_internal_free as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!(
            "System.Runtime.InteropServices.GCHandle",
            "InternalGet",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    gc::intrinsic_gchandle_internal_get as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!(
            "System.Runtime.InteropServices.GCHandle",
            "InternalSet",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    gc::intrinsic_gchandle_internal_set as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!(
            "System.Runtime.InteropServices.GCHandle",
            "InternalAddrOfPinnedObject",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    gc::intrinsic_gchandle_internal_addr_of_pinned_object as fn(_, _, _, _) -> _,
                )
            }
        );

        // Register Environment intrinsics
        register_intrinsic!("System.Environment", "GetEnvironmentVariableCore", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                gc::intrinsic_environment_get_variable_core as fn(_, _, _, _) -> _,
            )
        });

        // Register ArgumentNullException intrinsics
        register_intrinsic!("System.ArgumentNullException", "ThrowIfNull", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                gc::intrinsic_argument_null_exception_throw_if_null as fn(_, _, _, _) -> _,
            )
        });

        // Register Threading intrinsics
        register_intrinsic!("System.Threading.Monitor", "Exit", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                threading::intrinsic_monitor_exit as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.Threading.Monitor", "ReliableEnter", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                threading::intrinsic_monitor_reliable_enter as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.Threading.Monitor", "TryEnter_FastPath", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                threading::intrinsic_monitor_try_enter_fast_path as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.Threading.Monitor", "TryEnter", params = 3, unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                threading::intrinsic_monitor_try_enter_timeout_ref as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.Threading.Monitor", "TryEnter", params = 2, unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                threading::intrinsic_monitor_try_enter_timeout as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!(
            "System.Threading.Interlocked",
            "CompareExchange",
            params = 3,
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    threading::intrinsic_interlocked_compare_exchange as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!("System.Threading.Volatile", "Read", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                threading::intrinsic_volatile_read as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.Threading.Volatile", "Write", params = 2, unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                threading::intrinsic_volatile_write_bool as fn(_, _, _, _) -> _,
            )
        });

        // Register String intrinsics
        register_intrinsic!("System.String", "Equals", params = 1, unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                string_ops::intrinsic_string_equals as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.String", "Equals", params = 2, unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                string_ops::intrinsic_string_equals as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.String", "FastAllocateString", params = 1, unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                string_ops::intrinsic_string_fast_allocate_string as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.String", "FastAllocateString", params = 2, unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                string_ops::intrinsic_string_fast_allocate_string as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.String", "get_Chars", params = 1, unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                string_ops::intrinsic_string_get_chars as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.String", "get_Length", params = 0, unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                string_ops::intrinsic_string_get_length as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.String", "Concat", params = 3, unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                string_ops::intrinsic_string_concat_three_spans as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.String", "GetHashCodeOrdinalIgnoreCase", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                string_ops::intrinsic_string_get_hash_code_ordinal_ignore_case
                    as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.String", "GetPinnableReference", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                string_ops::intrinsic_string_get_raw_data as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.String", "GetRawStringData", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                string_ops::intrinsic_string_get_raw_data as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.String", "IndexOf", params = 1, unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                string_ops::intrinsic_string_index_of as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.String", "IndexOf", params = 2, unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                string_ops::intrinsic_string_index_of as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.String", "Substring", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                string_ops::intrinsic_string_substring as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.String", "IsNullOrEmpty", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                string_ops::intrinsic_string_is_null_or_empty as fn(_, _, _, _) -> _,
            )
        });

        // Register Marshal intrinsics
        register_intrinsic!(
            "System.Runtime.InteropServices.Marshal",
            "GetLastPInvokeError",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    unsafe_ops::intrinsic_marshal_get_last_pinvoke_error as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!(
            "System.Runtime.InteropServices.Marshal",
            "SetLastPInvokeError",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    unsafe_ops::intrinsic_marshal_set_last_pinvoke_error as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!("System.Buffer", "Memmove", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                unsafe_ops::intrinsic_buffer_memmove as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!(
            "System.Runtime.InteropServices.MemoryMarshal",
            "GetArrayDataReference",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    unsafe_ops::intrinsic_memory_marshal_get_array_data_reference
                        as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!(
            "System.Runtime.CompilerServices.RuntimeHelpers",
            "GetMethodTable",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    reflection::intrinsic_runtime_helpers_get_method_table as fn(_, _, _, _) -> _,
                )
            }
        );

        // Register Span and MemoryExtensions intrinsics
        register_intrinsic!("System.MemoryExtensions", "Equals", params = 3, unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                span::intrinsic_memory_extensions_equals_span_char as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.ReadOnlySpan`1", "get_Item", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                span::intrinsic_span_get_item as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.Span`1", "get_Length", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                span::intrinsic_span_get_length as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.ReadOnlySpan`1", "get_Length", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                span::intrinsic_span_get_length as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.String", "op_Implicit", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                span::intrinsic_string_as_span as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.MemoryExtensions", "AsSpan", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                span::intrinsic_string_as_span as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!(
            "System.Runtime.CompilerServices.RuntimeHelpers",
            "CreateSpan",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    span::intrinsic_runtime_helpers_create_span as fn(_, _, _, _) -> _,
                )
            }
        );

        // Register Unsafe intrinsics
        register_intrinsic!("System.Runtime.CompilerServices.Unsafe", "Add", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                unsafe_ops::intrinsic_unsafe_add as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!(
            "System.Runtime.CompilerServices.Unsafe",
            "AreSame",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    unsafe_ops::intrinsic_unsafe_are_same as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!(
            "System.Runtime.CompilerServices.Unsafe",
            "As",
            params = 1,
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    unsafe_ops::intrinsic_unsafe_as as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!(
            "System.Runtime.CompilerServices.Unsafe",
            "As",
            params = 2,
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    unsafe_ops::intrinsic_unsafe_as_generic as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!(
            "System.Runtime.CompilerServices.Unsafe",
            "AsRef",
            params = 1,
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    unsafe_ops::intrinsic_unsafe_as_ref_any as fn(_, _, _, _) -> _,
                )
            }
        );
        // We need a way to distinguish between AsRef(ref T) and AsRef(void*) if they both have 1 param.
        // Actually AsRef(ref T) has 1 param (ManagedPtr), and AsRef(void*) has 1 param (NativeInt).
        // Our registry uses param count. If they have same param count, we might have a conflict.
        // Let's check Unsafe.AsRef signatures.
        // public static ref T AsRef<T>(in T source);
        // public static ref T AsRef<T>(void* ptr);
        // Both have 1 parameter.

        register_intrinsic!("System.Runtime.CompilerServices.Unsafe", "SizeOf", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                unsafe_ops::intrinsic_unsafe_size_of as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!(
            "System.Runtime.CompilerServices.Unsafe",
            "ByteOffset",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    unsafe_ops::intrinsic_unsafe_byte_offset as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!(
            "System.Runtime.CompilerServices.Unsafe",
            "ReadUnaligned",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    unsafe_ops::intrinsic_unsafe_read_unaligned as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!(
            "System.Runtime.CompilerServices.Unsafe",
            "WriteUnaligned",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    unsafe_ops::intrinsic_unsafe_write_unaligned as fn(_, _, _, _) -> _,
                )
            }
        );

        // Register additional Marshal intrinsics
        register_intrinsic!(
            "System.Runtime.InteropServices.Marshal",
            "SizeOf",
            params = 0,
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    unsafe_ops::intrinsic_marshal_size_of as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!(
            "System.Runtime.InteropServices.Marshal",
            "SizeOf",
            params = 1,
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    unsafe_ops::intrinsic_marshal_size_of as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!(
            "System.Runtime.InteropServices.Marshal",
            "OffsetOf",
            params = 1,
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    unsafe_ops::intrinsic_marshal_offset_of as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!(
            "System.Runtime.InteropServices.Marshal",
            "OffsetOf",
            params = 2,
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    unsafe_ops::intrinsic_marshal_offset_of as fn(_, _, _, _) -> _,
                )
            }
        );

        // Register additional RuntimeHelpers intrinsics
        register_intrinsic!(
            "System.Runtime.CompilerServices.RuntimeHelpers",
            "IsBitwiseEquatable",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    reflection::intrinsic_runtime_helpers_is_bitwise_equatable
                        as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!(
            "System.Runtime.CompilerServices.RuntimeHelpers",
            "IsReferenceOrContainsReferences",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    reflection::intrinsic_runtime_helpers_is_reference_or_contains_references
                        as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!(
            "System.Runtime.CompilerServices.RuntimeHelpers",
            "RunClassConstructor",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    reflection::intrinsic_runtime_helpers_run_class_constructor
                        as fn(_, _, _, _) -> _,
                )
            }
        );

        // Register Vector and Numeric intrinsics
        register_intrinsic!(
            "System.Runtime.Intrinsics.Vector128",
            "get_IsHardwareAccelerated",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    math::intrinsic_vector_is_hardware_accelerated as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!(
            "System.Runtime.Intrinsics.Vector256",
            "get_IsHardwareAccelerated",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    math::intrinsic_vector_is_hardware_accelerated as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!(
            "System.Runtime.Intrinsics.Vector512",
            "get_IsHardwareAccelerated",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    math::intrinsic_vector_is_hardware_accelerated as fn(_, _, _, _) -> _,
                )
            }
        );

        register_intrinsic!("System.Byte", "CreateTruncating", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                math::intrinsic_numeric_create_truncating as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.SByte", "CreateTruncating", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                math::intrinsic_numeric_create_truncating as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.UInt16", "CreateTruncating", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                math::intrinsic_numeric_create_truncating as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.Int16", "CreateTruncating", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                math::intrinsic_numeric_create_truncating as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.UInt32", "CreateTruncating", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                math::intrinsic_numeric_create_truncating as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.Int32", "CreateTruncating", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                math::intrinsic_numeric_create_truncating as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.UInt64", "CreateTruncating", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                math::intrinsic_numeric_create_truncating as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.Int64", "CreateTruncating", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                math::intrinsic_numeric_create_truncating as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.UIntPtr", "CreateTruncating", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                math::intrinsic_numeric_create_truncating as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.IntPtr", "CreateTruncating", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                math::intrinsic_numeric_create_truncating as fn(_, _, _, _) -> _,
            )
        });

        // Register Misc intrinsics
        register_intrinsic!("System.Activator", "CreateInstance", params = 0, unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                reflection::intrinsic_activator_create_instance as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!(
            "System.Collections.Generic.EqualityComparer`1",
            "get_Default",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    math::intrinsic_equality_comparer_get_default as fn(_, _, _, _) -> _,
                )
            }
        );

        // Register System.Reflection and System.Type intrinsics
        register_intrinsic!("System.Type", "GetTypeFromHandle", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                reflection::intrinsic_get_from_handle as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!(
            "System.Reflection.MethodBase",
            "GetMethodFromHandle",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    reflection::intrinsic_get_from_handle as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!(
            "System.Reflection.FieldInfo",
            "GetFieldFromHandle",
            unsafe {
                std::mem::transmute::<_, IntrinsicHandler>(
                    reflection::intrinsic_get_from_handle as fn(_, _, _, _) -> _,
                )
            }
        );
        register_intrinsic!("System.RuntimeTypeHandle", "ToIntPtr", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                reflection::intrinsic_type_handle_to_int_ptr as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.RuntimeMethodHandle", "GetFunctionPointer", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                reflection::intrinsic_method_handle_get_function_pointer as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.Type", "get_IsValueType", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                reflection::intrinsic_type_get_is_value_type as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.Type", "get_IsEnum", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                reflection::intrinsic_type_get_is_enum as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.Type", "get_IsInterface", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                reflection::intrinsic_type_get_is_interface as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.Type", "op_Equality", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                reflection::intrinsic_type_op_equality as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.Type", "op_Inequality", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                reflection::intrinsic_type_op_inequality as fn(_, _, _, _) -> _,
            )
        });
        register_intrinsic!("System.Type", "get_TypeHandle", unsafe {
            std::mem::transmute::<_, IntrinsicHandler>(
                reflection::intrinsic_type_get_type_handle as fn(_, _, _, _) -> _,
            )
        });

        #[cfg(feature = "intrinsic-trace")]
        eprintln!(
            "Intrinsic registry initialized with {} handlers",
            registry.handlers.len()
        );
    }
}

/// Dispatches an intrinsic call via the registry.
///
/// This function provides the new dispatch mechanism that will eventually
/// replace the match_method! macro. Returns None if no handler is found,
/// allowing fallback to legacy code.
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
// End Phase 1 Infrastructure
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

    // Check the registry for migrated intrinsics
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

    if any_match_field!(field,
        [static System.IntPtr::Zero],
        [static System.String::Empty]
    ) {
        return true;
    }

    false
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

    // Phase 3: Check the new registry-based dispatch first
    // This provides O(1) lookup instead of O(N) string matching
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

    // Fallback to legacy string-based matching for intrinsics not in metadata
    // These are intrinsics that exist in the IL but aren't in the type metadata
    use crate::any_match_method;
    if any_match_method!(method,
        [System.String::get_Length()],
        [System.String::get_Chars(int)],
    ) {
        // Delegate to string_ops module
        return string_ops::handle_string_intrinsic(gc, stack, method, generics);
    }

    panic!("unsupported intrinsic {:?}", method);
}

pub fn intrinsic_field<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    field: FieldDescription,
    _type_generics: Vec<ConcreteType>,
) {
    match_field!(field, {
        [static System.IntPtr::Zero] => {
            stack.push_stack(gc, StackValue::NativeInt(0));
        },
        [static System.String::Empty] => {
            vm_push!(stack, gc, string(CLRString::new(vec![])));
        },
    })
    .expect("unsupported load from intrinsic field");
}
