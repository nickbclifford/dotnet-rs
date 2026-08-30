use crate::{
    AssemblyLoader, error::AssemblyLoadError, support::parse_runtime_slot_id,
    support_contract::RuntimeSlotId,
};
use dotnetdll::prelude::*;

fn bare_loader() -> AssemblyLoader {
    AssemblyLoader::new_bare("support-contract-validation-test-root".to_owned())
        .expect("embedded support assembly should satisfy its ABI contract")
}

#[derive(Clone, Copy)]
enum SyntheticSlotId {
    Known(RuntimeSlotId),
    Unknown(i32),
}

impl SyntheticSlotId {
    fn constructor_arg(self) -> FixedArg<'static> {
        // Real C# enum-valued attribute arguments are decoded as their Int32 payload. Keep the
        // synthetic fixture on that path while permitting IDs outside the generated enum.
        let value = match self {
            Self::Known(id) => id as i32,
            Self::Unknown(value) => value,
        };
        FixedArg::Integral(IntegralParam::Int32(value))
    }
}

/// Constructs just enough metadata to reach a selected exact-ID contract failure before the
/// validator checks the remaining required IDs. It intentionally controls the declared enum
/// constructor, raw ID payload, field signature, placement, staticness, and annotation count.
fn synthetic_slot_resolution(
    type_name: &'static str,
    id: SyntheticSlotId,
    field: Field<'static>,
    annotation_count: usize,
    used_implicitly: bool,
) -> Resolution<'static> {
    let mut resolution = Resolution::new(Module::new("fake-support.dll"));
    let runtime_slot_attribute = resolution.push_type_definition(TypeDefinition::new(
        Some("DotnetRs".into()),
        "RuntimeSlotAttribute",
    ));
    let runtime_slot_id = resolution.push_type_definition(TypeDefinition::new(
        Some("DotnetRs".into()),
        "RuntimeSlotId",
    ));
    let runtime_slot_constructor = resolution.push_method(
        runtime_slot_attribute,
        Method::constructor(
            Accessibility::Public,
            vec![Parameter::value(
                BaseType::valuetype(runtime_slot_id).into(),
            )],
            None,
        ),
    );
    let used_implicitly_attribute = resolution.push_type_definition(TypeDefinition::new(
        Some("JetBrains.Annotations".into()),
        "UsedImplicitlyAttribute",
    ));
    let used_implicitly_constructor = resolution.push_method(
        used_implicitly_attribute,
        Method::constructor(Accessibility::Public, vec![], None),
    );
    let (namespace, name) = type_name.rsplit_once('.').expect("type has a namespace");
    let type_index =
        resolution.push_type_definition(TypeDefinition::new(Some(namespace.into()), name));

    let field_index = resolution.push_field(type_index, field);
    for _ in 0..annotation_count {
        resolution[field_index].attributes.push(Attribute::new(
            runtime_slot_constructor.into(),
            CustomAttributeData {
                constructor_args: vec![id.constructor_arg()],
                named_args: vec![],
            },
        ));
    }
    if used_implicitly {
        resolution[field_index].attributes.push(Attribute::new(
            used_implicitly_constructor.into(),
            CustomAttributeData {
                constructor_args: vec![],
                named_args: vec![],
            },
        ));
    }
    resolution
}

fn handle_field() -> Field<'static> {
    Field::instance(Accessibility::Private, "_value", BaseType::IntPtr.into())
}

struct ContractViolation {
    type_name: Box<str>,
    field_name: Box<str>,
    reason: Box<str>,
}

fn contract_violation(
    result: Result<crate::support_contract::SupportSlotRegistry, AssemblyLoadError>,
) -> ContractViolation {
    match result {
        Err(AssemblyLoadError::SupportContractViolation {
            type_name,
            field_name,
            reason,
        }) => ContractViolation {
            type_name,
            field_name,
            reason,
        },
        other => panic!("expected SupportContractViolation, got {other:?}"),
    }
}

fn assert_context(violation: &ContractViolation, type_name: &str, field_name: &str) {
    assert_eq!(violation.type_name.as_ref(), type_name);
    assert_eq!(violation.field_name.as_ref(), field_name);
}

