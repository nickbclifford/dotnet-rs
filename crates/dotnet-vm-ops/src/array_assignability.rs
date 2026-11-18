//! Array assignability shared by casts and reflection.
use dotnet_types::{
    WellKnown, comparer::TypeComparer, error::TypeResolutionError, generics::ConcreteType,
};
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

fn reduced_type(
    loader: &impl dotnet_types::TypeResolver,
    ty: &ConcreteType,
) -> Result<Option<u8>, TypeResolutionError> {
    if let Some(reduced) = integer_reduced_type(ty.get()) {
        return Ok(Some(reduced));
    }
    if !matches!(ty.get(), BaseType::Type { .. }) {
        return Ok(None);
    }
    let td = loader.find_concrete_type(ty.clone())?;
    let Some(MemberType::Base(base)) = td.is_enum() else {
        return Ok(None);
    };
    Ok(integer_reduced_type(base))
}

pub fn vector_matches_array_type(
    loader: &impl dotnet_types::TypeResolver,
    element: &ConcreteType,
    target: &ConcreteType,
) -> Result<bool, TypeResolutionError> {
    let comparer = TypeComparer::new(loader);
    match target.get() {
        BaseType::Vector(_, target_element) => {
            if comparer.concrete_types_equal(element, target_element)? {
                return Ok(true);
            }
            let source_reduced = reduced_type(loader, element)?;
            let target_reduced = reduced_type(loader, target_element)?;
            if source_reduced.is_some() && source_reduced == target_reduced {
                return Ok(true);
            }
            if !element.try_is_value_type(loader)?
                && !target_element.try_is_value_type(loader)?
                && comparer.is_assignable_to(element, target_element)?
            {
                return Ok(true);
            }
            Ok(false)
        }
        _ => vector_matches_generic_array_interfaces(loader, element, target),
    }
}

pub fn vector_matches_generic_array_interfaces(
    loader: &impl dotnet_types::TypeResolver,
    element: &ConcreteType,
    target: &ConcreteType,
) -> Result<bool, TypeResolutionError> {
    let comparer = TypeComparer::new(loader);
    let element_is_value_type = element.try_is_value_type(loader)?;
    let target_element_reduced = match target.get() {
        BaseType::Type {
            source: TypeSource::Generic { parameters, .. },
            ..
        } if parameters.len() == 1 => Some(reduced_type(loader, &parameters[0])?),
        _ => None,
    };
    let element_reduced = if target_element_reduced.is_some() {
        Some(reduced_type(loader, element)?)
    } else {
        None
    };

    // Generic variance only applies to reference types. Value-type arrays can expose an
    // interface with the same element type (or its reduced integer type), but cannot use
    // boxing to become e.g. IEnumerable<object>.
    if element_is_value_type
        && let BaseType::Type {
            source: TypeSource::Generic { parameters, .. },
            ..
        } = target.get()
        && parameters.len() == 1
        && !comparer.concrete_types_equal(element, &parameters[0])?
        && element_reduced
            .flatten()
            .is_none_or(|r| Some(r) != target_element_reduced.flatten())
    {
        return Ok(false);
    }

    // The CLR also exposes these interfaces using an array element's reduced type.
    let compatible_element = match target.get() {
        BaseType::Type {
            source: TypeSource::Generic { parameters, .. },
            ..
        } if parameters.len() == 1
            && element_reduced.flatten().is_some()
            && element_reduced.flatten() == target_element_reduced.flatten() =>
        {
            &parameters[0]
        }
        _ => element,
    };

    for interface in [
        WellKnown::IEnumerable1,
        WellKnown::ICollection1,
        WellKnown::IList1,
        WellKnown::IReadOnlyCollection1,
        WellKnown::IReadOnlyList1,
    ] {
        let interface_td = loader.corlib_wkt(interface)?;
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

        if comparer.is_assignable_to(&interface_ct, target)? {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotnet_types::{TypeDescription, error::TypeResolutionError, resolution::ResolutionS};

    struct FailingResolver;

    impl dotnet_types::TypeResolver for FailingResolver {
        fn corlib_type(&self, name: &str) -> Result<TypeDescription, TypeResolutionError> {
            Err(TypeResolutionError::TypeNotFound(name.into()))
        }

        fn locate_type(
            &self,
            _resolution: ResolutionS,
            _handle: UserType,
        ) -> Result<TypeDescription, TypeResolutionError> {
            Err(TypeResolutionError::TypeNotFound(
                "missing user type".into(),
            ))
        }

        fn find_concrete_type(
            &self,
            _ty: ConcreteType,
        ) -> Result<TypeDescription, TypeResolutionError> {
            Err(TypeResolutionError::TypeNotFound(
                "missing concrete type".into(),
            ))
        }
    }

    fn concrete_type(base: BaseType<ConcreteType>) -> ConcreteType {
        ConcreteType::new(ResolutionS::NULL, base)
    }

    #[test]
    fn vector_matching_keeps_successful_equal_and_unequal_results() {
        let resolver = FailingResolver;
        let int32 = concrete_type(BaseType::Int32);
        let int64 = concrete_type(BaseType::Int64);
        let int32_vector = concrete_type(BaseType::vector(int32.clone()));
        let int64_vector = concrete_type(BaseType::vector(int64));

        assert_eq!(
            vector_matches_array_type(&resolver, &int32, &int32_vector),
            Ok(true)
        );
        assert_eq!(
            vector_matches_array_type(&resolver, &int32, &int64_vector),
            Ok(false)
        );
    }

    #[test]
    fn generic_array_matching_propagates_missing_core_library_metadata() {
        let resolver = FailingResolver;
        let element = concrete_type(BaseType::Int32);
        let target = concrete_type(BaseType::Object);

        assert_eq!(
            vector_matches_generic_array_interfaces(&resolver, &element, &target),
            Err(TypeResolutionError::TypeNotFound(
                WellKnown::IEnumerable1.name().into()
            ))
        );
    }
}
