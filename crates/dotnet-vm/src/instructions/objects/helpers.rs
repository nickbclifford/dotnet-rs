use dotnet_types::{comparer::TypeComparer, generics::ConcreteType};
use dotnet_value::object::{HeapStorage, Object};
use dotnetdll::prelude::*;

pub(super) fn vector_matches_generic_array_interfaces(
    loader: &impl dotnet_types::TypeResolver,
    element: &ConcreteType,
    target: &ConcreteType,
) -> bool {
    let comparer = TypeComparer::new(loader);

    [
        "System.Collections.Generic.IEnumerable`1",
        "System.Collections.Generic.ICollection`1",
        "System.Collections.Generic.IList`1",
        "System.Collections.Generic.IReadOnlyCollection`1",
        "System.Collections.Generic.IReadOnlyList`1",
    ]
    .iter()
    .any(|interface_name| {
        let Ok(interface_td) = loader.corlib_type(interface_name) else {
            return false;
        };

        let interface_ct = ConcreteType::new(
            interface_td.resolution.clone(),
            BaseType::Type {
                source: TypeSource::Generic {
                    base: UserType::Definition(interface_td.index),
                    parameters: vec![element.clone()],
                },
                value_kind: None,
            },
        );

        comparer.is_assignable_to(&interface_ct, target)
    })
}

pub(super) fn object_runtime_concrete_type(object: &Object<'_>) -> ConcreteType {
    let type_arity = object.description.definition().generic_parameters.len();
    if type_arity == 0 {
        return object.description.clone().into();
    }

    let type_args: Vec<_> = object
        .generics
        .type_generics
        .iter()
        .take(type_arity)
        .cloned()
        .collect();

    if type_args.len() != type_arity {
        return object.description.clone().into();
    }

    ConcreteType::new(
        object.description.resolution.clone(),
        BaseType::Type {
            source: TypeSource::Generic {
                base: UserType::Definition(object.description.index),
                parameters: type_args,
            },
            value_kind: None,
        },
    )
}

pub(super) fn heap_runtime_concrete_type(storage: &HeapStorage<'_>) -> Option<ConcreteType> {
    match storage {
        HeapStorage::Obj(o) => Some(object_runtime_concrete_type(o)),
        HeapStorage::Boxed(o) => Some(object_runtime_concrete_type(o)),
        _ => None,
    }
}
