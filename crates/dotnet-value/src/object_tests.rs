#[cfg(test)]
mod tests {
    use crate::object::ObjectRef;
    use gc_arena::{Arena, Rootable};
    
    #[test]
    fn test_read_branded_null() {
        type TestRoot = Rootable![()];
        
        let arena = Arena::<TestRoot>::new(|_mc| ());
        
        arena.mutate(|gc, _root| {
            let null_bytes = 0usize.to_ne_bytes();
            unsafe {
                let obj = ObjectRef::read_branded(&null_bytes, &gc);
                assert!(obj.0.is_none());
            }
        });
    }
}
