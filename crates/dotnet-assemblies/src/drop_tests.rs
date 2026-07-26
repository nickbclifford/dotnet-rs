use crate::AssemblyLoader;
use dotnet_types::{TypeResolver, WellKnown};
use dotnetdll::prelude::ResolvedDebug;
use std::sync::Arc;

#[test]
fn test_assembly_loader_drop_keeps_descriptors_valid() {
    // AssemblyLoader::new_bare doesn't need a real root for the support library parts.
    let loader =
        AssemblyLoader::new_bare("test_root".to_string()).expect("failed to create loader");
    let weak_arena = Arc::downgrade(&loader.metadata);

    // Get the support library descriptor (ResolutionS)
    let support_res = {
        let assemblies = loader.assemblies();
        assert!(
            !assemblies.is_empty(),
            "support library should be registered"
        );
        assemblies[0].clone()
    };

    // Original loader reference is dropped
    drop(loader);

    // Arena must still be alive because support_res holds an Arc reference to it.
    assert!(
        weak_arena.upgrade().is_some(),
        "MetadataArena should be kept alive by ResolutionS"
    );

    // Verify we can still access the metadata in support_res safely
    assert!(
        support_res.definition().assembly.is_some(),
        "Metadata should be valid"
    );
    assert_eq!(
        support_res.definition().assembly.as_ref().unwrap().name,
        "support",
        "Incorrect assembly name"
    );

    // Now drop the descriptor
    drop(support_res);

    // Now the arena should finally be dropped
    assert!(
        weak_arena.upgrade().is_none(),
        "MetadataArena should be dropped after last descriptor is gone"
    );
}

#[test]
fn test_type_description_keeps_arena_alive() {
    let loader =
        AssemblyLoader::new_bare("test_root".to_string()).expect("failed to create loader");
    let weak_arena = Arc::downgrade(&loader.metadata);

    // Find a type in the support library
    let type_desc = {
        let assemblies = loader.assemblies();
        let support_res = &assemblies[0];

        let some_type_index = support_res
            .definition()
            .type_definitions
            .iter()
            .enumerate()
            .next()
            .map(|(i, _)| i)
            .unwrap();
        // We need to get a TypeIndex. Resolution struct stores type definitions in a list.
        // ResolutionS provides the type index for a definition.
        let type_index = support_res
            .type_definition_index(some_type_index)
            .expect("failed to get type index");

        dotnet_types::TypeDescription::new(support_res.clone(), type_index)
    };

    drop(loader);
    assert!(
        weak_arena.upgrade().is_some(),
        "Arena should be kept alive by TypeDescription"
    );

    assert!(!type_desc.definition().type_name().is_empty());

    drop(type_desc);
    assert!(
        weak_arena.upgrade().is_none(),
        "Arena should be dropped after TypeDescription is gone"
    );
}

#[test]
#[cfg(not(miri))]
fn test_corlib_well_known_cache_and_dynamic_fallback() {
    let runtime_path = crate::find_dotnet_app_path().expect("could not find .NET shared path");
    let loader = AssemblyLoader::new(runtime_path.to_string_lossy().into_owned())
        .expect("failed to create loader");

    let first = TypeResolver::corlib_wkt(&loader, WellKnown::Object)
        .expect("System.Object should resolve through the TypeResolver override");
    assert!(
        loader.wkt_table[WellKnown::Object as usize].get().is_some(),
        "the TypeResolver override should populate the handle table"
    );
    assert!(
        !loader
            .dynamic_corlib_cache
            .contains_key(WellKnown::Object.name()),
        "the handle path should not populate the dynamic cache"
    );

    let repeated = loader
        .corlib_wkt(WellKnown::Object)
        .expect("cached System.Object should resolve");
    assert_eq!(first, repeated);

    let by_name = loader
        .corlib_type("System.Object")
        .expect("System.Object should resolve by name");
    assert_eq!(first, by_name);
    assert!(
        loader
            .dynamic_corlib_cache
            .contains_key(WellKnown::Object.name()),
        "the string path should use its separate dynamic cache"
    );

    let dynamic_name = "System.Threading.Tasks.Task";
    assert!(WellKnown::from_name(dynamic_name).is_none());
    let dynamic = loader
        .corlib_type(dynamic_name)
        .expect("dynamic non-WKT name should resolve");
    assert!(!dynamic.is_null());
    assert!(loader.dynamic_corlib_cache.contains_key(dynamic_name));
}

