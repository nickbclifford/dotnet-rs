#[cfg(test)]
mod tests {
    #[cfg(not(miri))]
    use crate::{
        StepResult,
        intrinsics::{IntrinsicKind, IntrinsicRegistry, dispatch_method_intrinsic},
        state::SharedGlobalState,
        sync::Arc,
        test_utils::{new_test_arena, test_gc_handle},
    };
    #[cfg(not(miri))]
    use dotnet_assemblies::AssemblyLoader;
    #[cfg(not(miri))]
    use dotnet_types::{generics::GenericLookup, members::MethodDescription};
    #[cfg(not(miri))]
    use dotnet_vm_ops::effects::NativeDispatchMode;
    #[cfg(not(miri))]
    use dotnet_vm_ops::ops::TypedStackOps;
    #[cfg(not(miri))]
    use dotnetdll::prelude::MethodMemberIndex;

    #[cfg(not(miri))]
    #[allow(clippy::arc_with_non_send_sync)] // Runtime lookup APIs require Arc; this fixture stays on the test thread.
    fn runtime_loader() -> Arc<AssemblyLoader> {
        let runtime = dotnet_assemblies::find_dotnet_app_path().unwrap();
        Arc::new(AssemblyLoader::new(runtime.to_string_lossy().into_owned()).unwrap())
    }

    #[cfg(not(miri))]
    fn method_descriptions(
        loader: &AssemblyLoader,
        type_name: &str,
        member_name: &str,
    ) -> Vec<MethodDescription> {
        let ty = loader.corlib_type(type_name).unwrap();
        ty.definition()
            .methods
            .iter()
            .enumerate()
            .filter(|(_, method)| method.name == member_name)
            .map(|(index, _)| {
                MethodDescription::new(
                    ty.clone(),
                    GenericLookup::default(),
                    ty.resolution.clone(),
                    MethodMemberIndex::Method(index),
                )
            })
            .collect()
    }

    #[cfg(not(miri))]
    fn has_primitive_parameter(method: &MethodDescription, primitive: &str) -> bool {
        method.signature().parameters.iter().any(|parameter| {
            matches!(
                (&parameter.1, primitive),
                (
                    dotnetdll::prelude::ParameterType::Value(
                        dotnetdll::prelude::MethodType::Base(base)
                    ),
                    "Int32"
                ) if matches!(base.as_ref(), dotnetdll::prelude::BaseType::Int32)
            ) || matches!(
                (&parameter.1, primitive),
                (
                    dotnetdll::prelude::ParameterType::Value(
                        dotnetdll::prelude::MethodType::Base(base)
                    ),
                    "Int64"
                ) if matches!(base.as_ref(), dotnetdll::prelude::BaseType::Int64)
            )
        })
    }

    #[cfg(not(miri))]
    fn is_int32_vector_parameter(method: &MethodDescription) -> bool {
        use dotnetdll::prelude::{BaseType, MethodType, ParameterType};

        method.signature().parameters.iter().any(|parameter| {
            matches!(
                &parameter.1,
                ParameterType::Value(MethodType::Base(base))
                    if matches!(base.as_ref(), BaseType::Vector(_, element)
                        if matches!(element, MethodType::Base(element)
                            if matches!(element.as_ref(), BaseType::Int32)))
            )
        })
    }

    #[cfg(not(miri))]
    fn has_float64_parameter(method: &MethodDescription) -> bool {
        use dotnetdll::prelude::{BaseType, MethodType, ParameterType};

        method.signature().parameters.iter().any(|parameter| {
            matches!(
                &parameter.1,
                ParameterType::Value(MethodType::Base(base))
                    if matches!(base.as_ref(), BaseType::Float64)
            )
        })
    }

    #[test]
    #[cfg(not(miri))]
    fn runtime_helper_registrations_resolve_to_handlers() {
        let loader = runtime_loader();
        let registry = IntrinsicRegistry::initialize();

        let (type_name, member_name) = (
            "System.Runtime.CompilerServices.RuntimeHelpers",
            "InitializeArray",
        );
        let methods = method_descriptions(&loader, type_name, member_name);
        let method = methods
            .iter()
            .find(|method| !method.signature().instance)
            .unwrap_or_else(|| panic!("missing static method {type_name}::{member_name}"));
        let metadata = registry
            .get_metadata(method, &loader)
            .unwrap_or_else(|| panic!("missing registry entry for {type_name}::{member_name}"));
        assert_ne!(
            metadata.handler,
            crate::intrinsics::MethodIntrinsicId::Missing,
            "{type_name}::{member_name} must have an executable handler"
        );
        assert_eq!(metadata.kind, IntrinsicKind::Static);
    }

