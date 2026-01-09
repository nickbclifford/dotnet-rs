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
//!             MethodName => submodule::my_intrinsic_handler,
//!         }
//!     });
//!     ```
//!     If the method is overloaded, specify the parameter count: `MethodName(1) => ...`.
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
    vm::{context::ResolutionContext, tracer::Tracer, CallStack, GCHandle, StepResult},
    vm_trace_intrinsic,
};
use phf::phf_set;
use std::{collections::HashMap, sync::Arc};

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

/// PHF set of types that are known to contain intrinsic methods.
/// This allows for a very fast negative check in is_intrinsic.
/// Returns a static intrinsic handler for the given method by name and parameter count.
/// This allows for compile-time resolution of intrinsics without runtime registration.
pub fn get_static_method_handler(
    type_name: &str,
    method_name: &str,
    param_count: usize,
) -> Option<IntrinsicHandler> {
    if !INTRINSIC_TYPES.contains(type_name) {
        return None;
    }

    match type_name {
        "System.GC" => match method_name {
            "KeepAlive" if param_count == 1 => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    gc::intrinsic_gc_keep_alive as fn(_, _, _, _) -> _,
                )
            }),
            "SuppressFinalize" if param_count == 1 => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    gc::intrinsic_gc_suppress_finalize as fn(_, _, _, _) -> _,
                )
            }),
            "ReRegisterForFinalize" if param_count == 1 => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    gc::intrinsic_gc_reregister_for_finalize as fn(_, _, _, _) -> _,
                )
            }),
            "Collect" => match param_count {
                0 => Some(unsafe {
                    std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                        gc::intrinsic_gc_collect_0 as fn(_, _, _, _) -> _,
                    )
                }),
                1 => Some(unsafe {
                    std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                        gc::intrinsic_gc_collect_1 as fn(_, _, _, _) -> _,
                    )
                }),
                2 => Some(unsafe {
                    std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                        gc::intrinsic_gc_collect_2 as fn(_, _, _, _) -> _,
                    )
                }),
                _ => None,
            },
            "WaitForPendingFinalizers" if param_count == 0 => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    gc::intrinsic_gc_wait_for_pending_finalizers as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.Runtime.InteropServices.GCHandle" => match method_name {
            "InternalAlloc" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    gc::intrinsic_gchandle_internal_alloc as fn(_, _, _, _) -> _,
                )
            }),
            "InternalFree" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    gc::intrinsic_gchandle_internal_free as fn(_, _, _, _) -> _,
                )
            }),
            "InternalGet" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    gc::intrinsic_gchandle_internal_get as fn(_, _, _, _) -> _,
                )
            }),
            "InternalSet" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    gc::intrinsic_gchandle_internal_set as fn(_, _, _, _) -> _,
                )
            }),
            "InternalAddrOfPinnedObject" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    gc::intrinsic_gchandle_internal_addr_of_pinned_object as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.Environment" => match method_name {
            "GetEnvironmentVariableCore" if param_count == 1 => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    gc::intrinsic_environment_get_variable_core as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.ArgumentNullException" => match method_name {
            "ThrowIfNull" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    gc::intrinsic_argument_null_exception_throw_if_null as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.Threading.Monitor" => match method_name {
            "Exit" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    threading::intrinsic_monitor_exit as fn(_, _, _, _) -> _,
                )
            }),
            "ReliableEnter" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    threading::intrinsic_monitor_reliable_enter as fn(_, _, _, _) -> _,
                )
            }),
            "TryEnter_FastPath" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    threading::intrinsic_monitor_try_enter_fast_path as fn(_, _, _, _) -> _,
                )
            }),
            "TryEnter" => match param_count {
                3 => Some(unsafe {
                    std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                        threading::intrinsic_monitor_try_enter_timeout_ref as fn(_, _, _, _) -> _,
                    )
                }),
                2 => Some(unsafe {
                    std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                        threading::intrinsic_monitor_try_enter_timeout as fn(_, _, _, _) -> _,
                    )
                }),
                _ => None,
            },
            _ => None,
        },
        "System.Threading.Interlocked" => match method_name {
            "CompareExchange" if param_count == 3 => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    threading::intrinsic_interlocked_compare_exchange as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.Threading.Volatile" => match method_name {
            "Read" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    threading::intrinsic_volatile_read as fn(_, _, _, _) -> _,
                )
            }),
            "Write" if param_count == 2 => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    threading::intrinsic_volatile_write_bool as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.String" => match method_name {
            "Equals" if param_count == 1 || param_count == 2 => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    string_ops::intrinsic_string_equals as fn(_, _, _, _) -> _,
                )
            }),
            "FastAllocateString" if param_count == 1 || param_count == 2 => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    string_ops::intrinsic_string_fast_allocate_string as fn(_, _, _, _) -> _,
                )
            }),
            "get_Chars" if param_count == 1 => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    string_ops::intrinsic_string_get_chars as fn(_, _, _, _) -> _,
                )
            }),
            "get_Length" if param_count == 0 => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    string_ops::intrinsic_string_get_length as fn(_, _, _, _) -> _,
                )
            }),
            "Concat" if param_count == 3 => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    string_ops::intrinsic_string_concat_three_spans as fn(_, _, _, _) -> _,
                )
            }),
            "GetHashCodeOrdinalIgnoreCase" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    string_ops::intrinsic_string_get_hash_code_ordinal_ignore_case
                        as fn(_, _, _, _) -> _,
                )
            }),
            "GetPinnableReference" | "GetRawStringData" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    string_ops::intrinsic_string_get_raw_data as fn(_, _, _, _) -> _,
                )
            }),
            "IndexOf" if param_count == 1 || param_count == 2 => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    string_ops::intrinsic_string_index_of as fn(_, _, _, _) -> _,
                )
            }),
            "Substring" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    string_ops::intrinsic_string_substring as fn(_, _, _, _) -> _,
                )
            }),
            "IsNullOrEmpty" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    string_ops::intrinsic_string_is_null_or_empty as fn(_, _, _, _) -> _,
                )
            }),
            "op_Implicit" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    span::intrinsic_string_as_span as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.Runtime.InteropServices.Marshal" => match method_name {
            "GetLastPInvokeError" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    unsafe_ops::intrinsic_marshal_get_last_pinvoke_error as fn(_, _, _, _) -> _,
                )
            }),
            "SetLastPInvokeError" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    unsafe_ops::intrinsic_marshal_set_last_pinvoke_error as fn(_, _, _, _) -> _,
                )
            }),
            "SizeOf" if param_count == 0 || param_count == 1 => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    unsafe_ops::intrinsic_marshal_size_of as fn(_, _, _, _) -> _,
                )
            }),
            "OffsetOf" if param_count == 1 || param_count == 2 => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    unsafe_ops::intrinsic_marshal_offset_of as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.Buffer" => match method_name {
            "Memmove" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    unsafe_ops::intrinsic_buffer_memmove as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.Runtime.InteropServices.MemoryMarshal" => match method_name {
            "GetArrayDataReference" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    unsafe_ops::intrinsic_memory_marshal_get_array_data_reference
                        as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.Runtime.CompilerServices.RuntimeHelpers" => match method_name {
            "GetMethodTable" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    reflection::intrinsic_runtime_helpers_get_method_table as fn(_, _, _, _) -> _,
                )
            }),
            "CreateSpan" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    span::intrinsic_runtime_helpers_create_span as fn(_, _, _, _) -> _,
                )
            }),
            "IsBitwiseEquatable" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    reflection::intrinsic_runtime_helpers_is_bitwise_equatable
                        as fn(_, _, _, _) -> _,
                )
            }),
            "IsReferenceOrContainsReferences" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    reflection::intrinsic_runtime_helpers_is_reference_or_contains_references
                        as fn(_, _, _, _) -> _,
                )
            }),
            "RunClassConstructor" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    reflection::intrinsic_runtime_helpers_run_class_constructor
                        as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.MemoryExtensions" => match method_name {
            "Equals" if param_count == 3 => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    span::intrinsic_memory_extensions_equals_span_char as fn(_, _, _, _) -> _,
                )
            }),
            "AsSpan" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    span::intrinsic_string_as_span as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.ReadOnlySpan`1" | "System.Span`1" => match method_name {
            "get_Item" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    span::intrinsic_span_get_item as fn(_, _, _, _) -> _,
                )
            }),
            "get_Length" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    span::intrinsic_span_get_length as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.Runtime.CompilerServices.Unsafe" => match method_name {
            "Add" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    unsafe_ops::intrinsic_unsafe_add as fn(_, _, _, _) -> _,
                )
            }),
            "AreSame" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    unsafe_ops::intrinsic_unsafe_are_same as fn(_, _, _, _) -> _,
                )
            }),
            "As" => match param_count {
                1 => Some(unsafe {
                    std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                        unsafe_ops::intrinsic_unsafe_as as fn(_, _, _, _) -> _,
                    )
                }),
                2 => Some(unsafe {
                    std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                        unsafe_ops::intrinsic_unsafe_as_generic as fn(_, _, _, _) -> _,
                    )
                }),
                _ => None,
            },
            "AsRef" if param_count == 1 => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    unsafe_ops::intrinsic_unsafe_as_ref_any as fn(_, _, _, _) -> _,
                )
            }),
            "SizeOf" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    unsafe_ops::intrinsic_unsafe_size_of as fn(_, _, _, _) -> _,
                )
            }),
            "ByteOffset" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    unsafe_ops::intrinsic_unsafe_byte_offset as fn(_, _, _, _) -> _,
                )
            }),
            "ReadUnaligned" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    unsafe_ops::intrinsic_unsafe_read_unaligned as fn(_, _, _, _) -> _,
                )
            }),
            "WriteUnaligned" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    unsafe_ops::intrinsic_unsafe_write_unaligned as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.Runtime.Intrinsics.Vector128"
        | "System.Runtime.Intrinsics.Vector256"
        | "System.Runtime.Intrinsics.Vector512" => match method_name {
            "get_IsHardwareAccelerated" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    math::intrinsic_vector_is_hardware_accelerated as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.Byte" | "System.SByte" | "System.UInt16" | "System.Int16" | "System.UInt32"
        | "System.Int32" | "System.UInt64" | "System.Int64" | "System.UIntPtr"
        | "System.IntPtr" => match method_name {
            "CreateTruncating" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    math::intrinsic_numeric_create_truncating as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.Activator" => match method_name {
            "CreateInstance" if param_count == 0 => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    reflection::intrinsic_activator_create_instance as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.Collections.Generic.EqualityComparer`1" => match method_name {
            "get_Default" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    math::intrinsic_equality_comparer_get_default as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.Type" => match method_name {
            "GetTypeFromHandle" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    reflection::intrinsic_get_from_handle as fn(_, _, _, _) -> _,
                )
            }),
            "get_IsValueType" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    reflection::intrinsic_type_get_is_value_type as fn(_, _, _, _) -> _,
                )
            }),
            "get_IsEnum" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    reflection::intrinsic_type_get_is_enum as fn(_, _, _, _) -> _,
                )
            }),
            "get_IsInterface" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    reflection::intrinsic_type_get_is_interface as fn(_, _, _, _) -> _,
                )
            }),
            "op_Equality" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    reflection::intrinsic_type_op_equality as fn(_, _, _, _) -> _,
                )
            }),
            "op_Inequality" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    reflection::intrinsic_type_op_inequality as fn(_, _, _, _) -> _,
                )
            }),
            "get_TypeHandle" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    reflection::intrinsic_type_get_type_handle as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.Reflection.MethodBase" => match method_name {
            "GetMethodFromHandle" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    reflection::intrinsic_get_from_handle as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.Reflection.FieldInfo" => match method_name {
            "GetFieldFromHandle" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    reflection::intrinsic_get_from_handle as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.RuntimeTypeHandle" => match method_name {
            "ToIntPtr" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    reflection::intrinsic_type_handle_to_int_ptr as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        "System.RuntimeMethodHandle" => match method_name {
            "GetFunctionPointer" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _) -> _, IntrinsicHandler>(
                    reflection::intrinsic_method_handle_get_function_pointer as fn(_, _, _, _) -> _,
                )
            }),
            _ => None,
        },
        _ => None,
    }
}

/// Returns a static intrinsic field handler for the given field by name.
pub fn get_static_field_handler(
    type_name: &str,
    field_name: &str,
) -> Option<IntrinsicFieldHandler> {
    match type_name {
        "System.IntPtr" => match field_name {
            "Zero" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _), IntrinsicFieldHandler>(
                    unsafe_ops::intrinsic_field_intptr_zero as fn(_, _, _, _),
                )
            }),
            _ => None,
        },
        "System.String" => match field_name {
            "Empty" => Some(unsafe {
                std::mem::transmute::<fn(_, _, _, _), IntrinsicFieldHandler>(
                    string_ops::intrinsic_field_string_empty as fn(_, _, _, _),
                )
            }),
            _ => None,
        },
        _ => None,
    }
}

static INTRINSIC_TYPES: phf::Set<&'static str> = phf_set! {
    "System.GC",
    "System.Runtime.InteropServices.GCHandle",
    "System.Environment",
    "System.ArgumentNullException",
    "System.Threading.Monitor",
    "System.Threading.Interlocked",
    "System.Threading.Volatile",
    "System.String",
    "System.Runtime.InteropServices.Marshal",
    "System.Buffer",
    "System.Runtime.InteropServices.MemoryMarshal",
    "System.Runtime.CompilerServices.RuntimeHelpers",
    "System.MemoryExtensions",
    "System.ReadOnlySpan`1",
    "System.Span`1",
    "System.Runtime.CompilerServices.Unsafe",
    "System.Runtime.Intrinsics.Vector128",
    "System.Runtime.Intrinsics.Vector256",
    "System.Runtime.Intrinsics.Vector512",
    "System.Byte",
    "System.SByte",
    "System.UInt16",
    "System.Int16",
    "System.UInt32",
    "System.Int32",
    "System.UInt64",
    "System.Int64",
    "System.IntPtr",
    "System.UIntPtr",
    "System.Single",
    "System.Double",
    "System.Math",
    "System.MathF",
    "System.Runtime.Serialization.FormatterServices",
    "System.Type",
    "System.Reflection.MethodBase",
    "System.Reflection.MethodInfo",
    "System.Reflection.ConstructorInfo",
    "System.Reflection.FieldInfo",
    "System.Reflection.RuntimeMethodInfo",
    "System.Reflection.RuntimeConstructorInfo",
    "System.Reflection.RuntimeFieldInfo",
    "DotnetRs.RuntimeType",
    "DotnetRs.MethodInfo",
    "DotnetRs.ConstructorInfo",
    "DotnetRs.FieldInfo",
};

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
        // First check static handlers
        if let Some(handler) = get_static_method_handler(
            method.parent.type_name().as_ref(),
            &method.method.name,
            method.method.signature.parameters.len(),
        ) {
            return Some(handler);
        }

        let key = IntrinsicKey::from_method(*method);
        self.method_handlers.get(&key).copied()
    }

    /// Looks up an intrinsic handler for the given field.
    pub fn get_field(&self, field: &FieldDescription) -> Option<IntrinsicFieldHandler> {
        // First check static handlers
        if let Some(handler) =
            get_static_field_handler(field.parent.type_name().as_ref(), &field.field.name)
        {
            return Some(handler);
        }

        let key = IntrinsicKey::from_field(*field);
        self.field_handlers.get(&key).copied()
    }

    /// Initializes a new registry with intrinsic handlers.
    #[allow(unused_variables)] // loader is used in the register_intrinsic! macro
    #[allow(clippy::missing_transmute_annotations)] // Transmute safety documented at file level
    pub fn initialize(loader: &AssemblyLoader, tracer: Option<&mut Tracer>) -> Self {
        let registry = Self::new();

        // Methods and fields are now handled by compile-time static handlers in mod.rs.
        // initialize is kept for any future dynamic registration needs.

        if let Some(tracer) = tracer {
            tracer.trace_intrinsic(
                0,
                "INIT",
                "Intrinsic registry initialized (static handlers preferred)",
            );
        }

        registry
    }
}

