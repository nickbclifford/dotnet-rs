#![no_main]
use arbitrary::Arbitrary;
use dotnet_utils::{ByteOffset, StackSlotIndex};
use dotnet_value::pointer::ManagedPtr;
use libfuzzer_sys::fuzz_target;
use std::ptr::NonNull;

const STORAGE_LEN: usize = 256;

/// A `ManagedPtr` together with an offset delta that can be safely applied by
/// this target.
///
/// The input selects an origin class, but never supplies an address or a GC
/// handle. The target builds every pointer from live local storage, so an
/// assertion failure cannot format or otherwise dereference a fuzz-crafted
/// pointer while reporting the failure.
#[derive(Arbitrary, Debug)]
struct OffsetInput {
    origin: OffsetOrigin,
    initial_offset: u8,
    offset_delta: i16,
}

#[derive(Arbitrary, Debug)]
enum OffsetOrigin {
    Unmanaged,
    Stack { slot: u16 },
    Static,
}

fuzz_target!(|input: OffsetInput| {
    let initial_offset = usize::from(input.initial_offset);
    let offset_delta = isize::from(input.offset_delta);
    let Some(new_offset) = initial_offset.checked_add_signed(offset_delta) else {
        return;
    };
    // Keep the whole operation within this live allocation. This makes the
    // provenance assertion below meaningful without dereferencing the result.
    if new_offset >= STORAGE_LEN {
        return;
    }

    let mut storage = [0u8; STORAGE_LEN];
    let base = NonNull::new(storage.as_mut_ptr()).expect("array pointers are non-null");
    let address = NonNull::new(base.as_ptr().wrapping_add(initial_offset))
        .expect("in-bounds array offset is non-null");
    let ptr = match input.origin {
        OffsetOrigin::Unmanaged => ManagedPtr::new(
            Some(address),
            dotnet_types::TypeDescription::NULL,
            None,
            false,
            Some(ByteOffset::new(initial_offset)),
        ),
        OffsetOrigin::Stack { slot } => ManagedPtr::new(
            Some(address),
            dotnet_types::TypeDescription::NULL,
            None,
            false,
            Some(ByteOffset::new(initial_offset)),
        )
        .with_stack_origin(StackSlotIndex::new(usize::from(slot))),
        OffsetOrigin::Static => ManagedPtr::new_static(
            Some(address),
            dotnet_types::TypeDescription::NULL,
            dotnet_types::TypeDescription::NULL,
            dotnet_types::generics::GenericLookup::default(),
            false,
            ByteOffset::new(initial_offset),
        ),
    };

    let original_ptr = ptr.clone();
    // SAFETY: F3.InteriorPointerRebased — `ptr` was derived from `storage`, and
    // the checked result remains within that allocation for this target.
    let new_ptr = unsafe { ptr.offset(offset_delta) };

    assert_eq!(
        new_ptr.origin(),
        original_ptr.origin(),
        "Offset must preserve the provenance-carrying origin"
    );
    assert_eq!(
        new_ptr.byte_offset().as_usize(),
        new_offset,
        "Offset mismatch"
    );

    let expected_addr = original_ptr
        .into_info()
        .address
        .expect("fixture pointer has an address")
        .as_ptr()
        .wrapping_offset(offset_delta)
        .addr();
    assert_eq!(
        new_ptr
            .into_info()
            .address
            .expect("offset pointer has an address")
            .as_ptr()
            .addr(),
        expected_addr,
        "Offset must retain the live allocation's address provenance"
    );
});
