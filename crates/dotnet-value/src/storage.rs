use crate::layout::{FieldLayoutManager, HasLayout};
use dotnet_types::TypeDescription;
use dotnet_utils::atomic::Atomic;
use dotnet_utils::sync::{
    Arc, MappedRwLockReadGuard, MappedRwLockWriteGuard, RwLock, RwLockReadGuard, RwLockWriteGuard,
};
use gc_arena::{Collect, Collection};
use std::{
    collections::HashSet,
    fmt::{self, Debug, Formatter},
};

pub struct FieldStorage {
    layout: Arc<FieldLayoutManager>,
    data: RwLock<Vec<u8>>,
}

impl Clone for FieldStorage {
    fn clone(&self) -> Self {
        let data = self.data.read();
        Self {
            layout: self.layout.clone(),
            data: RwLock::new(data.clone()),
        }
    }
}

impl PartialEq for FieldStorage {
    fn eq(&self, other: &Self) -> bool {
        if !Arc::ptr_eq(&self.layout, &other.layout) {
            return false;
        }
        *self.data.read() == *other.data.read()
    }
}

impl FieldStorage {
    pub fn new(layout: Arc<FieldLayoutManager>, data: Vec<u8>) -> Self {
        Self {
            layout,
            data: RwLock::new(data),
        }
    }

    pub fn get(&self) -> MappedRwLockReadGuard<'_, [u8]> {
        RwLockReadGuard::map(self.data.read(), |v| v.as_slice())
    }

    pub fn get_mut(&self) -> MappedRwLockWriteGuard<'_, [u8]> {
        RwLockWriteGuard::map(self.data.write(), |v| v.as_mut_slice())
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

        RwLockReadGuard::map(self.data.read(), |v| &v[range])
    }

    pub fn get_field_mut_local(
        &self,
        owner: TypeDescription,
        name: &str,
    ) -> MappedRwLockWriteGuard<'_, [u8]> {
        let field = self.layout.get_field(owner, name).expect("Field not found");
        let range = field.as_range();

        RwLockWriteGuard::map(self.data.write(), |v| &mut v[range])
    }

    /// Returns a copy of the field's data, synchronized via RwLock to prevent tearing.
    /// NOTE: This method ignores the `Ordering` parameter and should only be used
    /// for non-volatile access where tearing prevention is the primary concern.
    pub fn get_field_synchronized(
        &self,
        owner: TypeDescription,
        name: &str,
        _ord: std::sync::atomic::Ordering,
    ) -> Vec<u8> {
        // With RwLock, simple read_unchecked is atomic enough regarding tearing vs other writers
        self.get_field_local(owner, name).to_vec()
    }

    /// Sets the field's data, synchronized via RwLock to prevent tearing.
    pub fn set_field_synchronized(
        &self,
        owner: TypeDescription,
        name: &str,
        value: &[u8],
        _ord: std::sync::atomic::Ordering,
    ) {
        let mut dest = self.get_field_mut_local(owner, name);
        dest.copy_from_slice(value);
    }

    /// Returns a copy of the field's data using atomic operations for supported sizes.
    /// This method respects the provided `Ordering` and is suitable for volatile access.
    /// It still acquires a read lock to ensure memory safety and prevent tearing
    /// against synchronized writers for large field sizes.
    pub fn get_field_atomic(
        &self,
        owner: TypeDescription,
        name: &str,
        ord: std::sync::atomic::Ordering,
    ) -> Vec<u8> {
        let field = self.layout.get_field(owner, name).expect("Field not found");
        let size = field.layout.size();
        let data = self.get();
        let ptr = data.as_ptr();
        let field_ptr = unsafe { ptr.add(field.position) };
        unsafe { Atomic::load_field(field_ptr, size, ord) }
    }

    /// Sets the field's data using atomic operations for supported sizes.
    /// This method respects the provided `Ordering` and is suitable for volatile access.
    /// It acquires a write lock to ensure memory safety and prevent tearing
    /// against other synchronized readers/writers for large field sizes.
    pub fn set_field_atomic(
        &self,
        owner: TypeDescription,
        name: &str,
        value: &[u8],
        ord: std::sync::atomic::Ordering,
    ) {
        let field = self.layout.get_field(owner, name).expect("Field not found");
        // Let's just use get_mut() to be safe and consistent with synchronized.
        let data = self.get_mut();
        let ptr = data.as_ptr() as *mut u8;
        let field_ptr = unsafe { ptr.add(field.position) };
        unsafe { Atomic::store_field(field_ptr, value, ord) }
    }

    unsafe fn raw_data_unsynchronized(&self) -> &[u8] {
        unsafe { &*self.data.data_ptr() }
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
        let len = self.data.read().len();
        write!(f, "FieldStorage({} bytes)", len)
    }
}
