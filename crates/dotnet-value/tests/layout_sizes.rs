use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectInner},
    pointer::{ManagedPtr, PointerOrigin},
};
use std::mem::{align_of, size_of};

#[cfg(all(
    target_pointer_width = "64",
    any(debug_assertions, feature = "memory-validation")
))]
const EXPECTED_STACK_VALUE_SIZE: usize = 128;
#[cfg(all(
    target_pointer_width = "64",
    any(debug_assertions, feature = "memory-validation"),
    feature = "multithreading"
))]
const EXPECTED_MANAGED_PTR_SIZE: usize = 80;
#[cfg(all(
    target_pointer_width = "64",
    any(debug_assertions, feature = "memory-validation"),
    not(feature = "multithreading")
))]
const EXPECTED_MANAGED_PTR_SIZE: usize = 72;

#[cfg(all(
    target_pointer_width = "64",
    not(any(debug_assertions, feature = "memory-validation"))
))]
const EXPECTED_STACK_VALUE_SIZE: usize = 120;
#[cfg(all(
    target_pointer_width = "64",
    not(any(debug_assertions, feature = "memory-validation")),
    feature = "multithreading"
))]
const EXPECTED_MANAGED_PTR_SIZE: usize = 72;
#[cfg(all(
    target_pointer_width = "64",
    not(any(debug_assertions, feature = "memory-validation")),
    not(feature = "multithreading")
))]
const EXPECTED_MANAGED_PTR_SIZE: usize = 64;
#[cfg(all(target_pointer_width = "64", feature = "multithreading"))]
const EXPECTED_POINTER_ORIGIN_SIZE: usize = 24;
#[cfg(all(target_pointer_width = "64", not(feature = "multithreading")))]
const EXPECTED_POINTER_ORIGIN_SIZE: usize = 16;
#[cfg(all(
    target_pointer_width = "64",
    any(feature = "memory-validation", debug_assertions),
    any(feature = "multithreading", feature = "memory-validation")
))]
const EXPECTED_OBJECT_INNER_OVERHEAD: usize = 16;
#[cfg(all(
    target_pointer_width = "64",
    any(feature = "memory-validation", debug_assertions),
    not(any(feature = "multithreading", feature = "memory-validation"))
))]
const EXPECTED_OBJECT_INNER_OVERHEAD: usize = 8;
#[cfg(all(
    target_pointer_width = "64",
    not(any(feature = "memory-validation", debug_assertions)),
    any(feature = "multithreading", feature = "memory-validation")
))]
const EXPECTED_OBJECT_INNER_OVERHEAD: usize = 8;
#[cfg(all(
    target_pointer_width = "64",
    not(any(feature = "memory-validation", debug_assertions)),
    not(any(feature = "multithreading", feature = "memory-validation"))
))]
const EXPECTED_OBJECT_INNER_OVERHEAD: usize = 0;

#[cfg(target_pointer_width = "64")]
const _: () = {
    const EXPECTED_OBJECT_INNER_SIZE: usize =
        size_of::<HeapStorage<'static>>() + EXPECTED_OBJECT_INNER_OVERHEAD;
    assert!(size_of::<StackValue<'static>>() == EXPECTED_STACK_VALUE_SIZE);
    assert!(size_of::<ManagedPtr<'static>>() == EXPECTED_MANAGED_PTR_SIZE);
    assert!(size_of::<PointerOrigin<'static>>() == EXPECTED_POINTER_ORIGIN_SIZE);
    assert!(size_of::<ObjectInner<'static>>() == EXPECTED_OBJECT_INNER_SIZE);
    assert!(align_of::<StackValue<'static>>() == 8);
    assert!(align_of::<ManagedPtr<'static>>() == 8);
    assert!(align_of::<PointerOrigin<'static>>() == 8);
    assert!(align_of::<ObjectInner<'static>>() == 8);
};

#[test]
fn layout_sizes_match_expected() {
    #[cfg(target_pointer_width = "64")]
    {
        let expected_object_inner_size =
            size_of::<HeapStorage<'static>>() + EXPECTED_OBJECT_INNER_OVERHEAD;
        assert_eq!(size_of::<StackValue<'static>>(), EXPECTED_STACK_VALUE_SIZE);
        assert_eq!(size_of::<ManagedPtr<'static>>(), EXPECTED_MANAGED_PTR_SIZE);
        assert_eq!(
            size_of::<PointerOrigin<'static>>(),
            EXPECTED_POINTER_ORIGIN_SIZE
        );
        assert_eq!(
            size_of::<ObjectInner<'static>>(),
            expected_object_inner_size
        );
        assert_eq!(align_of::<StackValue<'static>>(), 8);
        assert_eq!(align_of::<ManagedPtr<'static>>(), 8);
        assert_eq!(align_of::<PointerOrigin<'static>>(), 8);
        assert_eq!(align_of::<ObjectInner<'static>>(), 8);
    }
}
