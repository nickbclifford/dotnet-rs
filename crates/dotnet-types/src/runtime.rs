use crate::{
    TypeDescription, TypeResolver, generics::ConcreteType, members::MethodDescription,
    resolution::ResolutionS,
};
use dotnetdll::prelude::{BaseType, TypeSource, UserType, ValueKind, CallingConvention};
use dotnetdll::binary::signature::kinds::StandAloneCallingConvention;
use dotnetdll::resolved::signature::{MethodSignature, Parameter, ReturnType, ParameterType};
use gc_arena::static_collect;
use std::{fmt::Debug, hash::Hash};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RuntimeMethodSignature {
    pub instance: bool,
    pub explicit_this: bool,
    pub calling_convention: CallingConvention,
    pub return_type: Box<RuntimeType>,
    pub parameters: Vec<RuntimeType>,
}

impl Hash for RuntimeMethodSignature {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.instance.hash(state);
        self.explicit_this.hash(state);
        match self.calling_convention {
            CallingConvention::Default => 0u8.hash(state),
            CallingConvention::Vararg => 1u8.hash(state),
            CallingConvention::Generic(i) => {
                2u8.hash(state);
                i.hash(state);
            }
        }
        self.return_type.hash(state);
        self.parameters.hash(state);
    }
}

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
        FunctionPointer(sig) => {
            let mut name = "method* ".to_string();
            name.push_str(&sig.return_type.get_name());
            name.push('(');
            for (i, p) in sig.parameters.iter().enumerate() {
                if i > 0 {
                    name.push_str(", ");
                }
                name.push_str(&p.get_name());
            }
            name.push(')');
            name
        },
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
        FunctionPointer(sig) => {
            let parameters = sig.parameters.iter().map(|p| {
                Parameter(vec![], ParameterType::Value(p.to_concrete(loader)))
            }).collect();
            let return_type = match &*sig.return_type {
                RuntimeType::Void => ReturnType(vec![], None),
                t => ReturnType(vec![], Some(ParameterType::Value(t.to_concrete(loader)))),
            };
            let calling_convention = match sig.calling_convention {
                CallingConvention::Default => StandAloneCallingConvention::DefaultManaged,
                CallingConvention::Vararg => StandAloneCallingConvention::Vararg,
                _ => StandAloneCallingConvention::DefaultManaged, // Default fallback
            };
            let managed_method = MethodSignature {
                instance: sig.instance,
                explicit_this: sig.explicit_this,
                calling_convention,
                parameters,
                return_type,
                varargs: None,
            };
            ConcreteType::new(corlib_res, BaseType::FunctionPointer(managed_method))
        },
    }
}

static_collect!(RuntimeType);
