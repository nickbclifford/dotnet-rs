//! Heap payload storage.
//!
//! The [`HeapStorage::payload_len`] and [`HeapStorage::payload_base_ptr`] helpers
//! distinguish the initialized byte extent and allocation base from
//! `size_bytes()` (which remains allocation-pressure accounting). The raw base
//! is obtained without a payload slice/reference, and its address alone grants
//! no access right.
//!
//! For vectors use the backing `Vec<u8>` length and raw base. For objects and
//! boxed values use `FieldStorage`'s backing Vec length and raw base, retaining
//! the necessary object and field synchronization; a len-only accessor must
//! not create a payload slice. The length accessor holds the field read lock
//! while reading Vec metadata through `data_ptr`, excluding field writers. For
//! strings use the UTF-16 element count times two and obtain the base directly
//! from the owned Vec or `Arc::as_ptr`, not through `CLRString`'s slice Deref.
//! String observation and reads do not create a writable contract, particularly
//! for shared interned `Arc` data.
//!
//! A retained owner and the applicable borrow/lease must cover each scoped
//! read; observing an address does not retain those guards after return. Writes,
//! copies, and atomics have separate rights and retain their existing locks
//! and barriers. These helpers provide metadata and a raw address only; callers
//! must retain the owner and establish separate synchronization for accesses.

use crate::{
    object::types::{Object, Vector},
    string::CLRString,
};
use gc_arena::{Collect, collect::Trace};
use std::{collections::HashSet, ptr::NonNull};

#[derive(Clone, Debug, PartialEq)]
pub enum HeapStorage<'gc> {
    Vec(Box<Vector<'gc>>),
    Obj(Box<Object<'gc>>),
    Str(CLRString),
    Boxed(Box<Object<'gc>>),
}

// SAFETY: F5.TracesEveryGcRef — HeapStorage is an enum where each variant either implements Collect or
// contains no GC references (like CLRString). We manually trace each variant.
unsafe impl<'gc> Collect<'gc> for HeapStorage<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        match self {
            Self::Vec(v) => v.trace(cc),
            Self::Obj(o) => o.trace(cc),
            Self::Boxed(v) => v.trace(cc),
            Self::Str(_) => {}
        }
    }
}

impl<'gc> HeapStorage<'gc> {
    /// Returns the initialized byte extent of this payload, excluding any
    /// vector/object headers, array dimensions, and backing allocation capacity.
    pub fn payload_len(&self) -> usize {
        match self {
            HeapStorage::Vec(v) => v.storage.len(),
            HeapStorage::Obj(o) => o.instance_storage.data_len(),
            HeapStorage::Str(s) => s.len() * 2,
            HeapStorage::Boxed(o) => o.instance_storage.data_len(),
        }
    }

    /// Returns the raw base address of this payload without constructing a payload reference.
    ///
    /// # Safety
    /// The owner must remain alive and the backing storage must not be mutated,
    /// reallocated, or otherwise invalidated while the returned pointer is used.
    /// For object and boxed storage, the caller must also provide synchronization
    /// against field writers. This pointer does not retain the owner or a lock,
    /// and observation does not grant permission to read, write, copy, or perform
    /// atomic access through it; each access requires its own valid guard/right.
    pub unsafe fn payload_base_ptr(&self) -> *const u8 {
        match self {
            HeapStorage::Vec(v) => v.storage.as_ptr(),
            HeapStorage::Obj(o) => {
                // SAFETY: F10.RawMemoryAccessValid — The caller upholds this helper's owner and
                // synchronization contract for the field-storage base pointer.
                unsafe { o.instance_storage.raw_data_ptr().cast_const() }
            }
            HeapStorage::Str(s) => s.raw_data_ptr(),
            HeapStorage::Boxed(o) => {
                // SAFETY: F10.RawMemoryAccessValid — The caller upholds this helper's owner and
                // synchronization contract for the field-storage base pointer.
                unsafe { o.instance_storage.raw_data_ptr().cast_const() }
            }
        }
    }

