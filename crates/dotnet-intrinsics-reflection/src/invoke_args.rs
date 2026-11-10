//! Argument binding rules for reflection invocation.
//!
//! These follow what `MethodBase.Invoke` and the default binder accept in the CLR: exact types,
//! reference conversions, primitive and enum widening, `null` as `default(T)` for value types, and
//! `Nullable<T>` accepting `null` or a boxed `T`. The classification is shared by invocation,
//! constructor selection for `Activator.CreateInstance` and `Type.GetConstructor`.

use crate::ReflectionIntrinsicHost;
use dotnet_types::{
    TypeDescription,
    comparer::decompose_type_source,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
};
use dotnet_value::{
    StackValue,
    layout::HasLayout,
    object::{HeapStorage, ObjectRef},
};
use dotnet_vm_data::StepResult;
use dotnetdll::prelude::{BaseType, MemberType, MethodType, TypeSource, UserType};

/// Primitive kinds that take part in CLR argument widening.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Prim {
    Bool,
    Char,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
}

impl Prim {
    fn from_base<T>(base: &BaseType<T>) -> Option<Self> {
        Some(match base {
            BaseType::Boolean => Self::Bool,
            BaseType::Char => Self::Char,
            BaseType::Int8 => Self::I8,
            BaseType::UInt8 => Self::U8,
            BaseType::Int16 => Self::I16,
            BaseType::UInt16 => Self::U16,
            BaseType::Int32 => Self::I32,
            BaseType::UInt32 => Self::U32,
            BaseType::Int64 => Self::I64,
            BaseType::UInt64 => Self::U64,
            BaseType::Float32 => Self::F32,
            BaseType::Float64 => Self::F64,
            _ => return None,
        })
    }

    /// The primitive a type stores as: itself for `System.*` primitives, the underlying type for
    /// enums.
    pub(crate) fn of_type(td: &TypeDescription) -> Option<Self> {
        if let Some(underlying) = td.is_enum() {
            return match underlying {
                MemberType::Base(base) => Self::from_base(&**base),
                _ => None,
            };
        }
        let definition = td.definition();
        if definition.namespace.as_deref() != Some("System") {
            return None;
        }
        Some(match definition.name.as_ref() {
            "Boolean" => Self::Bool,
            "Char" => Self::Char,
            "SByte" => Self::I8,
            "Byte" => Self::U8,
            "Int16" => Self::I16,
            "UInt16" => Self::U16,
            "Int32" => Self::I32,
            "UInt32" => Self::U32,
            "Int64" => Self::I64,
            "UInt64" => Self::U64,
            "Single" => Self::F32,
            "Double" => Self::F64,
            _ => return None,
        })
    }

    /// The CLR's `CanPrimitiveWiden` table.
    pub(crate) fn can_widen_to(self, to: Self) -> bool {
        use Prim::*;
        if self == to {
            return true;
        }
        match self {
            Bool | F64 => false,
            Char => matches!(to, U16 | I32 | U32 | I64 | U64 | F32 | F64),
            I8 => matches!(to, I16 | I32 | I64 | F32 | F64),
            U8 => matches!(to, Char | I16 | U16 | I32 | U32 | I64 | U64 | F32 | F64),
            I16 => matches!(to, I32 | I64 | F32 | F64),
            U16 => matches!(to, I32 | U32 | I64 | U64 | F32 | F64),
            I32 => matches!(to, I64 | F32 | F64),
            U32 => matches!(to, I64 | U64 | F32 | F64),
            I64 | U64 => matches!(to, F32 | F64),
            F32 => matches!(to, F64),
        }
    }

    fn read(self, data: &[u8]) -> Option<Num> {
        Some(match self {
            Self::Bool | Self::U8 => Num::Int(i128::from(*data.first()?)),
            Self::I8 => Num::Int(i128::from(*data.first()? as i8)),
            Self::Char | Self::U16 => Num::Int(i128::from(u16::from_le_bytes(
                data.get(..2)?.try_into().ok()?,
            ))),
            Self::I16 => Num::Int(i128::from(i16::from_le_bytes(
                data.get(..2)?.try_into().ok()?,
            ))),
            Self::I32 => Num::Int(i128::from(i32::from_le_bytes(
                data.get(..4)?.try_into().ok()?,
            ))),
            Self::U32 => Num::Int(i128::from(u32::from_le_bytes(
                data.get(..4)?.try_into().ok()?,
            ))),
            Self::I64 => Num::Int(i128::from(i64::from_le_bytes(
                data.get(..8)?.try_into().ok()?,
            ))),
            Self::U64 => Num::Int(i128::from(u64::from_le_bytes(
                data.get(..8)?.try_into().ok()?,
            ))),
            Self::F32 => Num::Float(f64::from(f32::from_le_bytes(
                data.get(..4)?.try_into().ok()?,
            ))),
            Self::F64 => Num::Float(f64::from_le_bytes(data.get(..8)?.try_into().ok()?)),
        })
    }

