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
//! All method calls flow through a unified dispatch pipeline (see `src/dispatch/mod.rs`):
//! 1. **Generic resolution**: Resolve the method from instruction operand.
//! 2. **Virtual resolution**: Resolve the runtime type-specific method (if virtual).
//! 3. **Intrinsic check**: Use [`classify_intrinsic`] and the generated PHF table to resolve an
//!    intrinsic metadata entry containing a `MethodIntrinsicId`.
//! 4. **Execution**: Dispatch by ID (`dispatch_method_intrinsic`), P/Invoke, or managed bytecode.
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
//!     pub fn math_min_double<'gc, T: VesOps<'gc>>(
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
//!   ├─→ intrinsic_call() → dispatch_method_intrinsic(id, ...) [if intrinsic]
//!   ├─→ external_call() [if P/Invoke]
//!   └─→ call_frame() [managed CIL]
//! ```
use crate::{
    error::ExecutionError,
    instructions::objects::get_ptr_info,
    stack::ops::{
        EvalStackOps, ExceptionOps, LoaderOps, MemoryOps, RawMemoryOps, ReflectionOps,
        TypedStackOps, VesOps, VmReflectionOps,
    },
};
use dotnet_assemblies::AssemblyLoader;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    runtime::{RuntimeType, runtime_type_from_concrete},
};
use dotnet_value::{
    StackValue,
    layout::{LayoutManager, Scalar},
    object::{HeapStorage, Object, ObjectRef},
    pointer::ManagedPtr,
    string::CLRString,
};
use dotnet_vm_ops::NULL_REF_MSG;
use dotnetdll::prelude::{BaseType, TypeSource, UserType};

use std::sync::Arc;

pub mod app_context;
pub mod constants;
pub mod cpu_intrinsics;
pub mod diagnostics;
pub mod exceptions;
pub mod gc;
pub mod metadata;
pub mod object_ops;
pub mod static_registry;

include!(concat!(env!("OUT_DIR"), "/intrinsics_dispatch.rs"));
include!(concat!(env!("OUT_DIR"), "/intrinsics_phf.rs"));

pub mod text_ops;

pub use metadata::{IntrinsicKind, IntrinsicMetadata, classify_intrinsic};

use super::{StepResult, context::ResolutionContext};

pub const INTRINSIC_ATTR: &str = "System.Runtime.CompilerServices.IntrinsicAttribute";

#[cold]
#[inline(never)]
fn unsupported_intrinsic_step_result(method: &MethodDescription) -> StepResult {
    StepResult::Error(
        ExecutionError::NotImplemented(format!("unsupported intrinsic {:?}", method).into()).into(),
    )
}

#[cold]
#[inline(never)]
fn unsupported_intrinsic_field_step_result(field: &FieldDescription) -> StepResult {
    StepResult::Error(
        ExecutionError::NotImplemented(
            format!("unsupported load from intrinsic field: {:?}", field).into(),
        )
        .into(),
    )
}

// ============================================================================
// Intrinsic Registry Infrastructure
// ============================================================================

/// Type aliases for generated intrinsic dispatch IDs.
///
/// `MethodIntrinsicId` and `FieldIntrinsicId` are generated by `build.rs` into
/// `$OUT_DIR/intrinsics_dispatch.rs` from `#[dotnet_intrinsic]` and
/// `#[dotnet_intrinsic_field]` annotations across VM/intrinsic crates.
pub type IntrinsicMethodId = MethodIntrinsicId;

pub type IntrinsicFieldId = FieldIntrinsicId;

pub fn missing_intrinsic_handler<'gc, T: VesOps<'gc>>(
    _ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    StepResult::Error(
        ExecutionError::NotImplemented(
            format!("Missing intrinsic implementation for method: {:?}", method).into(),
        )
        .into(),
    )
}

/// Registry for intrinsic method/field dispatch IDs.
///
/// This provides O(1) lookup via generated PHF keys from method/field metadata,
/// replacing the older O(N) macro-based matching approach.
///
/// The registry is lazily initialized on first use via OnceLock.
pub struct IntrinsicRegistry;

impl IntrinsicRegistry {
    /// Initializes a new registry facade over generated static tables.
    pub fn initialize() -> Self {
        Self
    }

    /// Looks up a generated intrinsic method ID for the given method.
    pub fn get(
        &self,
        method: &MethodDescription,
        loader: &AssemblyLoader,
    ) -> Option<IntrinsicMethodId> {
        self.get_metadata(method, loader).map(|m| m.handler)
    }