/// Dispatches an intrinsic call via the registry.
pub fn dispatch_intrinsic<'gc, 'm>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> Option<StepResult> {
    let result = stack.shared.intrinsic_registry.get(&method);
    result.map(|handler| handler(gc, stack, method, generics))
}

// ============================================================================
// End Intrinsic Registry Infrastructure
// ============================================================================

pub fn is_intrinsic(
    method: MethodDescription,
    loader: &AssemblyLoader,
    registry: &IntrinsicRegistry,
) -> bool {
    if method.method.internal_call {
        return true;
    }

    // Fast path: check if the type is known to have intrinsics
    if !INTRINSIC_TYPES.contains(method.parent.type_name().as_ref()) {
        return false;
    }

    // Check static handlers first (compile-time resolution)
    if get_static_method_handler(
        method.parent.type_name().as_ref(),
        &method.method.name,
        method.method.signature.parameters.len(),
    )
    .is_some()
    {
        return true;
    }

    // Check the registry for dynamic intrinsics
    if registry.get(&method).is_some() {
        return true;
    }

    // Check for IntrinsicAttribute
    for a in &method.method.attributes {
        let ctor = loader.locate_attribute(method.resolution(), a);
        if ctor.parent.type_name() == INTRINSIC_ATTR {
            return true;
        }
    }

    false
}

