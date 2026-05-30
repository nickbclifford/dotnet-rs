use dotnet_types::{
    comparer::decompose_type_source, error::TypeResolutionError, generics::ConcreteType,
    resolution::ResolutionS,
};
use dotnet_value::layout::{FieldLayoutManager, LayoutManager, Scalar};
use dotnet_vm_ops::ops::PInvokeContext;
use dotnetdll::prelude::*;
use libffi::middle::Type;

pub(crate) fn type_to_layout(
    res: ResolutionS,
    t: &TypeSource<ConcreteType>,
    ctx: &dyn PInvokeContext<'_>,
) -> FieldLayoutManager {
    let (_, _type_generics) = decompose_type_source::<ConcreteType>(t);
    // Since we can't easily create a new resolution context here without ResolutionContext struct,
    // we assume the caller has provided a ctx that already has the correct generics if needed,
    // or we'd need a more complex trait.
    // For P/Invoke marshalling of structs, usually the type is already concrete or resolved.

    let concrete = ConcreteType::new(
        res,
        BaseType::Type {
            source: t.clone(),
            value_kind: Some(ValueKind::ValueType),
        },
    );
    let td = ctx
        .loader()
        .find_concrete_type(concrete)
        .expect("failed to resolve type in pinvoke");

    ctx.instance_field_layout(td)
        .expect("P/Invoke layout resolution failed")
}

pub(crate) fn layout_to_ffi(l: &LayoutManager) -> Type {
    match l {
        LayoutManager::Field(f) => {
            let mut fields: Vec<_> = f.fields.values().collect();
            fields.sort_by_key(|f| f.position);

            Type::structure(fields.into_iter().map(|f| layout_to_ffi(&f.layout)))
        }
        LayoutManager::Array(a) => {
            let elem_type = layout_to_ffi(&a.element_layout);
            Type::structure(std::iter::repeat_n(elem_type, a.length))
        }
        LayoutManager::Scalar(s) => match s {
            Scalar::Int8 => Type::i8(),
            Scalar::UInt8 => Type::u8(),
            Scalar::Int16 => Type::i16(),
            Scalar::UInt16 => Type::u16(),
            Scalar::Int32 => Type::i32(),
            Scalar::Int64 => Type::i64(),
            Scalar::ObjectRef => Type::pointer(),
            Scalar::NativeInt => Type::isize(),
            Scalar::Float32 => Type::f32(),
            Scalar::Float64 => Type::f64(),
            Scalar::ManagedPtr => Type::pointer(),
        },
    }
}

pub(crate) fn type_to_ffi(t: &ConcreteType, ctx: &dyn PInvokeContext<'_>) -> Type {
    match t.get() {
        BaseType::Boolean => Type::u8(),
        BaseType::Char => Type::u16(),
        BaseType::Int8 => Type::i8(),
        BaseType::UInt8 => Type::u8(),
        BaseType::Int16 => Type::i16(),
        BaseType::UInt16 => Type::u16(),
        BaseType::Int32 => Type::i32(),
        BaseType::UInt32 => Type::u32(),
        BaseType::Int64 => Type::i64(),
        BaseType::UInt64 => Type::u64(),
        BaseType::Float32 => Type::f32(),
        BaseType::Float64 => Type::f64(),
        BaseType::IntPtr => Type::isize(),
        BaseType::UIntPtr => Type::usize(),
        BaseType::ValuePointer(_, _) | BaseType::FunctionPointer(_) => Type::pointer(),
        BaseType::Type {
            value_kind: Some(ValueKind::ValueType),
            source,
        } => {
            let layout = type_to_layout(t.resolution(), source, ctx);
            layout_to_ffi(&layout.into())
        }
        BaseType::Type {
            value_kind: None | Some(ValueKind::Class),
            ..
        } => Type::pointer(),
        BaseType::Array { .. } | BaseType::Vector { .. } | BaseType::String | BaseType::Object => {
            Type::pointer()
        }
    }
}

pub(crate) fn param_to_type(
    p: &ParameterType<MethodType>,
    ctx: &dyn PInvokeContext<'_>,
) -> Result<Type, TypeResolutionError> {
    match p {
        ParameterType::Value(t) => Ok(type_to_ffi(&ctx.make_concrete(t)?, ctx)),
        ParameterType::Ref(_) => Ok(Type::pointer()),
        ParameterType::TypedReference => {
            Ok(Type::structure(vec![Type::pointer(), Type::pointer()]))
        }
    }
}
