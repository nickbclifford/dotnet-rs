use crate::memory::heap::HeapManager;
use dotnet_types::{TypeDescription, generics::GenericLookup};
use dotnet_utils::{ArenaId, ByteOffset, atomic::validate_atomic_access, gc::GCHandle};
use dotnet_value::{
    StackValue,
    layout::{HasLayout, LayoutManager, Scalar},
    object::{HeapStorage, Object as ObjectInstance, ObjectRef},
    pointer::ManagedPtr,
    storage::FieldStorage,
};
use std::{ptr, sync::Arc};

#[cfg(feature = "multithreaded-gc")]
use dotnet_value::{object::ObjectPtr, pointer::PointerOrigin};

#[derive(Copy, Clone)]
pub enum MemoryOwner<'gc> {
    Local(ObjectRef<'gc>),
    #[cfg(feature = "multithreaded-gc")]
    CrossArena(ObjectPtr, ArenaId),
}

impl<'gc> MemoryOwner<'gc> {
    pub fn owner_id(&self) -> ArenaId {
        match self {
            Self::Local(r) => {
                r.0.map(|h| unsafe { (*h.as_ptr()).owner_id })
                    .unwrap_or(ArenaId(0))
            }
            #[cfg(feature = "multithreaded-gc")]
            Self::CrossArena(_, tid) => *tid,
        }
    }

    pub fn with_data<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        match self {
            Self::Local(r) => r.with_data(f),
            #[cfg(feature = "multithreaded-gc")]
            Self::CrossArena(p, _) => p.with_data(f),
        }
    }

    pub fn with_data_mut<T>(&self, gc: GCHandle<'gc>, f: impl FnOnce(&mut [u8]) -> T) -> T {
        match self {
            Self::Local(r) => r.with_data_mut(gc, f),
            #[cfg(feature = "multithreaded-gc")]
            Self::CrossArena(p, _) => p.with_data_mut(gc, f),
        }
    }

    pub fn as_heap_storage<T>(&self, f: impl FnOnce(&HeapStorage<'gc>) -> T) -> T {
        match self {
            Self::Local(r) => r.as_heap_storage(f),
            #[cfg(feature = "multithreaded-gc")]
            Self::CrossArena(p, _) => p.as_heap_storage(|s| {
                // SAFETY: Casting 'static to 'gc for transient access is safe.
                f(unsafe { std::mem::transmute::<&HeapStorage<'static>, &HeapStorage<'gc>>(s) })
            }),
        }
    }
}

/// Manages unsafe memory access, enforcing bounds checks, GC write barriers, and type integrity.
pub struct RawMemoryAccess<'a, 'gc> {
    _heap: &'a HeapManager<'gc>,
}

impl<'a, 'gc> RawMemoryAccess<'a, 'gc> {
    pub fn new(heap: &'a HeapManager<'gc>) -> Self {
        Self { _heap: heap }
    }

    /// Writes a value to a memory location (owner + offset), performing necessary checks.
    ///
    /// # Safety
    /// The caller must ensure that `offset` represents a valid memory location if `owner` is None.
    pub unsafe fn write_unaligned(
        &mut self,
        gc: GCHandle<'gc>,
        owner: Option<MemoryOwner<'gc>>,
        offset: ByteOffset,
        value: StackValue<'gc>,
        layout: &LayoutManager,
    ) -> Result<(), String> {
        if let Some(owner) = owner {
            owner.as_heap_storage(|_storage| {}); // Ensure object is valid and magic matches

            // Get layout before locking to avoid deadlock
            let dest_layout = self.get_layout_from_owner(owner);

            // SAFETY: with_data_mut ensures the lock is held for the duration of the closure.
            // perform_write will copy the data.
            owner.with_data_mut(gc, |data| {
                let base = data.as_ptr();
                let len = data.len();
                let ptr = (base as usize).wrapping_add(offset.0) as *mut u8;
                validate_atomic_access(ptr as *const u8, false);

                // 1. Bounds Check
                self.check_bounds_internal(ptr, base, len, layout.size().as_usize())?;

                // 2. Integrity Check
                self.check_integrity_internal_with_layout(ptr, dest_layout, base, layout)?;

                // 3. Perform Write
                unsafe { self.perform_write(gc, ptr, Some(owner), value, layout) }
            })
        } else {
            let ptr = offset.0 as *mut u8;
            if ptr.is_null() {
                return Err("NullReferenceException: writing to unmanaged null pointer".into());
            }
            validate_atomic_access(ptr as *const u8, false);
            // SAFETY: Caller ensures ptr is valid.
            unsafe {
                self.perform_write(gc, ptr, None, value, layout)?;
            }
            Ok(())
        }
    }