    fn stack_value<'gc>(self, value: Num) -> StackValue<'gc> {
        match (self, value) {
            (Self::F32, Num::Int(v)) => StackValue::NativeFloat(f64::from(v as f32)),
            (Self::F32, Num::Float(v)) => StackValue::NativeFloat(f64::from(v as f32)),
            (Self::F64, Num::Int(v)) => StackValue::NativeFloat(v as f64),
            (Self::F64, Num::Float(v)) => StackValue::NativeFloat(v),
            (Self::I64 | Self::U64, Num::Int(v)) => StackValue::Int64(v as i64),
            (_, Num::Int(v)) => StackValue::Int32(v as i32),
            // Widening never narrows a float into an integer.
            (_, Num::Float(v)) => StackValue::Int32(v as i32),
        }
    }
}

#[derive(Clone, Copy)]
enum Num {
    Int(i128),
    Float(f64),
}

/// A parameter type resolved far enough to classify arguments against it.
#[derive(Clone)]
pub(crate) struct ParamShape {
    pub concrete: ConcreteType,
    pub td: TypeDescription,
    pub value_type: bool,
}

/// How one argument is converted to its parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArgPlan {
    /// A reference argument that is assignable to a reference parameter.
    Reference,
    /// A boxed value of exactly the parameter type.
    Exact,
    /// A primitive or enum converted by CLR widening.
    Widen {
        from: Prim,
        to: Prim,
    },
    /// `null` for a value-type parameter: `default(T)`.
    Default,
    /// A boxed `T` for a `Nullable<T>` parameter.
    NullableSome,
    Reject,
}

pub(crate) fn param_shape<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    method: &MethodDescription,
    lookup: &GenericLookup,
    ty: &MethodType,
) -> Result<ParamShape, dotnet_types::error::TypeResolutionError> {
    let concrete = lookup.make_concrete(method.resolution(), ty.clone(), ctx.loader().as_ref())?;
    let td = ctx.loader().find_concrete_type(concrete.clone())?;
    let value_type = ctx.reflection_is_value_type_with_lookup(td.clone(), lookup)?;
    Ok(ParamShape {
        concrete,
        td,
        value_type,
    })
}

fn is_nullable(td: &TypeDescription) -> bool {
    let definition = td.definition();
    definition.name == "Nullable`1" && definition.namespace.as_deref() == Some("System")
}

/// The generic argument of a closed `Nullable<T>`.
pub(crate) fn nullable_inner(concrete: &ConcreteType) -> Option<ConcreteType> {
    let BaseType::Type { source, .. } = concrete.get() else {
        return None;
    };
    decompose_type_source::<ConcreteType>(source)
        .1
        .into_iter()
        .next()
}

/// The runtime type of an object, with its generic arguments, when it is a class instance, string
/// or box. Arrays are not described and callers must treat them as unchecked.
fn heap_concrete<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    object: ObjectRef<'gc>,
) -> Option<ConcreteType> {
    object.0?;
    let described = object.as_heap_storage(|storage| match storage {
        HeapStorage::Obj(o) | HeapStorage::Boxed(o) => {
            let arity = o.description.definition().generic_parameters.len();
            let args: Vec<_> = o
                .generics
                .type_generics
                .iter()
                .take(arity)
                .cloned()
                .collect();
            Some(Some(if arity == 0 || args.len() != arity {
                o.description.clone().into()
            } else {
                ConcreteType::new(
                    o.description.resolution.clone(),
                    BaseType::Type {
                        source: TypeSource::Generic {
                            base: UserType::Definition(o.description.index),
                            parameters: args,
                        },
                        value_kind: None,
                    },
                )
            }))
        }
        // Strings carry no generics, so the heap description is the whole type.
        HeapStorage::Str(_) => Some(None),
        HeapStorage::Vec(_) => None,
    })?;
    described.or_else(|| ctx.get_heap_description(object).ok().map(Into::into))
}

/// The declaring type of `method`, with the generic arguments of `lookup`.
fn declaring_concrete(method: &MethodDescription, lookup: &GenericLookup) -> ConcreteType {
    let parent = &method.parent;
    let arity = parent.definition().generic_parameters.len();
    let args: Vec<_> = lookup.type_generics.iter().take(arity).cloned().collect();
    if arity == 0 || args.len() != arity {
        return parent.clone().into();
    }
    ConcreteType::new(
        parent.resolution.clone(),
        BaseType::Type {
            source: TypeSource::Generic {
                base: UserType::Definition(parent.index),
                parameters: args,
            },
            value_kind: None,
        },
    )
}

