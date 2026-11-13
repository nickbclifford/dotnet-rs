//! Array assignability shared by casts and reflection.
use dotnet_types::{WellKnown, comparer::TypeComparer, generics::ConcreteType};
use dotnetdll::prelude::*;

// ECMA-335 I.8.7: enum underlying types and signed/unsigned integer pairs
// have the same reduced type for array element compatibility.
fn integer_reduced_type<T>(ty: &BaseType<T>) -> Option<u8> {
    match ty {
        BaseType::Int8 | BaseType::UInt8 => Some(1),
        BaseType::Int16 | BaseType::UInt16 => Some(2),
        BaseType::Int32 | BaseType::UInt32 => Some(3),
        BaseType::Int64 | BaseType::UInt64 => Some(4),
        BaseType::IntPtr | BaseType::UIntPtr => Some(5),
        _ => None,
    }
}

fn reduced_type(loader: &impl dotnet_types::TypeResolver, ty: &ConcreteType) -> Option<u8> {
    if let Some(reduced) = integer_reduced_type(ty.get()) {
        return Some(reduced);
    }
    let td = loader.find_concrete_type(ty.clone()).ok()?;
    let MemberType::Base(base) = td.is_enum()? else {
        return None;
    };
    integer_reduced_type(base)
}

pub fn vector_matches_array_type(
    loader: &impl dotnet_types::TypeResolver,
    element: &ConcreteType,
    target: &ConcreteType,
) -> bool {
    let comparer = TypeComparer::new(loader);
    match target.get() {
        BaseType::Vector(_, target_element) => {
            comparer.concrete_types_equal(element, target_element)
                || reduced_type(loader, element)
                    .is_some_and(|r| Some(r) == reduced_type(loader, target_element))
                || (!element.is_value_type(loader)
                    && !target_element.is_value_type(loader)
                    && comparer.is_assignable_to(element, target_element))
        }
        _ => vector_matches_generic_array_interfaces(loader, element, target),
    }
}

pub fn vector_matches_generic_array_interfaces(
    loader: &impl dotnet_types::TypeResolver,
    element: &ConcreteType,
    target: &ConcreteType,
) -> bool {
    let comparer = TypeComparer::new(loader);

    // Generic variance only applies to reference types. Value-type arrays can expose an
    // interface with the same element type (or its reduced integer type), but cannot use
    // boxing to become e.g. IEnumerable<object>.
    if element.is_value_type(loader)
        && let BaseType::Type {
            source: TypeSource::Generic { parameters, .. },
            ..
        } = target.get()
        && parameters.len() == 1
        && !comparer.concrete_types_equal(element, &parameters[0])
        && !reduced_type(loader, element)
            .is_some_and(|r| Some(r) == reduced_type(loader, &parameters[0]))
    {
        return false;
    }

    // The CLR also exposes these interfaces using an array element's reduced type.
    let compatible_element = match target.get() {
        BaseType::Type {
            source: TypeSource::Generic { parameters, .. },
            ..
        } if parameters.len() == 1
            && reduced_type(loader, element)
                .is_some_and(|r| Some(r) == reduced_type(loader, &parameters[0])) =>
        {
            &parameters[0]
        }
        _ => element,
    };

    [
        WellKnown::IEnumerable1,
        WellKnown::ICollection1,
        WellKnown::IList1,
        WellKnown::IReadOnlyCollection1,
        WellKnown::IReadOnlyList1,
    ]
    .into_iter()
    .any(|interface| {
        let Ok(interface_td) = loader.corlib_wkt(interface) else {
            return false;
        };

        let interface_ct = ConcreteType::new(
            interface_td.resolution.clone(),
            BaseType::Type {
                source: TypeSource::Generic {
                    base: UserType::Definition(interface_td.index),
                    parameters: vec![compatible_element.clone()],
                },
                value_kind: None,
            },
        );

        comparer.is_assignable_to(&interface_ct, target)
    })
}