    /// Looks up a generated intrinsic field ID for the given field.
    pub fn get_field(
        &self,
        field: &FieldDescription,
        loader: &AssemblyLoader,
    ) -> Option<IntrinsicFieldId> {
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
        let is_static = !method.signature().instance;
        let arity = if method.signature().instance {
            method.signature().parameters.len() + 1
        } else {
            method.signature().parameters.len()
        };
        let raw_type_name = method.parent.type_name();
        let canonical_name = loader.canonical_type_name(&raw_type_name);
        let normalized_name = canonical_name.replace('/', "+");
        write!(
            w,
            "M:{}::{}#{}:{}",
            normalized_name,
            &*method.method().name,
            arity,
            if is_static { "S" } else { "I" }
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
        write!(
            w,
            "F:{}::{}:{}",
            normalized_name,
            &*field.field().name,
            if field.field().static_member {
                "S"
            } else {
                "I"
            }
        )
        .ok()?;
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
    let res = field.parent.resolution.definition();
    let field_attrs = res
        .field_index(field.parent.index, field.index)
        .and_then(|idx| res.field_attributes(idx).ok())
        .unwrap_or_default();
    for a in field_attrs {
        if let Ok(ctor) = loader.locate_attribute(field.parent.resolution.clone(), &a)
            && ctor.parent.type_name() == INTRINSIC_ATTR
        {
            return true;
        }
    }

    false
}

pub fn intrinsic_call<'gc, T: VesOps<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _res_ctx = ResolutionContext::for_method(
        method.clone(),
        ctx.loader_arc(),
        generics,
        ctx.shared().caches.clone(),
        Some(Arc::downgrade(ctx.shared())),
    );

    vm_trace_intrinsic!(
        ctx,
        "CALL",
        &format!("{}.{}", method.parent.type_name(), method.method().name)
    );

    #[cfg(feature = "bench-instrumentation")]
    {
        let arity = if method.signature().instance {
            method.signature().parameters.len() + 1
        } else {
            method.signature().parameters.len()
        };
        let signature = format!(
            "{}::{}#{}",
            method.parent.type_name(),
            method.method().name,
            arity
        );
        ctx.shared()
            .metrics
            .record_intrinsic_signature_call(signature);
    }

    let metadata = classify_intrinsic(
        method.clone(),
        ctx.loader(),
        Some(&ctx.shared().caches.intrinsic_registry),
    );
    // `intrinsic_call` is entered from paths that already classify/cached the method as
    // intrinsic, so this branch should almost always be taken.
    if vm_likely!(metadata.is_some()) {
        let metadata = metadata.unwrap();
        ctx.set_current_intrinsic(Some(method.clone()));
        let res = dispatch_method_intrinsic(metadata.handler, ctx, method, generics);
        ctx.set_current_intrinsic(None);
        return res;
    }

    unsupported_intrinsic_step_result(&method)
}

pub fn intrinsic_field<'gc, T: VesOps<'gc>>(
    ctx: &mut T,
    field: FieldDescription,
    type_generics: Arc<[ConcreteType]>,
    is_address: bool,
) -> StepResult {
    vm_trace_intrinsic!(
        ctx,
        "FIELD-LOAD",
        &format!("{}.{}", field.parent.type_name(), field.field().name)
    );
    if let Some(handler) = ctx
        .shared()
        .caches
        .intrinsic_registry
        .get_field(&field, ctx.loader())
    {
        dispatch_field_intrinsic(handler, ctx, field, type_generics, is_address)
    } else {
        unsupported_intrinsic_field_step_result(&field)
    }
}

#[dotnet_intrinsic("string System.Object::ToString()")]
fn object_to_string<
    'gc,
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
    let obj_ref = ObjectRef::new(ctx.gc_with_token(&ctx.no_active_borrows_token()), storage);
    ctx.push_obj(obj_ref);
    StepResult::Continue
}

fn integral_constant_as_i128(c: &dotnetdll::prelude::Constant) -> Option<i128> {
    use dotnetdll::prelude::Constant;
    match c {
        Constant::Boolean(v) => Some(i128::from(u8::from(*v))),
        Constant::Char(v) => Some(i128::from(*v)),
        Constant::Int8(v) => Some(i128::from(*v)),
        Constant::UInt8(v) => Some(i128::from(*v)),
        Constant::Int16(v) => Some(i128::from(*v)),
        Constant::UInt16(v) => Some(i128::from(*v)),
        Constant::Int32(v) => Some(i128::from(*v)),
        Constant::UInt32(v) => Some(i128::from(*v)),
        Constant::Int64(v) => Some(i128::from(*v)),
        Constant::UInt64(v) => Some(i128::from(*v)),
        Constant::Float32(_) | Constant::Float64(_) | Constant::String(_) | Constant::Null => None,
    }
}

