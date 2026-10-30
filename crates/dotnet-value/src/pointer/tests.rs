use crate::{
    layout::{ArrayLayoutManager, FieldLayoutManager, GcDesc, LayoutManager, Scalar},
    object::{HeapStorage, Object, ObjectRef, Vector},
    pointer::*,
    ptr_common::PointerLike,
    storage::FieldStorage,
    test_helpers::with_test_gc_context,
};
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    resolution::ResolutionS,
};
use dotnetdll::prelude::BaseType;
use gc_arena::{Arena, Gc, Rootable};
use hashbrown::HashMap;
use std::{
    cell::Cell,
    ptr::NonNull,
    sync::{Mutex, OnceLock},
};

use dotnet_utils::sync::Arc;

#[derive(Copy, Clone)]
struct TestManagedPtrResolver {
    stack_base: Option<NonNull<u8>>,
    static_base: Option<NonNull<u8>>,
}

impl<'gc> ManagedPtrResolver<'gc> for TestManagedPtrResolver {
    fn stack_slot_base(&self, _slot: StackSlotIndex) -> Option<NonNull<u8>> {
        self.stack_base
    }

    fn static_storage_base(&self, _metadata: &StaticMetadata) -> Option<NonNull<u8>> {
        self.static_base
    }
}

const NO_MANAGED_BASES: TestManagedPtrResolver = TestManagedPtrResolver {
    stack_base: None,
    static_base: None,
};

#[derive(Default)]
struct CountingHeapCache<'gc> {
    entries: hashbrown::HashMap<usize, ObjectRef<'gc>>,
    hits: usize,
    misses: usize,
}

impl<'gc> HeapManagedPtrDecodeCache<'gc> for CountingHeapCache<'gc> {
    fn get_heap_handle(&mut self, serialized_handle: usize) -> Option<ObjectRef<'gc>> {
        match self.entries.get(&serialized_handle).copied() {
            Some(owner) => {
                self.hits += 1;
                Some(owner)
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    fn insert_heap_handle(&mut self, serialized_handle: usize, owner: ObjectRef<'gc>) {
        self.entries.insert(serialized_handle, owner);
    }
}

fn static_reg_test_lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

fn managed_ptr_to_heap_object_start<'gc>(obj: ObjectRef<'gc>) -> ManagedPtr<'gc> {
    let ptr = obj.with_data(|d| NonNull::new(d.as_ptr().cast_mut()));
    ManagedPtr::new(
        ptr,
        TypeDescription::NULL,
        Some(obj),
        false,
        Some(ByteOffset::new(0)),
    )
}

fn m1d_transient_object() -> (Object<'static>, Arc<FieldLayoutManager>) {
    let layout = Arc::new(FieldLayoutManager {
        fields: HashMap::new(),
        total_size: 1,
        alignment: 1,
        gc_desc: GcDesc::default(),
        has_ref_fields: false,
    });
    let probe = Arc::clone(&layout);
    let storage = FieldStorage::new(layout, vec![0xA5]);
    (
        Object::new(TypeDescription::NULL, GenericLookup::default(), storage),
        probe,
    )
}

#[test]
fn m1d_transient_native_int_conversion_is_rejected() {
    let (obj, _probe) = m1d_transient_object();
    let base = obj.with_data(|data| NonNull::new(data.as_ptr().cast_mut()).unwrap());
    let transient =
        ManagedPtr::new_transient(Some(base), TypeDescription::NULL, obj, ByteOffset::new(0));
    assert_eq!(transient.observe_address(), Some(base));
    assert_eq!(
        transient.try_observe_address(),
        Err(ManagedPtrOwnershipError::TransientEscapes)
    );
    let (obj, _probe) = m1d_transient_object();
    let null_transient =
        ManagedPtr::new_transient(None, TypeDescription::NULL, obj, ByteOffset::new(0));
    assert_eq!(
        null_transient.try_observe_address(),
        Err(ManagedPtrOwnershipError::TransientEscapes)
    );

    let mut bytes = [0xA5u8; 2];
    let base = NonNull::new(bytes.as_mut_ptr()).unwrap();
    let unmanaged = ManagedPtr::new(Some(base), TypeDescription::NULL, None, false, None);
    let null = ManagedPtr::new(None, TypeDescription::NULL, None, false, None);
    let static_ptr = ManagedPtr::new_static(
        Some(base),
        TypeDescription::NULL,
        TypeDescription::NULL,
        GenericLookup::default(),
        false,
        ByteOffset::new(0),
    );
    let stack = static_ptr.with_origin(PointerOrigin::Stack(StackSlotIndex::new(1)));
    for ptr in [&unmanaged, &null, &stack, &static_ptr] {
        assert_eq!(ptr.try_observe_address(), Ok(ptr.observe_address()));
    }
    assert_eq!(unmanaged.try_observe_address(), Ok(Some(base)));
    assert_eq!(null.try_observe_address(), Ok(None));
    assert_eq!(stack.try_observe_address(), Ok(Some(base)));
    assert_eq!(static_ptr.try_observe_address(), Ok(Some(base)));

    with_test_gc_context(|gc_handle| {
        let vector = Vector::new(
            ConcreteType::new(ResolutionS::NULL, BaseType::UInt8),
            ArrayLayoutManager {
                element_layout: Arc::new(LayoutManager::Scalar(Scalar::UInt8)),
                length: 2,
            },
            vec![0xA5; 2],
            vec![2],
        );
        let owner = ObjectRef::new(gc_handle, HeapStorage::Vec(Box::new(vector)));
        let heap = ManagedPtr::new(
            None,
            TypeDescription::NULL,
            Some(owner),
            false,
            Some(ByteOffset::new(1)),
        );
        let expected =
            owner.with_data(|data| NonNull::new(data.as_ptr().wrapping_add(1).cast_mut()));
        assert_eq!(heap.try_observe_address(), Ok(expected));
        assert_eq!(heap.try_observe_address(), Ok(heap.observe_address()));
    });
}

#[test]
fn m1d_cli_to_cts_rejects_transient_integer_escape() {
    use crate::{
        StackValue,
        cts_cli_conversion::{CliToCts, CtsScalarKind},
    };
    use dotnet_types::error::TypeResolutionError;

    for null in [false, true] {
        let (obj, _probe) = m1d_transient_object();
        let base = obj.with_data(|data| NonNull::new(data.as_ptr().cast_mut()));
        let managed = ManagedPtr::new_transient(
            if null { None } else { base },
            TypeDescription::NULL,
            obj,
            ByteOffset::new(0),
        );
        let value = || StackValue::ManagedPtr(managed.clone().into());
        assert!(matches!(
            CliToCts::convert_u32(value()),
            Err(TypeResolutionError::InvalidLayout(_))
        ));
        assert!(matches!(
            CliToCts::convert_num::<usize>(value()),
            Err(TypeResolutionError::InvalidLayout(_))
        ));
        for kind in [
            CtsScalarKind::UInt32,
            CtsScalarKind::NativeInt,
            CtsScalarKind::NativeUInt,
        ] {
            assert!(matches!(
                CliToCts::narrow_scalar_value(value(), kind),
                Err(TypeResolutionError::InvalidLayout(_))
            ));
        }
    }
    let mut byte = 0xA5u8;
    let base = NonNull::from(&mut byte);
    let unmanaged = ManagedPtr::new(Some(base), TypeDescription::NULL, None, false, None);
    assert_eq!(
        CliToCts::convert_u32(StackValue::ManagedPtr(unmanaged.clone().into())),
        Ok(base.as_ptr().addr() as u32)
    );
    assert_eq!(
        CliToCts::convert_num::<usize>(StackValue::ManagedPtr(unmanaged.into())),
        Ok(base.as_ptr().addr())
    );
    with_test_gc_context(|gc| {
        let vector = Vector::new(
            ConcreteType::new(ResolutionS::NULL, BaseType::UInt8),
            ArrayLayoutManager {
                element_layout: Arc::new(LayoutManager::Scalar(Scalar::UInt8)),
                length: 1,
            },
            vec![0xA5],
            vec![1],
        );
        let owner = ObjectRef::new(gc, HeapStorage::Vec(Box::new(vector)));
        let heap = managed_ptr_to_heap_object_start(owner);
        let expected = heap.observe_address().unwrap().as_ptr().addr();
        assert_eq!(
            CliToCts::convert_num::<usize>(StackValue::ManagedPtr(heap.into())),
            Ok(expected)
        );
    });
}

#[test]
fn m1d_retained_ptr_keeps_transient_owner_alive() {
    let (obj, probe) = m1d_transient_object();
    let base = obj.with_data(|data| NonNull::new(data.as_ptr().cast_mut()).unwrap());
    let managed =
        ManagedPtr::new_transient(Some(base), TypeDescription::NULL, obj, ByteOffset::new(0));
    let value = crate::StackValue::ManagedPtr(managed.into());
    assert_eq!(Arc::strong_count(&probe), 2);
    let retained = RetainedPtr::try_from_stack_value(value).unwrap();
    assert_eq!(Arc::strong_count(&probe), 2);
    assert!(!retained.is_null());
    assert_eq!(retained.addr(), base.as_ptr().addr());
    // SAFETY: F10.RawMemoryAccessValid — The retained transient owner holds the
    // initialized one-byte storage, and the read is in range while it is alive.
    assert_eq!(unsafe { retained.as_ptr().read() }, 0xA5);
    drop(retained);
    assert_eq!(Arc::strong_count(&probe), 1);
}

#[test]
fn m1d_retained_ptr_heap_origin_reads_through_owner() {
    with_test_gc_context(|gc_handle| {
        let vector = Vector::new(
            ConcreteType::new(ResolutionS::NULL, BaseType::UInt8),
            ArrayLayoutManager {
                element_layout: Arc::new(LayoutManager::Scalar(Scalar::UInt8)),
                length: 1,
            },
            vec![0xA5],
            vec![1],
        );
        let obj = ObjectRef::new(gc_handle, HeapStorage::Vec(Box::new(vector)));
        let value = crate::StackValue::ManagedPtr(managed_ptr_to_heap_object_start(obj).into());
        let retained = RetainedPtr::try_from_stack_value(value).unwrap();
        assert!(!retained.is_null());
        // SAFETY: F1.GcHandleRooted — The arena mutation keeps the object live
        // with no collection or competing access, and the read is within its
        // initialized one-byte payload. The retained value itself is not a GC root.
        assert_eq!(unsafe { retained.as_ptr().read() }, 0xA5);
        drop(retained);
    });
}

#[test]
fn m1d_retained_ptr_rejects_non_pointer_values() {
    use crate::StackValue;
    use dotnet_types::error::ExecutionError;

    let (obj, _probe) = m1d_transient_object();
    for (value, kind) in [
        (StackValue::Int32(7), "Int32"),
        (StackValue::Int64(7), "Int64"),
        (StackValue::NativeFloat(1.0), "NativeFloat"),
        (StackValue::ObjectRef(ObjectRef(None)), "ObjectRef"),
        (StackValue::ValueType(obj), "ValueType"),
        (StackValue::UninitializedTypedRef, "UninitializedTypedRef"),
    ] {
        assert_eq!(
            RetainedPtr::try_from_stack_value(value).unwrap_err(),
            ExecutionError::TypeMismatch {
                expected: "pointer",
                actual: kind.into(),
            }
        );
    }
}

#[test]
fn m1d_retained_ptr_null() {
    use crate::{StackValue, TypedReferenceSlot};

    let null_managed = ManagedPtr::new(None, TypeDescription::NULL, None, false, None);
    let values = [
        StackValue::NativeInt(0),
        StackValue::ManagedPtr(null_managed.clone().into()),
        StackValue::TypedRef(TypedReferenceSlot::new(
            null_managed.into(),
            Arc::new(TypeDescription::NULL),
        )),
    ];
    for value in values {
        let retained = RetainedPtr::try_from_stack_value(value).unwrap();
        assert!(retained.is_null());
        assert!(retained.as_ptr().is_null());
        assert_eq!(retained.addr(), 0);
    }

    let mut byte = 0xA5u8;
    let raw = &raw mut byte;
    let retained = RetainedPtr::try_from_stack_value(StackValue::unmanaged_ptr(raw)).unwrap();
    assert!(!retained.is_null());
    assert_eq!(retained.as_ptr(), raw);
    assert_eq!(retained.addr(), raw.addr());
}

