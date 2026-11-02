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

    // Independent recreation (not recovered originals): compact ObjectRef codec regressions.
    #[cfg(feature = "multithreading")]
    struct RecreationRegistration(crate::ArenaId);

    #[cfg(feature = "multithreading")]
    impl RecreationRegistration {
        fn new(id: u64) -> Self {
            let id = crate::ArenaId::new(id);
            dotnet_utils::gc::register_arena(
                id,
                dotnet_utils::sync::Arc::new(dotnet_utils::sync::AtomicBool::new(false)),
            );
            Self(id)
        }
    }

    #[cfg(feature = "multithreading")]
    impl Drop for RecreationRegistration {
        fn drop(&mut self) {
            dotnet_utils::gc::unregister_arena(self.0);
        }
    }

    fn recreation_object<'gc>(mc: &gc_arena::Mutation<'gc>, id: crate::ArenaId) -> ObjectRef<'gc> {
        let inner = crate::object::ObjectInner::new(
            HeapStorage::Str(crate::string::CLRString::from("codec recreation")),
            id,
        );
        ObjectRef(Some(gc_arena::Gc::new(
            mc,
            dotnet_utils::gc::ThreadSafeLock::new(inner),
        )))
    }

    fn recreation_check_payload(obj: ObjectRef<'_>) {
        let handle = obj.0.expect("live object decoded as null");
        let inner = handle.borrow();
        let HeapStorage::Str(value) = &inner.storage else {
            panic!("wrong payload")
        };
        assert_eq!(value.as_string(), "codec recreation");
    }

    /// Independent recreation (not recovered originals): registered owner 40000.
    #[cfg(feature = "multithreading")]
    #[test]
    fn recreation_codec_id40000() {
        let _registration = RecreationRegistration::new(40000);
        let arena = gc_arena::Arena::<gc_arena::Rootable![()]>::new(|_| ());
        arena.mutate(|mc, _| {
            let obj = recreation_object(mc, crate::ArenaId::new(40000));
            let ptr = obj.to_storage_ptr();
            assert_eq!(ptr.addr() & 7, 5);
            // SAFETY: F1.GcHandleRooted — The live allocation and its registered arena remain
            // in this mutation; the pointer comes directly from that allocation's codec.
            recreation_check_payload(unsafe { ObjectRef::from_storage_ptr(ptr) });
        });
    }

    /// Independent recreation (not recovered originals): registered compact owner boundaries.
    #[cfg(feature = "multithreading")]
    #[test]
    fn recreation_codec_registered_boundaries() {
        let arena = gc_arena::Arena::<gc_arena::Rootable![()]>::new(|_| ());
        for id in [0, 1, 32767, 32768, 40000, 65535] {
            let _registration = RecreationRegistration::new(id);
            arena.mutate(|mc, _| {
                let obj = recreation_object(mc, crate::ArenaId::new(id));
                let ptr = obj.to_storage_ptr();
                assert_eq!((ptr.addr() >> 48) as u16, id as u16);
                // SAFETY: F1.GcHandleRooted — The allocation is live and its matching owner
                // remains registered throughout this mutation; ptr is its own encoded pointer.
                recreation_check_payload(unsafe { ObjectRef::from_storage_ptr(ptr) });
            });
        }
    }

    /// Independent recreation (not recovered originals): unrepresentable owners must fail loudly.
    #[cfg(feature = "multithreading")]
    #[test]
    fn recreation_codec_reject_large_owners() {
        let arena = gc_arena::Arena::<gc_arena::Rootable![()]>::new(|_| ());
        let mut accepted = Vec::new();
        for id in [65536, u64::MAX - 1] {
            let _registration = RecreationRegistration::new(id);
            arena.mutate(|mc, _| {
                let obj = recreation_object(mc, crate::ArenaId::new(id));
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    obj.to_storage_ptr()
                })) {
                    Ok(_) => accepted.push(id),
                    Err(error) => {
                        let message = error
                            .downcast_ref::<String>()
                            .expect("host invariant panic message");
                        assert!(message.contains(
                            "ObjectRef codec host invariant violation: UnrepresentableOwner"
                        ));
                        assert!(message.contains(&id.to_string()));
                    }
                }
            });
        }
        assert!(
            accepted.is_empty(),
            "unrepresentable owners accepted: {accepted:?}"
        );
    }

    /// Independent recreation (not recovered originals): absent owner is a host invariant failure.
    #[cfg(feature = "multithreading")]
    #[test]
    fn recreation_codec_unavailable_owner() {
        let id = crate::ArenaId::new(43210);
        let registration = RecreationRegistration::new(id.as_u64());
        let arena = gc_arena::Arena::<gc_arena::Rootable![()]>::new(|_| ());
        arena.mutate(|mc, _| {
            let obj = recreation_object(mc, id);
            let ptr = obj.to_storage_ptr();
            drop(registration);
            let encode =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| obj.to_storage_ptr()));
            // SAFETY: F1.GcHandleRooted; F6.NoEscapeAcrossArena — The allocation remains live
            // in this mutation. The absent owner must be diagnosed before any pointer read.
            let decode = std::panic::catch_unwind(|| unsafe { ObjectRef::from_storage_ptr(ptr) });
            for error in [
                encode.expect_err("encode accepted absent owner"),
                decode.expect_err("decode accepted absent owner"),
            ] {
                let message = error
                    .downcast_ref::<String>()
                    .expect("host invariant panic message");
                assert!(
                    message.contains("ObjectRef codec host invariant violation: OwnerUnavailable")
                );
                assert!(message.contains("43210"));
            }
        });
    }

    /// Independent recreation (not recovered originals): INVALID stays local and untagged.
    #[test]
    fn recreation_codec_local_invalid_control() {
        let arena = gc_arena::Arena::<gc_arena::Rootable![()]>::new(|_| ());
        arena.mutate(|mc, _| {
            let obj = recreation_object(mc, crate::ArenaId::INVALID);
            let ptr = obj.to_storage_ptr();
            assert_eq!(ptr.addr() & 7, 0);
            // SAFETY: F1.GcHandleRooted — The untagged pointer was obtained from the live
            // allocation in this mutation and the arena is retained until the callback returns.
            recreation_check_payload(unsafe { ObjectRef::from_storage_ptr(ptr) });
        });
    }

    // Independent recreation (not recovered originals): a gc-arena publication model,
    // not the VM MarkObjects path. The raw slot models its manually traced root set.
    struct RecreationPublicationRoot(std::cell::Cell<Option<*const RecreationWitness>>);

    #[derive(gc_arena::Collect)]
    #[collect(require_static)]
    struct RecreationWitness(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    impl Drop for RecreationWitness {
        fn drop(&mut self) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    // SAFETY: F1.GcHandleRooted; F6.NoEscapeAcrossArena — The slot is populated only
    // with this arena's still-live allocation before sweeping. A missed publication is
    // cleared immediately after the first sweep, before any further root tracing; the
    // temporary rebranded handle never leaves trace. No collected pointer is accessed.
    unsafe impl<'gc> gc_arena::Collect<'gc> for RecreationPublicationRoot {
        fn trace<T: gc_arena::collect::Trace<'gc>>(&self, cc: &mut T) {
            if let Some(ptr) = self.0.get() {
                // SAFETY: F1.GcHandleRooted — While this slot can be traced, the
                // allocation belongs to this arena and has not been swept.
                unsafe { gc_arena::Gc::from_ptr(ptr) }.trace(cc);
            }
        }
    }

    fn recreation_publication_drops(barrier: bool) -> usize {
        use std::sync::atomic::Ordering;
        let drops = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut arena =
            gc_arena::Arena::<gc_arena::Rootable![RecreationPublicationRoot]>::new(|_| {
                RecreationPublicationRoot(std::cell::Cell::new(None))
            });
        // Allocate before marking, with no hidden root and no intervening collection.
        let ptr = arena.mutate(|mc, _| {
            gc_arena::Gc::as_ptr(gc_arena::Gc::new(mc, RecreationWitness(drops.clone())))
        });
        assert!(arena.finish_marking().is_some());
        if barrier {
            arena.mutate_root(|_, root| root.0.set(Some(ptr)));
        } else {
            arena.mutate(|_, root| root.0.set(Some(ptr)));
        }
        assert!(arena.finish_marking().is_some());
        arena.finish_cycle();
        let observed = drops.load(Ordering::SeqCst);
        // In the old path the slot may now be dangling; remove it without reading it
        // or tracing the root again. The barrier control instead checks another cycle.
        if barrier && observed == 0 {
            arena.finish_cycle();
            assert_eq!(drops.load(Ordering::SeqCst), 0);
        }
        arena.mutate_root(|_, root| root.0.set(None));
        observed
    }

    /// Independent recreation (not recovered originals): old publication loses the witness.
    #[test]
    fn recreation_publication_old_path_diagnostic() {
        assert_eq!(recreation_publication_drops(false), 1);
    }

    /// Independent recreation (not recovered originals): a root barrier preserves the witness.
    #[test]
    fn recreation_publication_root_barrier_control() {
        assert_eq!(recreation_publication_drops(true), 0);
    }

    /// Independent recreation (not recovered originals): publication through the root barrier survives.
    #[test]
    fn recreation_publication_pending_survival() {
        assert_eq!(
            recreation_publication_drops(true),
            0,
            "published witness was collected"
        );
    }

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