fn read_enum_value(obj: &Object<'_>) -> Option<(i128, bool)> {
    let enum_type = obj.description.clone();
    let underlying = enum_type.is_enum()?;
    let method_type: dotnetdll::prelude::MethodType = underlying.clone().into();
    let dotnetdll::prelude::MethodType::Base(base) = method_type else {
        return None;
    };

    match &*base {
        dotnetdll::prelude::BaseType::Int8 => Some((
            i128::from(
                obj.instance_storage
                    .field::<i8>(enum_type, "value__")?
                    .read(),
            ),
            true,
        )),
        dotnetdll::prelude::BaseType::UInt8 => Some((
            i128::from(
                obj.instance_storage
                    .field::<u8>(enum_type, "value__")?
                    .read(),
            ),
            false,
        )),
        dotnetdll::prelude::BaseType::Int16 => Some((
            i128::from(
                obj.instance_storage
                    .field::<i16>(enum_type, "value__")?
                    .read(),
            ),
            true,
        )),
        dotnetdll::prelude::BaseType::UInt16 => Some((
            i128::from(
                obj.instance_storage
                    .field::<u16>(enum_type, "value__")?
                    .read(),
            ),
            false,
        )),
        dotnetdll::prelude::BaseType::Int32 => Some((
            i128::from(
                obj.instance_storage
                    .field::<i32>(enum_type, "value__")?
                    .read(),
            ),
            true,
        )),
        dotnetdll::prelude::BaseType::UInt32 => Some((
            i128::from(
                obj.instance_storage
                    .field::<u32>(enum_type, "value__")?
                    .read(),
            ),
            false,
        )),
        dotnetdll::prelude::BaseType::Int64 => Some((
            i128::from(
                obj.instance_storage
                    .field::<i64>(enum_type, "value__")?
                    .read(),
            ),
            true,
        )),
        dotnetdll::prelude::BaseType::UInt64 => Some((
            i128::from(
                obj.instance_storage
                    .field::<u64>(enum_type, "value__")?
                    .read(),
            ),
            false,
        )),
        dotnetdll::prelude::BaseType::IntPtr => Some((
            obj.instance_storage
                .field::<isize>(enum_type, "value__")?
                .read() as i128,
            true,
        )),
        dotnetdll::prelude::BaseType::UIntPtr => Some((
            obj.instance_storage
                .field::<usize>(enum_type, "value__")?
                .read() as i128,
            false,
        )),
        _ => None,
    }
}

fn enum_object_from_stack<'gc, T: ExceptionOps<'gc>>(
    ctx: &mut T,
    value: StackValue<'gc>,
) -> Result<Object<'gc>, StepResult> {
    let enum_obj = match value {
        StackValue::ObjectRef(obj_ref) => {
            if obj_ref.0.is_none() {
                return Err(
                    ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG)
                );
            }

            obj_ref.as_heap_storage(|storage| match storage {
                HeapStorage::Boxed(obj) | HeapStorage::Obj(obj) => Some((**obj).clone()),
                HeapStorage::Str(_) | HeapStorage::Vec(_) => None,
            })
        }
        StackValue::ManagedPtr(ptr) => {
            if ptr.is_null() {
                return Err(
                    ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG)
                );
            }

            ptr.owner().and_then(|owner| {
                owner.as_heap_storage(|storage| match storage {
                    HeapStorage::Boxed(obj) | HeapStorage::Obj(obj) => Some((**obj).clone()),
                    HeapStorage::Str(_) | HeapStorage::Vec(_) => None,
                })
            })
        }
        StackValue::ValueType(vt) => Some(vt),
        _ => None,
    };

    enum_obj.ok_or_else(|| {
        ctx.throw_by_name_with_message(
            "System.InvalidCastException",
            "Specified cast is not valid.",
        )
    })
}

fn format_enum_value_from_type(
    enum_type: &TypeDescription,
    raw_value: i128,
    signed: bool,
) -> String {
    for field in &enum_type.definition().fields {
        if !field.literal {
            continue;
        }

        let Some(constant) = field.default.as_ref() else {
            continue;
        };

        if integral_constant_as_i128(constant) == Some(raw_value) {
            return field.name.to_string();
        }
    }

    if signed {
        raw_value.to_string()
    } else {
        (raw_value as u128).to_string()
    }
}

fn scalar_enum_value_from_stack<'gc>(value: StackValue<'gc>) -> Option<(i128, bool)> {
    match value.coerce_enum_to_underlying() {
        StackValue::Int32(v) => Some((i128::from(v), true)),
        StackValue::Int64(v) => Some((i128::from(v), true)),
        StackValue::NativeInt(v) => Some((v as i128, true)),
        StackValue::ValueType(vt) => read_enum_value(&vt),
        _ => None,
    }
}