#[test]
fn m1d_normalize_retains_transient_owner() {
    let (obj, probe) = m1d_transient_object();
    assert_eq!(Arc::strong_count(&probe), 2);
    let normalized = PointerOrigin::new_transient(obj).normalize();
    assert_eq!(
        Arc::strong_count(&probe),
        2,
        "normalized origin: {normalized:?} (discriminant {})",
        normalized.discriminant()
    );
    assert!(matches!(normalized, PointerOrigin::Transient(_)));
}

#[test]
fn m1d_normalize_transient_access_after_consume() {
    let (obj, _probe) = m1d_transient_object();
    let base = obj.with_data(|data| data.as_ptr());
    let normalized = PointerOrigin::new_transient(obj).normalize();
    // SAFETY: F10.RawMemoryAccessValid — normalized retains the one-byte
    // initialized storage through this read.
    let byte = unsafe { base.read() };
    assert_eq!(byte, 0xA5);
    drop(normalized);
}

#[test]
fn m1d_from_info_full_retains_transient_owner() {
    let (obj, probe) = m1d_transient_object();
    let base = obj.with_data(|data| NonNull::new(data.as_ptr().cast_mut()).unwrap());
    let info = ManagedPtrInfo {
        address: Some(base),
        origin: PointerOrigin::new_transient(obj),
        offset: ByteOffset::new(0),
    };
    assert_eq!(Arc::strong_count(&probe), 2);
    let m = ManagedPtr::from_info_full(info, TypeDescription::NULL, false);
    assert_eq!(
        Arc::strong_count(&probe),
        2,
        "reconstructed origin: {:?} (discriminant {})",
        m.origin(),
        m.origin().discriminant()
    );
    assert!(matches!(m.origin(), PointerOrigin::Transient(_)));
}

#[test]
fn m1d_from_info_full_transient_access_after_info_consumed() {
    let (obj, _probe) = m1d_transient_object();
    let base = obj.with_data(|data| NonNull::new(data.as_ptr().cast_mut()).unwrap());
    let info = ManagedPtrInfo {
        address: Some(base),
        origin: PointerOrigin::new_transient(obj),
        offset: ByteOffset::new(0),
    };
    let m = ManagedPtr::from_info_full(info, TypeDescription::NULL, false);
    // SAFETY: F10.RawMemoryAccessValid — m retains the initialized one-byte
    // storage throughout both reads, without competing access.
    let byte = unsafe { m.with_data(1, |data| data[0]) };
    assert_eq!(byte, 0xA5);
    assert_eq!(m.pointer(), Some(base));
    // SAFETY: F10.RawMemoryAccessValid — The cached base belongs to the same
    // live transient owner and points to its initialized byte.
    assert_eq!(unsafe { m.pointer().unwrap().as_ptr().read() }, 0xA5);
}

#[test]
fn m1d_transient_managed_ptr_clone_aliases_owner_storage() {
    let (obj, probe) = m1d_transient_object();
    let base = obj.with_data(|data| NonNull::new(data.as_ptr().cast_mut()).unwrap());
    let original =
        ManagedPtr::new_transient(Some(base), TypeDescription::NULL, obj, ByteOffset::new(0));
    let cloned = original.clone();
    // SAFETY: F10.RawMemoryAccessValid — Both live transient owners contain
    // initialized storage; these zero-length slices are only used to observe addresses.
    let original_addr = unsafe { original.with_data(0, |data| data.as_ptr().addr()) };
    // SAFETY: F10.RawMemoryAccessValid — The cloned owner is alive and the
    // zero-length slice is only used to observe its address.
    let cloned_addr = unsafe { cloned.with_data(0, |data| data.as_ptr().addr()) };
    assert_eq!(original_addr, cloned_addr);
    assert_eq!(original_addr, original.pointer().unwrap().as_ptr().addr());
    assert_eq!(cloned_addr, cloned.pointer().unwrap().as_ptr().addr());
    drop(original);
    assert_eq!(Arc::strong_count(&probe), 2);
    // SAFETY: F10.RawMemoryAccessValid — The clone retains the shared initialized
    // one-byte owner through both reads, with no competing access.
    assert_eq!(unsafe { cloned.with_data(1, |data| data[0]) }, 0xA5);
    // SAFETY: F10.RawMemoryAccessValid — The cached pointer addresses that same
    // live one-byte owner, which the clone retains until after this read.
    assert_eq!(unsafe { cloned.pointer().unwrap().as_ptr().read() }, 0xA5);
    drop(cloned);
    assert_eq!(Arc::strong_count(&probe), 1);
}

#[test]
fn m1a_one_byte_heap_read() {
    with_test_gc_context(|gc_handle| {
        let vector = Vector::new(
            ConcreteType::new(ResolutionS::NULL, BaseType::UInt8),
            ArrayLayoutManager {
                element_layout: Arc::new(LayoutManager::Scalar(Scalar::UInt8)),
                length: 1,
            },
            vec![0xA5],
            vec![1],
        );
        let obj = ObjectRef::new(gc_handle, HeapStorage::Vec(Box::new(vector)));
        let ptr = ManagedPtr::new(
            obj.pointer(),
            TypeDescription::NULL,
            Some(obj),
            false,
            Some(ByteOffset::new(0)),
        );

        // SAFETY: F1.GcHandleRooted — `obj` keeps the one-byte vector alive for this read;
        // `with_data` validates the exact one-byte payload window before forming the slice.
        unsafe {
            ptr.with_data(1, |data| {
                assert_eq!(data.len(), 1);
                assert_eq!(data[0], 0xA5);
            });
        }
    });
}

#[test]
fn m1a_heap_pointer_survives_typed_write() {
    with_test_gc_context(|gc_handle| {
        // Exercise both direct and decoded bases, with and without an exposed
        // integer round trip; Miri's exposed-provenance mode has known limitations.
        for (kind, decode, reconstruct) in (0..3).flat_map(|kind| {
            [false, true].into_iter().flat_map(move |decode| {
                [false, true].map(move |reconstruct| (kind, decode, reconstruct))
            })
        }) {
            let storage = if kind == 0 {
                HeapStorage::Vec(Box::new(Vector::new(
                    ConcreteType::new(ResolutionS::NULL, BaseType::UInt8),
                    ArrayLayoutManager {
                        element_layout: Arc::new(LayoutManager::Scalar(Scalar::UInt8)),
                        length: 1,
                    },
                    vec![0],
                    vec![1],
                )))
            } else {
                let fields = FieldStorage::new(
                    Arc::new(FieldLayoutManager {
                        fields: HashMap::new(),
                        total_size: 1,
                        alignment: 1,
                        gc_desc: GcDesc::default(),
                        has_ref_fields: false,
                    }),
                    vec![0],
                );
                let object = Object::new(TypeDescription::NULL, GenericLookup::default(), fields);
                if kind == 1 {
                    HeapStorage::Obj(Box::new(object))
                } else {
                    HeapStorage::Boxed(Box::new(object))
                }
            };
            let owner = ObjectRef::new(gc_handle, storage);
            let managed = ManagedPtr::new(
                None,
                TypeDescription::NULL,
                Some(owner),
                false,
                Some(ByteOffset::new(0)),
            );
            let base = managed.observe_address().expect("heap base");
            let observed = if decode {
                let serializable = ManagedPtr::new(
                    Some(base),
                    TypeDescription::NULL,
                    Some(owner),
                    false,
                    Some(ByteOffset::new(0)),
                );
                let mut buffer = ManagedPtr::serialization_buffer();
                serializable.write(&mut buffer);
                // SAFETY: F1.GcHandleRooted — the complete encoding refers to the
                // live owner retained throughout this test.
                unsafe { ManagedPtr::read_resolved_unchecked(&buffer, &NO_MANAGED_BASES) }
                    .expect("heap decode")
                    .address
                    .expect("decoded heap base")
                    .as_ptr()
            } else {
                base.as_ptr()
            };
            let address = reconstruct.then(|| observed.expose_provenance());
            owner.with_data_mut(gc_handle, |data| data[0] = 5);
            let raw = if let Some(address) = address {
                // SAFETY: F10.RawMemoryAccessValid — the owner retains the exposed,
                // one-byte allocation for the synchronized write below.
                unsafe { unmanaged_ptr_from_addr(address) }
            } else {
                observed
            };
            // SAFETY: F10.RawMemoryAccessValid — the owner remains live, the one-byte
            // allocation is stable, and no guarded access overlaps this raw write.
            unsafe { raw.write(7) };
            assert_eq!(owner.with_data(|data| data[0]), 7);
        }
    });
}

#[test]
fn heap_read_windows_are_exact_for_array_object_and_boxed_payloads() {
    with_test_gc_context(|gc_handle| {
        #[derive(Copy, Clone)]
        enum StorageKind {
            Array,
            Object,
            Boxed,
        }

        for kind in [StorageKind::Array, StorageKind::Object, StorageKind::Boxed] {
            for len in [0usize, 1, 4] {
                let expected: Vec<u8> = (0..len).map(|i| 0x30 + i as u8).collect();
                let storage = match kind {
                    StorageKind::Array => HeapStorage::Vec(Box::new(Vector::new(
                        ConcreteType::new(ResolutionS::NULL, BaseType::UInt8),
                        ArrayLayoutManager {
                            element_layout: Arc::new(LayoutManager::Scalar(Scalar::UInt8)),
                            length: len,
                        },
                        expected.clone(),
                        vec![len, 1],
                    ))),
                    StorageKind::Object | StorageKind::Boxed => {
                        let fields = FieldStorage::new(
                            Arc::new(FieldLayoutManager {
                                fields: HashMap::new(),
                                total_size: len,
                                alignment: 1,
                                gc_desc: GcDesc::default(),
                                has_ref_fields: false,
                            }),
                            expected.clone(),
                        );
                        let object =
                            Object::new(TypeDescription::NULL, GenericLookup::default(), fields);
                        match kind {
                            StorageKind::Object => HeapStorage::Obj(Box::new(object)),
                            StorageKind::Boxed => HeapStorage::Boxed(Box::new(object)),
                            StorageKind::Array => unreachable!(),
                        }
                    }
                };
                let owner = ObjectRef::new(gc_handle, storage);
                let base = owner.pointer().expect("heap payload base");

                let make_ptr = |offset| {
                    ManagedPtr::new(
                        Some(base),
                        TypeDescription::NULL,
                        Some(owner),
                        false,
                        Some(ByteOffset::new(offset)),
                    )
                };
                let start = make_ptr(0);
                // SAFETY: F1.GcHandleRooted — `owner` retains the exact payload and
                // `try_with_data` validates the requested length before constructing a slice.
                let whole = unsafe { start.try_with_data(len, |data| data.to_vec()) };
                assert_eq!(whole, Some(expected.clone()));
                // P/Invoke's explicit short-read contract returns only the actual remaining bytes.
                // SAFETY: F1.GcHandleRooted — `owner` retains this payload and the helper validates
                // the offset before clamping the requested size.
                let clamped = unsafe { start.with_data_up_to(len + 1, |data| data.to_vec()) };
                assert_eq!(clamped, expected);

                let end = make_ptr(len);
                // SAFETY: F1.GcHandleRooted — the empty range at the payload end is valid.
                let end_read = unsafe { end.try_with_data(0, |data| data.len()) };
                assert_eq!(end_read, Some(0));

                let past_end = make_ptr(len + 1);
                // SAFETY: F1.GcHandleRooted — this asserts rejection before the callback or
                // any payload reference is constructed.
                let past_end_read = unsafe { past_end.try_with_data(0, |data| data.len()) };
                assert_eq!(past_end_read, None);

                let overflow = make_ptr(1);
                // SAFETY: F1.GcHandleRooted — checked offset-plus-size overflow must reject.
                let overflow_read =
                    unsafe { overflow.try_with_data(usize::MAX, |data| data.len()) };
                assert_eq!(overflow_read, None);
            }
        }
    });
}

