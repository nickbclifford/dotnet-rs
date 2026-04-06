use crate::ReflectionIntrinsicHost;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
    runtime::RuntimeType,
};
use dotnet_value::{StackValue, object::ObjectRef};
use dotnet_vm_ops::{StepResult, ops::TypedStackOps};
use dotnetdll::prelude::{MethodMemberIndex, TypeSource};

#[dotnet_intrinsic(
    "static void System.Runtime.CompilerServices.RuntimeHelpers::RunClassConstructor(System.RuntimeTypeHandle)"
)]
pub fn intrinsic_runtime_helpers_run_class_constructor<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let arg = ctx.peek_stack();
    let StackValue::ValueType(handle) = arg else {
        panic!(
            "RunClassConstructor expects a RuntimeTypeHandle, received {:?}",
            arg
        )
    };

    let target_obj = handle
        .instance_storage
        .field::<ObjectRef<'gc>>(handle.description, "_value")
        .unwrap()
        .read();
    let target_type = crate::common::resolve_runtime_type(ctx, target_obj);
    let target_ct = target_type.to_concrete(ctx.loader().as_ref());
    let target_desc = ctx
        .loader()
        .find_concrete_type(target_ct)
        .expect("Type must exist for RunClassConstructor");

    let res = ctx.initialize_static_storage(target_desc, generics.clone());
    if res != StepResult::Continue {
        return res;
    }

    let _ = ctx.pop();
    StepResult::Continue
}

#[dotnet_intrinsic("static object System.Activator::CreateInstance()")]
pub fn intrinsic_activator_create_instance<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target_ct = generics.method_generics[0].clone();
    let target_td = ctx
        .loader()
        .find_concrete_type(target_ct.clone())
        .expect("Type must exist for Activator.CreateInstance");

    let empty_lookup = ctx.reflection_empty_generics();
    if vm_try!(ctx.reflection_is_value_type_with_lookup(target_td.clone(), &empty_lookup)) {
        let instance = vm_try!(ctx.new_object(target_td));
        ctx.push_value_type(instance);
        StepResult::Continue
    } else {
        let instance = vm_try!(ctx.new_object(target_td.clone()));
        let mut new_lookup = GenericLookup::default();
        if let dotnetdll::prelude::BaseType::Type {
            source: TypeSource::Generic { parameters, .. },
            ..
        } = target_ct.get()
        {
            new_lookup.type_generics = parameters.clone().into();
        }

        for (idx, m) in target_td.definition().methods.iter().enumerate() {
            if m.name == ".ctor" && m.signature.instance && m.signature.parameters.is_empty() {
                let desc = MethodDescription::new(
                    target_td.clone(),
                    new_lookup.clone(),
                    target_td.resolution.clone(),
                    MethodMemberIndex::Method(idx),
                );

                let info = vm_try!(ctx.reflection_method_info(desc, &new_lookup));
                vm_try!(ctx.reflection_constructor_frame(instance, info, new_lookup));
                return StepResult::FramePushed;
            }
        }

        panic!(
            "could not find a parameterless constructor in {:?}",
            target_td
        )
    }
}

pub fn handle_create_instance_check_this<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let _obj = ctx.pop_obj();
    StepResult::Continue
}

pub fn handle_make_generic_type<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let parameters = ctx.pop_obj();
    let target = ctx.pop_obj();

    if ctx.check_gc_safe_point() {
        return StepResult::Yield;
    }

    let target_rt = crate::common::resolve_runtime_type(ctx, target);

    if let RuntimeType::Type(td) | RuntimeType::Generic(td, _) = target_rt {
        let param_objs = parameters.as_vector(|v: &dotnet_value::object::Vector<'gc>| {
            v.get()
                .chunks_exact(ObjectRef::SIZE)
                .map(|chunk| unsafe { ObjectRef::read_branded(chunk, &gc) })
                .collect::<Vec<_>>()
        });
        let new_generics: Vec<_> = param_objs
            .into_iter()
            .map(|p_obj| crate::common::resolve_runtime_type(ctx, p_obj).clone())
            .collect();

        #[cfg(feature = "generic-constraint-validation")]
        {
            let loader = ctx.loader().clone();
            let new_generics_concrete: Vec<ConcreteType> = new_generics
                .iter()
                .map(|rt| rt.to_concrete(loader.as_ref()))
                .collect();
            let lookup = GenericLookup::new(new_generics_concrete);
            if let Err(_e) = lookup.validate_constraints(
                td.resolution.clone(),
                loader.as_ref(),
                &td.definition().generic_parameters,
                false,
            ) {
                return ctx.throw_by_name_with_message(
                    "System.TypeLoadException",
                    "Generic constraint violation.",
                );
            }
        }

        let new_rt = RuntimeType::Generic(td, new_generics);

        let rt_obj = crate::common::get_runtime_type(ctx, new_rt);
        ctx.push_obj(rt_obj);
        StepResult::Continue
    } else {
        ctx.throw_by_name_with_message(
            "System.InvalidOperationException",
            "MakeGenericType may only be called on a type for which IsGenericTypeDefinition is true.",
        )
    }
}

pub fn handle_create_instance_default_ctor<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _generics: &GenericLookup,
) -> StepResult {
    let _ = ctx.pop();
    let _ = ctx.pop();
    let target_obj = ctx.pop_obj();

    if ctx.check_gc_safe_point() {
        return StepResult::Yield;
    }

    let target_rt = crate::common::resolve_runtime_type(ctx, target_obj);

    let (td, type_generics) = match target_rt {
        RuntimeType::Type(td) => (td, vec![]),
        RuntimeType::Generic(td, args) => (td, args.clone()),
        _ => panic!("cannot create instance of {:?}", target_rt),
    };

    let type_generics_concrete: Vec<ConcreteType> = type_generics
        .iter()
        .map(|a| a.to_concrete(ctx.loader().as_ref()))
        .collect();
    let new_lookup = GenericLookup::new(type_generics_concrete);

    let instance = vm_try!(ctx.reflection_new_object_with_lookup(td.clone(), &new_lookup));

    for (idx, m) in td.definition().methods.iter().enumerate() {
        if m.runtime_special_name
            && m.name == ".ctor"
            && m.signature.instance
            && m.signature.parameters.is_empty()
        {
            let desc = MethodDescription::new(
                td.clone(),
                new_lookup.clone(),
                td.resolution.clone(),
                MethodMemberIndex::Method(idx),
            );

            let info = vm_try!(ctx.reflection_method_info(desc, &new_lookup));
            vm_try!(ctx.reflection_constructor_frame(instance, info, new_lookup));
            return StepResult::FramePushed;
        }
    }

    panic!("could not find a parameterless constructor in {:?}", td)
}