#[test]
fn generated_runtime_slot_ids_are_required() {
    assert_eq!(
        parse_runtime_slot_id(&[FixedArg::Integral(IntegralParam::Int32(
            RuntimeSlotId::RuntimeTypeHandleValue as i32,
        ))]),
        Ok(RuntimeSlotId::RuntimeTypeHandleValue),
    );
    for argument in [
        FixedArg::String(Some("Handle".into())),
        FixedArg::Enum("DotnetRs.OtherId".into(), IntegralParam::Int32(1)),
        FixedArg::Enum("DotnetRs.RuntimeSlotId".into(), IntegralParam::Int16(1)),
        FixedArg::Integral(IntegralParam::Int32(22)),
    ] {
        assert!(parse_runtime_slot_id(&[argument]).is_err());
    }
}

#[test]
fn unknown_id_is_a_contextual_contract_violation() {
    let resolution = synthetic_slot_resolution(
        "System.RuntimeTypeHandle",
        SyntheticSlotId::Unknown(22),
        handle_field(),
        1,
        true,
    );
    let violation = contract_violation(bare_loader().validate_support_contract(&resolution));
    assert_context(&violation, "System.RuntimeTypeHandle", "_value");
    assert_eq!(violation.reason.as_ref(), "unknown RuntimeSlot ID 22");
}

#[test]
fn missing_id_reports_its_exact_expected_slot() {
    let resolution = Resolution::new(Module::new("fake-support.dll"));
    let violation = contract_violation(bare_loader().validate_support_contract(&resolution));
    assert_context(&violation, "System.RuntimeTypeHandle", "_value");
    assert_eq!(
        violation.reason.as_ref(),
        "missing RuntimeSlot ID RuntimeTypeHandleValue: expected System.RuntimeTypeHandle._value with signature IntPtr"
    );
}

#[test]
fn duplicate_annotations_report_the_id_and_both_locations() {
    let resolution = synthetic_slot_resolution(
        "System.RuntimeTypeHandle",
        SyntheticSlotId::Known(RuntimeSlotId::RuntimeTypeHandleValue),
        handle_field(),
        2,
        true,
    );
    let violation = contract_violation(bare_loader().validate_support_contract(&resolution));
    assert_context(&violation, "System.RuntimeTypeHandle", "_value");
    assert_eq!(
        violation.reason.as_ref(),
        "duplicate RuntimeSlot ID RuntimeTypeHandleValue: found System.RuntimeTypeHandle._value, already used by System.RuntimeTypeHandle._value; expected System.RuntimeTypeHandle._value"
    );
}

#[test]
fn misplaced_id_reports_actual_and_expected_locations() {
    let resolution = synthetic_slot_resolution(
        "DotnetRs.Extra",
        SyntheticSlotId::Known(RuntimeSlotId::DelegateTarget),
        Field::instance(
            Accessibility::Private,
            "not_consumed_by_rust",
            ctype! { object },
        ),
        1,
        true,
    );
    let violation = contract_violation(bare_loader().validate_support_contract(&resolution));
    assert_context(&violation, "DotnetRs.Extra", "not_consumed_by_rust");
    assert_eq!(
        violation.reason.as_ref(),
        "RuntimeSlot ID DelegateTarget is misplaced: found DotnetRs.Extra.not_consumed_by_rust, expected System.Delegate._target"
    );
}

#[test]
fn exact_signature_mismatch_reports_expected_and_found_shape() {
    let resolution = synthetic_slot_resolution(
        "DotnetRs.PropertyInfo",
        SyntheticSlotId::Known(RuntimeSlotId::PropertyInfoGetter),
        Field::instance(Accessibility::Private, "getter", ctype! { string }),
        1,
        true,
    );
    let violation = contract_violation(bare_loader().validate_support_contract(&resolution));
    assert_context(&violation, "DotnetRs.PropertyInfo", "getter");
    assert_eq!(
        violation.reason.as_ref(),
        "RuntimeSlot ID PropertyInfoGetter has an exact signature mismatch at DotnetRs.PropertyInfo.getter: expected Class(ReflectionMethodInfo), found by_ref=false type Base(String)"
    );
}

#[test]
fn wrong_staticness_reports_actual_and_expected_staticness() {
    let mut field = handle_field();
    field.static_member = true;
    let resolution = synthetic_slot_resolution(
        "System.RuntimeTypeHandle",
        SyntheticSlotId::Known(RuntimeSlotId::RuntimeTypeHandleValue),
        field,
        1,
        true,
    );
    let violation = contract_violation(bare_loader().validate_support_contract(&resolution));
    assert_context(&violation, "System.RuntimeTypeHandle", "_value");
    assert_eq!(
        violation.reason.as_ref(),
        "RuntimeSlot ID RuntimeTypeHandleValue has wrong staticness at System.RuntimeTypeHandle._value: expected instance field, found static field"
    );
}