#[test]
#[should_panic(expected = "ManagedPtr::with_data: heap range overflow or out of bounds")]
fn heap_with_data_rejects_short_range() {
    with_test_gc_context(|gc_handle| {
        let vector = Vector::new(
            ConcreteType::new(ResolutionS::NULL, BaseType::UInt8),
            ArrayLayoutManager {
                element_layout: Arc::new(LayoutManager::Scalar(Scalar::UInt8)),
                length: 1,
            },
            vec![0xA5],
            vec![1],
        );
        let owner = ObjectRef::new(gc_handle, HeapStorage::Vec(Box::new(vector)));
        let ptr = ManagedPtr::new(
            owner.pointer(),
            TypeDescription::NULL,
            Some(owner),
            false,
            Some(ByteOffset::new(0)),
        );

        // SAFETY: F1.GcHandleRooted — `owner` retains a one-byte payload; the
        // requested two-byte read must be rejected before invoking the callback.
        unsafe { ptr.with_data(2, |_| unreachable!("short reads must not succeed")) };
    });
}

#[cfg(feature = "multithreading")]
#[test]
fn cross_arena_heap_read_uses_exact_payload_range() {
    with_test_gc_context(|gc_handle| {
        let vector = Vector::new(
            ConcreteType::new(ResolutionS::NULL, BaseType::UInt8),
            ArrayLayoutManager {
                element_layout: Arc::new(LayoutManager::Scalar(Scalar::UInt8)),
                length: 1,
            },
            vec![0xA5],
            vec![1],
        );
        let owner = ObjectRef::new(gc_handle, HeapStorage::Vec(Box::new(vector)));
        let base = owner.pointer().expect("heap payload base");
        let (object_ptr, arena_id) = owner.as_ptr_info().expect("cross-arena pointer info");
        let ptr = ManagedPtr::new_cross_arena(
            Some(base),
            TypeDescription::NULL,
            object_ptr,
            arena_id,
            ByteOffset::new(0),
        );

        // SAFETY: F1.GcHandleRooted — `owner` retains the one-byte payload while the
        // cross-arena read validates its range under the object and field read guards.
        let byte = unsafe { ptr.with_data(1, |data| data[0]) };
        assert_eq!(byte, 0xA5);
        // SAFETY: F1.GcHandleRooted — the exact non-panicking API rejects this overlong range.
        let invalid = unsafe { ptr.try_with_data(2, |data| data.len()) };
        assert_eq!(invalid, None);
    });
}

#[test]
fn heap_address_observation_wraps_without_granting_read_access() {
    with_test_gc_context(|gc_handle| {
        let fields = || {
            FieldStorage::new(
                Arc::new(FieldLayoutManager {
                    fields: HashMap::new(),
                    total_size: 1,
                    alignment: 1,
                    gc_desc: GcDesc::default(),
                    has_ref_fields: false,
                }),
                vec![0xA5],
            )
        };
        let object = || Object::new(TypeDescription::NULL, GenericLookup::default(), fields());
        let storage = [
            HeapStorage::Vec(Box::new(Vector::new(
                ConcreteType::new(ResolutionS::NULL, BaseType::UInt8),
                ArrayLayoutManager {
                    element_layout: Arc::new(LayoutManager::Scalar(Scalar::UInt8)),
                    length: 1,
                },
                vec![0xA5],
                vec![1],
            ))),
            HeapStorage::Obj(Box::new(object())),
            HeapStorage::Boxed(Box::new(object())),
            HeapStorage::Str(crate::string::CLRString::from("x")),
        ];
        for storage in storage {
            let owner = ObjectRef::new(gc_handle, storage);
            let (base, len) = owner
                .as_heap_storage(|storage| (storage.observe_payload_base(), storage.payload_len()));
            let check = |ptr: ManagedPtr<'_>, offset: usize| {
                assert_eq!(
                    ptr.pointer().map(|p| p.as_ptr().addr()),
                    Some(base.addr().wrapping_add(offset))
                );
                // SAFETY: F1.GcHandleRooted — `owner` is live; the checked API must
                // reject any nonempty read beginning at or beyond the payload end.
                let invalid = unsafe { ptr.try_with_data(1, |_| unreachable!()) };
                assert_eq!(invalid, None);
            };
            for offset in [len, 4096] {
                check(
                    ManagedPtr::new(
                        None,
                        TypeDescription::NULL,
                        Some(owner),
                        false,
                        Some(ByteOffset::new(offset)),
                    ),
                    offset,
                );
                #[cfg(feature = "multithreading")]
                {
                    let (object_ptr, arena_id) = owner.as_ptr_info().unwrap();
                    check(
                        ManagedPtr::new_cross_arena(
                            None,
                            TypeDescription::NULL,
                            object_ptr,
                            arena_id,
                            ByteOffset::new(offset),
                        ),
                        offset,
                    );
                }
            }
            // Managed offsets are u32-packed, so full usize wrap is exercised on
            // the same address-formation helper rather than an unrepresentable ManagedPtr.
            assert_eq!(
                observed_address(base, usize::MAX).map(|p| p.as_ptr().addr()),
                match base.addr().wrapping_add(usize::MAX) {
                    0 => None,
                    address => Some(address),
                }
            );
            assert_eq!(
                observed_address(base, 0usize.wrapping_sub(base.addr())),
                None
            );
            assert_eq!(observed_address(std::ptr::null(), 1), None);
        }
    });
}

#[test]
fn m1a_safe_oob_address_observation() {
    with_test_gc_context(|gc_handle| {
        let vector = Vector::new(
            ConcreteType::new(ResolutionS::NULL, BaseType::UInt8),
            ArrayLayoutManager {
                element_layout: Arc::new(LayoutManager::Scalar(Scalar::UInt8)),
                length: 1,
            },
            vec![0xA5],
            vec![1],
        );
        let obj = ObjectRef::new(gc_handle, HeapStorage::Vec(Box::new(vector)));
        let base = obj.pointer().expect("retained heap allocation base");
        let ptr = ManagedPtr::new(
            Some(base),
            TypeDescription::NULL,
            Some(obj),
            false,
            Some(ByteOffset::new(4096)),
        );
        let expected = base.as_ptr().addr().wrapping_add(4096);

        let observed = PointerLike::pointer(&ptr);
        assert_eq!(observed.expect("observable").as_ptr().addr(), expected);
        assert_eq!(
            crate::StackValue::ManagedPtr(ptr.into()).as_ptr().addr(),
            expected
        );
    });
}

#[test]
#[cfg(feature = "memory-validation")]
#[should_panic(expected = "ManagedPtr::offset: bounds violation")]
fn test_managed_ptr_offset_oob() {
    with_test_gc_context(|gc_handle| {
        // Create a small object (4 bytes for Int32)
        let storage = HeapStorage::Str(crate::string::CLRString::from("test"));
        let obj = ObjectRef::new(gc_handle, storage);

        let ptr = managed_ptr_to_heap_object_start(obj);

        // Offset by much more than size of ValueType should panic.
        // SAFETY: F10.RawMemoryAccessValid — The test invokes the unsafe operation specifically to verify its
        // runtime bounds validation rejects this out-of-bounds offset.
        unsafe {
            ptr.offset(1000);
        }
    });
}

#[test]
fn test_managed_ptr_offset_valid() {
    with_test_gc_context(|gc_handle| {
        let storage = HeapStorage::Str(crate::string::CLRString::from("test"));
        let obj = ObjectRef::new(gc_handle, storage);

        let ptr = managed_ptr_to_heap_object_start(obj);

        // Offset by 4 bytes (end of object) should be valid
        // SAFETY: F1.GcHandleRooted — This test constructs valid backing storage and uses the pointer only within that storage's lifetime.
        unsafe {
            ptr.offset(4);
        }
    });
}

#[test]
fn test_managed_ptr_metadata_rejects_unknown_static_subtag() {
    let mut buffer = ManagedPtr::serialization_buffer();
    let ptr_size = ObjectRef::SIZE;
    let word0 = 7usize | (3 << 3);
    let word1 = 0usize;
    buffer[..ptr_size].copy_from_slice(&word0.to_ne_bytes());
    buffer[ptr_size..ptr_size * 2].copy_from_slice(&word1.to_ne_bytes());
    buffer[ptr_size * 2..ptr_size * 3].copy_from_slice(&(word0 ^ word1).to_ne_bytes());

    // SAFETY: F10.RawMemoryAccessValid — `buffer` is a complete, checksum-valid
    // ManagedPtr encoding deliberately carrying an unsupported subtag.
    let decoded = unsafe { ManagedPtr::read_metadata_unchecked(&buffer) };
    assert!(matches!(
        decoded,
        Err(dotnet_types::error::PointerDeserializationError::UnknownSubtag(3))
    ));
}

#[test]
fn test_heap_decode_cache_hit_miss_and_integrity() {
    with_test_gc_context(|gc_handle| {
        let object = ObjectRef::new(
            gc_handle,
            HeapStorage::Str(crate::string::CLRString::from("cached pointer")),
        );
        let offset = ByteOffset::new(2);
        let address = object.with_data(|data| {
            NonNull::new(data.as_ptr().wrapping_add(offset.as_usize()).cast_mut())
        });
        let ptr = ManagedPtr::new(
            address,
            TypeDescription::NULL,
            Some(object),
            false,
            Some(offset),
        );
        let mut buffer = ManagedPtr::serialization_buffer();
        ptr.write(&mut buffer);
        let mut cache = CountingHeapCache::default();

        // SAFETY: F10.RawMemoryAccessValid — The buffer was written from the live object above. This test
        // cache is short-lived inside the GC fixture; production cache owners
        // must instead trace their retained ObjectRefs across collection.
        let miss = unsafe {
            ManagedPtr::read_resolved_with_heap_cache_unchecked(
                &buffer,
                &NO_MANAGED_BASES,
                &mut cache,
            )
        }
        .unwrap();
        assert_eq!(miss.address, address);
        assert_eq!(cache.misses, 1);
        assert_eq!(cache.hits, 0);
        assert_eq!(cache.entries.len(), 1);

        // SAFETY: F1.GcHandleRooted — The cache now contains the live Heap handle decoded above;
        // the reader must still derive an address from freshly borrowed storage.
        let hit = unsafe {
            ManagedPtr::read_resolved_with_heap_cache_unchecked(
                &buffer,
                &NO_MANAGED_BASES,
                &mut cache,
            )
        }
        .unwrap();
        assert_eq!(hit.address, address);
        assert_eq!(cache.misses, 1);
        assert_eq!(cache.hits, 1);

        buffer[ManagedPtr::SIZE - 1] ^= 1;
        // SAFETY: F10.RawMemoryAccessValid — The corrupted buffer remains one complete representation;
        // checksum validation must reject it before consulting the cache.
        let corrupt = unsafe {
            ManagedPtr::read_resolved_with_heap_cache_unchecked(
                &buffer,
                &NO_MANAGED_BASES,
                &mut cache,
            )
        };
        assert!(matches!(
            corrupt,
            Err(dotnet_types::error::PointerDeserializationError::ChecksumMismatch)
        ));
        assert_eq!((cache.hits, cache.misses), (1, 1));
    });
}

