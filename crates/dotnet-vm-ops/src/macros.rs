#[macro_export]
macro_rules! vm_try {
    ($expr:expr) => {
        match $expr {
            Ok(v) => v,
            Err(e) => {
                return $crate::dotnet_vm_data::StepResult::Error(
                    dotnet_types::error::VmError::from(e),
                );
            }
        }
    };
}
