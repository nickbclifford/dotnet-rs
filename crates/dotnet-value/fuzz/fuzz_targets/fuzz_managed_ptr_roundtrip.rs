#![no_main]
use arbitrary::Arbitrary;
use dotnet_utils::{
    ArenaId, ByteOffset,
    gc::{ArenaHandle, ArenaHandleInner, GCHandle, register_arena, unregister_arena},
    sync::{Arc, AtomicBool, MANAGED_THREAD_ID},
};
use dotnet_value::{
    CLRString, HeapStorage, ObjectRef, StackSlotIndex,
    pointer::{ManagedPtr, ManagedPtrResolver, PointerOrigin, StaticMetadata},
};
use gc_arena::{Arena, Rootable};
use libfuzzer_sys::fuzz_target;
use std::ptr::NonNull;

const STORAGE_LEN: usize = 256;
const FUZZ_ARENA_ID: ArenaId = ArenaId::new(1);

struct ArenaRegistrationGuard {
    arena_id: ArenaId,
}

impl ArenaRegistrationGuard {
    fn register(arena_id: ArenaId) -> Self {
        register_arena(arena_id, Arc::new(AtomicBool::new(false)));
        Self { arena_id }
    }
}

impl Drop for ArenaRegistrationGuard {
    fn drop(&mut self) {
        unregister_arena(self.arena_id);
    }
}

struct ManagedThreadIdGuard {
    previous: Option<ArenaId>,
}

impl ManagedThreadIdGuard {
    fn set(arena_id: ArenaId) -> Self {
        let previous = MANAGED_THREAD_ID.with(|thread_id| {
            let previous = thread_id.get();
            thread_id.set(Some(arena_id));
            previous
        });
        Self { previous }
    }
}

impl Drop for ManagedThreadIdGuard {
    fn drop(&mut self) {
        MANAGED_THREAD_ID.with(|thread_id| thread_id.set(self.previous));
    }
}

/// Resolves the target-local Stack and Static origins from their live backing
/// storage so the roundtrip assertion can exercise executable deserialization.
struct FuzzResolver {
    storage_base: NonNull<u8>,
}

impl<'gc> ManagedPtrResolver<'gc> for FuzzResolver {
    fn stack_slot_base(&self, _slot: StackSlotIndex) -> Option<NonNull<u8>> {
        Some(self.storage_base)
    }

    fn static_storage_base(&self, _metadata: &StaticMetadata) -> Option<NonNull<u8>> {
        Some(self.storage_base)
    }
}

/// Runs a fuzz case with a live GC arena and its registered owner ID.
///
/// Keeping the arena handle, registration, and managed thread ID alive for the
/// callback lets Heap and CrossArena cases serialize only valid handles.
fn with_fuzz_gc_context<R>(f: impl for<'gc> FnOnce(GCHandle<'gc>) -> R) -> R {
    type FuzzRoot = Rootable![()];

    let arena = Arena::<FuzzRoot>::new(|_mc| ());
    let _arena_registration = ArenaRegistrationGuard::register(FUZZ_ARENA_ID);
    let _thread_id = ManagedThreadIdGuard::set(FUZZ_ARENA_ID);
    let arena_handle_owner = ArenaHandle::new(FUZZ_ARENA_ID);
    // SAFETY: F1.GcHandleRooted — `arena_handle_owner` remains alive for the entire `arena.mutate`
    // callback, so its inner handle is valid for the constructed `GCHandle`.
    let arena_handle = unsafe {
        std::mem::transmute::<&ArenaHandleInner, &'static ArenaHandleInner>(
            arena_handle_owner.as_inner(),
        )
    };

    arena.mutate(|gc, _root| f(GCHandle::new(gc, arena_handle)))
}

/// Inputs whose managed origins can be backed by live target-local storage.
///
/// Heap and CrossArena variants are constructed from the live fixture below;
/// their fuzz input supplies only an in-bounds offset, never a raw handle.
/// Transient origins are intentionally non-deserializable, so they remain
/// outside this success-roundtrip target.
#[derive(Arbitrary, Debug)]
enum RoundtripInput {
    Unmanaged { address: usize },
    Stack { slot: u16, offset: u8 },
    Static { offset: u8 },
    Heap { offset: u8 },
    CrossArena { offset: u8 },
}