    /// Reads a value from a memory location.
    ///
    /// # Safety
    /// The caller must ensure that `offset` represents a valid memory location if `owner` is None.
    pub unsafe fn read_unaligned(
        &self,
        gc: GCHandle<'gc>,
        owner: Option<MemoryOwner<'gc>>,
        offset: ByteOffset,
        layout: &LayoutManager,
        type_desc: Option<TypeDescription>,
    ) -> Result<StackValue<'gc>, String> {
        if let Some(owner) = owner {
            owner.as_heap_storage(|_storage| {}); // Ensure object is valid and magic matches

            // Get layout before locking to avoid deadlock
            let src_layout = self.get_layout_from_owner(owner);

            // SAFETY: with_data ensures the lock is held for the duration of the closure.
            // perform_read will read the data.
            owner.with_data(|data| {
                let base = data.as_ptr();
                let len = data.len();
                let ptr = (base as usize).wrapping_add(offset.0) as *const u8;
                validate_atomic_access(ptr, false);

                // 1. Bounds Check
                self.check_bounds_internal(ptr as *mut u8, base, len, layout.size().as_usize())?;

                // 2. Read Safety Check
                if offset.0 != 0 || src_layout.is_some() {
                    check_read_safety(layout, src_layout.as_ref(), offset.0);
                }

                // 3. Perform Read
                unsafe { self.perform_read(gc, ptr, Some(owner), layout, type_desc) }
            })
        } else {
            let ptr = offset.0 as *const u8;
            if ptr.is_null() {
                return Err("NullReferenceException: reading from unmanaged null pointer".into());
            }
            validate_atomic_access(ptr, false);
            // SAFETY: Caller ensures ptr is valid.
            unsafe { self.perform_read(gc, ptr, None, layout, type_desc) }
        }
    }

    /// Safely writes raw bytes to a memory location.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `offset` represents a valid memory location if `owner` is None.
    /// If `owner` is provided, it must be the object that contains the memory to ensure GC safety.
    pub unsafe fn write_bytes(
        &mut self,
        gc: GCHandle<'gc>,
        owner: Option<MemoryOwner<'gc>>,
        offset: ByteOffset,
        data: &[u8],
    ) -> Result<(), String> {
        if let Some(owner) = owner {
            owner.as_heap_storage(|_storage| {});

            // Get layout before locking to avoid deadlock
            #[cfg(feature = "multithreaded-gc")]
            let layout = self.get_layout_from_owner(owner);
            #[cfg(feature = "multithreaded-gc")]
            let owner_tid = owner.owner_id();

            owner.with_data_mut(gc, |obj_data| {
                let base = obj_data.as_ptr();
                let len = obj_data.len();
                let ptr = (base as usize).wrapping_add(offset.0) as *mut u8;
                validate_atomic_access(ptr as *const u8, false);

                // Bounds Check
                self.check_bounds_internal(ptr, base, len, data.len())?;

                // Perform Write
                unsafe {
                    ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
                }

                #[cfg(feature = "multithreaded-gc")]
                {
                    if let Some(layout) = layout {
                        unsafe {
                            self.record_refs_in_range(
                                gc,
                                base,
                                &layout,
                                owner_tid,
                                offset.as_usize(),
                                offset.as_usize() + data.len(),
                            );
                        }
                    }
                }
                Ok(())
            })
        } else {
            let ptr = offset.0 as *mut u8;
            if ptr.is_null() {
                return Err(
                    "NullReferenceException: writing bytes to unmanaged null pointer".into(),
                );
            }
            validate_atomic_access(ptr as *const u8, false);
            unsafe {
                ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
            }
            Ok(())
        }
    }

    /// Safely reads raw bytes from a memory location.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `offset` represents a valid memory location if `owner` is None.
    /// If `owner` is provided, it must be the object that contains the memory to ensure GC safety.
    pub unsafe fn read_bytes(
        &self,
        owner: Option<MemoryOwner<'gc>>,
        offset: ByteOffset,
        dest: &mut [u8],
    ) -> Result<(), String> {
        if let Some(owner) = owner {
            owner.with_data(|data| {
                let start = offset.0;
                let end = start + dest.len();
                if end > data.len() {
                    return Err(format!(
                        "Read out of bounds: range [{}, {}) in object of size {}",
                        start,
                        end,
                        data.len()
                    ));
                }
                dest.copy_from_slice(&data[start..end]);
                Ok(())
            })
        } else {
            let ptr = offset.0 as *const u8;
            validate_atomic_access(ptr, false);
            // SAFETY: Caller ensures ptr is valid.
            unsafe {
                ptr::copy_nonoverlapping(ptr, dest.as_mut_ptr(), dest.len());
            }
            Ok(())
        }
    }

    /// Atomically compares and exchanges a value in memory.
    ///
    /// # Safety
    /// Caller must ensure the offset and size are valid for the owner object.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn compare_exchange_atomic(
        &mut self,
        gc: GCHandle<'gc>,
        owner: Option<MemoryOwner<'gc>>,
        offset: ByteOffset,
        expected: u64,
        new: u64,
        size: usize,
        success: dotnet_utils::sync::Ordering,
        failure: dotnet_utils::sync::Ordering,
    ) -> Result<u64, u64> {
        use dotnet_utils::atomic::{AtomicAccess, StandardAtomicAccess};

        if let Some(owner) = owner {
            owner.with_data_mut(gc, |data| {
                let base = data.as_ptr();
                let len = data.len();
                let ptr = unsafe { base.add(offset.as_usize()) };

                if let Err(e) =
                    self.check_bounds_internal(ptr as *mut u8, base as *mut u8, len, size)
                {
                    panic!("Atomic operation bounds check failed: {}", e);
                }

                unsafe {
                    StandardAtomicAccess::compare_exchange_atomic(
                        ptr as *mut u8,
                        size,
                        expected,
                        new,
                        success,
                        failure,
                    )
                }
            })
        } else {
            let ptr = offset.as_usize() as *mut u8;
            unsafe {
                StandardAtomicAccess::compare_exchange_atomic(
                    ptr, size, expected, new, success, failure,
                )
            }
        }
    }

    /// Atomically exchanges a value in memory.
    ///
    /// # Safety
    /// Caller must ensure the offset and size are valid for the owner object.
    pub unsafe fn exchange_atomic(
        &mut self,
        gc: GCHandle<'gc>,
        owner: Option<MemoryOwner<'gc>>,
        offset: ByteOffset,
        value: u64,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<u64, String> {
        use dotnet_utils::atomic::{AtomicAccess, StandardAtomicAccess};

        if let Some(owner) = owner {
            owner.with_data_mut(gc, |data| {
                let base = data.as_ptr();
                let len = data.len();
                let ptr = unsafe { base.add(offset.as_usize()) };

                self.check_bounds_internal(ptr as *mut u8, base as *mut u8, len, size)?;

                Ok(unsafe {
                    StandardAtomicAccess::exchange_atomic(ptr as *mut u8, size, value, ordering)
                })
            })
        } else {
            let ptr = offset.as_usize() as *mut u8;
            Ok(unsafe { StandardAtomicAccess::exchange_atomic(ptr, size, value, ordering) })
        }
    }

    /// Atomically loads a value from memory.
    ///
    /// # Safety
    /// Caller must ensure the offset and size are valid for the owner object.
    pub unsafe fn load_atomic(
        &self,
        owner: Option<MemoryOwner<'gc>>,
        offset: ByteOffset,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<u64, String> {
        use dotnet_utils::atomic::{AtomicAccess, StandardAtomicAccess};
        if let Some(owner) = owner {
            owner.with_data(|data| {
                let base = data.as_ptr();
                let len = data.len();
                let ptr = unsafe { base.add(offset.as_usize()) };
                self.check_bounds_internal(ptr as *mut u8, base as *mut u8, len, size)?;
                Ok(unsafe { StandardAtomicAccess::load_atomic(ptr, size, ordering) })
            })
        } else {
            let ptr = offset.as_usize() as *const u8;
            Ok(unsafe { StandardAtomicAccess::load_atomic(ptr, size, ordering) })
        }
    }

    /// Atomically stores a value to memory.
    ///
    /// # Safety
    /// Caller must ensure the offset and size are valid for the owner object.
    pub unsafe fn store_atomic(
        &mut self,
        gc: GCHandle<'gc>,
        owner: Option<MemoryOwner<'gc>>,
        offset: ByteOffset,
        value: u64,
        size: usize,
        ordering: dotnet_utils::sync::Ordering,
    ) -> Result<(), String> {
        use dotnet_utils::atomic::{AtomicAccess, StandardAtomicAccess};
        if let Some(owner) = owner {
            owner.with_data_mut(gc, |data| {
                let base = data.as_ptr();
                let len = data.len();
                let ptr = unsafe { base.add(offset.as_usize()) };
                self.check_bounds_internal(ptr as *mut u8, base as *mut u8, len, size)?;
                unsafe {
                    StandardAtomicAccess::store_atomic(ptr as *mut u8, size, value, ordering);
                }
                Ok(())
            })
        } else {
            let ptr = offset.as_usize() as *mut u8;
            unsafe {
                StandardAtomicAccess::store_atomic(ptr, size, value, ordering);
            }
            Ok(())
        }
    }

    pub fn get_storage_base(&self, owner: ObjectRef<'gc>) -> (*const u8, usize) {
        if let Some(h) = owner.0 {
            let obj = h.borrow();
            match &obj.storage {
                HeapStorage::Obj(o) | HeapStorage::Boxed(o) => {
                    let guard = o.instance_storage.get();
                    (guard.as_ptr(), guard.len())
                }
                HeapStorage::Vec(v) => {
                    let guard = v.get();
                    (guard.as_ptr(), guard.len())
                }
                _ => (ptr::null(), 0),
            }
        } else {
            (ptr::null(), 0)
        }
    }

    fn check_bounds_internal(
        &self,
        ptr: *mut u8,
        base: *const u8,
        len: usize,
        size: usize,
    ) -> Result<(), String> {
        if !base.is_null() {
            let base_addr = base as usize;
            let ptr_addr = ptr as usize;

            if ptr_addr < base_addr
                || (ptr_addr - base_addr)
                    .checked_add(size)
                    .is_none_or(|end| end > len)
            {
                return Err(format!(
                    "Access out of bounds. ptr={:p}, base={:p}, offset={}, size={}, len={}",
                    ptr,
                    base,
                    ptr_addr.wrapping_sub(base_addr),
                    size,
                    len
                ));
            }
        }
        Ok(())
    }

    fn check_integrity_internal_with_layout(
        &self,
        ptr: *mut u8,
        dest_layout: Option<LayoutManager>,
        base: *const u8,
        src_layout: &LayoutManager,
    ) -> Result<(), String> {
        if !base.is_null() {
            let base_addr = base as usize;
            let ptr_addr = ptr as usize;
            let offset = ptr_addr.wrapping_sub(base_addr);

            if let Some(dl) = dest_layout {
                validate_ref_integrity(
                    &dl,
                    0,
                    offset,
                    offset + src_layout.size().as_usize(),
                    src_layout,
                );
            }
        }
        Ok(())
    }

    pub fn get_layout_from_owner(&self, owner: MemoryOwner<'gc>) -> Option<LayoutManager> {
        owner.as_heap_storage(|storage| match storage {
            HeapStorage::Obj(o) => Some(LayoutManager::Field(
                o.instance_storage.layout().as_ref().clone(),
            )),
            HeapStorage::Vec(v) => Some(LayoutManager::Array(v.layout.clone())),
            HeapStorage::Boxed(o) => Some(LayoutManager::Field(
                o.instance_storage.layout().as_ref().clone(),
            )),
            _ => None,
        })
    }

    pub(crate) unsafe fn perform_write(
        &mut self,
        _gc: GCHandle<'gc>,
        ptr: *mut u8,
        _owner: Option<MemoryOwner<'gc>>,
        value: StackValue<'gc>,
        layout: &LayoutManager,
    ) -> Result<(), String> {
        // Safety: `write_unaligned` ensures `ptr` is valid.
        unsafe {
            if ptr.is_null() {
                return Err("RawMemoryAccess::perform_write called with null pointer!".to_string());
            }

            match layout {
                LayoutManager::Scalar(s) => match s {
                    Scalar::Int8 => {
                        let v = extract_int(value)? as i8;
                        ptr::write_unaligned(ptr as *mut i8, v);
                    }
                    Scalar::UInt8 => {
                        let v = extract_int(value)? as u8;
                        ptr::write_unaligned(ptr, v);
                    }
                    Scalar::Int16 => {
                        let v = extract_int(value)? as i16;
                        ptr::write_unaligned(ptr as *mut i16, v);
                    }
                    Scalar::UInt16 => {
                        let v = extract_int(value)? as u16;
                        ptr::write_unaligned(ptr as *mut u16, v);
                    }
                    Scalar::Int32 => {
                        let v = extract_int(value)?;
                        ptr::write_unaligned(ptr as *mut i32, v);
                    }
                    Scalar::Int64 => {
                        let v = extract_long(value)?;
                        ptr::write_unaligned(ptr as *mut i64, v);
                    }
                    Scalar::NativeInt => {
                        let v = extract_native_int(value)?;
                        ptr::write_unaligned(ptr as *mut isize, v);
                    }
                    Scalar::Float32 => {
                        let v = extract_float(value)? as f32;
                        ptr::write_unaligned(ptr as *mut f32, v);
                    }
                    Scalar::Float64 => {
                        let v = extract_float(value)?;
                        ptr::write_unaligned(ptr as *mut f64, v);
                    }
                    Scalar::ManagedPtr => {
                        if let StackValue::ManagedPtr(m) = value {
                            if ptr.is_null() {
                                panic!("perform_write: ptr is null!");
                            }
                            m.write(std::slice::from_raw_parts_mut(ptr, ManagedPtr::SIZE));

                            #[cfg(feature = "multithreaded-gc")]
                            if let Some(owner) = _owner {
                                let owner_tid = owner.owner_id();
                                match m.origin {
                                    PointerOrigin::Heap(r) => {
                                        if let Some(h) = r.0 {
                                            // SAFETY: During write, we hold a lock on the owner.
                                            // If h is the same as the owner, we must not call borrow() as it would deadlock.
                                            // owner_id is immutable after object creation, so we can safely read it via as_ptr().
                                            let target_tid =
                                                (*(*gc_arena::Gc::as_ptr(h)).as_ptr()).owner_id;
                                            if target_tid != owner_tid {
                                                dotnet_utils::gc::record_cross_arena_ref(
                                                    target_tid,
                                                    h.as_ptr() as usize,
                                                );
                                            }
                                        }
                                    }
                                    PointerOrigin::CrossArenaObjectRef(p, target_tid) => {
                                        if target_tid != owner_tid {
                                            dotnet_utils::gc::record_cross_arena_ref(
                                                target_tid,
                                                p.as_ptr() as usize,
                                            );
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        } else {
                            return Err("Expected ManagedPtr".into());
                        }
                    }
                    Scalar::ObjectRef => {
                        if let StackValue::ObjectRef(r) = value {
                            // Use ObjectRef::write() to properly serialize the pointer.
                            // This ensures cross-arena references are tagged correctly.
                            r.write(std::slice::from_raw_parts_mut(ptr, ObjectRef::SIZE));

                            #[cfg(feature = "multithreaded-gc")]
                            if let Some(owner) = _owner {
                                let owner_tid = owner.owner_id();
                                if let Some((val_ptr, target_tid)) =
                                    r.as_ptr_info().filter(|&(_, tid)| tid != owner_tid)
                                {
                                    dotnet_utils::gc::record_cross_arena_ref(
                                        target_tid,
                                        val_ptr.as_ptr() as usize,
                                    );
                                }
                            }
                        } else {
                            return Err("Expected ObjectRef".into());
                        }
                    }
                },
                LayoutManager::Field(flm) => {
                    if let StackValue::ValueType(src_obj) = value {
                        let src_ptr = src_obj.instance_storage.raw_data_ptr();
                        ptr::copy_nonoverlapping(src_ptr, ptr, flm.size().as_usize());

                        #[cfg(feature = "multithreaded-gc")]
                        if let Some(owner) = _owner {
                            let owner_tid = owner.owner_id();
                            self.record_refs_recursive(_gc, ptr, layout, owner_tid);
                        }
                    } else {
                        return Err("Expected ValueType for Struct write".into());
                    }
                }
                LayoutManager::Array(_) => return Err("Cannot write entire array unaligned".into()),
            }
            Ok(())
        }
    }

    #[cfg(feature = "multithreaded-gc")]
    unsafe fn record_refs_recursive(
        &self,
        gc: GCHandle<'gc>,
        ptr: *const u8,
        layout: &LayoutManager,
        owner_tid: dotnet_utils::ArenaId,
    ) {
        match layout {
            LayoutManager::Scalar(Scalar::ObjectRef) => {
                let mut buf = [0u8; ObjectRef::SIZE];
                unsafe {
                    ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), ObjectRef::SIZE);
                    let r = ObjectRef::read_branded(&buf, &gc);
                    if let Some(h) = r.0 {
                        // SAFETY: owner_id is immutable; use raw access to avoid deadlock with write-locked owner.
                        let target_tid = (*(*gc_arena::Gc::as_ptr(h)).as_ptr()).owner_id;
                        if target_tid != owner_tid {
                            dotnet_utils::gc::record_cross_arena_ref(
                                target_tid,
                                h.as_ptr() as usize,
                            );
                        }
                    }
                }
            }
            LayoutManager::Scalar(Scalar::ManagedPtr) => {
                let info = unsafe {
                    ManagedPtr::read_branded(std::slice::from_raw_parts(ptr, ManagedPtr::SIZE), &gc)
                };
                match info.origin {
                    PointerOrigin::Heap(r) => {
                        if let Some(h) = r.0 {
                            // SAFETY: owner_id is immutable; use raw access to avoid deadlock with write-locked owner.
                            let target_tid =
                                unsafe { (*(*gc_arena::Gc::as_ptr(h)).as_ptr()).owner_id };
                            if target_tid != owner_tid {
                                unsafe {
                                    dotnet_utils::gc::record_cross_arena_ref(
                                        target_tid,
                                        h.as_ptr() as usize,
                                    );
                                }
                            }
                        }
                    }
                    PointerOrigin::CrossArenaObjectRef(p, target_tid) => {
                        if target_tid != owner_tid {
                            dotnet_utils::gc::record_cross_arena_ref(
                                target_tid,
                                p.as_ptr() as usize,
                            );
                        }
                    }
                    _ => {}
                }
            }
            LayoutManager::Field(flm) => {
                for field in flm.fields.values() {
                    if field.layout.is_or_contains_refs() {
                        unsafe {
                            self.record_refs_recursive(
                                gc,
                                ptr.add(field.position.as_usize()),
                                &field.layout,
                                owner_tid,
                            );
                        }
                    }
                }
            }
            LayoutManager::Array(arr) => {
                if arr.element_layout.is_or_contains_refs() {
                    let elem_size = arr.element_layout.size().as_usize();
                    for i in 0..arr.length {
                        unsafe {
                            self.record_refs_recursive(
                                gc,
                                ptr.add(i * elem_size),
                                &arr.element_layout,
                                owner_tid,
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    #[cfg(feature = "multithreaded-gc")]
    unsafe fn record_refs_in_range(
        &self,
        gc: GCHandle<'gc>,
        ptr: *const u8, // Base of the layout
        layout: &LayoutManager,
        owner_tid: dotnet_utils::ArenaId,
        range_start: usize,
        range_end: usize,
    ) {
        if !layout.is_or_contains_refs() {
            return;
        }
        match layout {
            LayoutManager::Scalar(Scalar::ObjectRef)
            | LayoutManager::Scalar(Scalar::ManagedPtr) => {
                // If the scalar overlaps at all with the written range, we should re-record it
                // because it might have been partially or fully overwritten.
                unsafe { self.record_refs_recursive(gc, ptr, layout, owner_tid) };
            }
            LayoutManager::Field(flm) => {
                for field in flm.fields.values() {
                    let f_start = field.position.as_usize();
                    let f_end = f_start + field.layout.size().as_usize();
                    if f_start < range_end && f_end > range_start {
                        unsafe {
                            self.record_refs_in_range(
                                gc,
                                ptr.add(f_start),
                                &field.layout,
                                owner_tid,
                                range_start.saturating_sub(f_start),
                                range_end.saturating_sub(f_start),
                            );
                        }
                    }
                }
            }
            LayoutManager::Array(alm) => {
                let elem_size = alm.element_layout.size().as_usize();
                if elem_size > 0 {
                    let start_idx = range_start / elem_size;
                    let end_idx = range_end.div_ceil(elem_size);
                    let start_idx = start_idx.min(alm.length);
                    let end_idx = end_idx.min(alm.length);

                    for i in start_idx..end_idx {
                        let f_start = i * elem_size;
                        unsafe {
                            self.record_refs_in_range(
                                gc,
                                ptr.add(f_start),
                                &alm.element_layout,
                                owner_tid,
                                range_start.saturating_sub(f_start),
                                range_end.saturating_sub(f_start),
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) unsafe fn perform_read(
        &self,
        gc: GCHandle<'gc>,
        ptr: *const u8,
        _owner: Option<MemoryOwner<'gc>>,
        layout: &LayoutManager,
        type_desc: Option<TypeDescription>,
    ) -> Result<StackValue<'gc>, String> {
        // SAFETY: The caller must ensure `ptr` is valid for reads and within bounds.
        // This is verified by `read_unaligned` before calling this method.
        unsafe {
            if ptr.is_null() {
                return Err("RawMemoryAccess::perform_read called with null pointer!".to_string());
            }

            Ok(match layout {
                LayoutManager::Scalar(s) => match s {
                    Scalar::Int8 => StackValue::Int32(ptr::read_unaligned(ptr as *const i8) as i32),
                    Scalar::UInt8 => StackValue::Int32(ptr::read_unaligned(ptr) as i32),
                    Scalar::Int16 => {
                        StackValue::Int32(ptr::read_unaligned(ptr as *const i16) as i32)
                    }
                    Scalar::UInt16 => {
                        StackValue::Int32(ptr::read_unaligned(ptr as *const u16) as i32)
                    }
                    Scalar::Int32 => StackValue::Int32(ptr::read_unaligned(ptr as *const i32)),
                    Scalar::Int64 => StackValue::Int64(ptr::read_unaligned(ptr as *const i64)),
                    Scalar::NativeInt => {
                        StackValue::NativeInt(ptr::read_unaligned(ptr as *const isize))
                    }
                    Scalar::Float32 => {
                        StackValue::NativeFloat(ptr::read_unaligned(ptr as *const f32) as f64)
                    }
                    Scalar::Float64 => {
                        StackValue::NativeFloat(ptr::read_unaligned(ptr as *const f64))
                    }
                    Scalar::ObjectRef => {
                        let mut buf = [0u8; ObjectRef::SIZE];
                        ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), ObjectRef::SIZE);
                        StackValue::ObjectRef(ObjectRef::read_branded(&buf, &gc))
                    }
                    Scalar::ManagedPtr => {
                        let info = ManagedPtr::read_branded(
                            std::slice::from_raw_parts(ptr, ManagedPtr::SIZE),
                            &gc,
                        );

                        let actual_desc = type_desc.unwrap_or(TypeDescription::from_raw(
                            dotnet_types::resolution::ResolutionS::new(ptr::null()),
                            None,
                            std::mem::zeroed(),
                        ));

                        let m = ManagedPtr::from_info_full(info, actual_desc, false);
                        StackValue::ManagedPtr(m)
                    }
                },
                LayoutManager::Field(flm) => {
                    if let Some(desc) = type_desc {
                        let size = flm.size();
                        let mut data = vec![0u8; size.as_usize()];
                        ptr::copy_nonoverlapping(ptr, data.as_mut_ptr(), size.as_usize());

                        let storage = FieldStorage::new(Arc::new(flm.clone()), data);
                        let obj = ObjectInstance::new(desc, GenericLookup::default(), storage);

                        StackValue::ValueType(obj)
                    } else {
                        return Err("Struct read requires TypeDescription, which is not passed to read_unaligned".into());
                    }
                }
                _ => return Err("Array read not supported".to_string()),
            })
        }
    }
}

fn extract_int(val: StackValue) -> Result<i32, String> {
    match val {
        StackValue::Int32(v) => Ok(v),
        _ => Err(format!("Expected Int32, got {:?}", val)),
    }
}

fn extract_long(val: StackValue) -> Result<i64, String> {
    match val {
        StackValue::Int64(v) => Ok(v),
        _ => Err(format!("Expected Int64, got {:?}", val)),
    }
}

fn extract_native_int(val: StackValue) -> Result<isize, String> {
    match val {
        StackValue::NativeInt(v) => Ok(v),
        _ => Err(format!("Expected NativeInt, got {:?}", val)),
    }
}

fn extract_float(val: StackValue) -> Result<f64, String> {
    match val {
        StackValue::NativeFloat(v) => Ok(v),
        _ => Err(format!("Expected NativeFloat, got {:?}", val)),
    }
}

// Integrity Checks

pub fn check_read_safety(
    result_layout: &LayoutManager,
    src_layout: Option<&LayoutManager>,
    src_ptr_offset: usize,
) {
    check_refs_in_layout(result_layout, 0, &mut |ref_offset| {
        let target_src = src_ptr_offset + ref_offset;

        if let Some(sl) = src_layout {
            if !has_ref_at(sl, target_src) {
                panic!(
                    "Heap Corruption: Reading ObjectRef from non-ref memory at offset {}",
                    target_src
                );
            }
            if !target_src.is_multiple_of(8) {
                panic!(
                    "Heap Corruption: Reading misaligned ObjectRef at {}",
                    target_src
                );
            }
        } else {
            // Reading Ref from unmanaged memory (src_layout is None)
            // Relaxed for Stack Scanning compatibility (Unsafe Code) and Unsafe.ReadUnaligned
        }
    });
}

pub fn has_ref_at(layout: &LayoutManager, offset: usize) -> bool {
    match layout {
        LayoutManager::Scalar(s) => match s {
            Scalar::ObjectRef | Scalar::ManagedPtr => offset == 0,
            _ => false,
        },
        LayoutManager::Field(fm) => {
            for f in fm.fields.values() {
                if offset >= f.position.as_usize()
                    && offset < (f.position + f.layout.size()).as_usize()
                {
                    return has_ref_at(&f.layout, offset - f.position.as_usize());
                }
            }
            false
        }
        LayoutManager::Array(am) => {
            let elem_size = am.element_layout.size();
            if elem_size.as_usize() == 0 {
                return false;
            }
            let idx = offset / elem_size.as_usize();
            if idx >= am.length {
                return false;
            }
            let rel = offset % elem_size.as_usize();
            has_ref_at(&am.element_layout, rel)
        }
    }
}

fn check_refs_in_layout<F>(layout: &LayoutManager, base: usize, callback: &mut F)
where
    F: FnMut(usize) + ?Sized,
{
    match layout {
        LayoutManager::Scalar(s) => match s {
            Scalar::ObjectRef | Scalar::ManagedPtr => callback(base),
            _ => {}
        },
        LayoutManager::Field(fm) => {
            for f in fm.fields.values() {
                check_refs_in_layout(&f.layout, base + f.position.as_usize(), callback);
            }
        }
        LayoutManager::Array(am) => {
            if am.element_layout.is_or_contains_refs() {
                let sz = am.element_layout.size();
                for i in 0..am.length {
                    check_refs_in_layout(&am.element_layout, base + (sz * i).as_usize(), callback);
                }
            }
        }
    }
}

fn validate_ref_integrity(
    dest_layout: &LayoutManager,
    base_offset: usize,
    range_start: usize,
    range_end: usize,
    src_layout: &LayoutManager,
) {
    match dest_layout {
        LayoutManager::Scalar(s) => match s {
            Scalar::ObjectRef | Scalar::ManagedPtr => {
                let ref_start = base_offset;
                let ref_end = base_offset + 8;

                if ref_start < range_end && ref_end > range_start {
                    if ref_start < range_start {
                        panic!(
                            "Heap Corruption: Write starts in the middle of an ObjectRef at {}",
                            ref_start
                        );
                    }

                    if ref_end > range_end {
                        panic!(
                            "Heap Corruption: Write ends in the middle of an ObjectRef at {}",
                            ref_start
                        );
                    }

                    let src_offset = ref_start - range_start;
                    if !has_ref_at(src_layout, src_offset) {
                        panic!(
                            "Heap Corruption: Writing non-ref data over ObjectRef at offset {}",
                            ref_start
                        );
                    }

                    if !ref_start.is_multiple_of(8) {
                        panic!(
                            "Heap Corruption: Misaligned ObjectRef in destination at {}",
                            ref_start
                        );
                    }
                }
            }
            _ => {}
        },
        LayoutManager::Field(fm) => {
            for f in fm.fields.values() {
                let f_start = base_offset + f.position.as_usize();
                let f_end = f_start + f.layout.size().as_usize();
                if f_start < range_end && f_end > range_start {
                    validate_ref_integrity(&f.layout, f_start, range_start, range_end, src_layout);
                }
            }
        }
        LayoutManager::Array(am) => {
            if am.element_layout.is_or_contains_refs() {
                let elem_size = am.element_layout.size();
                if elem_size.as_usize() == 0 {
                    return;
                }

                let rel_start = range_start.saturating_sub(base_offset);
                let rel_end = range_end.saturating_sub(base_offset);

                let start_idx = rel_start / elem_size.as_usize();
                let end_idx = rel_end.div_ceil(elem_size.as_usize());
                let end_idx = end_idx.min(am.length);

                for i in start_idx..end_idx {
                    validate_ref_integrity(
                        &am.element_layout,
                        base_offset + (elem_size * i).as_usize(),
                        range_start,
                        range_end,
                        src_layout,
                    );
                }
            }
        }
    }
}
