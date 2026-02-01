use crate::object::ObjectRef;
use dotnet_types::TypeDescription;
use gc_arena::{unsafe_empty_collect, Collect, Collection};
use std::{
    cmp::Ordering,
    collections::HashSet,
    fmt::{self, Debug, Formatter},
    mem::size_of,
    ptr::NonNull,
};

#[derive(Copy, Clone, Debug, PartialEq, PartialOrd)]
pub struct UnmanagedPtr(pub NonNull<u8>);
unsafe_empty_collect!(UnmanagedPtr);

// Kept for compatibility but effectively replaced by ObjectRef for heap owners

#[derive(Copy, Clone)]
#[repr(C)]
pub struct ManagedPtr<'gc> {
    pub value: Option<NonNull<u8>>,
    /// The object that owns the memory pointed to by this pointer.
    /// This ensures the object stays alive while the pointer is in use.
    pub owner: Option<ObjectRef<'gc>>,
    pub inner_type: TypeDescription,
    pub pinned: bool,
    pub _marker: std::marker::PhantomData<&'gc ()>,
}

impl Debug for ManagedPtr<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {:?} (owner: {:?}, pinned: {})",
            self.inner_type.type_name(),
            self.value,
            self.owner,
            self.pinned
        )
    }
}

impl PartialEq for ManagedPtr<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl PartialOrd for ManagedPtr<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.value.partial_cmp(&other.value)
    }
}

impl<'gc> ManagedPtr<'gc> {
    /// One word for the pointer, one word for the owner.
    pub const MEMORY_SIZE: usize = size_of::<usize>() * 2;

    pub fn new(
        value: Option<NonNull<u8>>,
        inner_type: TypeDescription,
        owner: Option<ObjectRef<'gc>>,
        pinned: bool,
    ) -> Self {
        Self {
            value,
            inner_type,
            owner,
            pinned,
            _marker: std::marker::PhantomData,
        }
    }

    /// Read pointer and owner from memory.
    /// Returns (pointer, owner). Type info must be supplied by caller.
    pub unsafe fn read_from_bytes(source: &[u8]) -> (Option<NonNull<u8>>, Option<ObjectRef<'gc>>) {
        let ptr_size = size_of::<usize>();
        
        // Read Pointer (Offset 0)
        let mut value_bytes = [0u8; size_of::<usize>()];
        value_bytes.copy_from_slice(&source[0..ptr_size]);
        let value_ptr = usize::from_ne_bytes(value_bytes) as *mut u8;
        let ptr = NonNull::new(value_ptr);

        // Read Owner (Offset 8)
        let owner = ObjectRef::read_unchecked(&source[ptr_size..]);
        
        (ptr, Some(owner))
    }
    
    // Legacy / Raw read: Reads only the pointer (first word).
    pub fn read_raw_ptr_unsafe(source: &[u8]) -> Option<NonNull<u8>> {
        let ptr_size = size_of::<usize>();
        let mut value_bytes = [0u8; size_of::<usize>()];
        value_bytes.copy_from_slice(&source[0..ptr_size]);
        let value_ptr = usize::from_ne_bytes(value_bytes) as *mut u8;
        NonNull::new(value_ptr)
    }

    /// Write pointer and owner to memory.
    pub fn write(&self, dest: &mut [u8]) {
        let ptr_size = size_of::<usize>();
        
        // Write Pointer (Offset 0)
        let val = self.value.map(|p| p.as_ptr() as usize).unwrap_or(0);
        let value_bytes = val.to_ne_bytes();
        dest[0..ptr_size].copy_from_slice(&value_bytes);

        // Write Owner (Offset 8)
        if let Some(owner) = self.owner {
            owner.write(&mut dest[ptr_size..]);
        } else {
             // Write null ref
             ObjectRef(None).write(&mut dest[ptr_size..]);
        }
    }

    pub fn map_value(
        self,
        transform: impl FnOnce(Option<NonNull<u8>>) -> Option<NonNull<u8>>,
    ) -> Self {
        ManagedPtr {
            value: transform(self.value),
            inner_type: self.inner_type,
            owner: self.owner,
            pinned: self.pinned,
            _marker: std::marker::PhantomData,
        }
    }

    /// # Safety
    ///
    /// The caller must ensure that the resulting pointer is within the bounds of the same
    /// allocated object as the original pointer.
    pub unsafe fn offset(self, bytes: isize) -> Self {
        self.map_value(|p| {
            p.and_then(|ptr| {
                // SAFETY: generic caller safety invariant regarding object bounds still applies.
                let new_ptr = unsafe { ptr.as_ptr().offset(bytes) };
                NonNull::new(new_ptr)
            })
        })
    }
}

unsafe impl<'gc> Collect for ManagedPtr<'gc> {
    fn trace(&self, cc: &Collection) {
        if let Some(owner) = &self.owner {
            owner.trace(cc);
        }
    }
}

impl<'gc> ManagedPtr<'gc> {
    pub fn resurrect(&self, fc: &gc_arena::Finalization<'gc>, visited: &mut HashSet<usize>) {
        if let Some(owner) = &self.owner {
            owner.resurrect(fc, visited);
        }
    }
}
