#[cfg(test)]
mod tests {
    use crate::{
        layout::{ArrayLayoutManager, FieldLayoutManager, GcDesc, LayoutManager, Scalar},
        object::{HeapStorage, Object, ObjectRef, Vector},
        storage::FieldStorage,
        test_helpers::with_test_gc_context,
    };
    use dotnet_types::{
        TypeDescription,
        generics::{ConcreteType, GenericLookup},
        resolution::ResolutionS,
    };
    use dotnet_utils::sync::Arc;
    use dotnetdll::prelude::BaseType;
    use hashbrown::HashMap;

    fn field_storage() -> (FieldStorage, *const u8) {
        let mut payload = Vec::with_capacity(32);
        payload.extend_from_slice(&[0x11, 0x22, 0x33]);
        assert!(payload.capacity() > payload.len());
        let base = payload.as_ptr();
        let layout = Arc::new(FieldLayoutManager {
            fields: HashMap::new(),
            total_size: payload.len(),
            alignment: 1,
            gc_desc: GcDesc::default(),
            has_ref_fields: false,
        });
        (FieldStorage::new(layout, payload), base)
    }

    #[test]
    fn heap_storage_payload_extents_and_raw_bases() {
        with_test_gc_context(|_gc| {
            let mut array_payload = Vec::with_capacity(32);
            array_payload.extend_from_slice(&[0x44, 0x55, 0x66]);
            assert!(array_payload.capacity() > array_payload.len());
            let array_base = array_payload.as_ptr();
            let vector = Vector::new(
                ConcreteType::new(ResolutionS::NULL, BaseType::UInt8),
                ArrayLayoutManager {
                    element_layout: Arc::new(LayoutManager::Scalar(Scalar::UInt8)),
                    length: 3,
                },
                array_payload,
                vec![3, 1],
            );
            let array = HeapStorage::Vec(Box::new(vector));
            assert_eq!(array.payload_len(), 3);
            assert!(array.size_bytes() > array.payload_len());
            // SAFETY: F10.RawMemoryAccessValid — The test retains the payload allocation and only compares numeric addresses.
            let observed_array_base = unsafe { array.payload_base_ptr() };
            assert_eq!(observed_array_base.addr(), array_base.addr());

            let (instance_storage, object_base) = field_storage();
            let object = Object::new(
                TypeDescription::NULL,
                GenericLookup::default(),
                instance_storage,
            );
            let object = HeapStorage::Obj(Box::new(object));
            assert_eq!(object.payload_len(), 3);
            assert!(object.size_bytes() > object.payload_len());
            // SAFETY: F10.RawMemoryAccessValid — The test retains the payload allocation and only compares numeric addresses.
            let observed_object_base = unsafe { object.payload_base_ptr() };
            assert_eq!(observed_object_base.addr(), object_base.addr());

            let (instance_storage, boxed_base) = field_storage();
            let boxed = Object::new(
                TypeDescription::NULL,
                GenericLookup::default(),
                instance_storage,
            );
            let boxed = HeapStorage::Boxed(Box::new(boxed));
            assert_eq!(boxed.payload_len(), 3);
            assert!(boxed.size_bytes() > boxed.payload_len());
            // SAFETY: F10.RawMemoryAccessValid — The test retains the payload allocation and only compares numeric addresses.
            let observed_boxed_base = unsafe { boxed.payload_base_ptr() };
            assert_eq!(observed_boxed_base.addr(), boxed_base.addr());

            let string = crate::string::CLRString::from("abc");
            let string_base = string.as_ptr().cast::<u8>();
            let string = HeapStorage::Str(string);
            assert_eq!(string.payload_len(), 6);
            assert!(string.size_bytes() > string.payload_len());
            // SAFETY: F10.RawMemoryAccessValid — The test retains the payload allocation and only compares numeric addresses.
            let observed_string_base = unsafe { string.payload_base_ptr() };
            assert_eq!(observed_string_base.addr(), string_base.addr());
            // SAFETY: F10.RawMemoryAccessValid — The test retains the string allocation and only compares numeric addresses.
            let raw_string_base = unsafe { string.raw_data_ptr() };
            assert_eq!(raw_string_base.addr(), observed_string_base.addr());
        });
    }

    #[test]
    fn test_read_branded_null() {
        with_test_gc_context(|gc_handle| {
            let mut null_bytes = [0u8; ObjectRef::SIZE];
            ObjectRef(None).write(&mut null_bytes);
            // SAFETY: F1.GcHandleRooted — The test fixture keeps the referenced object and its backing storage alive for this operation.
            unsafe {
                let obj = ObjectRef::read_branded(&null_bytes, &gc_handle);
                assert!(obj.0.is_none());
            }
        });
    }

    #[test]
    fn test_read_valid_object() {
        with_test_gc_context(|gc_handle| {
            let storage = HeapStorage::Str(crate::string::CLRString::from("test"));
            let obj = ObjectRef::new(gc_handle, storage);
            let mut buffer = [0u8; ObjectRef::SIZE];
            obj.write(&mut buffer);
            // SAFETY: F1.GcHandleRooted — The test fixture keeps the referenced object and its backing storage alive for this operation.
            unsafe {
                let read_obj = ObjectRef::read_branded(&buffer, &gc_handle);
                assert_eq!(read_obj, obj);
            }
        });
    }

    #[test]
    fn test_storage_pointer_roundtrip() {
        with_test_gc_context(|gc_handle| {
            assert!(
                // SAFETY: F1.GcHandleRooted — A null storage pointer decodes without dereferencing.
                unsafe { ObjectRef::from_storage_ptr(std::ptr::null_mut()) }
                    .0
                    .is_none()
            );

            let storage = HeapStorage::Str(crate::string::CLRString::from("storage pointer"));
            let obj = ObjectRef::new(gc_handle, storage);
            let ptr = obj.to_storage_ptr();
            // SAFETY: F1.GcHandleRooted — `ptr` was just produced from the live rooted object above.
            let decoded = unsafe { ObjectRef::from_storage_ptr(ptr) };
            assert_eq!(decoded, obj);
        });
    }

    #[test]
    fn test_non_object_tag_decodes_to_null() {
        // The tag is checked before any dereference; this value intentionally has no provenance.
        let tagged = std::ptr::NonNull::<u8>::dangling()
            .as_ptr()
            .map_addr(|addr| addr | 1);
        // SAFETY: F1.GcHandleRooted — Odd non-object tags are ignored without dereferencing their pointer value.
        let decoded = unsafe { ObjectRef::from_storage_ptr(tagged) };
        assert!(decoded.0.is_none());
    }
}