    #[test]
    #[cfg(not(miri))]
    #[allow(clippy::arc_with_non_send_sync)] // The runtime registry fixture is single-threaded and the API requires Arc.
    fn math_sqrt_registration_dispatches_the_loaded_method() {
        let loader = runtime_loader();
        let registry = IntrinsicRegistry::initialize();
        let method = method_descriptions(&loader, "System.Math", "Sqrt")
            .into_iter()
            .find(|method| {
                method.signature().parameters.len() == 1 && has_float64_parameter(method)
            })
            .expect("double Math.Sqrt overload exists in the loaded CoreLib");
        let handler = registry
            .get(&method, &loader)
            .expect("loaded Math.Sqrt(double) method maps to a generated handler");
        let shared = Arc::new(SharedGlobalState::new(loader));
        let mut arena = new_test_arena(&shared);
        arena.mutate_root(|gc, engine| {
            // SAFETY: F1.GcHandleRooted — the engine is the live arena root throughout dispatch.
            let gc = unsafe { test_gc_handle(gc, engine) };
            let mut ctx = engine.ves_context(gc);
            ctx.push_f64(9.0);
            assert_eq!(
                dispatch_method_intrinsic(
                    handler,
                    &mut ctx,
                    method.clone(),
                    &GenericLookup::default(),
                    NativeDispatchMode::Other,
                ),
                StepResult::Continue
            );
            assert_eq!(ctx.pop_f64(), 3.0);
        });
    }

    #[test]
    #[cfg(not(miri))]
    fn array_get_value_registry_filters_managed_overload_shapes() {
        let loader = runtime_loader();
        let registry = IntrinsicRegistry::initialize();
        let methods = method_descriptions(&loader, "System.Array", "GetValue");
        let find = |arity: usize, predicate: &dyn Fn(&MethodDescription) -> bool| {
            methods
                .iter()
                .find(|method| {
                    method.signature().parameters.len() == arity
                        && method.signature().instance
                        && predicate(method)
                })
                .unwrap_or_else(|| {
                    panic!("missing System.Array::GetValue overload with {arity} parameters")
                })
        };
        let get_i32 = find(1, &|method| has_primitive_parameter(method, "Int32"));
        let get_i64 = find(1, &|method| has_primitive_parameter(method, "Int64"));
        let get_i32_array = find(1, &is_int32_vector_parameter);

        let i32_metadata = registry
            .get_metadata(get_i32, &loader)
            .expect("GetValue(int)");
        let i64_metadata = registry
            .get_metadata(get_i64, &loader)
            .expect("GetValue(long)");
        let array_metadata = registry
            .get_metadata(get_i32_array, &loader)
            .expect("GetValue(int[])");
        assert!(
            registry.get(get_i32, &loader).is_some(),
            "GetValue(int) dispatches"
        );
        assert!(
            registry.get(get_i64, &loader).is_some(),
            "GetValue(long) dispatches"
        );
        assert!(
            registry.get(get_i32_array, &loader).is_some(),
            "GetValue(int[]) dispatches"
        );
        assert_ne!(
            i32_metadata.handler,
            crate::intrinsics::MethodIntrinsicId::Missing
        );
        assert_ne!(
            i64_metadata.handler,
            crate::intrinsics::MethodIntrinsicId::Missing
        );
        assert_ne!(
            array_metadata.handler,
            crate::intrinsics::MethodIntrinsicId::Missing
        );

        let i32_filter = i32_metadata
            .signature_filter
            .expect("primitive overload metadata retains its filter");
        let i64_filter = i64_metadata
            .signature_filter
            .expect("primitive overload metadata retains its filter");
        assert!(i32_filter(get_i32));
        assert!(!i32_filter(get_i64));
        assert!(!i32_filter(get_i32_array));
        assert!(i64_filter(get_i64));
        assert!(!i64_filter(get_i32));
        assert!(!i64_filter(get_i32_array));
    }
}
