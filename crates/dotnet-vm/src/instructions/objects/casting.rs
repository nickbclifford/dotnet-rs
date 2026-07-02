use crate::{
    StepResult,
    stack::ops::{EvalStackOps, ExceptionOps, LoaderOps, ReflectionOps, ResolutionOps},
};

const INVALID_PROGRAM_MSG: &str = "Common Language Runtime detected an invalid program.";
const INVALID_CAST_MSG: &str = "Specified cast is not valid.";
use dotnet_macros::dotnet_instruction;
use dotnet_types::{comparer::TypeComparer, generics::ConcreteType};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, Object, ObjectRef},
};
use dotnetdll::prelude::*;

fn vector_matches_array_type(
    loader: &impl dotnet_types::TypeResolver,
    source_element: &ConcreteType,
    target: &ConcreteType,
) -> bool {
    let comparer = TypeComparer::new(loader);

    match target.get() {
        BaseType::Vector(_, target_element) => {
            comparer.concrete_types_equal(source_element, target_element)
                || (!source_element.is_value_type(loader)
                    && !target_element.is_value_type(loader)
                    && comparer.is_assignable_to(source_element, target_element))
        }
        _ => vector_matches_generic_array_interfaces(loader, source_element, target),
    }
}

fn vector_matches_generic_array_interfaces(
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

fn object_runtime_concrete_type(object: &Object<'_>) -> ConcreteType {
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

fn heap_runtime_concrete_type(storage: &HeapStorage<'_>) -> Option<ConcreteType> {
    match storage {
        HeapStorage::Obj(o) => Some(object_runtime_concrete_type(o)),
        HeapStorage::Boxed(o) => Some(object_runtime_concrete_type(o)),
        _ => None,
    }
}

#[dotnet_instruction(CastClass { param0 })]
pub fn castclass<
    'gc,
    T: ResolutionOps<'gc> + ReflectionOps<'gc> + ExceptionOps<'gc> + EvalStackOps<'gc> + LoaderOps,
>(
    ctx: &mut T,
    param0: &MethodType,
) -> StepResult {
    let target_obj_val = vm_pop!(ctx);
    let StackValue::ObjectRef(target_obj) = target_obj_val else {
        return ctx
            .throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
    };

    if let ObjectRef(Some(_)) = target_obj {
        let target_ct = dotnet_vm_ops::vm_try!(ctx.make_concrete(param0));
        let array_type_match =
            dotnet_vm_ops::vm_try!(target_obj.try_as_heap_storage(|storage| match storage {
                HeapStorage::Vec(v) => {
                    vector_matches_array_type(ctx.loader().as_ref(), &v.element, &target_ct)
                }
                _ => false,
            },));

        if array_type_match {
            ctx.push(StackValue::ObjectRef(target_obj));
        } else {
            let source_ct = if let Some(source_ct) = dotnet_vm_ops::vm_try!(
                target_obj.try_as_heap_storage(|storage| heap_runtime_concrete_type(storage),)
            ) {
                source_ct
            } else {
                dotnet_vm_ops::vm_try!(ctx.get_heap_description(target_obj)).into()
            };

            if dotnet_vm_ops::vm_try!(ctx.is_a(source_ct, target_ct)) {
                ctx.push(StackValue::ObjectRef(target_obj));
            } else {
                return ctx
                    .throw_by_name_with_message("System.InvalidCastException", INVALID_CAST_MSG);
            }
        }
    } else {
        // castclass returns null for null (III.4.3)
        ctx.push(StackValue::ObjectRef(ObjectRef(None)));
    }
    StepResult::Continue
}

#[dotnet_instruction(IsInstance(param0))]
pub fn isinst<
    'gc,
    T: ResolutionOps<'gc> + ReflectionOps<'gc> + ExceptionOps<'gc> + EvalStackOps<'gc> + LoaderOps,
>(
    ctx: &mut T,
    param0: &MethodType,
) -> StepResult {
    let target_obj_val = vm_pop!(ctx);
    let StackValue::ObjectRef(target_obj) = target_obj_val else {
        return ctx
            .throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
    };

    if let ObjectRef(Some(_)) = target_obj {
        let target_ct = dotnet_vm_ops::vm_try!(ctx.make_concrete(param0));
        let array_type_match =
            dotnet_vm_ops::vm_try!(target_obj.try_as_heap_storage(|storage| match storage {
                HeapStorage::Vec(v) => {
                    vector_matches_array_type(ctx.loader().as_ref(), &v.element, &target_ct)
                }
                _ => false,
            },));

        if array_type_match {
            ctx.push(StackValue::ObjectRef(target_obj));
        } else {
            let source_ct = if let Some(source_ct) = dotnet_vm_ops::vm_try!(
                target_obj.try_as_heap_storage(|storage| heap_runtime_concrete_type(storage),)
            ) {
                source_ct
            } else {
                dotnet_vm_ops::vm_try!(ctx.get_heap_description(target_obj)).into()
            };

            if dotnet_vm_ops::vm_try!(ctx.is_a(source_ct, target_ct)) {
                ctx.push(StackValue::ObjectRef(target_obj));
            } else {
                ctx.push(StackValue::ObjectRef(ObjectRef(None)));
            }
        }
    } else {
        // isinst returns null for null inputs (III.4.6)
        ctx.push(StackValue::ObjectRef(ObjectRef(None)));
    }
    StepResult::Continue
}