#[test]
fn test_managed_ptr_serialization_roundtrip() {
    let _guard = static_reg_test_lock().lock().unwrap();

    with_test_gc_context(|gc_handle| {
        let mut buf = ManagedPtr::serialization_buffer();

        // 1. Unmanaged
        let unmanaged_addr = 0xDEADBEEFusize;
        let ptr_unmanaged = ManagedPtr::new(
            NonNull::new(std::ptr::without_provenance_mut(unmanaged_addr)),
            TypeDescription::NULL,
            None,
            false,
            None,
        );
        ptr_unmanaged.write(&mut buf);
        // SAFETY: F3.InteriorPointerRebased — `NoManagedPtrResolver` is sufficient for the Unmanaged encoding.
        let info = unsafe { ManagedPtr::read_resolved_unchecked(&buf, &NO_MANAGED_BASES) }.unwrap();
        assert_eq!(
            info.address,
            NonNull::new(std::ptr::without_provenance_mut(unmanaged_addr))
        );
        assert_eq!(info.origin, PointerOrigin::Unmanaged);
        assert_eq!(info.offset.as_usize(), unmanaged_addr);

        // 2. Stack
        let stack_slot = StackSlotIndex::new(123);
        let mut stack_storage = [0u8; 512];
        let stack_base = NonNull::new(stack_storage.as_mut_ptr()).unwrap();
        let stack_offset = 456;
        let stack_addr = NonNull::new(stack_base.as_ptr().wrapping_add(stack_offset));
        let ptr_stack = ManagedPtr::new(
            stack_addr,
            TypeDescription::NULL,
            None,
            false,
            Some(ByteOffset::new(stack_offset)),
        )
        .with_stack_origin(stack_slot);
        ptr_stack.write(&mut buf);
        let word1 = usize::from_ne_bytes(buf[8..16].try_into().unwrap());
        assert_eq!(
            word1, stack_offset,
            "Stack word1 must encode the byte offset"
        );
        // SAFETY: F3.InteriorPointerRebased — The bytes are complete Stack ManagedPtr encoding, but this resolver deliberately has no stack base.
        let unresolved_stack =
            unsafe { ManagedPtr::read_resolved_unchecked(&buf, &NO_MANAGED_BASES) };
        assert!(matches!(
            unresolved_stack,
            Err(dotnet_types::error::PointerDeserializationError::UnresolvedStackSlot(123))
        ));
        // SAFETY: F3.StackSlotMatchesView — The resolver returns the live stack-slot base used above.
        let info = unsafe {
            ManagedPtr::read_resolved_unchecked(
                &buf,
                &TestManagedPtrResolver {
                    stack_base: Some(stack_base),
                    static_base: None,
                },
            )
        }
        .unwrap();
        assert_eq!(info.address, stack_addr);
        assert_eq!(info.origin, PointerOrigin::Stack(stack_slot));

        // 3. Heap
        let s = crate::string::CLRString::from("test");
        let obj = ObjectRef::new(gc_handle, HeapStorage::Str(s));
        let offset = 2;
        let ptr = obj.with_data(|d| NonNull::new(d.as_ptr().wrapping_add(offset).cast_mut()));
        let ptr_heap = ManagedPtr::new(
            ptr,
            TypeDescription::NULL,
            Some(obj),
            false,
            Some(ByteOffset::new(offset)),
        );
        ptr_heap.write(&mut buf);
        // SAFETY: F6.NoEscapeAcrossArena — Heap decoding gets its base from the branded ObjectRef handle.
        let info = unsafe { ManagedPtr::read_resolved_unchecked(&buf, &NO_MANAGED_BASES) }.unwrap();
        assert_eq!(info.address, ptr);
        assert_eq!(info.origin, PointerOrigin::Heap(obj));
        assert_eq!(info.offset.as_usize(), offset);
        // SAFETY: F6.NoEscapeAcrossArena — `info.address` was resolved from the
        // live Heap handle recovered from `buf`, and the offset selects the
        // second UTF-16 code unit in the rooted string.
        let recovered_code_unit = unsafe { info.address.unwrap().cast::<u16>().as_ptr().read() };
        assert_eq!(recovered_code_unit, 'e' as u16);

        // 4. Static
        let type_desc = TypeDescription::NULL;
        let generics = GenericLookup::default();
        let mut static_storage = [0u8; 16];
        let static_base = NonNull::new(static_storage.as_mut_ptr()).unwrap();
        let static_offset = 8;
        let static_addr = NonNull::new(static_base.as_ptr().wrapping_add(static_offset));
        let ptr_static = ManagedPtr::new_static(
            static_addr,
            TypeDescription::NULL,
            type_desc,
            generics,
            false,
            ByteOffset::new(static_offset),
        );
        ptr_static.write(&mut buf);
        let word1 = usize::from_ne_bytes(buf[8..16].try_into().unwrap());
        assert_eq!(
            word1, static_offset,
            "Static word1 must encode the byte offset"
        );
        // SAFETY: F3.InteriorPointerRebased — The bytes are complete Static ManagedPtr encoding, but this resolver deliberately has no static base.
        let unresolved_static =
            unsafe { ManagedPtr::read_resolved_unchecked(&buf, &NO_MANAGED_BASES) };
        assert!(matches!(
            unresolved_static,
            Err(dotnet_types::error::PointerDeserializationError::UnresolvedStaticStorage)
        ));
        // SAFETY: F1.GcHandleRooted — The resolver returns the live static-storage base used above.
        let info = unsafe {
            ManagedPtr::read_resolved_unchecked(
                &buf,
                &TestManagedPtrResolver {
                    stack_base: None,
                    static_base: Some(static_base),
                },
            )
        }
        .unwrap();
        assert_eq!(info.address, static_addr);
        assert_eq!(info.offset.as_usize(), static_offset);

        // 5. CrossArenaObjectRef (if enabled)
        #[cfg(feature = "multithreading")]
        {
            use crate::object::ObjectPtr;
            let ptr_raw = obj.with_data(|d| d.as_ptr());
            // SAFETY: F1.GcHandleRooted — `obj` is kept alive for the duration of this test closure.
            // We only use this raw pointer to validate serialization/deserialization logic.
            let ptr_lock = Gc::as_ptr(obj.0.unwrap())
                .cast::<dotnet_utils::gc::ThreadSafeLock<crate::object::ObjectInner<'static>>>();
            // SAFETY: F1.GcHandleRooted — This test constructs valid backing storage and uses the pointer only within that storage's lifetime.
            let ptr = unsafe { ObjectPtr::from_raw(ptr_lock).unwrap() };
            let arena_id = ptr.owner_id();
            let cross_offset = 12;
            let cross_ptr = NonNull::new(ptr_raw.wrapping_add(cross_offset).cast_mut());
            let ptr_cross = ManagedPtr::new_cross_arena(
                cross_ptr,
                TypeDescription::NULL,
                ptr,
                arena_id,
                ByteOffset::new(cross_offset),
            );

            ptr_cross.write(&mut buf);
            // SAFETY: F1.GcHandleRooted — CrossArena decoding obtains its base while holding the encoded arena lease.
            let info =
                unsafe { ManagedPtr::read_resolved_unchecked(&buf, &NO_MANAGED_BASES) }.unwrap();
            assert_eq!(info.address, cross_ptr);
            if let PointerOrigin::CrossArenaObjectRef(recovered_ptr, recovered_arena) = info.origin
            {
                assert_eq!(recovered_ptr, ptr);
                assert_eq!(recovered_arena, arena_id);
            } else {
                panic!("Expected CrossArenaObjectRef, got {:?}", info.origin);
            }
            assert_eq!(info.offset.as_usize(), cross_offset);
        }
    });
}

#[test]
fn test_gc_alignment() {
    type TestRoot = Rootable![()];
    let arena = Arena::<TestRoot>::new(|_mc| ());
    arena.mutate(|mc, _| {
        for _ in 0..1000 {
            let gc = Gc::new(mc, 0u64); // u64 should need 8-byte alignment
            let ptr = Gc::as_ptr(gc) as usize;
            assert_eq!(ptr % 8, 0, "Gc pointer {:#x} is not 8-byte aligned", ptr);
        }
    });
}

#[test]
fn test_managed_ptr_serialization_bugs_reproduction() {
    with_test_gc_context(|_gc_handle| {
        let mut buf = [0u8; ManagedPtr::SIZE];

        // 1. Transient origin (Fixed behavior in Stage 1)
        let layout = Arc::new(crate::layout::FieldLayoutManager {
            fields: hashbrown::HashMap::new(),
            total_size: 0,
            alignment: 1,
            gc_desc: crate::layout::GcDesc::default(),
            has_ref_fields: false,
        });
        let storage = crate::storage::FieldStorage::new(layout, vec![]);
        let obj = Object::new(TypeDescription::NULL, GenericLookup::default(), storage);
        let transient_addr = 0x5000usize;
        let ptr_transient = ManagedPtr::new_transient(
            NonNull::new(std::ptr::without_provenance_mut(transient_addr)),
            TypeDescription::NULL,
            obj.clone(),
            ByteOffset::new(123), // Use a non-zero offset
        );

        ptr_transient.write(&mut buf);
        // SAFETY: F1.GcHandleRooted — This test constructs valid backing storage and uses the pointer only within that storage's lifetime.
        let result = unsafe { ManagedPtr::read_metadata_unchecked(&buf) };
        assert!(result.is_err(), "Transient recovery should fail for safety");

        let word0 = usize::from_ne_bytes(buf[0..8].try_into().unwrap());
        assert_eq!(word0 & 7, 7, "Transient should use Tag 7");
        assert_eq!((word0 >> 3) & 7, 2, "Transient should use Subtag 2");
        let word1 = usize::from_ne_bytes(buf[8..16].try_into().unwrap());
        assert_eq!(word1, 123, "Transient word1 must encode the byte offset");

        // 2. Tag Collision (Verified safe)
        // Any pointer with bit 0 = 0 is now guaranteed to be treated as Heap or Unmanaged,
        // never as Stack/Static/Transient.
        // We don't test misaligned pointers here as they would (correctly) panic in ObjectRef::read_unchecked.
    });
}

#[test]
fn test_stack_rebase_preserves_golden_three_word_encoding_and_full_offset() {
    let _guard = static_reg_test_lock().lock().unwrap();
    reset_static_registry();

    let stack_offset = (1usize << 31) + 17;
    let stack_slot = StackSlotIndex::new(9);
    let mut old_stack_storage = [0u8; 1];
    let old_stack_base = NonNull::new(old_stack_storage.as_mut_ptr()).unwrap();
    let mut new_stack_storage = [0u8; 1];
    let new_stack_base = NonNull::new(new_stack_storage.as_mut_ptr()).unwrap();
    assert_ne!(old_stack_base, new_stack_base);
    let stack = ManagedPtr::new(
        NonNull::new(old_stack_base.as_ptr().wrapping_add(stack_offset)),
        TypeDescription::NULL,
        None,
        false,
        Some(ByteOffset::new(stack_offset)),
    )
    .with_origin(PointerOrigin::Stack(stack_slot));
    let mut buffer = ManagedPtr::serialization_buffer();
    stack.write(&mut buffer);

    let ptr_size = ObjectRef::SIZE;
    let expected_word0 = 1 | ((stack_slot.as_usize() & 0x3FFF_FFFF) << 3) | (stack_offset << 33);
    let expected_word1 = stack_offset;
    let expected_word2 = expected_word0 ^ expected_word1;
    assert_eq!(
        usize::from_ne_bytes(buffer[..ptr_size].try_into().unwrap()),
        expected_word0
    );
    assert_eq!(
        usize::from_ne_bytes(buffer[ptr_size..ptr_size * 2].try_into().unwrap()),
        expected_word1,
        "Stack word1 must retain the full compact offset"
    );
    assert_eq!(
        usize::from_ne_bytes(buffer[ptr_size * 2..ptr_size * 3].try_into().unwrap()),
        expected_word2,
        "Stack word2 must remain the word0 XOR word1 checksum"
    );
    let golden_bytes = buffer;

    assert!(
        ManagedPtr::rebase_stack_pointer(&mut buffer, |slot| {
            assert_eq!(slot, stack_slot);
            Some(new_stack_base)
        })
        .unwrap()
    );
    assert_eq!(
        buffer, golden_bytes,
        "Stack rebase must preserve the canonical three-word representation"
    );

    // SAFETY: F3.InteriorPointerRebased — The canonical bytes are complete and the resolver
    // supplies the distinct current base for their stable logical slot.
    let rebased = unsafe {
        ManagedPtr::read_resolved_unchecked(
            &buffer,
            &TestManagedPtrResolver {
                stack_base: Some(new_stack_base),
                static_base: None,
            },
        )
    }
    .unwrap();
    assert_eq!(
        rebased.address,
        NonNull::new(new_stack_base.as_ptr().wrapping_add(stack_offset))
    );
    assert_eq!(rebased.origin, PointerOrigin::Stack(stack_slot));
    assert_eq!(rebased.offset, ByteOffset::new(stack_offset));

    // SAFETY: F3.InteriorPointerRebased — `buffer` was just written as one complete Stack ManagedPtr.
    let stack_info = unsafe { ManagedPtr::read_metadata_unchecked(&buffer) }.unwrap();
    assert_eq!(stack_info.offset, ByteOffset::new(stack_offset));

    // A self-consistent XOR alone must not permit the canonical word-1 offset
    // to disagree with the legacy packed mirror in word 0.
    let word0 = usize::from_ne_bytes(buffer[..ptr_size].try_into().unwrap());
    let mismatched_word1 = stack_offset + 1;
    buffer[ptr_size..ptr_size * 2].copy_from_slice(&mismatched_word1.to_ne_bytes());
    buffer[ptr_size * 2..ptr_size * 3].copy_from_slice(&(word0 ^ mismatched_word1).to_ne_bytes());
    // SAFETY: F10.RawMemoryAccessValid — The complete buffer is deliberately malformed to exercise validation.
    let mismatched = unsafe { ManagedPtr::read_metadata_unchecked(&buffer) };
    assert!(matches!(
        mismatched,
        Err(dotnet_types::error::PointerDeserializationError::OffsetMismatch)
    ));

    let address = NonNull::new(std::ptr::without_provenance_mut(0x4000)).unwrap();
    let static_offset = (1usize << 26) + 23;
    let static_ptr = ManagedPtr::new_static(
        Some(address),
        TypeDescription::NULL,
        TypeDescription::NULL,
        GenericLookup::default(),
        false,
        ByteOffset::new(static_offset),
    );
    static_ptr.write(&mut buffer);
    // SAFETY: F3.InteriorPointerRebased — `buffer` was just written as one complete Static ManagedPtr and
    // its metadata remains registered for this test.
    let static_info = unsafe { ManagedPtr::read_metadata_unchecked(&buffer) }.unwrap();
    assert_eq!(static_info.offset, ByteOffset::new(static_offset));
}

