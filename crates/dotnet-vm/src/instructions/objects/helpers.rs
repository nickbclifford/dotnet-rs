use crate::{
    StepResult,
    stack::ops::{LoaderOps, ReflectionOps, ResolutionOps},
};
use dotnet_types::{WellKnown, comparer::TypeComparer, generics::ConcreteType};
use dotnet_value::object::{HeapStorage, Object, ObjectRef};
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

fn vector_matches_array_type(
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

pub(super) fn check_object_assignability<
    'gc,
    T: ResolutionOps<'gc> + ReflectionOps<'gc> + LoaderOps,
>(
    ctx: &mut T,
    target_obj: ObjectRef<'gc>,
    param0: &MethodType,
) -> Result<bool, StepResult> {
    let target_ct = ctx
        .make_concrete(param0)
        .map_err(|e| StepResult::Error(e.into()))?;
    let array_type_match = target_obj
        .try_as_heap_storage(|storage| match storage {
            HeapStorage::Vec(v) => {
                vector_matches_array_type(ctx.loader().as_ref(), &v.element, &target_ct)
            }
            _ => false,
        })
        .map_err(|e| StepResult::Error(e.into()))?;
    if array_type_match {
        return Ok(true);
    }
    let source_ct = if let Some(source_ct) = target_obj
        .try_as_heap_storage(heap_runtime_concrete_type)
        .map_err(|e| StepResult::Error(e.into()))?
    {
        source_ct
    } else {
        ctx.get_heap_description(target_obj)
            .map_err(|e| StepResult::Error(e.into()))?
            .into()
    };
    ctx.is_a(source_ct, target_ct)
        .map_err(|e| StepResult::Error(e.into()))
}

pub(super) fn vector_matches_generic_array_interfaces(
    loader: &impl dotnet_types::TypeResolver,
    element: &ConcreteType,
    target: &ConcreteType,
) -> bool {
    let comparer = TypeComparer::new(loader);

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
