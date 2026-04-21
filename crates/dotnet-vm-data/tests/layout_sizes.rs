use dotnet_vm_data::{ExceptionState, StackFrame, StepResult};
use std::mem::{align_of, size_of};

#[cfg(target_pointer_width = "64")]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct LayoutExpectations {
    step_result_size: usize,
    exception_state_size: usize,
    stack_frame_size: usize,
}

#[cfg(target_pointer_width = "64")]
const STEP_RESULT_SIZE_MATRIX: [(bool, usize); 2] = [
    // (multithreading, expected_size)
    (false, 72),
    (true, 72),
];

#[cfg(target_pointer_width = "64")]
const EXCEPTION_STATE_SIZE_MATRIX: [(bool, usize); 2] = [
    // (multithreading, expected_size)
    (false, 136),
    (true, 136),
];

#[cfg(target_pointer_width = "64")]
const STACK_FRAME_SIZE_MATRIX: [(bool, usize); 2] = [
    // (multithreading, expected_size)
    (false, 496),
    (true, 496),
];

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
    let multithreading = cfg!(feature = "multithreading");
    LayoutExpectations {
        step_result_size: lookup_bool(STEP_RESULT_SIZE_MATRIX.as_slice(), multithreading, 0),
        exception_state_size: lookup_bool(
            EXCEPTION_STATE_SIZE_MATRIX.as_slice(),
            multithreading,
            0,
        ),
        stack_frame_size: lookup_bool(STACK_FRAME_SIZE_MATRIX.as_slice(), multithreading, 0),
    }
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    const EXPECTED_LAYOUTS: LayoutExpectations = expected_layouts();
    assert!(size_of::<StepResult>() == EXPECTED_LAYOUTS.step_result_size);
    assert!(size_of::<ExceptionState<'static>>() == EXPECTED_LAYOUTS.exception_state_size);
    assert!(size_of::<StackFrame<'static>>() == EXPECTED_LAYOUTS.stack_frame_size);
    assert!(align_of::<StepResult>() == 8);
    assert!(align_of::<ExceptionState<'static>>() == 8);
    assert!(align_of::<StackFrame<'static>>() == 8);
};

#[test]
fn layout_sizes_match_expected() {
    #[cfg(target_pointer_width = "64")]
    {
        let expected_layouts = expected_layouts();
        assert_eq!(size_of::<StepResult>(), expected_layouts.step_result_size);
        assert_eq!(
            size_of::<ExceptionState<'static>>(),
            expected_layouts.exception_state_size
        );
        assert_eq!(
            size_of::<StackFrame<'static>>(),
            expected_layouts.stack_frame_size
        );
        assert_eq!(align_of::<StepResult>(), 8);
        assert_eq!(align_of::<ExceptionState<'static>>(), 8);
        assert_eq!(align_of::<StackFrame<'static>>(), 8);
    }
}