#[test]
fn test_corlib_wkt_resolution_failures_are_retryable() {
    let mut loader =
        AssemblyLoader::new_bare("test_root".to_string()).expect("failed to create loader");

    assert!(loader.corlib_wkt(WellKnown::Object).is_err());
    assert!(loader.wkt_table[WellKnown::Object as usize].get().is_none());

    let replacement = loader
        .stubs
        .get(WellKnown::Delegate.name())
        .expect("support library should provide the delegate stub")
        .clone();
    loader
        .stubs
        .insert(WellKnown::Object.name().to_owned(), replacement.clone());

    assert_eq!(
        loader
            .corlib_wkt(WellKnown::Object)
            .expect("a failed resolution must be retried"),
        replacement
    );
    assert!(loader.wkt_table[WellKnown::Object as usize].get().is_some());
}

#[test]
fn test_exception_dispatch_state_prefers_slash_then_falls_back_to_plus() {
    const SLASH_NAME: &str = "System.Exception/DispatchState";

    let mut loader =
        AssemblyLoader::new_bare("test_root".to_string()).expect("failed to create loader");
    let slash_type = loader
        .stubs
        .get(WellKnown::Delegate.name())
        .expect("support library should provide the delegate stub")
        .clone();
    let plus_type = loader
        .stubs
        .get(WellKnown::MulticastDelegate.name())
        .expect("support library should provide the multicast-delegate stub")
        .clone();
    loader
        .stubs
        .insert(SLASH_NAME.to_owned(), slash_type.clone());
    loader.stubs.insert(
        WellKnown::ExceptionDispatchState.name().to_owned(),
        plus_type,
    );
    assert_eq!(
        loader
            .corlib_wkt(WellKnown::ExceptionDispatchState)
            .expect("slash notation should resolve"),
        slash_type
    );

    let mut fallback_loader =
        AssemblyLoader::new_bare("test_root".to_string()).expect("failed to create loader");
    let plus_type = fallback_loader
        .stubs
        .get(WellKnown::MulticastDelegate.name())
        .expect("support library should provide the multicast-delegate stub")
        .clone();
    fallback_loader.stubs.remove(SLASH_NAME);
    fallback_loader.stubs.insert(
        WellKnown::ExceptionDispatchState.name().to_owned(),
        plus_type.clone(),
    );
    assert_eq!(
        fallback_loader
            .corlib_wkt(WellKnown::ExceptionDispatchState)
            .expect("plus notation should be used as the fallback"),
        plus_type
    );
}

#[test]
fn test_memory_extensions_slow_path_signature_uses_read_only_span() {
    let loader =
        AssemblyLoader::new_bare("test_root".to_string()).expect("failed to create loader");
    let td = loader
        .corlib_wkt(WellKnown::MemoryExtensions)
        .expect("System.MemoryExtensions should resolve");
    let res = td.resolution.definition();

    let (slow_path_idx, _) = td
        .definition()
        .methods
        .iter()
        .enumerate()
        .find(|(_, m)| m.name == "SequenceEqualSlowPath")
        .expect("SequenceEqualSlowPath should exist in System.MemoryExtensions");

    use dotnet_types::members::MethodDescription;
    use dotnetdll::prelude::MethodMemberIndex;
    let slow_path_desc = MethodDescription::new(
        td.clone(),
        dotnet_types::generics::GenericLookup::default(),
        td.resolution.clone(),
        MethodMemberIndex::Method(slow_path_idx),
    );
    let sig = slow_path_desc.signature();

    assert_eq!(
        sig.parameters.len(),
        2,
        "SequenceEqualSlowPath should take two parameters"
    );

    let p0 = sig.parameters[0].show(res);
    let p1 = sig.parameters[1].show(res);
    assert!(
        p0.contains("ReadOnlySpan") && p1.contains("ReadOnlySpan"),
        "Expected ReadOnlySpan parameters, got ({p0}, {p1})"
    );
}
