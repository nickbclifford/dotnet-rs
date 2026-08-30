use crate::{
    AssemblyLoader, SUPPORT_ASSEMBLY,
    support_contract::{
        RUNTIME_SLOT_DESCRIPTORS, RuntimeSlotId, RuntimeSlotSignature, SupportSlotOps,
    },
};
use dotnet_types::{TypeDescription, WellKnown};
use dotnet_utils::{ArenaId, gc::ThreadSafeLock, sync::Arc};
use dotnet_value::{
    layout::{FieldKey, FieldLayout, FieldLayoutManager, GcDesc, LayoutManager, Scalar},
    object::{HeapStorage, ObjectInner, ObjectRef},
    pointer::ManagedPtr,
    storage::{FieldRef, FieldStorage},
    string::CLRString,
};
use dotnetdll::prelude::{Module, Resolution, TypeDefinition};
use gc_arena::{Arena, Gc, Rootable};
use hashbrown::HashMap;

fn bare_loader() -> AssemblyLoader {
    AssemblyLoader::new_bare("support-contract-test-root".to_owned())
        .expect("support assembly should satisfy its ABI contract")
}

#[test]
fn test_registry_slot_count() {
    let loader = bare_loader();

    // Keep this a literal census total so a deletion cannot be hidden by updating the generated
    // definition at the same time.
    assert_eq!(loader.support_slots().len(), 21);
}

#[test]
fn generated_slot_descriptors_preserve_stable_ids_and_exact_shapes() {
    assert_eq!(RuntimeSlotId::COUNT, 21);
    assert_eq!(RUNTIME_SLOT_DESCRIPTORS.len(), 21);
    assert_eq!(
        RuntimeSlotId::from_i32(1),
        Some(RuntimeSlotId::RuntimeTypeHandleValue)
    );
    assert_eq!(RuntimeSlotId::from_i32(22), None);

    let span_reference = RuntimeSlotId::SpanReference.descriptor();
    assert_eq!(span_reference.id as i32, 18);
    assert_eq!(span_reference.field_name, "_reference");
    assert_eq!(
        span_reference.signature,
        RuntimeSlotSignature::ByRefTypeGeneric(0)
    );
}

#[test]
fn generated_runtime_slot_enum_is_in_embedded_support_assembly() {
    let loader = bare_loader();
    let support = loader
        .get_assembly(SUPPORT_ASSEMBLY)
        .expect("embedded support assembly should be registered");

    assert!(
        support
            .enumerate_type_definitions()
            .any(|(_, ty)| ty.type_name() == "DotnetRs.RuntimeSlotId"),
        "support.csproj must compile the generated RuntimeSlotId.g.cs source"
    );
}

#[test]
fn handle_override_is_limited_to_the_support_descriptor() {
    let loader = bare_loader();
    let support_handle = loader
        .corlib_wkt(WellKnown::RuntimeTypeHandle)
        .expect("support RuntimeTypeHandle stub is registered");
    assert_eq!(
        loader.support_slot_id_for_field(&support_handle, "_value"),
        Some(RuntimeSlotId::RuntimeTypeHandleValue)
    );

    let mut resolution = Resolution::new(Module::new("same-name-user-assembly.dll"));
    let same_name_index = resolution.push_type_definition(TypeDefinition::new(
        Some("System".into()),
        "RuntimeTypeHandle",
    ));
    let resolution = loader.register_owned_assembly(resolution);
    let same_name_type = TypeDescription::new(resolution, same_name_index);

    assert!(
        loader
            .support_slot_id_for_field(&same_name_type, "_value")
            .is_none(),
        "a same-named non-support type must retain its metadata-derived layout"
    );
}

#[test]
fn loaded_support_metadata_has_an_independent_literal_slot_census() {
    let loader = bare_loader();
    let support = loader
        .get_assembly(SUPPORT_ASSEMBLY)
        .expect("embedded support assembly should be registered");
    let mut ids = std::collections::HashSet::new();
    let mut count = 0;

    // Deliberately scan loaded metadata rather than the generated contract or registry. The
    // literal policy count catches a coordinated change to those Rust products.
    for (type_index, _) in support.enumerate_type_definitions() {
        for (field_index, _) in support.enumerate_fields(type_index) {
            for attribute in support
                .field_attributes(field_index)
                .expect("support field attributes must decode")
            {
                let dotnetdll::prelude::UserMethod::Definition(constructor) = attribute.constructor
                else {
                    continue;
                };
                if support[constructor.parent_type()].type_name() != "DotnetRs.RuntimeSlotAttribute"
                {
                    continue;
                }
                let data = attribute
                    .instantiation_data(&&loader, &support)
                    .expect("RuntimeSlot attribute data must decode");
                let [
                    dotnetdll::prelude::FixedArg::Integral(
                        dotnetdll::prelude::IntegralParam::Int32(id),
                    ),
                ] = data.constructor_args.as_slice()
                else {
                    panic!("RuntimeSlot attribute must contain one Int32 enum payload");
                };
                count += 1;
                assert!(
                    ids.insert(*id),
                    "RuntimeSlot ID {id} is duplicated in metadata"
                );
            }
        }
    }

    assert_eq!(
        count, 21,
        "support metadata slot census is a fixed ABI policy"
    );
    assert_eq!(ids.len(), 21);
}