pub fn is_intrinsic_field(
    field: FieldDescription,
    loader: &AssemblyLoader,
    registry: &IntrinsicRegistry,
) -> bool {
    // Fast path: check if the type is known to have intrinsics
    if !INTRINSIC_TYPES.contains(field.parent.type_name().as_ref()) {
        return false;
    }

    // Check static handlers first
    if get_static_field_handler(field.parent.type_name().as_ref(), &field.field.name).is_some() {
        return true;
    }

    // Check for IntrinsicAttribute
    for a in &field.field.attributes {
        let ctor = loader.locate_attribute(field.parent.resolution, a);
        if ctor.parent.type_name() == INTRINSIC_ATTR {
            return true;
        }
    }

    registry.get_field(&field).is_some()
}

pub fn intrinsic_call<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    // Note: Registry is initialized during VM startup, so no need to initialize here

    let ctx = ResolutionContext::for_method(method, stack.loader(), generics, stack.shared.clone());

    vm_trace_intrinsic!(
        stack,
        "CALL",
        &format!("{}.{}", method.parent.type_name(), method.method.name)
    );

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
    vm_trace_intrinsic!(
        stack,
        "FIELD-LOAD",
        &format!("{}.{}", field.parent.type_name(), field.field.name)
    );
    if let Some(handler) = stack.shared.intrinsic_registry.get_field(&field) {
        handler(gc, stack, field, type_generics);
    } else {
        panic!("unsupported load from intrinsic field: {:?}", field);
    }
}
