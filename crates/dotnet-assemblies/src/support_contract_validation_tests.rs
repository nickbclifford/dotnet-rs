use crate::{AssemblyLoader, error::AssemblyLoadError, support_contract::SlotKind};
use dotnetdll::prelude::*;

const EXPECTED_SLOTS: &[(&str, &str, SlotKind)] = &[
    ("System.RuntimeTypeHandle", "_value", SlotKind::Handle),
    ("System.RuntimeFieldHandle", "_value", SlotKind::Handle),
    ("System.RuntimeMethodHandle", "_value", SlotKind::Handle),
    ("System.RuntimeType", "index", SlotKind::Index),
    ("DotnetRs.MethodInfo", "index", SlotKind::Index),
    ("DotnetRs.ConstructorInfo", "index", SlotKind::Index),
    ("DotnetRs.FieldInfo", "index", SlotKind::Index),
    ("DotnetRs.ParameterInfo", "method_index", SlotKind::Index),
    ("DotnetRs.ParameterInfo", "position", SlotKind::ScalarInt),
    ("DotnetRs.PropertyInfo", "name", SlotKind::GcRef),
    ("DotnetRs.PropertyInfo", "getter", SlotKind::GcRef),
    ("DotnetRs.PropertyInfo", "setter", SlotKind::GcRef),
    ("DotnetRs.PropertyInfo", "declaringType", SlotKind::GcRef),
    ("DotnetRs.PropertyInfo", "propertyType", SlotKind::GcRef),
    ("System.Delegate", "_target", SlotKind::GcRef),
    ("System.Delegate", "_method", SlotKind::Index),
    ("System.MulticastDelegate", "targets", SlotKind::GcRef),
    ("System.Span`1", "_reference", SlotKind::Byref),
    ("System.Span`1", "_length", SlotKind::ScalarInt),
    ("System.ReadOnlySpan`1", "_reference", SlotKind::Byref),
    ("System.ReadOnlySpan`1", "_length", SlotKind::ScalarInt),
    ("DotnetRs.Module", "resolution", SlotKind::NativePtr),
    ("DotnetRs.Assembly", "resolution", SlotKind::NativePtr),
];

fn bare_loader() -> AssemblyLoader {
    AssemblyLoader::new_bare("support-contract-validation-test-root".to_owned())
        .expect("embedded support assembly should satisfy its ABI contract")
}

fn kind_name(kind: SlotKind) -> &'static str {
    match kind {
        SlotKind::Handle => "Handle",
        SlotKind::Index => "Index",
        SlotKind::GcRef => "GcRef",
        SlotKind::Byref => "Byref",
        SlotKind::ScalarInt => "ScalarInt",
        SlotKind::NativePtr => "NativePtr",
    }
}

fn field_for_slot(field_name: &'static str, kind: SlotKind) -> Field<'static> {
    let return_type = match kind {
        SlotKind::Handle | SlotKind::Index | SlotKind::NativePtr => BaseType::IntPtr.into(),
        SlotKind::GcRef => ctype! { object },
        SlotKind::Byref => MemberType::TypeGeneric(0),
        SlotKind::ScalarInt => ctype! { int },
    };
    let mut field = Field::instance(Accessibility::Private, field_name, return_type);
    field.by_ref = kind == SlotKind::Byref;
    field
}

/// Constructs only the metadata required by the RuntimeSlot parser: the attribute type and
/// constructor plus a declaring type and annotated field for every supplied slot.
fn synthetic_support_resolution(
    slots: &[(&'static str, &'static str, SlotKind)],
) -> Resolution<'static> {
    synthetic_support_resolution_with_wrong_metadata(slots, None)
}

fn synthetic_support_resolution_with_wrong_metadata(
    slots: &[(&'static str, &'static str, SlotKind)],
    wrong_metadata_slot: Option<(&str, &str)>,
) -> Resolution<'static> {
    let mut resolution = Resolution::new(Module::new("fake-support.dll"));
    let attribute_type = resolution.push_type_definition(TypeDefinition::new(
        Some("DotnetRs".into()),
        "RuntimeSlotAttribute",
    ));
    let attribute_constructor = resolution.push_method(
        attribute_type,
        Method::constructor(
            Accessibility::Public,
            vec![Parameter::value(ctype! { string })],
            None,
        ),
    );

    for &(type_name, field_name, kind) in slots {
        let (namespace, name) = type_name
            .rsplit_once('.')
            .expect("support type names include a namespace");
        let type_index =
            resolution.push_type_definition(TypeDefinition::new(Some(namespace.into()), name));
        let field = if wrong_metadata_slot == Some((type_name, field_name)) {
            Field::instance(Accessibility::Private, field_name, ctype! { int })
        } else {
            field_for_slot(field_name, kind)
        };
        let field_index = resolution.push_field(type_index, field);
        resolution[field_index].attributes.push(Attribute::new(
            attribute_constructor.into(),
            CustomAttributeData {
                constructor_args: vec![FixedArg::String(Some(kind_name(kind).into()))],
                named_args: vec![],
            },
        ));
    }

    resolution
}