#[test]
fn test_static_registry_deduplication() {
    let _guard = static_reg_test_lock().lock().unwrap();
    reset_static_registry();

    let type_desc = TypeDescription::NULL;
    let generics = GenericLookup::default();
    let mut buf1 = ManagedPtr::serialization_buffer();
    let mut buf2 = ManagedPtr::serialization_buffer();

    let ptr1 = ManagedPtr::new_static(
        NonNull::new(std::ptr::without_provenance_mut(0x1000)),
        TypeDescription::NULL,
        type_desc.clone(),
        generics.clone(),
        false,
        ByteOffset::new(0),
    );

    let ptr2 = ManagedPtr::new_static(
        NonNull::new(std::ptr::without_provenance_mut(0x2000)),
        TypeDescription::NULL,
        type_desc,
        generics.clone(),
        false,
        ByteOffset::new(0),
    );

    ptr1.write(&mut buf1);
    ptr2.write(&mut buf2);

    let word0_1 = usize::from_ne_bytes(buf1[0..8].try_into().unwrap());
    let word0_2 = usize::from_ne_bytes(buf2[0..8].try_into().unwrap());

    let id1 = (word0_1 >> 6) & 0xFFFFFFFF;
    let id2 = (word0_2 >> 6) & 0xFFFFFFFF;

    assert_eq!(
        id1, id2,
        "Static pointers with same metadata should have same ID"
    );
    assert_eq!(id1, 1, "First ID should be 1");
}

#[test]
fn test_stack_rebase_roundtrip_miri() {
    let mut buf = [0u8; ManagedPtr::SIZE];
    let slot_idx = StackSlotIndex::new(42);
    let offset = 8;
    let w0: usize = 1 | ((slot_idx.as_usize() & 0x3FFFFFFF) << 3) | (offset << 33);
    let w1 = offset;
    let ptr_size = ObjectRef::SIZE;

    buf[..ptr_size].copy_from_slice(&w0.to_ne_bytes());
    buf[ptr_size..ptr_size * 2].copy_from_slice(&w1.to_ne_bytes());
    buf[ptr_size * 2..ptr_size * 3].copy_from_slice(&(w0 ^ w1).to_ne_bytes());
    let original = buf;

    let mut old_stack_storage = [0u8; 16];
    let old_stack_base = NonNull::new(old_stack_storage.as_mut_ptr()).unwrap();
    let mut new_stack_storage = [0u8; 16];
    let new_stack_base = NonNull::new(new_stack_storage.as_mut_ptr()).unwrap();
    assert_ne!(old_stack_base, new_stack_base);

    assert!(
        ManagedPtr::rebase_stack_pointer(&mut buf, |slot| {
            assert_eq!(slot, slot_idx);
            Some(new_stack_base)
        })
        .unwrap()
    );
    assert_eq!(buf, original);

    // SAFETY: F3.InteriorPointerRebased — The hand-written canonical bytes are complete and
    // the resolver supplies the current base for their stable slot.
    let info = unsafe {
        ManagedPtr::read_resolved_unchecked(
            &buf,
            &TestManagedPtrResolver {
                stack_base: Some(new_stack_base),
                static_base: None,
            },
        )
    }
    .unwrap();
    assert_eq!(
        info.address,
        NonNull::new(new_stack_base.as_ptr().wrapping_add(offset))
    );
    assert_eq!(info.origin, PointerOrigin::Stack(slot_idx));
    assert_eq!(info.offset.as_usize(), offset);
}

#[test]
fn test_safe_stack_rebase_rejects_invalid_input_without_mutation() {
    let _guard = static_reg_test_lock().lock().unwrap();
    reset_static_registry();
    let slot = StackSlotIndex::new(27);
    let offset = 24usize;
    let ptr_size = ObjectRef::SIZE;
    let word0 = 1 | ((slot.as_usize() & 0x3FFF_FFFF) << 3) | (offset << 33);
    let word1 = offset;
    let mut valid_stack = ManagedPtr::serialization_buffer();
    valid_stack[..ptr_size].copy_from_slice(&word0.to_ne_bytes());
    valid_stack[ptr_size..ptr_size * 2].copy_from_slice(&word1.to_ne_bytes());
    valid_stack[ptr_size * 2..ptr_size * 3].copy_from_slice(&(word0 ^ word1).to_ne_bytes());

    for actual in [0, ManagedPtr::SIZE - 1] {
        let mut truncated = vec![0xA5; actual];
        let original = truncated.clone();
        let resolver_called = Cell::new(false);
        assert_eq!(
            ManagedPtr::rebase_stack_pointer(&mut truncated, |_| {
                resolver_called.set(true);
                None
            }),
            Err(
                dotnet_types::error::PointerDeserializationError::BufferTooSmall {
                    expected: ManagedPtr::SIZE,
                    actual,
                }
            )
        );
        assert_eq!(truncated, original);
        assert!(!resolver_called.get());
    }

    let mut bad_checksum = valid_stack;
    bad_checksum[ManagedPtr::SIZE - 1] ^= 1;
    let original = bad_checksum;
    let resolver_called = Cell::new(false);
    assert_eq!(
        ManagedPtr::rebase_stack_pointer(&mut bad_checksum, |_| {
            resolver_called.set(true);
            None
        }),
        Err(dotnet_types::error::PointerDeserializationError::ChecksumMismatch)
    );
    assert_eq!(bad_checksum, original);
    assert!(!resolver_called.get());

    let mut offset_mismatch = valid_stack;
    let mismatched_word1 = word1 + 1;
    offset_mismatch[ptr_size..ptr_size * 2].copy_from_slice(&mismatched_word1.to_ne_bytes());
    offset_mismatch[ptr_size * 2..ptr_size * 3]
        .copy_from_slice(&(word0 ^ mismatched_word1).to_ne_bytes());
    let original = offset_mismatch;
    let resolver_called = Cell::new(false);
    assert_eq!(
        ManagedPtr::rebase_stack_pointer(&mut offset_mismatch, |_| {
            resolver_called.set(true);
            None
        }),
        Err(dotnet_types::error::PointerDeserializationError::OffsetMismatch)
    );
    assert_eq!(offset_mismatch, original);
    assert!(!resolver_called.get());

    let mut unresolved = valid_stack;
    let original = unresolved;
    let resolved_slot = Cell::new(None);
    assert_eq!(
        ManagedPtr::rebase_stack_pointer(&mut unresolved, |resolved| {
            resolved_slot.set(Some(resolved));
            None
        }),
        Err(dotnet_types::error::PointerDeserializationError::UnresolvedStackSlot(slot.as_usize()))
    );
    assert_eq!(unresolved, original);
    assert_eq!(resolved_slot.get(), Some(slot));

    let unmanaged = ManagedPtr::new(
        Some(NonNull::new(std::ptr::without_provenance_mut(0x5000)).unwrap()),
        TypeDescription::NULL,
        None,
        false,
        None,
    );
    let mut non_stack = ManagedPtr::serialization_buffer();
    unmanaged.write(&mut non_stack);
    let mut static_storage = [0u8; 1];
    let static_ptr = ManagedPtr::new_static(
        NonNull::new(static_storage.as_mut_ptr()),
        TypeDescription::NULL,
        TypeDescription::NULL,
        GenericLookup::default(),
        false,
        ByteOffset::ZERO,
    );
    let mut static_non_stack = ManagedPtr::serialization_buffer();
    static_ptr.write(&mut static_non_stack);

    for non_stack in [&mut non_stack, &mut static_non_stack] {
        let original = *non_stack;
        let resolver_called = Cell::new(false);
        assert_eq!(
            ManagedPtr::rebase_stack_pointer(non_stack, |_| {
                resolver_called.set(true);
                None
            }),
            Ok(false)
        );
        assert_eq!(*non_stack, original);
        assert!(!resolver_called.get());
    }
}

#[test]
fn test_cached_stack_rebase_updates_address_and_preserves_unresolved_or_non_stack_pointers() {
    let slot = StackSlotIndex::new(12);
    let offset = ByteOffset::new(4);
    let mut old_storage = [0u8; 16];
    let old_base = NonNull::new(old_storage.as_mut_ptr()).unwrap();
    let old_address = NonNull::new(old_base.as_ptr().wrapping_add(offset.as_usize())).unwrap();
    let mut new_storage = [0u8; 16];
    let new_base = NonNull::new(new_storage.as_mut_ptr()).unwrap();
    let new_address = NonNull::new(new_base.as_ptr().wrapping_add(offset.as_usize())).unwrap();
    assert_ne!(old_base, new_base);

    let mut stack_pointer = ManagedPtr::new(
        Some(old_address),
        TypeDescription::NULL,
        None,
        false,
        Some(offset),
    )
    .with_stack_origin(slot);
    assert_eq!(
        stack_pointer.rebase_cached_stack_pointer(|resolved_slot| {
            assert_eq!(resolved_slot, slot);
            Some(new_base)
        }),
        Ok(true)
    );

    assert_eq!(
        stack_pointer.rebase_cached_stack_pointer(|_| None),
        Err(dotnet_types::error::PointerDeserializationError::UnresolvedStackSlot(slot.as_usize()))
    );
    let stack_info = stack_pointer.into_info();
    assert_eq!(stack_info.address, Some(new_address));
    assert_eq!(stack_info.origin, PointerOrigin::Stack(slot));
    assert_eq!(stack_info.offset, offset);

    let mut non_stack_pointer = ManagedPtr::null();
    let resolver_called = Cell::new(false);
    assert_eq!(
        non_stack_pointer.rebase_cached_stack_pointer(|_| {
            resolver_called.set(true);
            None
        }),
        Ok(false)
    );
    assert!(!resolver_called.get());
    let non_stack_info = non_stack_pointer.into_info();
    assert_eq!(non_stack_info.address, None);
    assert_eq!(non_stack_info.origin, PointerOrigin::Unmanaged);
    assert_eq!(non_stack_info.offset, ByteOffset::ZERO);
}

