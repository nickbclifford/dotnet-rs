#![no_main]
use arbitrary::{Arbitrary, Unstructured};
use dotnet_value::pointer::{ManagedPtr, ManagedPtrInfo, PointerOrigin};
use libfuzzer_sys::fuzz_target;

/// A `ManagedPtr` together with an offset delta that can be safely applied by
/// this target.
///
/// Managed origins store their offsets in a compact `u32` representation.
/// Malformed fuzz input exceeding that range must be rejected before
/// `ManagedPtr::from_info_full` attempts to construct the pointer.
#[derive(Debug)]
struct OffsetInput {
    ptr: ManagedPtr<'static>,
    offset_delta_i32: i32,
}

impl<'a> Arbitrary<'a> for OffsetInput {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let info: ManagedPtrInfo<'static> = u.arbitrary()?;
        let pinned = u.arbitrary()?;

        if !matches!(&info.origin, PointerOrigin::Unmanaged)
            && info.offset.as_usize() > u32::MAX as usize
        {
            return Err(arbitrary::Error::IncorrectFormat);
        }

        Ok(Self {
            ptr: ManagedPtr::from_info_full(info, u.arbitrary()?, pinned),
            offset_delta_i32: u.arbitrary()?,
        })
    }
}

fuzz_target!(|input: OffsetInput| {
    let OffsetInput {
        ptr,
        offset_delta_i32,
    } = input;
    let offset_delta = offset_delta_i32 as isize;

    let initial_offset = ptr.byte_offset().as_usize() as isize;

    // Avoid expected panics.
    let Some(new_offset) = initial_offset.checked_add(offset_delta) else {
        return;
    };
    if new_offset < 0 {
        return;
    }
    if !matches!(ptr.origin(), PointerOrigin::Unmanaged) && new_offset > u32::MAX as isize {
        return;
    }

    // `ManagedPtr::offset` rejects wrapping an address to null.
    if let Some(address) = ptr.clone().into_info().address
        && address.as_ptr().wrapping_offset(offset_delta).is_null()
    {
        return;
    }

    let original_ptr = ptr.clone();
    // SAFETY: F3.InteriorPointerRebased — the target prevalidates offset and address arithmetic, while
    // `ManagedPtr::offset` uses wrapping pointer arithmetic.
    let new_ptr = unsafe { ptr.offset(offset_delta) };

    assert_eq!(
        new_ptr.origin(),
        original_ptr.origin(),
        "Origin must be preserved across offset"
    );

    let expected_offset = if matches!(original_ptr.origin(), PointerOrigin::Unmanaged)
        && new_offset > u32::MAX as isize
    {
        original_ptr
            .clone()
            .into_info()
            .address
            .map_or(0, |address| {
                address.as_ptr().wrapping_offset(offset_delta).addr()
            })
    } else {
        new_offset as usize
    };
    assert_eq!(
        new_ptr.byte_offset().as_usize(),
        expected_offset,
        "Offset mismatch"
    );

    if let Some(orig_addr) = original_ptr.into_info().address {
        let expected_addr = orig_addr.as_ptr().wrapping_offset(offset_delta).addr();
        assert_eq!(
            new_ptr.into_info().address.map(|p| p.as_ptr() as usize),
            Some(expected_addr),
            "Address mismatch"
        );
    }
});