    /// Snapshots the retained base without a payload slice or an access right.
    /// Field storage takes its own read guard while inspecting the Vec descriptor;
    /// that guard ends before the returned address is used and is not retained by it.
    /// The address itself is not a GC root.
    pub(crate) fn observe_payload_base(&self) -> *const u8 {
        match self {
            HeapStorage::Obj(o) | HeapStorage::Boxed(o) => o.instance_storage.observe_data_base(),
            // SAFETY: F10.BorrowedStorageStable — The shared object borrow keeps these
            // Vec/string descriptors stable while their raw bases are obtained.
            _ => unsafe { self.payload_base_ptr() },
        }
    }

    /// Observes an address within or beyond this payload using wrapping arithmetic.
    ///
    /// This does not check bounds, retain the owner or a borrow/lock, or grant
    /// permission to dereference the returned address. A later access requires
    /// its own valid owner, synchronization, range, and access right.
    pub fn observe_address(&self, offset: usize) -> Option<NonNull<u8>> {
        let base = self.observe_payload_base();
        if base.is_null() {
            return None;
        }
        NonNull::new(base.wrapping_add(offset).cast_mut())
    }

    /// Runs a callback over exactly the checked payload range, if it is in bounds.
    pub(crate) fn try_with_data_range<T>(
        &self,
        offset: usize,
        size: usize,
        f: impl FnOnce(&[u8]) -> T,
    ) -> Option<T> {
        match self {
            HeapStorage::Vec(v) => {
                with_payload_window(v.storage.as_ptr(), v.storage.len(), offset, size, false, f)
            }
            HeapStorage::Obj(o) => o.instance_storage.try_with_data_range(offset, size, f),
            HeapStorage::Str(s) => {
                with_payload_window(s.raw_data_ptr(), s.len() * 2, offset, size, false, f)
            }
            HeapStorage::Boxed(o) => o.instance_storage.try_with_data_range(offset, size, f),
        }
    }

    /// Runs a callback over up to the requested number of payload bytes. This
    /// explicit short-read path is retained for P/Invoke's zero-filled buffers.
    pub(crate) fn with_data_up_to<T>(
        &self,
        offset: usize,
        max_size: usize,
        f: impl FnOnce(&[u8]) -> T,
    ) -> Option<T> {
        match self {
            HeapStorage::Vec(v) => with_payload_window(
                v.storage.as_ptr(),
                v.storage.len(),
                offset,
                max_size,
                true,
                f,
            ),
            HeapStorage::Obj(o) => o.instance_storage.with_data_up_to(offset, max_size, f),
            HeapStorage::Str(s) => {
                with_payload_window(s.raw_data_ptr(), s.len() * 2, offset, max_size, true, f)
            }
            HeapStorage::Boxed(o) => o.instance_storage.with_data_up_to(offset, max_size, f),
        }
    }

    pub fn size_bytes(&self) -> usize {
        match self {
            HeapStorage::Vec(v) => v.size_bytes(),
            HeapStorage::Obj(o) => o.size_bytes(),
            HeapStorage::Str(s) => s.size_bytes(),
            HeapStorage::Boxed(o) => o.size_bytes(),
        }
    }

    pub fn validate_resurrection_invariants(&self) {
        match self {
            HeapStorage::Vec(v) => v.validate_resurrection_invariants(),
            HeapStorage::Obj(o) => o.validate_resurrection_invariants(),
            HeapStorage::Str(_) => {}
            HeapStorage::Boxed(o) => o.validate_resurrection_invariants(),
        }
    }

