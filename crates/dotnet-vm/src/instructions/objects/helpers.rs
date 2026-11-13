use crate::{
    StepResult,
    stack::ops::{LoaderOps, ReflectionOps, ResolutionOps},
};
use dotnet_types::generics::ConcreteType;
use dotnet_value::object::{HeapStorage, Object, ObjectRef};
use dotnet_vm_ops::array_assignability::vector_matches_array_type;
use dotnetdll::prelude::*;

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
