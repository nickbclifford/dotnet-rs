pub trait PInvokeSandbox: Send + Sync {
    fn allow_library(&self, name: &str) -> bool;
    fn allow_function(&self, lib: &str, func: &str) -> bool;
}

pub struct DefaultSandbox;
impl PInvokeSandbox for DefaultSandbox {
    fn allow_library(&self, _name: &str) -> bool {
        true
    }

    fn allow_function(&self, _lib: &str, _func: &str) -> bool {
        true
    }
}

#[cfg(feature = "fuzzing")]
pub struct DenySandbox;
#[cfg(feature = "fuzzing")]
impl PInvokeSandbox for DenySandbox {
    fn allow_library(&self, _name: &str) -> bool {
        false
    }

    fn allow_function(&self, _lib: &str, _func: &str) -> bool {
        false
    }
}
