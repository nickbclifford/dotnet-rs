#![no_main]
use arbitrary::Arbitrary;
use dotnet_utils::ByteOffset;
use dotnet_value::pointer::ManagedPtr;
use dotnet_value::pointer::{FrameHomeSerial, PointerOrigin};
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
    FrameHome { slot: u16 },
    Static,
}

fuzz_target!(|input: OffsetInput| {
    let initial_offset = usize::from(input.initial_offset);
    let offset_delta = isize::from(input.offset_delta);
    let Some(new_offset) = initial_offset.checked_add_signed(offset_delta) else {
        return;
    };
    // Keep the whole operation within this live allocation so the returned
    // pointer can be checked with an actual read below.
    if new_offset >= STORAGE_LEN {
        return;
    }

    let mut storage = [0u8; STORAGE_LEN];
    for (index, byte) in storage.iter_mut().enumerate() {
        *byte = (index % 251) as u8;
    }
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
        OffsetOrigin::FrameHome { slot } => ManagedPtr::new(
            Some(address),
            dotnet_types::TypeDescription::NULL,
            None,
            false,
            Some(ByteOffset::new(initial_offset)),
        )
        .with_origin(PointerOrigin::FrameHome(FrameHomeSerial::new(
            u64::from(slot) + 1,
        ))),
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
    // SAFETY: F10.RawMemoryAccessValid — `ptr` was derived from `storage`, and
    // the checked result remains within that allocation for this target.
    let new_ptr = unsafe { ptr.offset(offset_delta) };

    assert_eq!(
        new_ptr.origin(),
        original_ptr.origin(),
        "Offset must preserve the provenance-carrying origin"
    );
    assert_eq!(
        new_ptr.byte_offset().unwrap().as_usize(),
        new_offset,
        "Offset mismatch"
    );

    let expected_addr = base.as_ptr().wrapping_add(new_offset).addr();
    let expected_byte = storage[new_offset];
    let result_address = new_ptr
        .into_info()
        .address
        .expect("offset pointer has an address");
    assert_eq!(
        result_address.as_ptr().addr(),
        expected_addr,
        "Offset must produce the expected address within the live allocation"
    );
    // SAFETY: F10.RawMemoryAccessValid — the target bounded `new_offset` to the
    // live `storage` array and `result_address` is the in-bounds result of
    // offsetting a pointer derived from that array.
    let actual_byte = unsafe { *result_address.as_ptr() };
    assert_eq!(
        actual_byte, expected_byte,
        "offset result must read live storage"
    );
});
