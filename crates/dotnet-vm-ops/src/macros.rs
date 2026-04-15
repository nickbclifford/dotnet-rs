#[macro_export]
macro_rules! vm_try {
    ($expr:expr) => {
        match $expr {
            Ok(v) => v,
            Err(e) => return $crate::StepResult::Error(dotnet_types::error::VmError::from(e)),
        }
    };
}
