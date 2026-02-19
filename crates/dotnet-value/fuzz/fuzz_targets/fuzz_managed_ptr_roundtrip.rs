#![no_main]
use dotnet_value::pointer::{ManagedPtr, ManagedPtrInfo};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|info: ManagedPtrInfo<'static>| {
    let mut buf = [0u8; ManagedPtr::SIZE];
    
    // Create a ManagedPtr from info to use its write method
    let ptr = ManagedPtr {
        #[cfg(debug_assertions)]
        magic: 0x504F_494E,
        _value: info.address,
        origin: info.origin.clone(),
        offset: info.offset,
        inner_type: dotnet_types::TypeDescription::NULL,
        pinned: false,
        _marker: std::marker::PhantomData,
    };
    
    ptr.write(&mut buf);
    
    // Now read it back
    let read_info = unsafe { ManagedPtr::read_unchecked(&buf) }.expect("Roundtrip read failed");
    
    // Normalize origins for comparison as some variants (like Transient) are serialized with loss
    let info_norm = info.origin.normalize();
    let read_info_norm = read_info.origin.normalize();
    
    assert_eq!(read_info_norm, info_norm, "Origin mismatch");
    
    // Offset is reconstructed correctly for non-unmanaged origins.
    // For Unmanaged, offset is reconstructed from word1 (address).
    if !matches!(info_norm, dotnet_value::pointer::PointerOrigin::Unmanaged) {
        assert_eq!(read_info.offset, info.offset, "Offset mismatch");
    }
});
