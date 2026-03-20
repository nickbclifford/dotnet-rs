use crate::AssemblyLoader;
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
fn test_memory_extensions_slow_path_signature_uses_read_only_span() {
    let loader =
        AssemblyLoader::new_bare("test_root".to_string()).expect("failed to create loader");
    let td = loader
        .corlib_type("System.MemoryExtensions")
        .expect("System.MemoryExtensions should resolve");
    let res = td.resolution.definition();

    let slow_path = td
        .definition()
        .methods
        .iter()
        .find(|m| m.name == "SequenceEqualSlowPath")
        .expect("SequenceEqualSlowPath should exist in System.MemoryExtensions");

    assert_eq!(
        slow_path.signature.parameters.len(),
        2,
        "SequenceEqualSlowPath should take two parameters"
    );

    let p0 = slow_path.signature.parameters[0].show(res);
    let p1 = slow_path.signature.parameters[1].show(res);
    assert!(
        p0.contains("ReadOnlySpan") && p1.contains("ReadOnlySpan"),
        "Expected ReadOnlySpan parameters, got ({p0}, {p1})"
    );
}
