#![no_main]
use arbitrary::Arbitrary;
use dotnet_utils::ByteOffset;
use dotnet_value::{
    StackSlotIndex,
    pointer::{ManagedPtr, ManagedPtrResolver, PointerOrigin, StaticMetadata},
};
use libfuzzer_sys::fuzz_target;
use std::ptr::NonNull;

const STORAGE_LEN: usize = 256;

/// Selects a dedicated valid case or sends raw bytes to the safe rebase
/// boundary. The raw-byte case intentionally includes truncated and malformed
/// encodings without constructing a managed Heap or CrossArena handle.
#[derive(Arbitrary, Debug)]
enum RebaseInput {
    ValidStack { slot: u16, offset: u8 },
    ValidNonStack,
    ArbitraryBytes(Vec<u8>),
}

struct StackResolver {
    base: NonNull<u8>,
}

impl<'gc> ManagedPtrResolver<'gc> for StackResolver {
    fn stack_slot_base(&self, _slot: StackSlotIndex) -> Option<NonNull<u8>> {
        Some(self.base)
    }

    fn static_storage_base(&self, _metadata: &StaticMetadata) -> Option<NonNull<u8>> {
        None
    }
}

fn fuzz_valid_stack(slot: u16, offset: u8) {
    let offset = usize::from(offset);
    let mut old_storage = [0u8; STORAGE_LEN];
    let old_base = NonNull::new(old_storage.as_mut_ptr()).expect("array pointers are non-null");
    let mut new_storage = [0u8; STORAGE_LEN];
    let new_base = NonNull::new(new_storage.as_mut_ptr()).expect("array pointers are non-null");
    assert_ne!(old_base, new_base, "live backing arrays must have distinct bases");
    let slot = StackSlotIndex::new(usize::from(slot));

    let address = NonNull::new(old_base.as_ptr().wrapping_add(offset))
        .expect("in-bounds array offset is non-null");
    let ptr = ManagedPtr::new(
        Some(address),
        dotnet_types::TypeDescription::NULL,
        None,
        false,
        Some(ByteOffset::new(offset)),
    )
    .with_stack_origin(slot);
    let mut bytes = ManagedPtr::serialization_buffer();
    ptr.write(&mut bytes);
    let original = bytes;

    assert_eq!(
        ManagedPtr::rebase_stack_pointer(&mut bytes, |resolved_slot| {
            assert_eq!(resolved_slot, slot);
            Some(new_base)
        }),
        Ok(true)
    );
    assert_eq!(
        bytes, original,
        "Stack rebase must preserve the three-word wire representation"
    );

    // SAFETY: F3.InteriorPointerRebased — `bytes` was written as one complete
    // Stack pointer and the resolver supplies the bounded new backing array.
    let resolved = unsafe {
        ManagedPtr::read_resolved_unchecked(&bytes, &StackResolver { base: new_base })
    }
    .expect("rebased Stack pointer must resolve against the new base");
    assert_eq!(resolved.origin, PointerOrigin::Stack(slot));
    assert_eq!(resolved.offset, ByteOffset::new(offset));
    assert_eq!(
        resolved.address,
        NonNull::new(new_base.as_ptr().wrapping_add(offset)),
        "resolved address must use the new base plus the serialized offset"
    );
}

fn fuzz_valid_non_stack() {
    let mut storage = [0u8; 1];
    let ptr = ManagedPtr::new(
        NonNull::new(storage.as_mut_ptr()),
        dotnet_types::TypeDescription::NULL,
        None,
        false,
        None,
    );
    let mut bytes = ManagedPtr::serialization_buffer();
    ptr.write(&mut bytes);
    let original = bytes;

    assert_eq!(
        ManagedPtr::rebase_stack_pointer(&mut bytes, |_| {
            panic!("valid non-Stack rebase must not invoke its resolver")
        }),
        Ok(false)
    );
    assert_eq!(bytes, original, "non-Stack bytes must be preserved");
}

fn fuzz_arbitrary_bytes(mut bytes: Vec<u8>) {
    let original = bytes.clone();
    let mut new_storage = [0u8; STORAGE_LEN];
    let new_base = NonNull::new(new_storage.as_mut_ptr()).expect("array pointers are non-null");

    // The safe API may return any typed decode error for raw fuzz input, but
    // it must not panic, read beyond the supplied slice, or mutate it.
    let _ = ManagedPtr::rebase_stack_pointer(&mut bytes, |_| Some(new_base));
    assert_eq!(bytes, original, "rebase must preserve arbitrary input bytes");
}

fuzz_target!(|input: RebaseInput| match input {
    RebaseInput::ValidStack { slot, offset } => fuzz_valid_stack(slot, offset),
    RebaseInput::ValidNonStack => fuzz_valid_non_stack(),
    RebaseInput::ArbitraryBytes(bytes) => fuzz_arbitrary_bytes(bytes),
});
