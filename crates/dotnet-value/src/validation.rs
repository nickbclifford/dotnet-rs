#[cfg(any(feature = "memory-validation", debug_assertions))]
use gc_arena::Collect;

#[cfg(any(feature = "memory-validation", debug_assertions))]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Collect)]
#[collect(require_static)]
pub struct ValidationTag(u64);

#[cfg(not(any(feature = "memory-validation", debug_assertions)))]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ValidationTag;

#[cfg(not(any(feature = "memory-validation", debug_assertions)))]
// SAFETY: ValidationTag is empty when validation is disabled.
unsafe impl<'gc> gc_arena::Collect<'gc> for ValidationTag {
    fn trace<Tr: gc_arena::collect::Trace<'gc>>(&self, _cc: &mut Tr) {}
}

#[cfg(all(not(feature = "memory-validation"), not(debug_assertions)))]
const _: () = assert!(core::mem::size_of::<ValidationTag>() == 0);

impl ValidationTag {
    #[inline(always)]
    pub fn new(_magic: u64) -> Self {
        #[cfg(any(feature = "memory-validation", debug_assertions))]
        return Self(_magic);
        #[cfg(not(any(feature = "memory-validation", debug_assertions)))]
        return Self;
    }

    #[inline(always)]
    pub fn validate(&self, _expected: u64, _name: &str) {
        #[cfg(any(feature = "memory-validation", debug_assertions))]
        if self.0 != _expected {
            panic!("Invalid magic in {}: 0x{:016X}", _name, self.0);
        }
    }
}

#[cfg(feature = "fuzzing")]
impl<'a> arbitrary::Arbitrary<'a> for ValidationTag {
    fn arbitrary(_u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        // We don't really care about the value during fuzzing if it's not enabled,
        // but if it IS enabled, we might want it to be "valid" by default if we
        // are just generating structures. However, usually magic is set by the constructor.
        // For now, let's just return a default one.
        Ok(Self::new(0))
    }
}