#[test]
fn test_managed_ptr_unmanaged_roundtrip_miri() {
    let mut buf = [0u8; ManagedPtr::SIZE];
    let addr = 0xAAAA_BBBB_CCCC_DDDDusize;
    let ptr = ManagedPtr::new(
        NonNull::new(std::ptr::without_provenance_mut(addr)),
        TypeDescription::NULL,
        None,
        false,
        None,
    );

    ptr.write(&mut buf);
    // SAFETY: F1.GcHandleRooted — This test constructs valid backing storage and uses the pointer only within that storage's lifetime.
    let info = unsafe { ManagedPtr::read_resolved_unchecked(&buf, &NO_MANAGED_BASES) }.unwrap();
    assert_eq!(
        info.address,
        NonNull::new(std::ptr::without_provenance_mut(addr))
    );
    assert_eq!(info.origin, PointerOrigin::Unmanaged);
    assert_eq!(info.offset.as_usize(), addr);
}

#[test]
fn m1b_managed_ptr_checked_offset_reports_errors() {
    let mut storage = [0u8; 8];
    storage[3] = 0x5A;
    storage[5] = 0xA5;
    let base = storage.as_mut_ptr();
    let address = NonNull::new(base.wrapping_add(3)).unwrap();
    let ptr = ManagedPtr::new(
        Some(address),
        TypeDescription::NULL,
        None,
        false,
        Some(ByteOffset::new(3)),
    )
    .with_origin(PointerOrigin::Stack(StackSlotIndex::new(1)));

    assert_eq!(
        ptr.clone().checked_offset(-4).err(),
        Some(ManagedPtrOffsetError::NegativeResult)
    );
    assert_eq!(
        ptr.clone().checked_offset(isize::MIN).err(),
        Some(ManagedPtrOffsetError::NegativeResult)
    );
    #[cfg(target_pointer_width = "64")]
    assert_eq!(
        ptr.clone().checked_offset(isize::MAX).err(),
        Some(ManagedPtrOffsetError::ArithmeticOverflow)
    );
    #[cfg(target_pointer_width = "32")]
    assert_eq!(
        ptr.clone()
            .checked_offset(isize::MAX)
            .unwrap()
            .byte_offset(),
        ByteOffset::new(isize::MAX as usize + 3)
    );
    #[cfg(target_pointer_width = "64")]
    assert_eq!(
        ptr.clone().checked_offset(u32::MAX as isize).err(),
        Some(ManagedPtrOffsetError::CompactRangeExceeded)
    );
    let displaced = ptr.checked_offset(2).unwrap();
    assert_eq!(displaced.byte_offset(), ByteOffset::new(5));
    assert_eq!(
        displaced.observe_address().unwrap().as_ptr(),
        base.wrapping_add(5)
    );
    // SAFETY: F10.RawMemoryAccessValid — The cached address points to byte 5 of live storage.
    assert_eq!(unsafe { displaced.with_data(1, |bytes| bytes[0]) }, 0xA5);

    let unmanaged = ManagedPtr::new(Some(address), TypeDescription::NULL, None, false, None);
    let round_trip = unmanaged
        .clone()
        .checked_offset(isize::MIN)
        .unwrap()
        .checked_offset(isize::MIN)
        .unwrap();
    // SAFETY: F10.RawMemoryAccessValid — The full-width round trip returns to live byte 3.
    assert_eq!(unsafe { round_trip.with_data(1, |bytes| bytes[0]) }, 0x5A);
    let zero_delta = (address.as_ptr().addr() as isize).wrapping_neg();
    let null = unmanaged.checked_offset(zero_delta).unwrap();
    assert!(null.is_null());
    assert_eq!(null.byte_offset(), ByteOffset::ZERO);
    assert!(matches!(null.origin(), PointerOrigin::Unmanaged));
    assert!(null.observe_address().is_none());
    assert!(null.clone().checked_offset(0).unwrap().is_null());
    let exposed = address.as_ptr().expose_provenance();
    let restored = null.clone().checked_offset(exposed as isize).unwrap();
    assert!(!restored.is_null());
    assert_eq!(restored.observe_address().unwrap().as_ptr().addr(), exposed);
    // SAFETY: F10.RawMemoryAccessValid — the exposed address is byte 3 of live storage.
    assert_eq!(unsafe { restored.with_data(1, |bytes| bytes[0]) }, 0x5A);
    let stack_null = crate::StackValue::ManagedPtr(null.clone().into());
    let stack_restored = (stack_null + crate::StackValue::NativeInt(exposed as isize)).unwrap();
    assert!(!stack_restored.is_null());
    // SAFETY: F10.RawMemoryAccessValid — the stack result reconstructs the same live byte.
    assert_eq!(unsafe { stack_restored.as_ptr().read() }, 0x5A);
    let high_from_null = null.checked_offset(-1).unwrap();
    assert_eq!(
        high_from_null.observe_address().unwrap().as_ptr().addr(),
        usize::MAX
    );

    // These synthetic addresses are formed but never dereferenced.
    let high = NonNull::new(std::ptr::without_provenance_mut::<u8>(usize::MAX)).unwrap();
    let far = ManagedPtr::new(
        Some(high),
        TypeDescription::NULL,
        None,
        false,
        Some(ByteOffset::new(u32::MAX as usize)),
    )
    .with_origin(PointerOrigin::Stack(StackSlotIndex::new(1)));
    assert_eq!(
        far.clone().checked_offset(-1).unwrap().byte_offset(),
        ByteOffset::new(u32::MAX as usize - 1)
    );
    assert_eq!(
        far.checked_offset(1).err(),
        Some(ManagedPtrOffsetError::CompactRangeExceeded)
    );
    let wrapped = ManagedPtr::new(Some(high), TypeDescription::NULL, None, false, None)
        .checked_offset(1)
        .unwrap();
    assert!(wrapped.is_null());
    let one = NonNull::new(std::ptr::without_provenance_mut::<u8>(1)).unwrap();
    let stack = ManagedPtr::new(
        Some(one),
        TypeDescription::NULL,
        None,
        false,
        Some(ByteOffset::new(1)),
    )
    .with_origin(PointerOrigin::Stack(StackSlotIndex::new(1)));
    assert_eq!(
        stack.checked_offset(-1).err(),
        Some(ManagedPtrOffsetError::NullCachedAddress)
    );
}

#[test]
fn m1b_unmanaged_ptr_null_contract() {
    use crate::StackValue;

    let null = StackValue::unmanaged_ptr(std::ptr::null_mut());
    assert!(matches!(null, StackValue::NativeInt(0)));
    assert!(null.is_null());

    let mut allocation = Box::new([0xA5u8; 64]);
    let base = UnmanagedPtr(NonNull::new(allocation.as_mut_ptr()).unwrap());
    let zero_delta = (base.addr() as isize).wrapping_neg();
    for delta in [0, 1, -1, isize::MIN, isize::MAX, zero_delta] {
        let expected = base.addr().wrapping_add_signed(delta);
        let result = base.wrapping_byte_offset(delta);
        let stack_result = StackValue::unmanaged_ptr_offset(base, delta);
        if expected == 0 {
            assert!(result.is_none());
            assert!(matches!(stack_result, StackValue::NativeInt(0)));
            assert!(stack_result.is_null());
        } else {
            let displaced = result.unwrap();
            assert_eq!(displaced.addr(), expected);
            assert!(matches!(stack_result, StackValue::UnmanagedPtr(_)));
            assert!(!stack_result.is_null());
            let restored = StackValue::unmanaged_ptr_offset(displaced, delta.wrapping_neg());
            m1b_assert_unmanaged_sentinel(&restored);
        }
    }

    // These synthetic addresses are only observed, never dereferenced.
    for (address, delta) in [(1usize, -1isize), (usize::MAX, 1)] {
        let ptr = UnmanagedPtr(NonNull::new(std::ptr::without_provenance_mut(address)).unwrap());
        assert!(ptr.wrapping_byte_offset(delta).is_none());
        let result = StackValue::unmanaged_ptr_offset(ptr, delta);
        assert!(matches!(result, StackValue::NativeInt(0)));
        assert!(result.is_null());
        assert!(ptr.wrapping_byte_offset(0).is_some());
    }
}

fn m1b_assert_unmanaged_sentinel(value: &crate::StackValue<'_>) {
    // SAFETY: F10.RawMemoryAccessValid — the test keeps the 64-byte Box allocation
    // alive, and `value` is expected to point back to its initialized first byte.
    let byte = unsafe { value.as_ptr().read() };
    assert_eq!(byte, 0xA5);
}

fn m1b_assert_unmanaged_round_trip(raw: *mut u8, delta: isize, int32: bool, subtract: bool) {
    let pointer = crate::StackValue::unmanaged_ptr(raw);
    let delta_value = || {
        if int32 {
            crate::StackValue::Int32(delta as i32)
        } else {
            crate::StackValue::NativeInt(delta)
        }
    };
    let displaced = if subtract {
        (pointer - delta_value()).unwrap()
    } else {
        (pointer + delta_value()).unwrap()
    };
    let restored = if subtract {
        (displaced + delta_value()).unwrap()
    } else {
        (displaced - delta_value()).unwrap()
    };
    m1b_assert_unmanaged_sentinel(&restored);
}

#[test]
fn m1b_unmanaged_offsets_wrap_and_round_trip_with_deref() {
    let mut allocation = Box::new([0u8; 64]);
    let raw = allocation.as_mut_ptr();
    // SAFETY: F10.RawMemoryAccessValid — `raw` points to the initialized first byte
    // of the live Box allocation, which remains owned for the whole test.
    unsafe { raw.write(0xA5) };

    for delta in [1, -1, 8, i32::MIN as isize] {
        for int32 in [true, false] {
            for subtract in [false, true] {
                m1b_assert_unmanaged_round_trip(raw, delta, int32, subtract);
            }
        }
    }
    for delta in [isize::MIN, isize::MAX] {
        for subtract in [false, true] {
            m1b_assert_unmanaged_round_trip(raw, delta, false, subtract);
        }
    }

    let wrapped = (((crate::StackValue::unmanaged_ptr(raw)
        + crate::StackValue::NativeInt(isize::MAX))
    .unwrap()
        + crate::StackValue::NativeInt(isize::MAX))
    .unwrap()
        + crate::StackValue::Int32(2))
    .unwrap();
    m1b_assert_unmanaged_sentinel(&wrapped);
    drop(allocation);
}

#[test]
fn m1b_unmanaged_arithmetic_wraps_to_explicit_null() {
    let mut allocation = Box::new([0u8; 64]);
    let raw = allocation.as_mut_ptr();
    // SAFETY: F10.RawMemoryAccessValid — `raw` points to the initialized first byte
    // of the live Box allocation, which remains owned for the whole test.
    unsafe { raw.write(0xA5) };

    let base = crate::StackValue::unmanaged_ptr(raw);
    let address = base.as_ptr().addr() as isize;
    let zero = (base - crate::StackValue::NativeInt(address)).unwrap();
    assert!(zero == crate::StackValue::NativeInt(0));
    assert!(zero.is_null());

    let max_address =
        crate::StackValue::unmanaged_ptr(std::ptr::without_provenance_mut(usize::MAX));
    let wrapped_from_max = (max_address + crate::StackValue::Int32(1)).unwrap();
    assert!(wrapped_from_max == crate::StackValue::NativeInt(0));
    assert!(wrapped_from_max.is_null());

    let address_one = crate::StackValue::unmanaged_ptr(std::ptr::without_provenance_mut(1));
    let wrapped_from_one = (address_one - crate::StackValue::Int32(1)).unwrap();
    assert!(wrapped_from_one == crate::StackValue::NativeInt(0));
    assert!(wrapped_from_one.is_null());

    let exposed_address = raw.expose_provenance();
    let null = crate::StackValue::unmanaged_ptr(std::ptr::null_mut());
    assert!(null.is_null());
    let reconstructed = (null + crate::StackValue::NativeInt(exposed_address as isize)).unwrap();
    m1b_assert_unmanaged_sentinel(&reconstructed);
    drop(allocation);
}