/// Whether `object` can be stored in a reference-typed location of type `target`.
///
/// Arrays are accepted without a check because their covariance and generic-interface rules live
/// in the cast instructions rather than in the type comparer.
fn reference_fits<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    object: ObjectRef<'gc>,
    target: &ConcreteType,
) -> bool {
    if object.0.is_none() || matches!(target.get(), BaseType::Object) {
        return true;
    }
    if object.as_heap_storage(|storage| matches!(storage, HeapStorage::Vec(_))) {
        return true;
    }
    let Some(source) = heap_concrete(ctx, object) else {
        return true;
    };
    ctx.is_a(source, target.clone()).unwrap_or(false)
}

/// Whether `receiver` is an acceptable `this` for an instance `method`.
pub(crate) fn receiver_fits<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    method: &MethodDescription,
    lookup: &GenericLookup,
    receiver: ObjectRef<'gc>,
) -> bool {
    reference_fits(ctx, receiver, &declaring_concrete(method, lookup))
}

pub(crate) fn plan_argument<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    arg: ObjectRef<'gc>,
    shape: &ParamShape,
    byref: bool,
) -> ArgPlan {
    if !shape.value_type {
        return if reference_fits(ctx, arg, &shape.concrete) {
            ArgPlan::Reference
        } else {
            ArgPlan::Reject
        };
    }
    if arg.0.is_none() {
        return ArgPlan::Default;
    }
    let Some(boxed) = arg.as_heap_storage(|storage| match storage {
        HeapStorage::Boxed(o) => Some(o.description.clone()),
        _ => None,
    }) else {
        return ArgPlan::Reject;
    };
    if boxed == shape.td {
        return ArgPlan::Exact;
    }
    if byref {
        return ArgPlan::Reject;
    }
    if is_nullable(&shape.td) {
        let fits = nullable_inner(&shape.concrete)
            .and_then(|inner| ctx.loader().find_concrete_type(inner).ok())
            .is_some_and(|inner| inner == boxed);
        return if fits {
            ArgPlan::NullableSome
        } else {
            ArgPlan::Reject
        };
    }
    match (Prim::of_type(&boxed), Prim::of_type(&shape.td)) {
        (Some(from), Some(to)) if from.can_widen_to(to) => ArgPlan::Widen { from, to },
        _ => ArgPlan::Reject,
    }
}

/// Convert the boxed primitive or enum in `arg` to the primitive kind `to`.
pub(crate) fn widen_boxed<'gc>(
    arg: ObjectRef<'gc>,
    from: Prim,
    to: Prim,
) -> Option<StackValue<'gc>> {
    let value = arg.as_heap_storage(|storage| match storage {
        HeapStorage::Boxed(o) => o.instance_storage.with_data(|data| from.read(data)),
        _ => None,
    })?;
    Some(to.stack_value(value))
}

/// Build `Nullable<T>` holding the value boxed in `arg`.
pub(crate) fn nullable_with_value<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    shape: &ParamShape,
    arg: ObjectRef<'gc>,
) -> Result<StackValue<'gc>, StepResult> {
    let lookup = shape.concrete.make_lookup();
    let instance = ctx
        .reflection_new_object_with_lookup(shape.td.clone(), &lookup)
        .map_err(|e| StepResult::Error(e.into()))?;
    let layout = instance.instance_storage.layout().clone();
    // `Nullable<T>`'s fields are private BCL layout, so they are found by name.
    let field = |names: [&str; 2]| {
        layout
            .fields
            .iter()
            .find(|(key, _)| names.contains(&key.name.as_ref()))
            .map(|(_, value)| value)
    };
    let (Some(has_value), Some(value)) =
        (field(["hasValue", "_hasValue"]), field(["value", "_value"]))
    else {
        return Err(StepResult::internal_error(
            "Nullable<T> layout has no hasValue/value fields",
        ));
    };
    let (value_position, value_size) = (value.position.as_usize(), value.layout.size().as_usize());
    let copied = arg.as_heap_storage(|storage| match storage {
        HeapStorage::Boxed(boxed) => boxed.instance_storage.with_data(|source| {
            instance.instance_storage.with_data_mut(|target| {
                target[has_value.position.as_usize()] = 1;
                target[value_position..value_position + value_size]
                    .copy_from_slice(&source[..value_size]);
            });
            true
        }),
        _ => false,
    });
    if !copied {
        return Err(StepResult::internal_error(
            "Nullable<T> argument is not boxed",
        ));
    }
    Ok(StackValue::ValueType(instance))
}
