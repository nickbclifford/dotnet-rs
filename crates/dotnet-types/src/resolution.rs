use dotnet_utils::sync::Mutex;
use dotnetdll::prelude::*;
use gc_arena::static_collect;
use std::{
    fmt::{Debug, Formatter},
    hash::Hash,
    ops::Deref,
    ptr::{self, NonNull},
    sync::Arc,
};

#[cfg(feature = "fuzzing")]
use arbitrary::Arbitrary;

/// Metadata arena that owns leaked metadata and byte slices.
/// Shared via Arc to keep metadata alive as long as any descriptor references it.
pub struct MetadataArena {
    resolutions: Mutex<Vec<*mut Resolution<'static>>>,
    u64_slices: Mutex<Vec<*mut [u64]>>,
}

impl MetadataArena {
    pub fn new() -> Self {
        Self {
            resolutions: Mutex::new(Vec::new()),
            u64_slices: Mutex::new(Vec::new()),
        }
    }

    /// Adds a resolution pointer to the arena.
    ///
    /// # Safety
    ///
    /// The pointer must have been obtained from `Box::into_raw(Box::new(res))`.
    pub unsafe fn add_resolution(self: &Arc<Self>, res: *const Resolution<'static>) {
        self.resolutions.lock().push(res as *mut _);
    }

    /// Adds a u64 slice pointer to the arena.
    ///
    /// # Safety
    ///
    /// The pointer must have been obtained from `Box::into_raw(vec.into_boxed_slice())`.
    pub unsafe fn add_u64_slice(self: &Arc<Self>, slice: *mut [u64]) {
        self.u64_slices.lock().push(slice);
    }
}

impl Default for MetadataArena {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for MetadataArena {
    fn drop(&mut self) {
        // Drop resolutions first as they may contain references to the slices
        for res in self.resolutions.lock().drain(..) {
            // SAFETY: The pointer was added via add_resolution, which requires it to be from Box::into_raw.
            unsafe {
                drop(Box::from_raw(res));
            }
        }
        for slice in self.u64_slices.lock().drain(..) {
            // SAFETY: The pointer was added via add_u64_slice, which requires it to be from Box::into_raw.
            unsafe {
                drop(Box::from_raw(slice));
            }
        }
    }
}

// SAFETY: MetadataArena contains raw pointers to metadata that are owned exclusively.
// The metadata is immutable after construction. Access to the storage is guarded by Mutex.
// The raw pointers themselves point to 'static data (leaked boxes), and the Arc ensures
// the arena lives as long as any descriptor pointing into it.
unsafe impl Send for MetadataArena where Resolution<'static>: Send {}
unsafe impl Sync for MetadataArena where Resolution<'static>: Sync {}

#[derive(Clone)]
pub struct ResolutionS(Option<(Arc<MetadataArena>, NonNull<Resolution<'static>>)>);

#[cfg(feature = "fuzzing")]
impl<'a> Arbitrary<'a> for ResolutionS {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let ptr_val: usize = u.arbitrary()?;
        let arena = Arc::new(MetadataArena::new());
        Ok(Self::new(ptr_val as *const _, arena))
    }
}
static_collect!(ResolutionS);

// SAFETY: ResolutionS carries an Arc<MetadataArena> alongside a raw pointer.
// The Arc ensures the metadata arena (and thus all metadata) lives as long as
// any ResolutionS exists. The raw pointer lifetime is guaranteed by the Arc.
// Thread-safety is guaranteed by the Arc's ref-counting and the read-only nature
// of metadata once loaded.
unsafe impl Send for ResolutionS where Resolution<'static>: Send {}
unsafe impl Sync for ResolutionS where Resolution<'static>: Sync {}
impl ResolutionS {
    pub const NULL: Self = Self(None);

    pub fn new(ptr: *const Resolution<'static>, arena: Arc<MetadataArena>) -> Self {
        NonNull::new(ptr as *mut _).map_or(Self(None), |p| Self(Some((arena, p))))
    }

    pub fn as_raw(&self) -> *const Resolution<'static> {
        match &self.0 {
            Some((_, p)) => p.as_ptr() as *const _,
            None => ptr::null(),
        }
    }

    pub const fn is_null(&self) -> bool {
        self.0.is_none()
    }

    pub fn definition(&self) -> &'static Resolution<'static> {
        match &self.0 {
            Some((_, p)) => {
                // SAFETY: The pointer in ResolutionS is kept alive by the Arc<MetadataArena>
                // that is co-carried in the same Option variant. The arena owns the metadata
                // and will not drop it until all ResolutionS instances are dropped.
                unsafe { &*p.as_ptr() }
            }
            None => {
                panic!("ResolutionS NULL");
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    #[test]
    fn test_descriptor_lifetime_safety() {
        let weak_arena;
        let res_s;
        {
            let arena = Arc::new(MetadataArena::new());
            weak_arena = Arc::downgrade(&arena);

            // We use a dangling pointer for testing the Arc lifetime.
            // ResolutionS::new only stores the pointer and the Arc, it doesn't dereference it.
            let dummy_ptr = NonNull::dangling().as_ptr();
            res_s = ResolutionS::new(dummy_ptr, arena);

            // 'arena' (the original Arc) goes out of scope here.
            // Ref count should still be 1 because res_s holds it.
        }

        // Arena should still be alive because res_s holds an Arc to it.
        assert!(
            weak_arena.upgrade().is_some(),
            "Arena should be kept alive by ResolutionS"
        );

        // Even if we clone res_s, it should still be alive.
        {
            let res_s_clone = res_s.clone();
            assert!(weak_arena.upgrade().is_some());
            drop(res_s_clone);
        }
        assert!(weak_arena.upgrade().is_some());

        drop(res_s);

        // Now it should be dropped.
        assert!(
            weak_arena.upgrade().is_none(),
            "Arena should be dropped after last ResolutionS is gone"
        );
    }

    #[test]
    fn test_multiple_descriptors_one_arena() {
        let arena = Arc::new(MetadataArena::new());
        let weak_arena = Arc::downgrade(&arena);

        let dummy_ptr1 = NonNull::dangling().as_ptr();
        let dummy_ptr2 = NonNull::dangling().as_ptr();

        let res_s1 = ResolutionS::new(dummy_ptr1, arena.clone());
        let res_s2 = ResolutionS::new(dummy_ptr2, arena.clone());

        drop(arena);
        assert!(weak_arena.upgrade().is_some());

        drop(res_s1);
        assert!(weak_arena.upgrade().is_some());

        drop(res_s2);
        assert!(weak_arena.upgrade().is_none());
    }
}
impl Deref for ResolutionS {
    type Target = Resolution<'static>;
    fn deref(&self) -> &'static Self::Target {
        self.definition()
    }
}
impl Debug for ResolutionS {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            None => write!(f, "ResolutionS(NULL)"),
            Some(_) => write!(
                f,
                "ResolutionS({} @ {:#?})",
                self.definition().assembly.as_ref().unwrap().name,
                self.as_raw()
            ),
        }
    }
}
impl PartialEq for ResolutionS {
    fn eq(&self, other: &Self) -> bool {
        // Pointer-identity comparison only (not Arc comparison) to preserve HashMap semantics
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some((_, p1)), Some((_, p2))) => p1 == p2,
            _ => false,
        }
    }
}
impl Eq for ResolutionS {}
impl Hash for ResolutionS {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash pointer only (not Arc) to preserve HashMap semantics
        match &self.0 {
            None => None::<usize>.hash(state),
            Some((_, p)) => p.hash(state),
        }
    }
}