    pub fn resurrect(
        &self,
        fc: &'gc gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
        depth: usize,
    ) {
        match self {
            HeapStorage::Vec(v) => v.resurrect(fc, visited, depth),
            HeapStorage::Obj(o) => o.resurrect(fc, visited, depth),
            HeapStorage::Boxed(o) => o.resurrect(fc, visited, depth),
            HeapStorage::Str(_) => {}
        }
    }
    pub fn as_obj(&self) -> Option<&Object<'gc>> {
        match self {
            HeapStorage::Obj(o) => Some(o),
            _ => None,
        }
    }
    pub fn as_obj_mut(&mut self) -> Option<&mut Object<'gc>> {
        match self {
            HeapStorage::Obj(o) => Some(o),
            _ => None,
        }
    }

    pub fn with_data<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        match self {
            HeapStorage::Vec(v) => f(&v.storage),
            // SAFETY: F10.RawMemoryAccessValid — The matched storage variant owns valid backing bytes, and delegated raw-pointer calls preserve the caller contract.
            HeapStorage::Str(s) => unsafe {
                let bytes = std::slice::from_raw_parts(s.as_ptr() as *const u8, s.len() * 2);
                f(bytes)
            },
            HeapStorage::Obj(o) => o.instance_storage.with_data(f),
            HeapStorage::Boxed(o) => o.instance_storage.with_data(f),
        }
    }

    pub fn with_data_mut<T>(&mut self, f: impl FnOnce(&mut [u8]) -> T) -> T {
        match self {
            HeapStorage::Vec(v) => f(&mut v.storage),
            HeapStorage::Str(_) => panic!("Cannot modify CLRString directly"),
            HeapStorage::Obj(o) => o.instance_storage.with_data_mut(f),
            HeapStorage::Boxed(o) => o.instance_storage.with_data_mut(f),
        }
    }

    /// Returns the raw payload base without taking a field access guard.
    ///
    /// The pointer's allocation provenance does not retain the owner, root it, or
    /// retain a borrow/lock; its `*mut` type does not grant a write right (in
    /// particular for strings backed by shared interned `Arc` data).
    ///
    /// # Safety
    /// The caller must retain the owner and ensure synchronization is provided
    /// elsewhere (e.g. during STW GC), with no writer or reallocation while the
    /// pointer is used. Every read, write, copy, or atomic operation needs its own
    /// valid access rights and range.
    pub unsafe fn raw_data_ptr(&self) -> *mut u8 {
        match self {
            // SAFETY: F10.RawMemoryAccessValid — The matched storage variant owns valid backing bytes, and delegated raw-pointer calls preserve the caller contract.
            HeapStorage::Vec(v) => unsafe { v.raw_data_ptr() },
            // The mutable pointer type does not grant write access to interned strings.
            HeapStorage::Str(s) => s.raw_data_ptr().cast_mut(),
            // SAFETY: F10.RawMemoryAccessValid — The matched storage variant owns valid backing bytes, and delegated raw-pointer calls preserve the caller contract.
            HeapStorage::Obj(o) => unsafe { o.instance_storage.raw_data_ptr() },
            // SAFETY: F10.RawMemoryAccessValid — The matched storage variant owns valid backing bytes, and delegated raw-pointer calls preserve the caller contract.
            HeapStorage::Boxed(o) => unsafe { o.instance_storage.raw_data_ptr() },
        }
    }
}

fn with_payload_window<T>(
    base: *const u8,
    payload_len: usize,
    offset: usize,
    requested_size: usize,
    allow_short: bool,
    f: impl FnOnce(&[u8]) -> T,
) -> Option<T> {
    let available = payload_len.checked_sub(offset)?;
    let size = if allow_short {
        requested_size.min(available)
    } else {
        requested_size
    };
    let end = offset.checked_add(size)?;
    if end > payload_len {
        return None;
    }

    // SAFETY: F10.RawMemoryAccessValid — The storage variant supplies a non-null base for its
    // retained payload, and `offset <= payload_len` was checked before pointer formation.
    let range_ptr = unsafe { base.add(offset) };
    // SAFETY: F10.RawMemoryAccessValid — `end <= payload_len` was checked before constructing
    // this requested window; the caller keeps the storage and its synchronization live while `f` runs.
    let bytes = unsafe { std::slice::from_raw_parts(range_ptr, size) };
    Some(f(bytes))
}
