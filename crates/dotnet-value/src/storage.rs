use crate::layout::FieldLayoutManager;
use dotnet_types::TypeDescription;
use dotnet_utils::sync::{MappedRwLockReadGuard, MappedRwLockWriteGuard};
use gc_arena::{Collect, Collection};
use std::{
    collections::HashSet,
    fmt::{self, Debug, Formatter},
    sync::Arc,
};

#[cfg(feature = "multithreading")]
use dotnet_utils::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

#[cfg(not(feature = "multithreading"))]
use std::cell::RefCell as RwLock;

#[derive(Clone)]
pub struct FieldStorage {
    layout: Arc<FieldLayoutManager>,
    #[cfg(feature = "multithreading")]
    data: Arc<RwLock<Vec<u8>>>, // RwLock is not Clone, wrap in Arc or implement Clone manually
    #[cfg(not(feature = "multithreading"))]
    data: RwLock<Vec<u8>>,
}

impl PartialEq for FieldStorage {
    fn eq(&self, other: &Self) -> bool {
        if !Arc::ptr_eq(&self.layout, &other.layout) {
            return false;
        }
        #[cfg(feature = "multithreading")]
        {
            *self.data.read() == *other.data.read()
        }
        #[cfg(not(feature = "multithreading"))]
        {
            *self.data.borrow() == *other.data.borrow()
        }
    }
}

impl FieldStorage {
    pub fn new(layout: Arc<FieldLayoutManager>, data: Vec<u8>) -> Self {
        #[cfg(feature = "multithreading")]
        let data = Arc::new(RwLock::new(data));
        #[cfg(not(feature = "multithreading"))]
        let data = RwLock::new(data);

        Self { layout, data }
    }

    pub fn get(&self) -> MappedRwLockReadGuard<'_, [u8]> {
        #[cfg(feature = "multithreading")]
        return RwLockReadGuard::map(self.data.read(), |v| v.as_slice());
        #[cfg(not(feature = "multithreading"))]
        return std::cell::Ref::map(self.data.borrow(), |v| v.as_slice());
    }

    // Note: get_mut returning MappedRwLockWriteGuard might be tricky if we need &mut [u8].
    // If the caller needs to write back, MappedRwLockWriteGuard works.
    pub fn get_mut(&self) -> MappedRwLockWriteGuard<'_, [u8]> {
        #[cfg(feature = "multithreading")]
        return RwLockWriteGuard::map(self.data.write(), |v| v.as_mut_slice());
        #[cfg(not(feature = "multithreading"))]
        return std::cell::RefMut::map(self.data.borrow_mut(), |v| v.as_mut_slice());
    }

    pub fn layout(&self) -> &Arc<FieldLayoutManager> {
        &self.layout
    }

    pub fn has_field(&self, owner: TypeDescription, name: &str) -> bool {
        self.layout.get_field(owner, name).is_some()
    }

    pub fn get_field_local(
        &self,
        owner: TypeDescription,
        name: &str,
    ) -> MappedRwLockReadGuard<'_, [u8]> {
        let field = self.layout.get_field(owner, name).expect("Field not found");
        let range = field.as_range();

        #[cfg(feature = "multithreading")]
        return RwLockReadGuard::map(self.data.read(), |v| &v[range]);
        #[cfg(not(feature = "multithreading"))]
        return std::cell::Ref::map(self.data.borrow(), |v| &v[range]);
    }

    pub fn get_field_mut_local(
        &self,
        owner: TypeDescription,
        name: &str,
    ) -> MappedRwLockWriteGuard<'_, [u8]> {
        let field = self.layout.get_field(owner, name).expect("Field not found");
        let range = field.as_range();

        #[cfg(feature = "multithreading")]
        return RwLockWriteGuard::map(self.data.write(), |v| &mut v[range]);
        #[cfg(not(feature = "multithreading"))]
        return std::cell::RefMut::map(self.data.borrow_mut(), |v| &mut v[range]);
    }

    pub fn get_field_atomic(
        &self,
        owner: TypeDescription,
        name: &str,
        _ord: std::sync::atomic::Ordering,
    ) -> Vec<u8> {
        // With RwLock, simple read_unchecked is atomic enough regarding tearing vs other writers
        self.get_field_local(owner, name).to_vec()
    }

    pub fn set_field_atomic(
        &self,
        owner: TypeDescription,
        name: &str,
        value: &[u8],
        _ord: std::sync::atomic::Ordering,
    ) {
        let mut dest = self.get_field_mut_local(owner, name);
        dest.copy_from_slice(value);
    }

    unsafe fn raw_data_unsynchronized(&self) -> &[u8] {
        unsafe {
            #[cfg(feature = "multithreading")]
            {
                &*self.data.data_ptr()
            }
            #[cfg(not(feature = "multithreading"))]
            {
                &*self.data.as_ptr()
            }
        }
    }

    pub fn resurrect<'gc>(&self, fc: &gc_arena::Finalization<'gc>, visited: &mut HashSet<usize>) {
        // SAFETY: Resurrection happens during a stop-the-world pause, so no other
        // threads are running. We can safely access the inner value without
        // acquiring the lock. This avoids deadlock (or panic) if a thread was
        // already holding the write lock when it reached a safe point.
        let data = unsafe { self.raw_data_unsynchronized() };

        self.layout.resurrect(data, fc, visited);
    }
}

unsafe impl Collect for FieldStorage {
    fn trace(&self, cc: &Collection) {
        // SAFETY: Tracing also happens during a stop-the-world pause, same reasoning as above
        let data = unsafe { self.raw_data_unsynchronized() };

        self.layout.trace(data, cc);
    }
}

impl Debug for FieldStorage {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        #[cfg(feature = "multithreading")]
        let len = self.data.read().len();
        #[cfg(not(feature = "multithreading"))]
        let len = self.data.borrow().len();

        write!(f, "FieldStorage({} bytes)", len)
    }
}