fn storage_for_slot(desc: TypeDescription, id: RuntimeSlotId) -> FieldStorage {
    let scalar = match id.descriptor().primary_accessor.value_type {
        crate::support_contract::RuntimeSlotAccessorType::ObjectRef => Scalar::ObjectRef,
        crate::support_contract::RuntimeSlotAccessorType::Usize => Scalar::NativeInt,
        crate::support_contract::RuntimeSlotAccessorType::I32 => Scalar::Int32,
        crate::support_contract::RuntimeSlotAccessorType::ManagedPtr => Scalar::ManagedPtr,
    };
    let mut gc_desc = GcDesc::default();
    if scalar == Scalar::ObjectRef {
        gc_desc.set_offset(0);
    }
    let layout = Arc::new(FieldLayoutManager {
        fields: HashMap::from_iter([(
            FieldKey {
                owner: desc,
                name: id.descriptor().field_name.into(),
            },
            FieldLayout {
                position: 0.into(),
                layout: Arc::new(LayoutManager::Scalar(scalar.clone())),
            },
        )]),
        total_size: scalar.size_const(),
        alignment: scalar.alignment_const(),
        gc_desc,
        has_ref_fields: scalar == Scalar::ManagedPtr,
    });
    FieldStorage::new(layout.clone(), vec![0; layout.total_size])
}

macro_rules! assert_primary_accessor {
    ($loader:expr, $id:ident, $method:ident, $value:ty) => {{
        let id = RuntimeSlotId::$id;
        let desc = $loader
            .corlib_wkt(id.descriptor().declaring_well_known)
            .expect("validated support owner must resolve");
        let storage = storage_for_slot(desc.clone(), id);
        let field: Option<FieldRef<'_, $value>> = $loader.$method(&storage, desc);
        assert!(field.is_some(), "{} must resolve", stringify!($method));
    }};
}

