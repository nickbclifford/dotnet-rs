use super::helpers::{heap_runtime_concrete_type, vector_matches_generic_array_interfaces};
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
    object::{HeapStorage, ObjectRef},
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

fn check_object_assignability<'gc, T: ResolutionOps<'gc> + ReflectionOps<'gc> + LoaderOps>(
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
        let assignable = match check_object_assignability(ctx, target_obj, param0) {
            Ok(assignable) => assignable,
            Err(result) => return result,
        };

        if assignable {
            ctx.push(StackValue::ObjectRef(target_obj));
        } else {
            return ctx.throw_by_name_with_message("System.InvalidCastException", INVALID_CAST_MSG);
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
        let assignable = match check_object_assignability(ctx, target_obj, param0) {
            Ok(assignable) => assignable,
            Err(result) => return result,
        };

        if assignable {
            ctx.push(StackValue::ObjectRef(target_obj));
        } else {
            ctx.push(StackValue::ObjectRef(ObjectRef(None)));
        }
    } else {
        // isinst returns null for null inputs (III.4.6)
        ctx.push(StackValue::ObjectRef(ObjectRef(None)));
    }
    StepResult::Continue
}
