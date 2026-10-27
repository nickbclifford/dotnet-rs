#[cfg(test)]
mod tests {
    use crate::{
        object::{HeapStorage, ObjectRef},
        test_helpers::with_test_gc_context,
    };

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
