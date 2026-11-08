//! Production field-storage transport for references between test arenas.

use crate::sync::Arc;
use dotnet_value::object::{HeapStorage, ObjectRef};

/// Pointer-bearing field storage moves intact between threads; no integer pointer transport.
pub(crate) fn encode_reference(obj: ObjectRef<'_>) -> dotnet_value::storage::FieldStorage {
    use dotnet_value::layout::{
        FieldKey, FieldLayout, FieldLayoutManager, GcDesc, LayoutManager, Scalar,
    };
    let mut fields = hashbrown::HashMap::new();
    fields.insert(
        FieldKey {
            owner: dotnet_types::TypeDescription::NULL,
            name: "reference".into(),
        },
        FieldLayout {
            position: dotnet_utils::ByteOffset::new(0),
            layout: Arc::new(LayoutManager::Scalar(Scalar::ObjectRef)),
        },
    );
    let mut gc_desc = GcDesc::default();
    gc_desc.set(0);
    let storage = dotnet_value::storage::FieldStorage::new(
        Arc::new(FieldLayoutManager {
            fields,
            total_size: ObjectRef::SIZE,
            alignment: 8,
            gc_desc,
            has_ref_fields: true,
        }),
        vec![0; ObjectRef::SIZE],
    );
    storage.field_at_offset::<ObjectRef>(0).unwrap().write(obj);
    storage
}

pub(crate) fn decode_reference<'gc>(
    storage: dotnet_value::storage::FieldStorage,
    handle: dotnet_utils::gc::GCHandle<'gc>,
) -> ObjectRef<'gc> {
    let carrier = ObjectRef::new(
        handle,
        HeapStorage::Obj(Box::new(dotnet_value::object::Object {
            description: dotnet_types::TypeDescription::NULL,
            generics: Default::default(),
            instance_storage: storage,
            has_finalizer: false,
            finalizer_suppressed: false,
            _phantom: std::marker::PhantomData,
        })),
    );
    carrier
        .try_as_object(|object| {
            let field = object
                .instance_storage
                .field_at_offset::<ObjectRef>(0)
                .unwrap();
            let reference = field.read();
            // The temporary carrier must never become an additional collection root.
            field.write(ObjectRef(None));
            reference
        })
        .unwrap()
}