fuzz_target!(|input: RoundtripInput| {
    with_fuzz_gc_context(|gc| {
        let mut buf = [0u8; ManagedPtr::SIZE];
        let mut storage = [0u8; STORAGE_LEN];
        let storage_base = NonNull::new(storage.as_mut_ptr()).expect("array pointers are non-null");

        // The string supplies 512 bytes of live GC-owned storage, so every
        // `u8` offset remains in bounds for both GC-backed origin variants.
        let object = ObjectRef::new(gc, HeapStorage::Str(CLRString::new(vec![0; STORAGE_LEN])));
        let (cross_arena_object, cross_arena_id) = object
            .as_ptr_info()
            .expect("live fuzz object must have an arena-backed pointer");

        let ptr = match input {
            RoundtripInput::Unmanaged { address } => ManagedPtr::new(
                NonNull::new(std::ptr::without_provenance_mut(address)),
                dotnet_types::TypeDescription::NULL,
                None,
                false,
                None,
            ),
            RoundtripInput::Stack { slot, offset } => {
                let offset = usize::from(offset);
                let address = NonNull::new(storage_base.as_ptr().wrapping_add(offset))
                    .expect("in-bounds array offset is non-null");
                ManagedPtr::new(
                    Some(address),
                    dotnet_types::TypeDescription::NULL,
                    None,
                    false,
                    Some(ByteOffset::new(offset)),
                )
                .with_stack_origin(StackSlotIndex::new(usize::from(slot)))
            }
            RoundtripInput::Static { offset } => {
                let offset = usize::from(offset);
                let address = NonNull::new(storage_base.as_ptr().wrapping_add(offset))
                    .expect("in-bounds array offset is non-null");
                ManagedPtr::new_static(
                    Some(address),
                    dotnet_types::TypeDescription::NULL,
                    dotnet_types::TypeDescription::NULL,
                    dotnet_types::generics::GenericLookup::default(),
                    false,
                    ByteOffset::new(offset),
                )
            }
            RoundtripInput::Heap { offset } => {
                let offset = usize::from(offset);
                let address = object.with_data(|data| {
                    NonNull::new(data.as_ptr().wrapping_add(offset).cast_mut())
                        .expect("object data pointers are non-null")
                });
                ManagedPtr::new(
                    Some(address),
                    dotnet_types::TypeDescription::NULL,
                    Some(object),
                    false,
                    Some(ByteOffset::new(offset)),
                )
            }
            RoundtripInput::CrossArena { offset } => {
                let offset = usize::from(offset);
                let address = object.with_data(|data| {
                    NonNull::new(data.as_ptr().wrapping_add(offset).cast_mut())
                        .expect("object data pointers are non-null")
                });
                ManagedPtr::new_cross_arena(
                    Some(address),
                    dotnet_types::TypeDescription::NULL,
                    cross_arena_object,
                    cross_arena_id,
                    ByteOffset::new(offset),
                )
            }
        };
        let info = ptr.clone().into_info();

        ptr.write(&mut buf);

        let ptr_size = ObjectRef::SIZE;
        let word0 = usize::from_ne_bytes(buf[..ptr_size].try_into().expect("word size is exact"));
        let word1 = usize::from_ne_bytes(
            buf[ptr_size..ptr_size * 2]
                .try_into()
                .expect("word size is exact"),
        );
        let word2 = usize::from_ne_bytes(
            buf[ptr_size * 2..ptr_size * 3]
                .try_into()
                .expect("word size is exact"),
        );
        assert_eq!(word2, word0 ^ word1, "Serialized checksum mismatch");

        let expected_tag = match &info.origin {
            PointerOrigin::Heap(_) | PointerOrigin::Unmanaged => 0,
            PointerOrigin::Stack(_) => 1,
            PointerOrigin::Static(_) | PointerOrigin::Transient(_) => 7,
            PointerOrigin::CrossArenaObjectRef(_, _) => 5,
        };
        assert_eq!(word0 & 7, expected_tag, "Serialized origin tag mismatch");

        // Now read it back.
        // SAFETY: F3.InteriorPointerRebased — `buf` was just populated with a complete ManagedPtr encoding.
        let read_info =
            unsafe { ManagedPtr::read_metadata_unchecked(&buf) }.expect("Roundtrip read failed");

        let info_norm = info.origin.normalize();
        let read_info_norm = read_info.origin.normalize();

        assert_eq!(read_info_norm, info_norm, "Origin mismatch");

        // Offset is reconstructed correctly for non-unmanaged origins.
        // For Unmanaged, offset is reconstructed from word1 (address).
        if !matches!(info_norm, PointerOrigin::Unmanaged) {
            assert_eq!(read_info.offset, info.offset, "Offset mismatch");
        }

        if !matches!(info_norm, PointerOrigin::Unmanaged) {
            let resolver = FuzzResolver { storage_base };
            // SAFETY: F3.InteriorPointerRebased — `buf` was just populated with a complete ManagedPtr
            // encoding, and the resolver supplies the live target-local bases.
            let resolved_info = unsafe { ManagedPtr::read_resolved_unchecked(&buf, &resolver) }
                .expect("Resolved roundtrip read failed");
            let resolved_ptr = ManagedPtr::from_info_full(
                resolved_info,
                dotnet_types::TypeDescription::NULL,
                false,
            );
            assert_eq!(
                resolved_ptr.into_info().address.map(|address| address.as_ptr().addr()),
                info.address.map(|address| address.as_ptr().addr()),
                "Resolved address mismatch"
            );
        }
    });
});
