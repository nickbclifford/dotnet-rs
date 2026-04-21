use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectInner, ObjectRef},
    pointer::{ManagedPtr, PointerOrigin},
};
use std::mem::{align_of, size_of};

#[cfg(target_pointer_width = "64")]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct LayoutExpectations {
    stack_value_size: usize,
    managed_ptr_size: usize,
    pointer_origin_size: usize,
    object_inner_overhead: usize,
    object_ref_size: usize,
    option_object_ref_size: usize,
}

#[cfg(target_pointer_width = "64")]
const STACK_VALUE_SIZE_MATRIX: [(bool, usize); 2] = [
    // (has_validation_tag, expected_size)
    (false, 104),
    (true, 112),
];

#[cfg(target_pointer_width = "64")]
const MANAGED_PTR_SIZE_MATRIX: [((bool, bool), usize); 4] = [
    // ((has_validation_tag, multithreading), expected_size)
    ((false, false), 56),
    ((false, true), 64),
    ((true, false), 64),
    ((true, true), 72),
];

#[cfg(target_pointer_width = "64")]
const OBJECT_INNER_OVERHEAD_MATRIX: [((bool, bool), usize); 4] = [
    // ((has_validation_tag, has_owner_id), expected_overhead)
    ((false, false), 8),
    ((false, true), 16),
    ((true, false), 16),
    ((true, true), 24),
];

#[cfg(target_pointer_width = "64")]
const POINTER_ORIGIN_SIZE_MATRIX: [(bool, usize); 2] = [
    // (multithreading, expected_size)
    (false, 16),
    (true, 24),
];

#[cfg(target_pointer_width = "64")]
const OBJECT_REF_SIZE_MATRIX: [(bool, usize); 2] = [
    // (multithreading, expected_size)
    (false, 8),
    (true, 8),
];

#[cfg(target_pointer_width = "64")]
const OPTION_OBJECT_REF_SIZE_MATRIX: [(bool, usize); 2] = [
    // (multithreading, expected_size)
    (false, 16),
    (true, 16),
];

#[cfg(target_pointer_width = "64")]
const fn lookup_bool2(
    matrix: &[((bool, bool), usize)],
    a: bool,
    b: bool,
    fallback: usize,
) -> usize {
    let mut i = 0;
    while i < matrix.len() {
        let ((m_a, m_b), value) = matrix[i];
        if m_a == a && m_b == b {
            return value;
        }
        i += 1;
    }
    fallback
}

#[cfg(target_pointer_width = "64")]
const fn lookup_bool(matrix: &[(bool, usize)], key: bool, fallback: usize) -> usize {
    let mut i = 0;
    while i < matrix.len() {
        let (matrix_key, value) = matrix[i];
        if matrix_key == key {
            return value;
        }
        i += 1;
    }
    fallback
}

#[cfg(target_pointer_width = "64")]
const fn expected_layouts() -> LayoutExpectations {
    let has_validation_tag = cfg!(any(debug_assertions, feature = "memory-validation"));
    let multithreading = cfg!(feature = "multithreading");
    let has_owner_id = cfg!(any(
        feature = "multithreading",
        feature = "memory-validation"
    ));

    LayoutExpectations {
        stack_value_size: lookup_bool(STACK_VALUE_SIZE_MATRIX.as_slice(), has_validation_tag, 0),
        managed_ptr_size: lookup_bool2(
            MANAGED_PTR_SIZE_MATRIX.as_slice(),
            has_validation_tag,
            multithreading,
            0,
        ),
        pointer_origin_size: lookup_bool(POINTER_ORIGIN_SIZE_MATRIX.as_slice(), multithreading, 0),
        object_inner_overhead: lookup_bool2(
            OBJECT_INNER_OVERHEAD_MATRIX.as_slice(),
            has_validation_tag,
            has_owner_id,
            0,
        ),
        object_ref_size: lookup_bool(OBJECT_REF_SIZE_MATRIX.as_slice(), multithreading, 0),
        option_object_ref_size: lookup_bool(
            OPTION_OBJECT_REF_SIZE_MATRIX.as_slice(),
            multithreading,
            0,
        ),
    }
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    const EXPECTED_LAYOUTS: LayoutExpectations = expected_layouts();
    const EXPECTED_OBJECT_INNER_SIZE: usize =
        size_of::<HeapStorage<'static>>() + EXPECTED_LAYOUTS.object_inner_overhead;
    assert!(size_of::<StackValue<'static>>() == EXPECTED_LAYOUTS.stack_value_size);
    assert!(size_of::<ManagedPtr<'static>>() == EXPECTED_LAYOUTS.managed_ptr_size);
    assert!(size_of::<PointerOrigin<'static>>() == EXPECTED_LAYOUTS.pointer_origin_size);
    assert!(size_of::<ObjectInner<'static>>() == EXPECTED_OBJECT_INNER_SIZE);
    assert!(size_of::<ObjectRef<'static>>() == EXPECTED_LAYOUTS.object_ref_size);
    assert!(size_of::<Option<ObjectRef<'static>>>() == EXPECTED_LAYOUTS.option_object_ref_size);
    assert!(align_of::<StackValue<'static>>() == 8);
    assert!(align_of::<ManagedPtr<'static>>() == 8);
    assert!(align_of::<PointerOrigin<'static>>() == 8);
    assert!(align_of::<ObjectInner<'static>>() == 8);
    assert!(align_of::<ObjectRef<'static>>() == 8);
    assert!(align_of::<Option<ObjectRef<'static>>>() == 8);
};

#[test]
fn layout_sizes_match_expected() {
    #[cfg(target_pointer_width = "64")]
    {
        let expected_layouts = expected_layouts();
        let expected_object_inner_size =
            size_of::<HeapStorage<'static>>() + expected_layouts.object_inner_overhead;
        assert_eq!(
            size_of::<StackValue<'static>>(),
            expected_layouts.stack_value_size
        );
        assert_eq!(
            size_of::<ManagedPtr<'static>>(),
            expected_layouts.managed_ptr_size
        );
        assert_eq!(
            size_of::<PointerOrigin<'static>>(),
            expected_layouts.pointer_origin_size
        );
        assert_eq!(
            size_of::<ObjectInner<'static>>(),
            expected_object_inner_size
        );
        assert_eq!(
            size_of::<ObjectRef<'static>>(),
            expected_layouts.object_ref_size
        );
        assert_eq!(
            size_of::<Option<ObjectRef<'static>>>(),
            expected_layouts.option_object_ref_size
        );
        assert_eq!(align_of::<StackValue<'static>>(), 8);
        assert_eq!(align_of::<ManagedPtr<'static>>(), 8);
        assert_eq!(align_of::<PointerOrigin<'static>>(), 8);
        assert_eq!(align_of::<ObjectInner<'static>>(), 8);
        assert_eq!(align_of::<ObjectRef<'static>>(), 8);
        assert_eq!(align_of::<Option<ObjectRef<'static>>>(), 8);
    }
}