fn enum_type_from_stack_value<'gc>(value: &StackValue<'gc>) -> Option<TypeDescription> {
    match value {
        StackValue::ValueType(vt) => Some(vt.description.clone()),
        StackValue::ObjectRef(obj_ref) => obj_ref.as_heap_storage(|storage| match storage {
            HeapStorage::Boxed(obj) | HeapStorage::Obj(obj) => Some(obj.description.clone()),
            HeapStorage::Str(_) | HeapStorage::Vec(_) => None,
        }),
        StackValue::ManagedPtr(ptr) => ptr.owner().and_then(|owner| {
            owner.as_heap_storage(|storage| match storage {
                HeapStorage::Boxed(obj) | HeapStorage::Obj(obj) => Some(obj.description.clone()),
                HeapStorage::Str(_) | HeapStorage::Vec(_) => None,
            })
        }),
        _ => None,
    }
}

fn generic_enum_type(generics: &GenericLookup) -> Option<TypeDescription> {
    let concrete = generics
        .method_generics
        .first()
        .or_else(|| generics.type_generics.first())?;

    let dotnetdll::prelude::BaseType::Type {
        source: TypeSource::User(UserType::Definition(index)),
        ..
    } = concrete.get()
    else {
        return None;
    };

    Some(TypeDescription::new(concrete.resolution(), *index))
}

fn format_enum_value<'gc, T: ExceptionOps<'gc>>(
    ctx: &mut T,
    value: StackValue<'gc>,
) -> Result<String, StepResult> {
    let enum_obj = enum_object_from_stack(ctx, value)?;
    let enum_type = enum_obj.description.clone();
    let Some((raw_value, signed)) = read_enum_value(&enum_obj) else {
        return Err(ctx.throw_by_name_with_message(
            "System.InvalidCastException",
            "Specified cast is not valid.",
        ));
    };

    Ok(format_enum_value_from_type(&enum_type, raw_value, signed))
}

fn write_i32_out_arg<'gc, T: ExceptionOps<'gc> + RawMemoryOps<'gc>>(
    ctx: &mut T,
    out_arg: &StackValue<'gc>,
    value: i32,
) -> Result<(), StepResult> {
    let (origin, offset) = get_ptr_info(ctx, out_arg)?;
    let layout = LayoutManager::Scalar(Scalar::Int32);
    unsafe {
        ctx.write_unaligned(origin, offset, StackValue::Int32(value), &layout)
            .map_err(|e| StepResult::Error(e.into()))
    }
}

fn try_write_utf16_to_span<'gc, T: RawMemoryOps<'gc>>(
    ctx: &mut T,
    destination: &StackValue<'gc>,
    chars: &[u16],
) -> Result<bool, StepResult> {
    let span = match destination {
        StackValue::ValueType(span) => span.clone(),
        other => {
            return Err(StepResult::type_error(
                "System.Span<char>",
                format!("{:?}", other),
            ));
        }
    };

    let span_len = dotnet_intrinsics_span::helpers::read_span_length(&span)
        .map_err(|e| StepResult::Error(e.into()))?;
    if span_len < 0 {
        return Ok(false);
    }

    let required_len = chars.len();
    if (span_len as usize) < required_len {
        return Ok(false);
    }

    if required_len == 0 {
        return Ok(true);
    }

    let span_ref = dotnet_intrinsics_span::helpers::read_span_reference(&span)
        .map_err(|e| StepResult::Error(e.into()))?;
    let span_ptr = ManagedPtr::from_info_full(span_ref, TypeDescription::NULL, false);

    let mut bytes = Vec::with_capacity(required_len * 2);
    for ch in chars {
        bytes.extend_from_slice(&ch.to_ne_bytes());
    }

    unsafe {
        ctx.write_bytes(span_ptr.origin().clone(), span_ptr.byte_offset(), &bytes)
            .map_err(|e| StepResult::Error(e.into()))?;
    }

    Ok(true)
}

