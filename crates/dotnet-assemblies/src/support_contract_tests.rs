use crate::{
    AssemblyLoader, support_contract::SlotDescriptor, support_contract_slots::EXPECTED_SLOTS,
};
use dotnet_types::{TypeDescription, WellKnown};
use dotnetdll::prelude::{Module, Resolution, TypeDefinition};

fn bare_loader() -> AssemblyLoader {
    AssemblyLoader::new_bare("support-contract-test-root".to_owned())
        .expect("support assembly should satisfy its ABI contract")
}

fn expected_slots() -> Vec<SlotDescriptor> {
    EXPECTED_SLOTS
        .iter()
        .copied()
        .map(|(type_name, field_name, kind)| SlotDescriptor {
            type_name: type_name.into(),
            field_name: field_name.into(),
            kind,
            is_static: false,
        })
        .collect()
}

fn sort_slots(slots: &mut [SlotDescriptor]) {
    slots.sort_unstable_by(|left, right| {
        left.type_name
            .cmp(&right.type_name)
            .then_with(|| left.field_name.cmp(&right.field_name))
    });
}

#[test]
fn test_registry_contains_expected_slots() {
    let loader = bare_loader();
    let mut actual = loader.support_slots().slots.clone();
    let mut expected = expected_slots();

    sort_slots(&mut actual);
    sort_slots(&mut expected);

    assert_eq!(actual, expected);
}

#[test]
fn test_registry_slot_count() {
    let loader = bare_loader();

    // Keep this a literal census total so a deletion cannot be hidden by updating the
    // expected-slot table above at the same time.
    assert_eq!(loader.support_slots().slots.len(), 42);
}

#[test]
fn handle_override_is_limited_to_the_support_descriptor() {
    let loader = bare_loader();
    let support_handle = loader
        .corlib_wkt(WellKnown::RuntimeTypeHandle)
        .expect("support RuntimeTypeHandle stub is registered");
    assert!(loader.is_handle_value_slot(&support_handle, "_value"));

    let mut resolution = Resolution::new(Module::new("same-name-user-assembly.dll"));
    let same_name_index = resolution.push_type_definition(TypeDefinition::new(
        Some("System".into()),
        "RuntimeTypeHandle",
    ));
    let resolution = loader.register_owned_assembly(resolution);
    let same_name_type = TypeDescription::new(resolution, same_name_index);

    assert!(
        !loader.is_handle_value_slot(&same_name_type, "_value"),
        "a same-named non-support type must retain its metadata-derived layout"
    );
}
