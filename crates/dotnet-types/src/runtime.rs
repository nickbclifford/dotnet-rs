use crate::{
    TypeDescription, TypeResolver, generics::ConcreteType, members::MethodDescription,
    resolution::ResolutionS,
};
use dotnetdll::prelude::{BaseType, TypeSource, UserType, ValueKind};
use gc_arena::{Collect, unsafe_empty_collect};
use std::{fmt::Debug, hash::Hash};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RuntimeMethodSignature; // TODO

runtime_type_impls! {
    simple_types: {
        Boolean => "Boolean",
        Char => "Char",
        Int8 => "SByte",
        UInt8 => "Byte",
        Int16 => "Int16",
        UInt16 => "UInt16",
        Int32 => "Int32",
        UInt32 => "UInt32",
        Int64 => "Int64",
        UInt64 => "UInt64",
        Float32 => "Single",
        Float64 => "Double",
        IntPtr => "IntPtr",
        UIntPtr => "UIntPtr",
        Object => "Object",
        String => "String",
    },
    complex_types: {
        Void,
        TypedReference,
        Type(TypeDescription),
        Generic(TypeDescription, Vec<RuntimeType>),
        Vector(Box<RuntimeType>),
        Array(Box<RuntimeType>, u32),
        Pointer(Box<RuntimeType>),
        ByRef(Box<RuntimeType>),
        ValuePointer(Box<RuntimeType>, bool),
        FunctionPointer(RuntimeMethodSignature),
        TypeParameter {
            owner: TypeDescription,
            index: u16,
        },
        MethodParameter {
            owner: MethodDescription,
            index: u16,
        },
    },
    resolution: |loader| {
        Void | TypedReference => loader
            .corlib_type("System.Object")
            .expect("System.Object must exist")
            .resolution,
        Type(td) => td.resolution,
        Generic(td, _) => td.resolution,
        TypeParameter { owner, .. } => owner.resolution,
        MethodParameter { owner, .. } => owner.resolution(),

        Vector(_)
        | Array(_, _)
        | Pointer(_)
        | ByRef(_)
        | ValuePointer(_, _)
        | FunctionPointer(_) => loader
            .corlib_type("System.Object")
            .expect("System.Object must exist")
            .resolution,
    },
    get_name: {
        Void => "Void".to_string(),
        TypedReference => "TypedReference".to_string(),
        Type(td) | Generic(td, _) => td.definition().name.to_string(),
        Vector(t) => format!("{}[]", t.get_name()),
        Array(t, rank) => {
            let commas = if *rank > 1 {
                ",".repeat(*rank as usize - 1)
            } else {
                "".to_string()
            };
            format!("{}[{}]", t.get_name(), commas)
        },
        Pointer(t) => format!("{}*", t.get_name()),
        ByRef(t) => format!("{}&", t.get_name()),
        ValuePointer(t, _) => format!("{}*", t.get_name()),
        TypeParameter { owner, index } => owner
            .definition()
            .generic_parameters
            .get(*index as usize)
            .map(|p| p.name.to_string())
            .unwrap_or_else(|| format!("!{}", index)),
        MethodParameter { index, .. } => format!("!!{}", index),
        FunctionPointer(_) => "method*".to_string(),
    },
    to_concrete: |loader, corlib_res| {
        Void => ConcreteType::from(
            loader
                .corlib_type("System.Void")
                .expect("System.Void must exist"),
        ),
        TypedReference => ConcreteType::from(
            loader
                .corlib_type("System.TypedReference")
                .expect("System.TypedReference must exist"),
        ),
        Type(td) => ConcreteType::from(*td),
        Generic(td, args) => {
            let source = TypeSource::Generic {
                base: UserType::Definition(td.index),
                parameters: args.iter().map(|a| a.to_concrete(loader)).collect(),
            };
            ConcreteType::new(
                td.resolution,
                BaseType::Type {
                    source,
                    value_kind: None,
                },
            )
        },
        Vector(t) => {
            ConcreteType::new(corlib_res, BaseType::Vector(vec![], t.to_concrete(loader)))
        },
        Array(t, rank) => ConcreteType::new(
            corlib_res,
            BaseType::Array(
                t.to_concrete(loader),
                dotnetdll::binary::signature::encoded::ArrayShape {
                    rank: *rank as usize,
                    sizes: vec![],
                    lower_bounds: vec![],
                },
            ),
        ),
        Pointer(t) | ValuePointer(t, _) => {
            ConcreteType::new(
                corlib_res,
                BaseType::ValuePointer(vec![], Some(t.to_concrete(loader))),
            )
        },
        ByRef(t) => {
            let by_ref_type = loader
                .corlib_type("System.ByReference`1")
                .expect("System.ByReference`1 not found");
            ConcreteType::new(
                by_ref_type.resolution,
                BaseType::Type {
                    source: TypeSource::Generic {
                        base: UserType::Definition(by_ref_type.index),
                        parameters: vec![t.to_concrete(loader)],
                    },
                    value_kind: Some(ValueKind::ValueType),
                },
            )
        },
        TypeParameter { .. } | MethodParameter { .. } => {
            ConcreteType::new(corlib_res, BaseType::Object)
        },
        rest => todo!("convert {rest:?} to ConcreteType"),
    }
}

unsafe_empty_collect!(RuntimeType);