#[test]
fn m1b_min_signed_operands_do_not_overflow() {
    let mut allocation = Box::new([0u8; 64]);
    let raw = allocation.as_mut_ptr();
    // SAFETY: F10.RawMemoryAccessValid — `raw` points to the initialized first byte
    // of the live Box allocation, which remains owned for the whole test.
    unsafe { raw.write(0xA5) };

    let displaced_int32 =
        (crate::StackValue::Int32(i32::MIN) + crate::StackValue::unmanaged_ptr(raw)).unwrap();
    let restored_int32 = (displaced_int32 - crate::StackValue::Int32(i32::MIN)).unwrap();
    m1b_assert_unmanaged_sentinel(&restored_int32);

    let displaced_native_min =
        (crate::StackValue::unmanaged_ptr(raw) - crate::StackValue::NativeInt(isize::MIN)).unwrap();
    let restored_native_min =
        (displaced_native_min + crate::StackValue::NativeInt(isize::MIN)).unwrap();
    m1b_assert_unmanaged_sentinel(&restored_native_min);

    let displaced_int32_sub =
        (crate::StackValue::unmanaged_ptr(raw) - crate::StackValue::Int32(i32::MIN)).unwrap();
    let restored_int32_sub = (displaced_int32_sub + crate::StackValue::Int32(i32::MIN)).unwrap();
    m1b_assert_unmanaged_sentinel(&restored_int32_sub);
    drop(allocation);
}

fn m1b_assert_unmanaged_byte(value: &crate::StackValue<'_>, expected: u8) {
    // SAFETY: F10.RawMemoryAccessValid — callers pass pointers into a live local Box allocation
    // (or a NativeInt with its provenance exposed from that allocation) and dereference only
    // initialized bytes within that allocation.
    let actual = unsafe { value.as_ptr().read() };
    assert_eq!(actual, expected);
}

#[test]
fn m1b_pointer_comparisons_are_value_bearing() {
    let mut allocation = Box::new([0u8; 16]);
    let base_raw = allocation.as_mut_ptr();
    // SAFETY: F10.RawMemoryAccessValid — `base_raw` points to the initialized first byte
    // of the live Box allocation, which remains owned for the whole test.
    unsafe { base_raw.write(0xA5) };
    // SAFETY: F10.RawMemoryAccessValid — byte eight is in the live allocation and is
    // initialized before the corresponding pointer is read below.
    unsafe { base_raw.wrapping_add(8).write(0x5A) };

    let base = crate::StackValue::unmanaged_ptr(base_raw);
    let later = crate::StackValue::unmanaged_ptr(base_raw.wrapping_add(8));
    assert!(
        base == base.clone(),
        "an unmanaged pointer must equal itself"
    );
    assert_eq!(
        base.compare(&later, crate::NumberSign::Unsigned),
        Some(std::cmp::Ordering::Less)
    );
    assert_eq!(
        later.compare(&base, crate::NumberSign::Unsigned),
        Some(std::cmp::Ordering::Greater)
    );
    assert_eq!(
        base.compare(&later, crate::NumberSign::Signed),
        Some(std::cmp::Ordering::Less)
    );
    assert_eq!(
        later.compare(&base, crate::NumberSign::Signed),
        Some(std::cmp::Ordering::Greater)
    );

    let base_address = base_raw.expose_provenance();
    let base_integer = crate::StackValue::NativeInt(base_address as isize);
    assert!(base == base_integer.clone());
    assert!(base_integer == base);
    assert_eq!(
        base.compare(&base_integer, crate::NumberSign::Unsigned),
        Some(std::cmp::Ordering::Equal)
    );
    assert_eq!(
        base_integer.compare(&base, crate::NumberSign::Unsigned),
        Some(std::cmp::Ordering::Equal)
    );
    assert_eq!(
        base.compare(&base_integer, crate::NumberSign::Signed),
        Some(std::cmp::Ordering::Equal)
    );
    assert_eq!(
        base_integer.compare(&base, crate::NumberSign::Signed),
        Some(std::cmp::Ordering::Equal)
    );

    // These two address-only values intentionally have no backing allocation and
    // must not be dereferenced. The real base and `later` pointers below are read
    // after the comparisons, proving the live allocation path remains value-bearing.
    let near_max =
        crate::StackValue::unmanaged_ptr(std::ptr::without_provenance_mut(usize::MAX - 7));
    assert_eq!(
        near_max.compare(&base, crate::NumberSign::Unsigned),
        Some(std::cmp::Ordering::Greater)
    );
    assert_eq!(
        near_max.compare(&base, crate::NumberSign::Signed),
        Some(std::cmp::Ordering::Greater)
    );
    let negative_one = crate::StackValue::NativeInt(-1);
    assert_eq!(
        negative_one.compare(&base, crate::NumberSign::Unsigned),
        Some(std::cmp::Ordering::Greater)
    );
    assert_eq!(
        negative_one.compare(&base, crate::NumberSign::Signed),
        Some(std::cmp::Ordering::Less)
    );

    let managed = ManagedPtr::new(
        NonNull::new(base_raw),
        TypeDescription::NULL,
        None,
        false,
        None,
    );
    let managed_value = crate::StackValue::ManagedPtr(managed.clone().into());
    let null_managed = crate::StackValue::ManagedPtr(ManagedPtr::null().into());
    let zero = crate::StackValue::NativeInt(0);
    assert!(null_managed == zero);
    assert!(zero == null_managed);
    assert!(base != zero);
    assert!(zero != base);
    assert!(managed_value != zero);
    assert!(managed_value == base);
    assert!(base == managed_value);
    assert!(managed_value == base_integer);
    assert!(base_integer == managed_value);
    assert_eq!(
        managed_value.partial_cmp(&base),
        Some(std::cmp::Ordering::Equal)
    );
    assert_eq!(
        base.partial_cmp(&managed_value),
        Some(std::cmp::Ordering::Equal)
    );
    assert_eq!(
        managed_value.partial_cmp(&later),
        Some(std::cmp::Ordering::Less)
    );
    assert_eq!(
        later.partial_cmp(&managed_value),
        Some(std::cmp::Ordering::Greater)
    );
    for sign in [crate::NumberSign::Signed, crate::NumberSign::Unsigned] {
        assert_eq!(
            near_max.compare(&managed_value, sign),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            managed_value.compare(&near_max, sign),
            Some(std::cmp::Ordering::Less)
        );
    }
    for pointer in [&base, &managed_value] {
        assert_eq!(
            pointer.partial_cmp(&negative_one),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            pointer.compare(&negative_one, crate::NumberSign::Unsigned),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(
            negative_one.compare(pointer, crate::NumberSign::Unsigned),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            pointer.compare(&negative_one, crate::NumberSign::Signed),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            negative_one.compare(pointer, crate::NumberSign::Signed),
            Some(std::cmp::Ordering::Less)
        );
    }
    // SAFETY: F10.RawMemoryAccessValid — the unmanaged-origin managed pointer
    // observes the initialized byte in the live Box allocation.
    let managed_byte = unsafe { managed.with_data(1, |bytes| bytes[0]) };
    assert_eq!(managed_byte, 0xA5);
    m1b_assert_unmanaged_byte(&base, 0xA5);
    m1b_assert_unmanaged_byte(&base_integer, 0xA5);
    m1b_assert_unmanaged_byte(&later, 0x5A);
    drop(allocation);
}

#[test]
fn m1b_managed_ptr_far_oob_compare_and_difference() {
    with_test_gc_context(|gc_handle| {
        let vector = Vector::new(
            ConcreteType::new(ResolutionS::NULL, BaseType::UInt8),
            ArrayLayoutManager {
                element_layout: Arc::new(LayoutManager::Scalar(Scalar::UInt8)),
                length: 1,
            },
            vec![0xA5],
            vec![1],
        );
        let owner = ObjectRef::new(gc_handle, HeapStorage::Vec(Box::new(vector)));
        let near = ManagedPtr::new(
            owner.pointer(),
            TypeDescription::NULL,
            Some(owner),
            false,
            Some(ByteOffset::new(0)),
        );
        // Construct logical far offsets directly: memory-validation's diagnostic
        // guard is intentionally exercised only by the later arithmetic test.
        let far = ManagedPtr::new(
            None,
            TypeDescription::NULL,
            Some(owner),
            false,
            Some(ByteOffset::new(4096)),
        );
        let far_u32_max = ManagedPtr::new(
            None,
            TypeDescription::NULL,
            Some(owner),
            false,
            Some(ByteOffset::new(u32::MAX as usize)),
        );

        let near_value = crate::StackValue::ManagedPtr(near.clone().into());
        let far_value = crate::StackValue::ManagedPtr(far.clone().into());
        assert_eq!(
            (far_value.clone() - near_value.clone()).unwrap(),
            crate::StackValue::NativeInt(4096)
        );
        assert_eq!(
            (near_value.clone() - far_value.clone()).unwrap(),
            crate::StackValue::NativeInt(-4096)
        );

        let observed_far = far.observe_address().expect("observed far address");
        assert!(far_value == crate::StackValue::NativeInt(observed_far.addr().get() as isize));
        let observed_base = near.observe_address().expect("observed base address");
        assert_eq!(
            far_value.compare(
                &crate::StackValue::NativeInt(observed_base.addr().get() as isize),
                crate::NumberSign::Unsigned,
            ),
            Some(std::cmp::Ordering::Greater)
        );

        let observed_max = far_u32_max
            .observe_address()
            .expect("observed u32::MAX address");
        let far_max_value = crate::StackValue::ManagedPtr(far_u32_max.into());
        assert!(far_max_value == crate::StackValue::NativeInt(observed_max.addr().get() as isize));

        let other_vector = Vector::new(
            ConcreteType::new(ResolutionS::NULL, BaseType::UInt8),
            ArrayLayoutManager {
                element_layout: Arc::new(LayoutManager::Scalar(Scalar::UInt8)),
                length: 1,
            },
            vec![0x5A],
            vec![1],
        );
        let other_owner = ObjectRef::new(gc_handle, HeapStorage::Vec(Box::new(other_vector)));
        let other_far = ManagedPtr::new(
            None,
            TypeDescription::NULL,
            Some(other_owner),
            false,
            Some(ByteOffset::new(4096)),
        );
        let other_far_value = crate::StackValue::ManagedPtr(other_far.clone().into());
        let observed_other_far = other_far
            .observe_address()
            .expect("other observed far address");
        assert_eq!(
            far_value.partial_cmp(&other_far_value),
            Some(observed_far.addr().cmp(&observed_other_far.addr()))
        );
        assert_eq!(
            far_value == other_far_value,
            observed_far.addr() == observed_other_far.addr()
        );

        // SAFETY: F1.GcHandleRooted — `near` retains the live one-byte vector owner,
        // and this access is within its initialized payload.
        let actual = unsafe { near.with_data(1, |bytes| bytes[0]) };
        assert_eq!(actual, 0xA5);
    });
}

#[test]
#[cfg_attr(
    feature = "memory-validation",
    ignore = "M1b managed-pointer arithmetic deliberately forms an out-of-bounds managed address; memory-validation's debug bound trap rejects it by design (F-TEST-003)"
)]
fn m1b_managed_ptr_arithmetic_boundaries_report_explicit_errors() {
    with_test_gc_context(|gc_handle| {
        let vector = Vector::new(
            ConcreteType::new(ResolutionS::NULL, BaseType::UInt8),
            ArrayLayoutManager {
                element_layout: Arc::new(LayoutManager::Scalar(Scalar::UInt8)),
                length: 1,
            },
            vec![0xA5],
            vec![1],
        );
        let owner = ObjectRef::new(gc_handle, HeapStorage::Vec(Box::new(vector)));
        let near = ManagedPtr::new(
            owner.pointer(),
            TypeDescription::NULL,
            Some(owner),
            false,
            Some(ByteOffset::new(0)),
        );
        let value = crate::StackValue::ManagedPtr(near.clone().into());

        let errors = [
            value.clone() + crate::StackValue::Int32(-1),
            crate::StackValue::Int32(i32::MIN) + value.clone(),
            value.clone() - crate::StackValue::NativeInt(1),
            value.clone() + crate::StackValue::NativeInt(isize::MIN),
            value.clone() - crate::StackValue::NativeInt(isize::MIN),
            value.clone() - crate::StackValue::Int32(1),
            value.clone() - crate::StackValue::Int64(1),
            value.clone() - crate::StackValue::Int64(i64::MIN),
        ];
        for result in errors {
            let Err(dotnet_types::error::ExecutionError::NotImplemented(message)) = result else {
                panic!("expected an explicit managed-offset NotImplemented error");
            };
            assert!(
                message.contains("negative")
                    && message.contains("compact")
                    && message.contains("M3")
            );
        }

        if let Ok(max) = isize::try_from(u32::MAX) {
            let far = (crate::StackValue::NativeInt(max) + value.clone()).unwrap();
            assert!(matches!(
                far.clone() + crate::StackValue::Int32(1),
                Err(dotnet_types::error::ExecutionError::NotImplemented(_))
            ));
            let restored = (far - crate::StackValue::Int64(i64::from(u32::MAX))).unwrap();
            let crate::StackValue::ManagedPtr(restored) = restored else {
                panic!("managed-pointer arithmetic changed the StackValue variant");
            };
            // SAFETY: F1.GcHandleRooted — the owner remains live, and subtraction
            // returns the pointer to the initialized one-byte payload.
            assert_eq!(unsafe { restored.with_data(1, |bytes| bytes[0]) }, 0xA5);
        }

        let displaced = (value + crate::StackValue::Int32(4096)).unwrap();
        let displaced = (displaced - crate::StackValue::Int32(1)).unwrap();
        let restored = (displaced - crate::StackValue::NativeInt(4095)).unwrap();
        let crate::StackValue::ManagedPtr(restored) = restored else {
            panic!("managed-pointer arithmetic changed the StackValue variant");
        };
        // SAFETY: F1.GcHandleRooted — `near` and the restored pointer retain the same
        // live one-byte vector owner, and the round trip returns to its valid payload.
        let actual = unsafe { restored.with_data(1, |bytes| bytes[0]) };
        assert_eq!(actual, 0xA5);
    });
}

