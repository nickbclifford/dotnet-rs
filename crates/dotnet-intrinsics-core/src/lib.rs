//! Core intrinsic handlers shared across VM call paths.
pub mod array_ops;
pub mod math;

macro_rules! vm_try {
    ($expr:expr) => {
        match $expr {
            Ok(v) => v,
            Err(e) => {
                return dotnet_vm_ops::StepResult::Error(dotnet_types::error::VmError::from(e))
            }
        }
    };
}

pub(crate) use vm_try;
