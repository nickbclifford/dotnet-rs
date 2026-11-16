pub trait PInvokeSandbox: Send + Sync {
    fn allow_library(&self, name: &str) -> bool;
    fn allow_function(&self, lib: &str, func: &str) -> bool;

    /// Whether native function pointers without a library or symbol identity may be called.
    fn allow_indirect_call(&self) -> bool {
        false
    }
}

pub struct DefaultSandbox;
impl PInvokeSandbox for DefaultSandbox {
    fn allow_library(&self, _name: &str) -> bool {
        true
    }

    fn allow_function(&self, _lib: &str, _func: &str) -> bool {
        true
    }

    fn allow_indirect_call(&self) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    struct NamedOnly;
    impl PInvokeSandbox for NamedOnly {
        fn allow_library(&self, _: &str) -> bool {
            true
        }
        fn allow_function(&self, _: &str, _: &str) -> bool {
            true
        }
    }

    #[test]
    fn indirect_calls_require_explicit_sandbox_permission() {
        assert!(DefaultSandbox.allow_indirect_call());
        assert!(!NamedOnly.allow_indirect_call());
        #[cfg(feature = "fuzzing")]
        assert!(!DenySandbox.allow_indirect_call());
    }
}
