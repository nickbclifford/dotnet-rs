use crate::{
    ArenaId, StackSlotIndex,
    object::{Object, ObjectRef},
};
use dotnet_types::{TypeDescription, generics::GenericLookup};
use gc_arena::{Collect, collect::Trace};
use std::collections::HashSet;

#[cfg(feature = "multithreading")]
use crate::object::ObjectPtr;
#[cfg(feature = "multithreading")]
use sptr::Strict;

#[cfg(feature = "fuzzing")]
use arbitrary::Arbitrary;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PointerOrigin<'gc> {
    Heap(ObjectRef<'gc>),
    Stack(StackSlotIndex),
    Static(TypeDescription, GenericLookup),
    Unmanaged,
    #[cfg(feature = "multithreading")]
    CrossArenaObjectRef(ObjectPtr, ArenaId),
    /// A value type resident on the evaluation stack (transient).
    Transient(Object<'gc>),
}

#[cfg(feature = "fuzzing")]
impl<'a, 'gc> Arbitrary<'a> for PointerOrigin<'gc> {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let variant = u.int_in_range(0..=5)?;
        match variant {
            0 => Ok(Self::Heap(u.arbitrary()?)),
            1 => Ok(Self::Stack(u.arbitrary()?)),
            2 => Ok(Self::Static(u.arbitrary()?, u.arbitrary()?)),
            3 => Ok(Self::Unmanaged),
            #[cfg(feature = "multithreading")]
            4 => Ok(Self::CrossArenaObjectRef(u.arbitrary()?, u.arbitrary()?)),
            #[cfg(not(feature = "multithreading"))]
            4 => Ok(Self::Unmanaged),
            5 => Ok(Self::Transient(u.arbitrary()?)),
            _ => unreachable!(),
        }
    }
}

// SAFETY: PointerOrigin contains several variants that hold GC-managed references.
// We manually implement trace and resurrect to ensure all such references (ObjectRef, Object)
// are correctly visited by the GC. Cross-arena references are recorded for coordinated GC.
unsafe impl<'gc> Collect<'gc> for PointerOrigin<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        match self {
            Self::Heap(r) => r.trace(cc),
            #[cfg(feature = "multithreading")]
            Self::CrossArenaObjectRef(ptr, tid) => {
                dotnet_utils::gc::record_cross_arena_ref(*tid, ptr.as_ptr() as usize);
            }
            Self::Transient(obj) => obj.trace(cc),
            _ => {}
        }
    }
}

impl<'gc> PointerOrigin<'gc> {
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
            Self::Static(_, _) => 2,
            Self::Unmanaged => 3,
            #[cfg(feature = "multithreading")]
            Self::CrossArenaObjectRef(_, _) => 4,
            Self::Transient(_) => 5,
        }
    }

    pub fn normalize(self) -> Self {
        match self {
            Self::Heap(r) if r.0.is_none() => Self::Unmanaged,
            Self::Transient(_) => Self::Unmanaged,
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

    pub fn write_barrier_owner_id(&self) -> Option<ArenaId> {
        match self {
            Self::Heap(r) => r.0.as_ref().map(|h| unsafe { (*h.as_ptr()).owner_id }),
            #[cfg(feature = "multithreading")]
            Self::CrossArenaObjectRef(_, tid) => Some(*tid),
            _ => None,
        }
    }

    #[cfg(feature = "multithreading")]
    pub fn write_barrier_owner(&self) -> Option<(ObjectPtr, ArenaId)> {
        match self {
            Self::Heap(r) => r.as_ptr_info(),
            Self::CrossArenaObjectRef(ptr, tid) => Some((*ptr, *tid)),
            _ => None,
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
            PointerOrigin::CrossArenaObjectRef(ptr, tid) => {
                dotnet_utils::gc::record_cross_arena_ref(*tid, ptr.as_ptr().expose_addr());
            }
            PointerOrigin::Transient(obj) => obj.resurrect(fc, visited, depth),
            _ => {}
        }
    }
}
