use crate::{
    assemblies::AssemblyLoader,
    types::{generics::ConcreteType, members::MethodDescription, TypeDescription},
    utils::ResolutionS,
};
use dotnetdll::prelude::{BaseType, TypeSource};
use std::{fmt::Debug, hash::Hash};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RuntimeMethodSignature; // TODO

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum RuntimeType {
    Void,
    Boolean,
    Char,
    Int8,
    UInt8,
    Int16,
    UInt16,
    Int32,
    UInt32,
    Int64,
    UInt64,
    Float32,
    Float64,
    IntPtr,
    UIntPtr,
    Object,
    String,
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
}

impl RuntimeType {
    pub fn resolution(&self, loader: &AssemblyLoader) -> ResolutionS {
        match self {
            RuntimeType::Void
            | RuntimeType::Boolean
            | RuntimeType::Char
            | RuntimeType::Int8
            | RuntimeType::UInt8
            | RuntimeType::Int16
            | RuntimeType::UInt16
            | RuntimeType::Int32
            | RuntimeType::UInt32
            | RuntimeType::Int64
            | RuntimeType::UInt64
            | RuntimeType::Float32
            | RuntimeType::Float64
            | RuntimeType::IntPtr
            | RuntimeType::UIntPtr
            | RuntimeType::Object
            | RuntimeType::String
            | RuntimeType::Vector(_)
            | RuntimeType::Array(_, _)
            | RuntimeType::Pointer(_)
            | RuntimeType::ByRef(_)
            | RuntimeType::ValuePointer(_, _)
            | RuntimeType::FunctionPointer(_) => loader.corlib_type("System.Object").resolution,
            RuntimeType::Type(td) => td.resolution,
            RuntimeType::Generic(td, _) => td.resolution,
            RuntimeType::TypeParameter { owner, .. } => owner.resolution,
            RuntimeType::MethodParameter { owner, .. } => owner.resolution(),
        }
    }

    pub fn get_name(&self) -> String {
        match self {
            RuntimeType::Void => "Void".to_string(),
            RuntimeType::Boolean => "Boolean".to_string(),
            RuntimeType::Char => "Char".to_string(),
            RuntimeType::Int8 => "SByte".to_string(),
            RuntimeType::UInt8 => "Byte".to_string(),
            RuntimeType::Int16 => "Int16".to_string(),
            RuntimeType::UInt16 => "UInt16".to_string(),
            RuntimeType::Int32 => "Int32".to_string(),
            RuntimeType::UInt32 => "UInt32".to_string(),
            RuntimeType::Int64 => "Int64".to_string(),
            RuntimeType::UInt64 => "UInt64".to_string(),
            RuntimeType::Float32 => "Single".to_string(),
            RuntimeType::Float64 => "Double".to_string(),
            RuntimeType::IntPtr => "IntPtr".to_string(),
            RuntimeType::UIntPtr => "UIntPtr".to_string(),
            RuntimeType::Object => "Object".to_string(),
            RuntimeType::String => "String".to_string(),
            RuntimeType::Type(td) | RuntimeType::Generic(td, _) => td.definition().name.to_string(),
            RuntimeType::Vector(t) => format!("{}[]", t.get_name()),
            RuntimeType::Array(t, rank) => {
                let commas = if *rank > 1 {
                    ",".repeat(*rank as usize - 1)
                } else {
                    "".to_string()
                };
                format!("{}[{}]", t.get_name(), commas)
            }
            RuntimeType::Pointer(t) => format!("{}*", t.get_name()),
            RuntimeType::ByRef(t) => format!("{}&", t.get_name()),
            RuntimeType::ValuePointer(t, _) => format!("{}*", t.get_name()),
            RuntimeType::TypeParameter { owner, index } => owner
                .definition()
                .generic_parameters
                .get(*index as usize)
                .map(|p| p.name.to_string())
                .unwrap_or_else(|| format!("!{}", index)),
            RuntimeType::MethodParameter { index, .. } => format!("!!{}", index),
            RuntimeType::FunctionPointer(_) => "method*".to_string(),
        }
    }

    pub fn to_concrete(&self, loader: &AssemblyLoader) -> ConcreteType {
        let corlib_res = loader.corlib_type("System.Object").resolution;
        match self {
            RuntimeType::Void => ConcreteType::from(loader.corlib_type("System.Void")),
            RuntimeType::Boolean => ConcreteType::new(corlib_res, BaseType::Boolean),
            RuntimeType::Char => ConcreteType::new(corlib_res, BaseType::Char),
            RuntimeType::Int8 => ConcreteType::new(corlib_res, BaseType::Int8),
            RuntimeType::UInt8 => ConcreteType::new(corlib_res, BaseType::UInt8),
            RuntimeType::Int16 => ConcreteType::new(corlib_res, BaseType::Int16),
            RuntimeType::UInt16 => ConcreteType::new(corlib_res, BaseType::UInt16),
            RuntimeType::Int32 => ConcreteType::new(corlib_res, BaseType::Int32),
            RuntimeType::UInt32 => ConcreteType::new(corlib_res, BaseType::UInt32),
            RuntimeType::Int64 => ConcreteType::new(corlib_res, BaseType::Int64),
            RuntimeType::UInt64 => ConcreteType::new(corlib_res, BaseType::UInt64),
            RuntimeType::Float32 => ConcreteType::new(corlib_res, BaseType::Float32),
            RuntimeType::Float64 => ConcreteType::new(corlib_res, BaseType::Float64),
            RuntimeType::IntPtr => ConcreteType::new(corlib_res, BaseType::IntPtr),
            RuntimeType::UIntPtr => ConcreteType::new(corlib_res, BaseType::UIntPtr),
            RuntimeType::Object => ConcreteType::new(corlib_res, BaseType::Object),
            RuntimeType::String => ConcreteType::new(corlib_res, BaseType::String),
            RuntimeType::Type(td) => ConcreteType::from(*td),
            RuntimeType::Generic(td, args) => {
                let index = td
                    .resolution
                    .definition()
                    .type_definitions
                    .iter()
                    .position(|t| std::ptr::eq(t, td.definition()))
                    .unwrap();
                let source = TypeSource::Generic {
                    base: dotnetdll::prelude::UserType::Definition(
                        td.resolution
                            .definition()
                            .type_definition_index(index)
                            .expect("invalid type definition"),
                    ),
                    parameters: args.iter().map(|a| a.to_concrete(loader)).collect(),
                };
                ConcreteType::new(
                    td.resolution,
                    BaseType::Type {
                        source,
                        value_kind: None,
                    },
                )
            }
            RuntimeType::Vector(t) => {
                ConcreteType::new(corlib_res, BaseType::Vector(vec![], t.to_concrete(loader)))
            }
            RuntimeType::Array(t, rank) => ConcreteType::new(
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
            RuntimeType::Pointer(t) | RuntimeType::ByRef(t) | RuntimeType::ValuePointer(t, _) => {
                ConcreteType::new(
                    corlib_res,
                    BaseType::ValuePointer(vec![], Some(t.to_concrete(loader))),
                )
            }
            RuntimeType::TypeParameter { .. } | RuntimeType::MethodParameter { .. } => {
                ConcreteType::new(corlib_res, BaseType::Object)
            }
            rest => todo!("convert {rest:?} to ConcreteType"),
        }
    }
}