#[dotnet_intrinsic("string System.Enum::ToString()")]
fn enum_to_string<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let this = ctx.pop();
    let formatted = match format_enum_value(ctx, this) {
        Ok(v) => v,
        Err(step) => return step,
    };

    ctx.push_string(CLRString::from(formatted));
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static bool System.Enum::TryFormatUnconstrained<M0>(M0, System.Span<char>, int&, System.ReadOnlySpan<char>)"
)]
fn enum_try_format_unconstrained<
    'gc,
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc> + LoaderOps,
>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _format = ctx.pop();
    let chars_written_out = ctx.pop();
    let destination = ctx.pop();
    let value = ctx.pop();

    let Some((raw_value, signed)) = scalar_enum_value_from_stack(value.clone()) else {
        if let Err(step) = write_i32_out_arg(ctx, &chars_written_out, 0) {
            return step;
        }
        ctx.push_i32(0);
        return StepResult::Continue;
    };

    let enum_type = enum_type_from_stack_value(&value).or_else(|| generic_enum_type(generics));
    let formatted = if let Some(enum_type) = enum_type {
        format_enum_value_from_type(&enum_type, raw_value, signed)
    } else if signed {
        raw_value.to_string()
    } else {
        (raw_value as u128).to_string()
    };

    let formatted_utf16: Vec<u16> = formatted.encode_utf16().collect();
    let wrote = match try_write_utf16_to_span(ctx, &destination, &formatted_utf16) {
        Ok(wrote) => wrote,
        Err(step) => return step,
    };

    if wrote {
        let chars_written = i32::try_from(formatted_utf16.len()).unwrap_or(i32::MAX);
        if let Err(step) = write_i32_out_arg(ctx, &chars_written_out, chars_written) {
            return step;
        }
        ctx.push_i32(1);
    } else {
        if let Err(step) = write_i32_out_arg(ctx, &chars_written_out, 0) {
            return step;
        }
        ctx.push_i32(0);
    }

    StepResult::Continue
}

#[dotnet_intrinsic("System.Type System.Object::GetType()")]
fn object_get_type<
    'gc,
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + ReflectionOps<'gc> + VmReflectionOps<'gc> + LoaderOps,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    fn runtime_type_from_desc(td: dotnet_types::TypeDescription) -> RuntimeType {
        match td.type_name().as_str() {
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
            _ => RuntimeType::Type(td),
        }
    }

    fn runtime_type_from_object(loader: &AssemblyLoader, object: &Object<'_>) -> RuntimeType {
        let type_arity = object.description.definition().generic_parameters.len();
        if type_arity == 0 {
            return runtime_type_from_desc(object.description.clone());
        }

        let type_args: Vec<_> = object
            .generics
            .type_generics
            .iter()
            .take(type_arity)
            .cloned()
            .collect();

        if type_args.len() != type_arity {
            return RuntimeType::Type(object.description.clone());
        }

        let concrete = ConcreteType::new(
            object.description.resolution.clone(),
            BaseType::Type {
                source: TypeSource::Generic {
                    base: UserType::Definition(object.description.index),
                    parameters: type_args,
                },
                value_kind: None,
            },
        );

        runtime_type_from_concrete(loader, &concrete)
            .unwrap_or_else(|| RuntimeType::Type(object.description.clone()))
    }

    fn runtime_type_from_heap(loader: &AssemblyLoader, object_ref: ObjectRef<'_>) -> RuntimeType {
        object_ref.as_heap_storage(|storage| match storage {
            HeapStorage::Obj(o) => runtime_type_from_object(loader, o),
            HeapStorage::Str(_) => RuntimeType::String,
            HeapStorage::Vec(v) => {
                let element_rt =
                    runtime_type_from_concrete(loader, &v.element).unwrap_or(RuntimeType::Object);
                if v.dims.len() <= 1 {
                    RuntimeType::Vector(Box::new(element_rt))
                } else {
                    RuntimeType::Array(Box::new(element_rt), v.dims.len() as u32)
                }
            }
            HeapStorage::Boxed(o) => runtime_type_from_object(loader, o),
        })
    }

    let this = ctx.pop();
    let rt = match this {
        StackValue::ObjectRef(this_ref) => {
            if this_ref.0.is_none() {
                return ctx
                    .throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
            }
            runtime_type_from_heap(ctx.loader().as_ref(), this_ref)
        }
        StackValue::ManagedPtr(this_ptr) => {
            if this_ptr.is_null() {
                return ctx
                    .throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
            }

            if let Some(owner) = this_ptr.owner() {
                runtime_type_from_heap(ctx.loader().as_ref(), owner)
            } else {
                runtime_type_from_desc(this_ptr.inner_type())
            }
        }
        StackValue::ValueType(value) => runtime_type_from_desc(value.description.clone()),
        other => {
            return StepResult::Error(
                ExecutionError::TypeMismatch {
                    expected: "ObjectRef/ManagedPtr/ValueType",
                    actual: format!("{other:?}").into(),
                }
                .into(),
            );
        }
    };

    let typ_obj = ctx.get_runtime_type(rt);
    ctx.push_obj(typ_obj);
    StepResult::Continue
}
