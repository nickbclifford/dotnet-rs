use crate::{
    AssemblyLoader,
    support_contract::{SlotDescriptor, SlotKind},
};
use dotnet_types::{TypeDescription, WellKnown};
use dotnetdll::prelude::{Module, Resolution, TypeDefinition};

fn bare_loader() -> AssemblyLoader {
    AssemblyLoader::new_bare("support-contract-test-root".to_owned())
        .expect("support assembly should satisfy its ABI contract")
}

fn expected_slots() -> Vec<SlotDescriptor> {
    [
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
        ("System.Threading.Tasks.ValueTask", "_task", SlotKind::GcRef),
        (
            "System.Threading.Tasks.ValueTask`1",
            "_task",
            SlotKind::GcRef,
        ),
        (
            "System.Threading.Tasks.ValueTask`1",
            "_result",
            SlotKind::Generic,
        ),
        (
            "System.Threading.Tasks.ValueTask`1",
            "_hasResult",
            SlotKind::ScalarBool,
        ),
        (
            "System.Threading.Tasks.Task",
            "_isCompleted",
            SlotKind::ScalarBool,
        ),
        ("System.Threading.Tasks.Task", "_exception", SlotKind::GcRef),
        (
            "System.Threading.Tasks.Task",
            "_continuation",
            SlotKind::GcRef,
        ),
        (
            "System.Threading.Tasks.Task`1",
            "_result",
            SlotKind::Generic,
        ),
        (
            "System.Threading.Tasks.Task`1",
            "_hasResult",
            SlotKind::ScalarBool,
        ),
        (
            "System.Threading.Tasks.TaskCompletionSource`1",
            "_task",
            SlotKind::GcRef,
        ),
        (
            "System.Runtime.CompilerServices.AsyncTaskMethodBuilder",
            "_task",
            SlotKind::GcRef,
        ),
        (
            "System.Runtime.CompilerServices.AsyncTaskMethodBuilder`1",
            "_task",
            SlotKind::GcRef,
        ),
        (
            "System.Runtime.CompilerServices.AsyncValueTaskMethodBuilder",
            "_task",
            SlotKind::GcRef,
        ),
        (
            "System.Runtime.CompilerServices.AsyncValueTaskMethodBuilder`1",
            "_task",
            SlotKind::GcRef,
        ),
        (
            "System.Runtime.CompilerServices.TaskAwaiter",
            "_task",
            SlotKind::GcRef,
        ),
        (
            "System.Runtime.CompilerServices.TaskAwaiter`1",
            "_task",
            SlotKind::GcRef,
        ),
        (
            "System.Runtime.CompilerServices.ValueTaskAwaiter",
            "_valueTask",
            SlotKind::ValueType,
        ),
        (
            "System.Runtime.CompilerServices.ValueTaskAwaiter`1",
            "_valueTask",
            SlotKind::ValueType,
        ),
        ("DotnetRs.StubAttribute", "InPlaceOf", SlotKind::GcRef),
    ]
    .into_iter()
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
