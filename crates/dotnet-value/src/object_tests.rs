#[cfg(test)]
mod tests {
    use crate::{
        object::{HeapStorage, ObjectRef},
        test_helpers::with_test_gc_context,
    };

    #[test]
    fn test_read_branded_null() {
        with_test_gc_context(|gc_handle| {
            let null_bytes = 0usize.to_ne_bytes();
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
            unsafe {
                let read_obj = ObjectRef::read_branded(&buffer, &gc_handle);
                assert_eq!(read_obj, obj);
            }
        });
    }
}