#[test]
fn every_generated_id_and_typed_accessor_resolves_its_prescribed_owner() {
    let loader = bare_loader();

    for descriptor in RUNTIME_SLOT_DESCRIPTORS {
        let owner = loader
            .corlib_wkt(descriptor.declaring_well_known)
            .expect("every generated support owner must resolve");
        assert_eq!(
            loader.support_slot_id_for_field(&owner, descriptor.field_name),
            Some(descriptor.id),
            "{} must retain its exact descriptor identity",
            descriptor.semantic_name
        );
    }

    assert_primary_accessor!(
        loader,
        RuntimeTypeHandleValue,
        rth_value_field,
        ObjectRef<'_>
    );
    assert_primary_accessor!(
        loader,
        RuntimeFieldHandleValue,
        rfh_value_field,
        ObjectRef<'_>
    );
    assert_primary_accessor!(
        loader,
        RuntimeMethodHandleValue,
        rmh_value_field,
        ObjectRef<'_>
    );
    assert_primary_accessor!(loader, RuntimeTypeIndex, runtime_type_index_field, usize);
    assert_primary_accessor!(loader, MethodInfoIndex, method_info_index_field, usize);
    assert_primary_accessor!(
        loader,
        ConstructorInfoIndex,
        constructor_info_index_field,
        usize
    );
    assert_primary_accessor!(loader, FieldInfoIndex, field_info_index_field, usize);
    assert_primary_accessor!(
        loader,
        ParameterInfoMethodIndex,
        parameter_info_method_index_field,
        usize
    );
    assert_primary_accessor!(
        loader,
        ParameterInfoPosition,
        parameter_info_position_field,
        i32
    );
    assert_primary_accessor!(
        loader,
        PropertyInfoName,
        property_info_name_field,
        ObjectRef<'_>
    );
    assert_primary_accessor!(
        loader,
        PropertyInfoGetter,
        property_info_getter_field,
        ObjectRef<'_>
    );
    assert_primary_accessor!(
        loader,
        PropertyInfoSetter,
        property_info_setter_field,
        ObjectRef<'_>
    );
    assert_primary_accessor!(
        loader,
        PropertyInfoDeclaringType,
        property_info_declaring_type_field,
        ObjectRef<'_>
    );
    assert_primary_accessor!(
        loader,
        PropertyInfoPropertyType,
        property_info_property_type_field,
        ObjectRef<'_>
    );
    assert_primary_accessor!(loader, DelegateTarget, delegate_target_field, ObjectRef<'_>);
    assert_primary_accessor!(
        loader,
        DelegateMethodIndex,
        delegate_method_index_field,
        usize
    );
    assert_primary_accessor!(
        loader,
        MulticastDelegateTargets,
        multicast_delegate_targets_field,
        ObjectRef<'_>
    );
    assert_primary_accessor!(loader, SpanReference, span_reference_field, ManagedPtr<'_>);
    assert_primary_accessor!(loader, SpanLength, span_length_field, i32);
    assert_primary_accessor!(
        loader,
        ReadOnlySpanReference,
        readonly_span_reference_field,
        ManagedPtr<'_>
    );
    assert_primary_accessor!(loader, ReadOnlySpanLength, readonly_span_length_field, i32);

    // The generated bounded helpers retain their concrete return types while accepting either
    // precisely enumerated span descriptor.
    let span = loader.corlib_wkt(WellKnown::Span1).unwrap();
    let reference_storage = storage_for_slot(span.clone(), RuntimeSlotId::SpanReference);
    let _: Option<FieldRef<'_, ManagedPtr<'_>>> =
        loader.span_or_readonly_span_reference_field(&reference_storage, span.clone());
    let length_storage = storage_for_slot(span.clone(), RuntimeSlotId::SpanLength);
    let _: Option<FieldRef<'_, i32>> =
        loader.span_or_readonly_span_length_field(&length_storage, span);
}

#[test]
fn runtime_type_handle_object_ref_and_usize_views_round_trip() {
    let loader = bare_loader();
    let id = RuntimeSlotId::RuntimeTypeHandleValue;
    let desc = loader
        .corlib_wkt(id.descriptor().declaring_well_known)
        .expect("RuntimeTypeHandle support owner must resolve");
    let storage = storage_for_slot(desc.clone(), id);

    // This mirrors the actual handle layout: its nint metadata representation is stored as an
    // ObjectRef and is therefore included in the GC descriptor before the usize view is used.
    let mut traced_slots = Vec::new();
    storage
        .layout()
        .gc_desc
        .for_each_word_index(|index| traced_slots.push(index));
    assert_eq!(traced_slots, vec![0]);
    type TestRoot = Rootable![()];
    let arena = Arena::<TestRoot>::new(|_| ());
    arena.mutate(|mutation, _| {
        // Build a live reference directly from the arena mutation context so this test is valid
        // for both single-threaded and multithreaded dependency feature configurations.
        let object = ObjectRef(Some(Gc::new(
            mutation,
            ThreadSafeLock::new(ObjectInner::new(
                HeapStorage::Str(CLRString::from("handle")),
                ArenaId::INVALID,
            )),
        )));
        loader
            .rth_value_field(&storage, desc.clone())
            .expect("ObjectRef handle view")
            .write(object);
        assert_ne!(
            loader
                .rth_value_usize_field(&storage, desc.clone())
                .expect("width-checked usize handle view")
                .read(),
            0
        );
        assert_eq!(
            loader
                .rth_value_field(&storage, desc)
                .expect("ObjectRef handle view")
                .read(),
            object
        );
    });
}

#[test]
fn separate_span_layout_accessors_preserve_their_offsets() {
    let loader = bare_loader();
    let span = loader.corlib_wkt(WellKnown::Span1).unwrap();
    let readonly_span = loader.corlib_wkt(WellKnown::ReadOnlySpan1).unwrap();
    let layout = FieldLayoutManager {
        fields: HashMap::from_iter([
            (
                FieldKey {
                    owner: span.clone(),
                    name: "_reference".into(),
                },
                FieldLayout {
                    position: 8.into(),
                    layout: Arc::new(Scalar::ManagedPtr.into()),
                },
            ),
            (
                FieldKey {
                    owner: span,
                    name: "_length".into(),
                },
                FieldLayout {
                    position: 32.into(),
                    layout: Arc::new(Scalar::Int32.into()),
                },
            ),
            (
                FieldKey {
                    owner: readonly_span.clone(),
                    name: "_reference".into(),
                },
                FieldLayout {
                    position: 48.into(),
                    layout: Arc::new(Scalar::ManagedPtr.into()),
                },
            ),
            (
                FieldKey {
                    owner: readonly_span,
                    name: "_length".into(),
                },
                FieldLayout {
                    position: 72.into(),
                    layout: Arc::new(Scalar::Int32.into()),
                },
            ),
        ]),
        total_size: 80,
        alignment: ObjectRef::SIZE,
        gc_desc: GcDesc::default(),
        has_ref_fields: true,
    };

    assert_eq!(
        loader
            .span_reference_layout(&layout)
            .unwrap()
            .position
            .as_usize(),
        8
    );
    assert_eq!(
        loader
            .span_length_layout(&layout)
            .unwrap()
            .position
            .as_usize(),
        32
    );
    assert_eq!(
        loader
            .readonly_span_reference_layout(&layout)
            .unwrap()
            .position
            .as_usize(),
        48
    );
    assert_eq!(
        loader
            .readonly_span_length_layout(&layout)
            .unwrap()
            .position
            .as_usize(),
        72
    );
    assert_eq!(
        loader
            .span_or_readonly_span_reference_layout(&layout)
            .unwrap()
            .position
            .as_usize(),
        8
    );
    assert_eq!(
        loader
            .span_or_readonly_span_length_layout(&layout)
            .unwrap()
            .position
            .as_usize(),
        32
    );
}
