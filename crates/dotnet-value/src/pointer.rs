use crate::object::{Object, ObjectHandle};
use crate::StackValue;
use dotnet_types::TypeDescription;
use dotnet_utils::gc::ThreadSafeLock;
use gc_arena::{unsafe_empty_collect, Collect, Collection, Gc};
use std::{
    cmp::Ordering,
    collections::HashSet,
    fmt::{self, Debug, Formatter},
    ptr::NonNull,
};

#[derive(Copy, Clone, Debug, PartialEq, PartialOrd)]
pub struct UnmanagedPtr(pub NonNull<u8>);
unsafe_empty_collect!(UnmanagedPtr);

#[derive(Copy, Clone)]
pub enum ManagedPtrOwner<'gc> {
    Heap(ObjectHandle<'gc>),
    Stack(NonNull<Object<'gc>>),
    StackSlot(Gc<'gc, ThreadSafeLock<StackValue<'gc>>>),
}

impl<'gc> ManagedPtrOwner<'gc> {
    pub fn as_ptr(&self) -> *const u8 {
        match self {
            Self::Heap(h) => Gc::as_ptr(*h) as *const u8,
            Self::Stack(s) => s.as_ptr() as *const u8,
            Self::StackSlot(s) => Gc::as_ptr(*s) as *const u8,
        }
    }
}

unsafe impl<'gc> Collect for ManagedPtrOwner<'gc> {
    fn trace(&self, cc: &Collection) {
        match self {
            Self::Heap(h) => h.trace(cc),
            // Stack objects are already traced through the VM stack.
            // Do NOT trace them here to avoid infinite recursion:
            // Object → StackValue → ManagedPtr → ManagedPtrOwner::Stack → Object (cycle!)
            Self::Stack(_) => {}
            Self::StackSlot(_) => {}
        }
    }
}

impl Debug for ManagedPtrOwner<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Heap(h) => write!(f, "Heap({:?})", Gc::as_ptr(*h)),
            Self::Stack(s) => write!(f, "Stack({:?})", s.as_ptr()),
            Self::StackSlot(s) => write!(f, "StackSlot({:?})", Gc::as_ptr(*s)),
        }
    }
}

#[derive(Copy, Clone)]
pub struct ManagedPtr<'gc> {
    pub value: Option<NonNull<u8>>,
    pub inner_type: TypeDescription,
    pub owner: Option<ManagedPtrOwner<'gc>>,
    pub pinned: bool,
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
    /// the actual value is pointer-sized when moved around inside the runtime
    pub const MEMORY_SIZE: usize = size_of::<usize>();

    pub fn new(
        value: Option<NonNull<u8>>,
        inner_type: TypeDescription,
        owner: Option<ManagedPtrOwner<'gc>>,
        pinned: bool,
    ) -> Self {
        Self {
            value,
            inner_type,
            owner,
            pinned,
        }
    }

    /// Read just the pointer value from memory (8 bytes).
    /// This is the new memory format. Metadata must be retrieved from the Object's side-table.
    pub fn read_raw_ptr_unsafe(source: &[u8]) -> Option<NonNull<u8>> {
        let mut value_bytes = [0u8; size_of::<usize>()];
        value_bytes.copy_from_slice(&source[0..size_of::<usize>()]);
        let value_ptr = usize::from_ne_bytes(value_bytes) as *mut u8;
        NonNull::new(value_ptr)
    }

    /// Write just the pointer value to memory (8 bytes).
    /// Metadata should be stored in the Object's side-table separately.
    pub fn write_ptr_only(&self, dest: &mut [u8]) {
        let val = self.value.map(|p| p.as_ptr() as usize).unwrap_or(0);
        let value_bytes = val.to_ne_bytes();
        dest[0..size_of::<usize>()].copy_from_slice(&value_bytes);
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
                // We use NonNull::new instead of new_unchecked to prevent creating "poisoned"
                // NonNull instances that wrap null (0). This can happen if the offset operation
                // results in a null pointer (e.g. during specific arithmetic in System.Private.CoreLib).
                // If it becomes null, we want the Option to become None, so that subsequent checks
                // for validity (like .is_some()) correctly identify it as invalid/null.
                let new_ptr = unsafe { ptr.as_ptr().offset(bytes) };
                NonNull::new(new_ptr)
            })
        })
    }
}

unsafe impl<'gc> Collect for ManagedPtr<'gc> {
    fn trace(&self, cc: &Collection) {
        if let Some(o) = self.owner {
            o.trace(cc);
        }
    }
}

impl<'gc> ManagedPtr<'gc> {
    pub fn resurrect(&self, fc: &gc_arena::Finalization<'gc>, visited: &mut HashSet<usize>) {
        if let Some(owner) = self.owner {
            match owner {
                ManagedPtrOwner::Heap(h) => {
                    let ptr = Gc::as_ptr(h) as usize;
                    if visited.insert(ptr) {
                        Gc::resurrect(fc, h);
                        h.borrow().storage.resurrect(fc, visited);
                    }
                }
                ManagedPtrOwner::Stack(s) => {
                    unsafe { s.as_ref().resurrect(fc, visited) };
                }
                ManagedPtrOwner::StackSlot(_) => {}
            }
        }
    }
}
