#[cfg(test)]
mod tests {
    use crate::object::{HeapStorage, ObjectRef, ValueType};
    use gc_arena::{Arena, Rootable};
    #[test]
    fn test_read_branded_null() {
        type TestRoot = Rootable![()];
        let arena = Arena::<TestRoot>::new(|_mc| ());
        arena.mutate(|gc, _root| {
            let null_bytes = 0usize.to_ne_bytes();
            unsafe {
                let obj = ObjectRef::read_branded(&null_bytes, gc);
                assert!(obj.0.is_none());
            }
        });
    }
    #[test]
    fn test_read_valid_object() {
        type TestRoot = Rootable![()];
        let arena = Arena::<TestRoot>::new(|_mc| ());
        arena.mutate(|gc, _root| {
            let storage = HeapStorage::Boxed(ValueType::Int32(42));
            let obj = ObjectRef::new(gc, storage);
            let mut buffer = [0u8; std::mem::size_of::<usize>()];
            obj.write(&mut buffer);
            unsafe {
                let read_obj = ObjectRef::read_branded(&buffer, gc);
                assert!(read_obj == obj);
            }
        });
    }
}
