#![no_main]
use dotnet_value::pointer::ManagedPtr;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: (ManagedPtr<'static>, i32)| {
    let (ptr, offset_delta_i32) = data;
    let offset_delta = offset_delta_i32 as isize;
    
    let initial_offset = ptr.offset.as_usize() as isize;
    
    // Avoid expected panics
    if initial_offset.checked_add(offset_delta).is_none() { return; }
    if initial_offset + offset_delta < 0 { return; }
    
    // Also avoid address overflow if address is present
    if let Some(addr) = ptr._value {
        let addr_val = addr.as_ptr() as usize;
        if (addr_val as isize).checked_add(offset_delta).is_none() { return; }
    }

    let original_ptr = ptr.clone();
    let new_ptr = unsafe { ptr.offset(offset_delta) };
    
    assert_eq!(new_ptr.offset.as_usize(), (initial_offset + offset_delta) as usize, "Offset mismatch");
    
    if let Some(orig_addr) = original_ptr._value {
        let expected_addr = (orig_addr.as_ptr() as usize as isize + offset_delta) as usize;
        assert_eq!(new_ptr._value.map(|p| p.as_ptr() as usize), Some(expected_addr), "Address mismatch");
    }
});