#[test]
fn runtime_slot_field_without_used_implicitly_is_a_contract_violation() {
    let resolution = synthetic_slot_resolution(
        "System.RuntimeTypeHandle",
        SyntheticSlotId::Known(RuntimeSlotId::RuntimeTypeHandleValue),
        handle_field(),
        1,
        false,
    );
    let violation = contract_violation(bare_loader().validate_support_contract(&resolution));
    assert_context(&violation, "System.RuntimeTypeHandle", "_value");
    assert_eq!(
        violation.reason.as_ref(),
        "fields marked [RuntimeSlot(...)] must carry [UsedImplicitly]"
    );
}

enum StubUse {
    Target(&'static str),
    MissingInPlaceOf,
    NullInPlaceOf,
}

fn synthetic_stub_schema_resolution(
    in_place_of_field: Field<'static>,
    uses: &[StubUse],
) -> Resolution<'static> {
    let mut resolution = Resolution::new(Module::new("fake-support.dll"));
    let stub_attribute = resolution.push_type_definition(TypeDefinition::new(
        Some("DotnetRs".into()),
        "StubAttribute",
    ));
    resolution.push_field(stub_attribute, in_place_of_field);
    let stub_constructor = resolution.push_method(
        stub_attribute,
        Method::constructor(Accessibility::Public, vec![], None),
    );
    for (index, use_) in uses.iter().enumerate() {
        let target = resolution.push_type_definition(TypeDefinition::new(
            Some("DotnetRs".into()),
            format!("StubTarget{index}"),
        ));
        let named_args = match use_ {
            StubUse::Target(target) => vec![NamedArg::Field(
                "InPlaceOf".into(),
                FixedArg::String(Some((*target).into())),
            )],
            StubUse::MissingInPlaceOf => vec![],
            StubUse::NullInPlaceOf => {
                vec![NamedArg::Field("InPlaceOf".into(), FixedArg::String(None))]
            }
        };
        resolution[target].attributes.push(Attribute::new(
            stub_constructor.into(),
            CustomAttributeData {
                constructor_args: vec![],
                named_args,
            },
        ));
    }
    resolution
}

fn assert_stub_schema_violation(result: Result<(), AssemblyLoadError>) {
    assert!(
        matches!(
            result,
            Err(AssemblyLoadError::InvalidFormat(ref reason))
                if reason.contains("StubAttribute") || reason.contains("[Stub]")
        ),
        "expected contextual StubAttribute schema error, got {result:?}"
    );
}

#[test]
fn stub_schema_requires_exact_instance_string_in_place_of_field() {
    let resolution = synthetic_stub_schema_resolution(
        Field::instance(Accessibility::Public, "InPlaceOf", ctype! { int }),
        &[StubUse::Target("System.StubTarget")],
    );
    assert_stub_schema_violation(bare_loader().validate_stub_schema_metadata(&resolution));
}

#[test]
fn stub_schema_requires_exactly_one_non_null_in_place_of_named_argument() {
    let loader = bare_loader();
    let valid_field = || Field::instance(Accessibility::Public, "InPlaceOf", ctype! { string });
    let missing = synthetic_stub_schema_resolution(valid_field(), &[StubUse::MissingInPlaceOf]);
    assert_stub_schema_violation(loader.validate_stub_schema_metadata(&missing));
    let null = synthetic_stub_schema_resolution(valid_field(), &[StubUse::NullInPlaceOf]);
    assert_stub_schema_violation(loader.validate_stub_schema_metadata(&null));
}

#[test]
fn stub_schema_rejects_duplicate_targets_without_mutating_registered_maps() {
    let loader = bare_loader();
    let stubs_before = loader.stubs.clone();
    let reverse_stubs_before = loader.reverse_stubs.clone();
    let resolution = synthetic_stub_schema_resolution(
        Field::instance(Accessibility::Public, "InPlaceOf", ctype! { string }),
        &[
            StubUse::Target("System.DuplicateTarget"),
            StubUse::Target("System.DuplicateTarget"),
        ],
    );

    assert_stub_schema_violation(loader.validate_stub_schema_metadata(&resolution));
    assert_eq!(loader.stubs, stubs_before);
    assert_eq!(loader.reverse_stubs, reverse_stubs_before);
}