fn assert_contract_violation(
    result: Result<crate::support_contract::SupportSlotRegistry, AssemblyLoadError>,
) {
    assert!(
        matches!(
            result,
            Err(AssemblyLoadError::SupportContractViolation { .. })
        ),
        "expected SupportContractViolation, got {result:?}"
    );
}

#[test]
fn missing_required_slot_is_a_contract_violation() {
    let slots: Vec<_> = EXPECTED_SLOTS
        .iter()
        .copied()
        .filter(|(type_name, field_name, _)| {
            !(*type_name == "System.RuntimeTypeHandle" && *field_name == "_value")
        })
        .collect();
    let resolution = synthetic_support_resolution(&slots);

    assert_contract_violation(bare_loader().validate_support_contract(&resolution));
}

#[test]
fn duplicate_required_slot_is_a_contract_violation() {
    let mut slots = EXPECTED_SLOTS.to_vec();
    slots.push(("System.RuntimeTypeHandle", "_value", SlotKind::Handle));
    let resolution = synthetic_support_resolution(&slots);

    assert_contract_violation(bare_loader().validate_support_contract(&resolution));
}

#[test]
fn wrong_slot_kind_is_a_contract_violation() {
    let mut slots = EXPECTED_SLOTS.to_vec();
    let handle_slot = slots
        .iter_mut()
        .find(|(type_name, field_name, _)| {
            *type_name == "System.RuntimeTypeHandle" && *field_name == "_value"
        })
        .expect("fixture includes RuntimeTypeHandle._value");
    handle_slot.2 = SlotKind::GcRef;
    let resolution = synthetic_support_resolution(&slots);

    assert_contract_violation(bare_loader().validate_support_contract(&resolution));
}

#[test]
fn wrong_field_metadata_is_a_contract_violation() {
    let resolution = synthetic_support_resolution_with_wrong_metadata(
        EXPECTED_SLOTS,
        Some(("System.RuntimeTypeHandle", "_value")),
    );
    let result = bare_loader().validate_support_contract(&resolution);

    assert!(
        matches!(
            result,
            Err(AssemblyLoadError::SupportContractViolation {
                ref type_name,
                ref field_name,
                ref reason,
            }) if type_name.as_ref() == "System.RuntimeTypeHandle"
                && field_name.as_ref() == "_value"
                && reason.contains("metadata type")
        ),
        "expected a metadata-shape contract violation, got {result:?}"
    );
}

#[test]
fn wrong_field_staticness_is_a_contract_violation() {
    let mut resolution = synthetic_support_resolution(EXPECTED_SLOTS);
    let type_index = resolution
        .enumerate_type_definitions()
        .find(|(_, ty)| ty.type_name() == "System.RuntimeTypeHandle")
        .map(|(index, _)| index)
        .expect("fixture includes RuntimeTypeHandle");
    let field_index = resolution
        .enumerate_fields(type_index)
        .find(|(_, field)| field.name == "_value")
        .map(|(index, _)| index)
        .expect("fixture includes RuntimeTypeHandle._value");
    resolution[field_index].static_member = true;

    let result = bare_loader().validate_support_contract(&resolution);
    assert!(
        matches!(
            result,
            Err(AssemblyLoadError::SupportContractViolation { ref reason, .. })
                if reason.contains("expected Handle instance field")
        ),
        "expected a staticness contract violation, got {result:?}"
    );
}

#[test]
fn extra_annotated_slot_is_unrecognized() {
    let mut slots = EXPECTED_SLOTS.to_vec();
    slots.push(("DotnetRs.Extra", "not_consumed_by_rust", SlotKind::GcRef));
    let resolution = synthetic_support_resolution(&slots);
    let result = bare_loader().validate_support_contract(&resolution);

    assert!(
        matches!(
            result,
            Err(AssemblyLoadError::UnrecognizedSupportSlot {
                ref type_name,
                ref field_name,
            }) if type_name.as_ref() == "DotnetRs.Extra"
                && field_name.as_ref() == "not_consumed_by_rust"
        ),
        "expected UnrecognizedSupportSlot for the synthetic extra field, got {result:?}"
    );
}
