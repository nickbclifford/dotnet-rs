use crate::{
    StackSlotIndex,
    object::{Object, ObjectRef},
    pointer::StaticMetadata,
};
use dotnet_types::{TypeDescription, generics::GenericLookup};
use dotnet_utils::sync::Arc;
use gc_arena::{Collect, collect::Trace};
use std::collections::HashSet;

#[cfg(feature = "multithreading")]
use crate::object::ObjectPtr;
#[cfg(feature = "fuzzing")]
use arbitrary::Arbitrary;
#[cfg(feature = "multithreading")]
use dotnet_utils::gc::{ArenaIdentity, ArenaLease, RegistrationToken};

/// An inert object pointer paired with its original arena registration.
/// It is neither a GC root nor an arena lease.
///
/// The registration token is captured together with the identity from the
/// same live lease, so serialization copies the original registration's
/// token and never issues one for a stale pointer.
#[cfg(feature = "multithreading")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CrossArenaOrigin {
    ptr: ObjectPtr,
    identity: ArenaIdentity,
    token: RegistrationToken,
}

#[cfg(feature = "multithreading")]
impl CrossArenaOrigin {
    /// Pairs `ptr` with the exact registration pinned by `lease`.
    ///
    /// # Safety
    /// `ptr` must originate from a live, rooted object belonging to exactly
    /// the registration pinned by `lease`; the caller must establish its
    /// lifetime and access discipline. A lease freshly acquired by a reused ID
    /// does not establish this association: the lease must be for the
    /// object's original registration.
    pub unsafe fn new(ptr: ObjectPtr, lease: &ArenaLease) -> Self {
        Self {
            ptr,
            identity: lease.identity(),
            token: lease.token(),
        }
    }

    pub fn identity(self) -> ArenaIdentity {
        self.identity
    }

    /// Inert transport form carrying this origin's captured identity (never refreshed by ID).
    pub fn cross_arena_ref(self) -> dotnet_utils::gc::CrossArenaRef {
        // SAFETY: F1.ArenaGenerationMatch — `identity` was captured from the lease for this
        // pointer's original registration in `new`; it is copied, not recaptured.
        unsafe { dotnet_utils::gc::CrossArenaRef::new(self.identity, self.ptr.to_opaque()) }
    }

    /// The non-reused registration token of the original registration.
    pub fn token(self) -> RegistrationToken {
        self.token
    }

    /// Returns the inert pointer without granting access or retaining a lease.
    pub fn as_ptr(
        self,
    ) -> *const dotnet_utils::gc::ThreadSafeLock<crate::object::ObjectInner<'static>> {
        self.ptr.as_ptr()
    }

    /// # Safety
    /// The original object must be rooted and live, the exact identity must
    /// be leased throughout any use, and access synchronization must be upheld.
    pub unsafe fn object_ptr(self) -> ObjectPtr {
        self.ptr
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PointerOrigin<'gc> {
    Heap(ObjectRef<'gc>),
    Stack(StackSlotIndex),
    Static(Arc<StaticMetadata>),
    Unmanaged,
    #[cfg(feature = "multithreading")]
    CrossArenaObjectRef(CrossArenaOrigin),
    /// A value type resident on the evaluation stack (transient).
    Transient(Arc<Object<'gc>>),
}

#[cfg(feature = "fuzzing")]
impl<'a, 'gc> Arbitrary<'a> for PointerOrigin<'gc> {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let variant = u.int_in_range(0..=5)?;
        match variant {
            0 => Ok(Self::Heap(u.arbitrary()?)),
            1 => Ok(Self::Stack(u.arbitrary()?)),
            2 => Ok(Self::Static(Arc::new(StaticMetadata {
                type_desc: u.arbitrary()?,
                generics: u.arbitrary()?,
            }))),
            3 => Ok(Self::Unmanaged),
            // Unscoped fuzz bytes cannot establish a live pointer/identity pair.
            // The round-trip fuzz target constructs that case from a live fixture.
            4 => Ok(Self::Unmanaged),
            // Shared ownership is required even when Object is not Sync.
            #[allow(clippy::arc_with_non_send_sync)]
            5 => Ok(Self::Transient(Arc::new(u.arbitrary()?))),
            _ => unreachable!(),
        }
    }
}

// SAFETY: F1.GcHandleRooted — PointerOrigin contains several variants that hold GC-managed references.
// We manually implement trace and resurrect to ensure all such references (ObjectRef, Object)
// are correctly visited by the GC. Cross-arena references are recorded for coordinated GC.
unsafe impl<'gc> Collect<'gc> for PointerOrigin<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        match self {
            Self::Heap(r) => r.trace(cc),
            #[cfg(feature = "multithreading")]
            Self::CrossArenaObjectRef(origin) => {
                // The origin's captured identity is validated, never refreshed by ID.
                dotnet_utils::gc::record_cross_arena_ref(origin.cross_arena_ref());
            }
            Self::Transient(obj) => obj.as_ref().trace(cc),
            _ => {}
        }
    }
}

impl<'gc> PointerOrigin<'gc> {
    pub fn new_static(type_desc: TypeDescription, generics: GenericLookup) -> Self {
        Self::Static(Arc::new(StaticMetadata {
            type_desc,
            generics,
        }))
    }

    // Arc preserves clone identity across feature configurations; it does not
    // by itself make Object thread-safe or permit transfer across threads.
    #[allow(clippy::arc_with_non_send_sync)]
    pub fn new_transient(obj: Object<'gc>) -> Self {
        Self::Transient(Arc::new(obj))
    }

    pub fn static_metadata(&self) -> Option<&StaticMetadata> {
        if let Self::Static(metadata) = self {
            Some(metadata.as_ref())
        } else {
            None
        }
    }

    pub fn static_parts(&self) -> Option<(&TypeDescription, &GenericLookup)> {
        self.static_metadata()
            .map(|metadata| (&metadata.type_desc, &metadata.generics))
    }

    pub fn is_null(&self) -> bool {
        match self {
            Self::Unmanaged => true,
            Self::Heap(r) => r.0.is_none(),
            _ => false,
        }
    }

    pub fn discriminant(&self) -> u8 {
        match self {
            Self::Heap(_) => 0,
            Self::Stack(_) => 1,
            Self::Static(_) => 2,
            Self::Unmanaged => 3,
            #[cfg(feature = "multithreading")]
            Self::CrossArenaObjectRef(_) => 4,
            Self::Transient(_) => 5,
        }
    }

    /// Canonicalizes a null Heap origin to Unmanaged, preserving all other
    /// origins, including the owner retained by Transient.
    pub fn normalize(self) -> Self {
        match self {
            Self::Heap(r) if r.0.is_none() => Self::Unmanaged,
            other => other,
        }
    }

    pub fn owner(&self) -> Option<ObjectRef<'gc>> {
        if let Self::Heap(r) = self {
            Some(*r)
        } else {
            None
        }
    }

    pub fn resurrect(
        &self,
        fc: &'gc gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
        depth: usize,
    ) {
        match self {
            PointerOrigin::Heap(r) => r.resurrect(fc, visited, depth),
            #[cfg(feature = "multithreading")]
            PointerOrigin::CrossArenaObjectRef(origin) => {
                dotnet_utils::gc::record_cross_arena_ref(origin.cross_arena_ref());
            }
            PointerOrigin::Transient(obj) => obj.as_ref().resurrect(fc, visited, depth),
            _ => {}
        }
    }
}