#[cfg(feature = "multithreading")]
mod m1d_cross_arena {
    use super::*;
    use crate::{ArenaId, object::ObjectPtr, test_helpers::next_test_arena_id};
    use dotnet_utils::{
        gc::{ArenaHandle, GCHandle, register_arena, try_acquire_lease, unregister_arena},
        sync::{AtomicBool, MANAGED_THREAD_ID, Ordering},
    };
    use std::thread;

    type TestRoot = Rootable![()];

    /// A registered arena owning a one-byte vector (`0xA5`), plus the pieces of a
    /// cross-arena pointer to it. Dropping the fields in declaration order is not
    /// relied upon; tests tear down explicitly.
    struct CrossArenaFixture {
        arena: Arena<TestRoot>,
        handle_owner: ArenaHandle,
        arena_id: ArenaId,
        object_ptr: ObjectPtr,
        base: NonNull<u8>,
    }

    impl CrossArenaFixture {
        fn new() -> Self {
            let arena_id = next_test_arena_id();
            register_arena(arena_id, Arc::new(AtomicBool::new(false)));
            MANAGED_THREAD_ID.with(|id| id.set(Some(arena_id)));
            let handle_owner = ArenaHandle::new(arena_id);
            // SAFETY: F1.GcHandleRooted — `handle_owner` outlives every `GCHandle` built from
            // this reference; the fixture only drops it after the arena has been dropped.
            let handle = unsafe {
                std::mem::transmute::<
                    &dotnet_utils::gc::ArenaHandleInner,
                    &'static dotnet_utils::gc::ArenaHandleInner,
                >(handle_owner.as_inner())
            };
            let arena = Arena::<TestRoot>::new(|_mc| ());
            let (object_ptr, base) = arena.mutate(|gc, _root| {
                let gc_handle = GCHandle::new(
                    gc,
                    handle,
                    #[cfg(feature = "memory-validation")]
                    arena_id,
                );
                let vector = Vector::new(
                    ConcreteType::new(ResolutionS::NULL, BaseType::UInt8),
                    ArrayLayoutManager {
                        element_layout: Arc::new(LayoutManager::Scalar(Scalar::UInt8)),
                        length: 1,
                    },
                    vec![0xA5],
                    vec![1],
                );
                let owner = ObjectRef::new(gc_handle, HeapStorage::Vec(Box::new(vector)));
                let base = owner.pointer().expect("heap payload base");
                let (object_ptr, tid) = owner.as_ptr_info().expect("cross-arena pointer info");
                assert_eq!(tid, arena_id);
                (object_ptr, base)
            });
            Self {
                arena,
                handle_owner,
                arena_id,
                object_ptr,
                base,
            }
        }

        fn managed_ptr<'gc>(&self) -> ManagedPtr<'gc> {
            ManagedPtr::new_cross_arena(
                Some(self.base),
                TypeDescription::NULL,
                self.object_ptr,
                self.arena_id,
                ByteOffset::new(0),
            )
        }

        /// Drop the arena (freeing the object) and then its handle.
        fn drop_arena(self) {
            drop(self.arena);
            drop(self.handle_owner);
            MANAGED_THREAD_ID.with(|id| id.set(None));
        }
    }

    #[test]
    fn m1d_cross_arena_access_after_owner_teardown_is_rejected() {
        let fixture = CrossArenaFixture::new();
        let ptr = fixture.managed_ptr();
        unregister_arena(fixture.arena_id);
        fixture.drop_arena();

        // Both accesses reject the unregistered arena without touching the freed object.
        assert_eq!(ptr.observe_address(), None);
        // SAFETY: F1.GcHandleRooted — the absent arena lease prevents any access to the freed owner.
        let byte = unsafe { ptr.try_with_data(1, |data| data[0]) };
        assert_eq!(byte, None);
    }

    #[test]
    fn m1d_cross_arena_lease_holds_teardown_until_access_completes() {
        let fixture = CrossArenaFixture::new();
        let ptr = fixture.managed_ptr();
        let unregistered = Arc::new(AtomicBool::new(false));
        let arena_id = fixture.arena_id;
        let access = |data: &[u8]| {
            let flag = Arc::clone(&unregistered);
            let teardown = thread::spawn(move || {
                unregister_arena(arena_id);
                flag.store(true, Ordering::Release);
            });
            while try_acquire_lease(arena_id).is_some() {
                thread::yield_now();
            }
            assert!(!unregistered.load(Ordering::Acquire));
            assert_eq!(data[0], 0xA5);
            assert_eq!(ptr.observe_address(), None);
            // SAFETY: F1.GcHandleRooted — the absent lease rejects the nested access.
            assert_eq!(unsafe { ptr.try_with_data(1, |_| ()) }, None);
            assert!(!unregistered.load(Ordering::Acquire));
            teardown
        };
        // SAFETY: F1.GcHandleRooted — the per-access lease keeps the owner alive
        // until this closure returns; the fixture drops the arena only after join.
        let teardown = unsafe { ptr.try_with_data(1, access) }
            .expect("registered owner must allow the in-flight read");
        teardown.join().expect("teardown thread");
        assert!(unregistered.load(Ordering::Acquire));
        fixture.drop_arena();
    }

    #[test]
    fn m1d_retained_ptr_cross_arena_lease_blocks_teardown() {
        use crate::{StackValue, TypedReferenceSlot};
        use dotnet_types::error::ExecutionError;

        // Exercise both pointer-like operands accepted by RetainedPtr.
        for typed_ref in [false, true] {
            let fixture = CrossArenaFixture::new();
            let arena_id = fixture.arena_id;
            let managed = fixture.managed_ptr();
            let fresh = managed.clone();
            let after_teardown = managed.clone();
            let value = if typed_ref {
                StackValue::TypedRef(TypedReferenceSlot::new(
                    managed.into(),
                    Arc::new(TypeDescription::NULL),
                ))
            } else {
                StackValue::ManagedPtr(managed.into())
            };
            let retained = RetainedPtr::try_from_stack_value(value).expect("registered owner");
            let unregistered = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&unregistered);
            let teardown = thread::spawn(move || {
                unregister_arena(arena_id);
                flag.store(true, Ordering::Release);
            });
            while try_acquire_lease(arena_id).is_some() {
                thread::yield_now();
            }
            assert!(!unregistered.load(Ordering::Acquire));
            // SAFETY: F10.RawMemoryAccessValid — the retained lease keeps the
            // fixture's initialized one-byte payload live and no other thread
            // accesses it; the fixture does not collect during this copy.
            let byte = unsafe { retained.as_ptr().read() };
            assert_eq!(byte, 0xA5);
            let fresh = if typed_ref {
                StackValue::TypedRef(TypedReferenceSlot::new(
                    fresh.into(),
                    Arc::new(TypeDescription::NULL),
                ))
            } else {
                StackValue::ManagedPtr(fresh.into())
            };
            assert!(matches!(
                RetainedPtr::try_from_stack_value(fresh),
                Err(ExecutionError::InternalError(_))
            ));
            assert!(!unregistered.load(Ordering::Acquire));
            drop(retained);
            teardown.join().expect("teardown thread");
            assert!(unregistered.load(Ordering::Acquire));
            fixture.drop_arena();
            assert!(matches!(
                RetainedPtr::try_from_stack_value(StackValue::ManagedPtr(after_teardown.into())),
                Err(ExecutionError::InternalError(_))
            ));
        }
    }

    /// F-GUARD-101 / 5.7: the pre-repair `SpanHelpers.Fill` byte fast path observed the
    /// destination address under `observe_address`'s temporary lease and then wrote with
    /// only the `StackValue` retained. Teardown after observation leaves the write
    /// dangling. Known-dangling by construction: run under pinned Miri `--ignored` only.
    #[test]
    #[ignore = "F-GUARD-101: documents the pre-repair Fill fast-path pattern (observe, then write unprotected); known-dangling, Miri --ignored only"]
    #[allow(
        clippy::undocumented_unsafe_blocks,
        reason = "intentional UB diagnostic: no safety predicate holds, so a `// SAFETY:` registry citation would be false"
    )]
    fn m1d_fill_fast_path_observed_pointer_dangles_after_teardown() {
        let fixture = CrossArenaFixture::new();
        let observed = fixture
            .managed_ptr()
            .observe_address()
            .expect("registered owner");
        unregister_arena(fixture.arena_id);
        fixture.drop_arena();
        // KNOWN-UB (deliberately not a SAFETY claim): the owner arena and payload are freed; Miri must flag this write.
        unsafe { observed.as_ptr().write(7) };
    }

    /// F-GUARD-103: validation must accept an object access made under a lease
    /// acquired before teardown removed the registry entry, while a fresh
    /// lease is still refused.
    #[test]
    #[cfg(feature = "memory-validation")]
    fn m1d_cross_arena_validation_accepts_in_flight_lease() {
        let fixture = CrossArenaFixture::new();
        let arena_id = fixture.arena_id;
        let object_ptr = fixture.object_ptr;
        // The fixture thread is the owner; pretend to be another thread so the
        // cross-arena validation path runs.
        MANAGED_THREAD_ID.with(|id| id.set(Some(next_test_arena_id())));
        let lease = try_acquire_lease(arena_id).expect("registered owner");
        let teardown = thread::spawn(move || unregister_arena(arena_id));
        while try_acquire_lease(arena_id).is_some() {
            thread::yield_now();
        }
        assert_eq!(object_ptr.as_heap_storage(|s| s.payload_len()), 1);
        drop(lease);
        teardown.join().expect("teardown thread");
        fixture.drop_arena();
    }
}
