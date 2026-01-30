use dotnet_types::TypeDescription;
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    layout::{HasLayout, LayoutManager, Scalar},
    object::{HeapStorage, ManagedPtrMetadata, Object as ObjectInstance, ObjectRef, ValueType},
    pointer::{ManagedPtr, ManagedPtrOwner},
    storage::FieldStorage,
    StackValue,
};
use std::{ptr, sync::Arc};

/// Manages unsafe memory access, enforcing bounds checks, GC write barriers, and type integrity.
pub struct RawMemoryAccess<'gc> {
    gc: GCHandle<'gc>,
}

impl<'gc> RawMemoryAccess<'gc> {
    pub fn new(gc: GCHandle<'gc>) -> Self {
        Self { gc }
    }

    /// Writes a value to a memory location (pointer + optional owner), performing necessary checks.
    ///
    /// # Safety
    /// The caller must ensure that `ptr` is valid for writes if `owner` is None.
    pub unsafe fn write_unaligned(
        &mut self,
        ptr: *mut u8,
        owner: Option<ManagedPtrOwner<'gc>>,
        value: StackValue<'gc>,
        layout: &LayoutManager,
    ) -> Result<(), String> {
        // 1. Bounds Check
        self.check_bounds(ptr, owner, layout.size())?;

        // 2. Integrity Check
        self.check_integrity(ptr, owner, layout)?;

        // 3. Perform Write & Write Barrier
        unsafe {
            self.perform_write(ptr, owner, value, layout)?;
        }

        Ok(())
    }

    /// Reads a value from a memory location.
    ///
    /// # Safety
    /// The caller must ensure that `ptr` is valid for reads if `owner` is None.
    pub unsafe fn read_unaligned(
        &self,
        ptr: *const u8,
        owner: Option<ManagedPtrOwner<'gc>>,
        layout: &LayoutManager,
        type_desc: Option<TypeDescription>,
    ) -> Result<StackValue<'gc>, String> {
        // 1. Bounds Check
        self.check_bounds(ptr as *mut u8, owner, layout.size())?;

        // 2. Read Safety Check
        let src_layout = if let Some(owner) = owner {
            self.get_layout_from_owner(owner)
        } else {
            None
        };

        let offset = if let Some(owner) = owner {
            let (base, _) = self.get_storage_base(owner);
            if !base.is_null() {
                (ptr as usize).wrapping_sub(base as usize)
            } else {
                0
            }
        } else {
            0
        };

        if offset != 0 || src_layout.is_some() {
            check_read_safety(layout, src_layout.as_ref(), offset);
        } else if layout.is_or_contains_refs() {
            return Err(
                "Heap Corruption: Reading ObjectRef from unmanaged memory is unsafe".to_string(),
            );
        }

        // 3. Perform Read
        unsafe { self.perform_read(ptr, owner, layout, type_desc) }
    }

    fn get_storage_base(&self, owner: ManagedPtrOwner<'gc>) -> (*const u8, usize) {
        match owner {
            ManagedPtrOwner::Heap(h) => {
                let obj = h.borrow();
                match &obj.storage {
                    HeapStorage::Obj(o) | HeapStorage::Boxed(ValueType::Struct(o)) => {
                        let guard = o.instance_storage.get();
                        (guard.as_ptr(), guard.len())
                    }
                    HeapStorage::Vec(v) => {
                        let guard = v.get();
                        (guard.as_ptr(), guard.len())
                    }
                    _ => (ptr::null(), 0),
                }
            }
            ManagedPtrOwner::Stack(s) => {
                let o = unsafe { s.as_ref() };
                let guard = o.instance_storage.get();
                (guard.as_ptr(), guard.len())
            }
        }
    }

    fn check_bounds(
        &self,
        ptr: *mut u8,
        owner: Option<ManagedPtrOwner<'gc>>,
        size: usize,
    ) -> Result<(), String> {
        if let Some(owner) = owner {
            let (base, len) = self.get_storage_base(owner);

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
        }
        Ok(())
    }

    fn check_integrity(
        &self,
        ptr: *mut u8,
        owner: Option<ManagedPtrOwner<'gc>>,
        src_layout: &LayoutManager,
    ) -> Result<(), String> {
        if let Some(owner) = owner {
            let (base, _) = self.get_storage_base(owner);

            if !base.is_null() {
                let base_addr = base as usize;
                let ptr_addr = ptr as usize;
                let offset = ptr_addr.wrapping_sub(base_addr);

                let dest_layout = self.get_layout_from_owner(owner);

                if let Some(dl) = dest_layout {
                    validate_ref_integrity(&dl, 0, offset, offset + src_layout.size(), src_layout);
                }
            }
        }
        Ok(())
    }

    fn get_layout_from_owner(&self, owner: ManagedPtrOwner<'gc>) -> Option<LayoutManager> {
        match owner {
            ManagedPtrOwner::Heap(h) => {
                let obj = h.borrow();
                match &obj.storage {
                    HeapStorage::Obj(o) => Some(LayoutManager::FieldLayoutManager(
                        o.instance_storage.layout().as_ref().clone(),
                    )),
                    HeapStorage::Vec(v) => {
                        Some(LayoutManager::ArrayLayoutManager(v.layout.clone()))
                    }
                    HeapStorage::Boxed(v) => match v {
                        ValueType::Struct(o) => Some(LayoutManager::FieldLayoutManager(
                            o.instance_storage.layout().as_ref().clone(),
                        )),
                        ValueType::Pointer(_) => Some(LayoutManager::Scalar(Scalar::ManagedPtr)),
                        _ => None,
                    },
                    _ => None,
                }
            }
            ManagedPtrOwner::Stack(s) => {
                let o = unsafe { s.as_ref() };
                Some(LayoutManager::FieldLayoutManager(
                    o.instance_storage.layout().as_ref().clone(),
                ))
            }
        }
    }

    unsafe fn perform_write(
        &mut self,
        ptr: *mut u8,
        owner: Option<ManagedPtrOwner<'gc>>,
        value: StackValue<'gc>,
        layout: &LayoutManager,
    ) -> Result<(), String> {
        match layout {
            LayoutManager::Scalar(s) => match s {
                Scalar::Int8 => {
                    let v = extract_int(value)? as i8;
                    ptr::write_unaligned(ptr as *mut i8, v);
                }
                Scalar::Int16 => {
                    let v = extract_int(value)? as i16;
                    ptr::write_unaligned(ptr as *mut i16, v);
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
                        m.write_ptr_only(std::slice::from_raw_parts_mut(
                            ptr,
                            ManagedPtr::MEMORY_SIZE,
                        ));
                        if let Some(dest_owner) = owner {
                            self.register_managed_ptr(dest_owner, ptr, m);
                        }
                    } else {
                        return Err("Expected ManagedPtr".into());
                    }
                }
                Scalar::ObjectRef => {
                    if let StackValue::ObjectRef(r) = value {
                        let val_ptr = r.0.map(gc_arena::Gc::as_ptr).unwrap_or(ptr::null());
                        ptr::write_unaligned(ptr as *mut *const (), val_ptr as *const ());
                    } else {
                        return Err("Expected ObjectRef".into());
                    }
                }
            },
            LayoutManager::FieldLayoutManager(flm) => {
                if let StackValue::ValueType(src_obj_box) = value {
                    let src_obj = src_obj_box.as_ref();
                    let src_ptr = src_obj.instance_storage.get().as_ptr();
                    ptr::copy_nonoverlapping(src_ptr, ptr, flm.size());

                    if let Some(dest_owner) = owner {
                        self.update_owner_side_table(src_obj, dest_owner, ptr);
                    }
                } else {
                    return Err("Expected ValueType for Struct write".into());
                }
            }
            LayoutManager::ArrayLayoutManager(_) => {
                return Err("Cannot write entire array unaligned".into())
            }
        }
        Ok(())
    }

    unsafe fn perform_read(
        &self,
        ptr: *const u8,
        owner: Option<ManagedPtrOwner<'gc>>,
        layout: &LayoutManager,
        type_desc: Option<TypeDescription>,
    ) -> Result<StackValue<'gc>, String> {
        match layout {
            LayoutManager::Scalar(s) => match s {
                Scalar::Int8 => Ok(StackValue::Int32(
                    ptr::read_unaligned(ptr as *const i8) as i32
                )),
                Scalar::Int16 => Ok(StackValue::Int32(
                    ptr::read_unaligned(ptr as *const i16) as i32
                )),
                Scalar::Int32 => Ok(StackValue::Int32(ptr::read_unaligned(ptr as *const i32))),
                Scalar::Int64 => Ok(StackValue::Int64(ptr::read_unaligned(ptr as *const i64))),
                Scalar::NativeInt => Ok(StackValue::NativeInt(ptr::read_unaligned(
                    ptr as *const isize,
                ))),
                Scalar::Float32 => Ok(StackValue::NativeFloat(
                    ptr::read_unaligned(ptr as *const f32) as f64,
                )),
                Scalar::Float64 => Ok(StackValue::NativeFloat(ptr::read_unaligned(
                    ptr as *const f64,
                ))),
                Scalar::ObjectRef => {
                    let slice = std::slice::from_raw_parts(ptr, 8);
                    Ok(StackValue::ObjectRef(ObjectRef::read_unchecked(slice)))
                }
                Scalar::ManagedPtr => {
                    let ptr_value = ptr::read_unaligned(ptr as *const usize);
                    let ptr_val = ptr::NonNull::new(ptr_value as *mut u8);

                    let mut metadata = (None, false);
                    if let Some(owner) = owner {
                        if let Some(m) = self.check_side_table(owner, ptr) {
                            metadata = (m.recover_owner(), m.pinned);
                        }
                    }

                    let void_desc = TypeDescription::from_raw(
                        dotnet_types::resolution::ResolutionS::new(ptr::null()),
                        None,
                    );

                    Ok(StackValue::ManagedPtr(ManagedPtr::new(
                        ptr_val, void_desc, metadata.0, metadata.1,
                    )))
                }
            },
            LayoutManager::FieldLayoutManager(flm) => {
                if let Some(desc) = type_desc {
                    let size = flm.size();
                    let mut data = vec![0u8; size];
                    ptr::copy_nonoverlapping(ptr, data.as_mut_ptr(), size);

                    let storage = FieldStorage::new(Arc::new(flm.clone()), data);
                    let obj = ObjectInstance::new(desc, storage);

                    if let Some(src_owner) = owner {
                        self.copy_metadata_range(src_owner, ptr, size, &obj);
                    }

                    Ok(StackValue::ValueType(Box::new(obj)))
                } else {
                    Err("Struct read requires TypeDescription, which is not passed to read_unaligned".into())
                }
            }
            _ => Err("Array read not supported".to_string()),
        }
    }

    // Helpers

    fn copy_metadata_range(
        &self,
        src_owner: ManagedPtrOwner<'gc>,
        src_ptr: *const u8,
        size: usize,
        dest_obj: &ObjectInstance<'gc>,
    ) {
        let (src_base, _) = self.get_storage_base(src_owner);
        if src_base.is_null() {
            return;
        }

        let src_base_addr = src_base as usize;
        let src_ptr_addr = src_ptr as usize;
        let start_offset = src_ptr_addr.wrapping_sub(src_base_addr);
        let end_offset = start_offset + size;

        match src_owner {
            ManagedPtrOwner::Heap(h) => {
                let o = h.borrow();
                match &o.storage {
                    HeapStorage::Obj(oo) => {
                        let table = oo.managed_ptr_metadata.borrow();
                        for (&offset, metadata) in &table.metadata {
                            if offset >= start_offset && offset < end_offset {
                                let dest_offset = offset - start_offset;
                                dest_obj.register_metadata(dest_offset, metadata.clone(), self.gc);
                            }
                        }
                    }
                    HeapStorage::Vec(v) => {
                        let table = v.managed_ptr_metadata.borrow();
                        for (&offset, metadata) in &table.metadata {
                            if offset >= start_offset && offset < end_offset {
                                let dest_offset = offset - start_offset;
                                dest_obj.register_metadata(dest_offset, metadata.clone(), self.gc);
                            }
                        }
                    }
                    _ => {}
                }
            }
            ManagedPtrOwner::Stack(s) => {
                let o = unsafe { s.as_ref() };
                let table = o.managed_ptr_metadata.borrow();
                for (&offset, metadata) in &table.metadata {
                    if offset >= start_offset && offset < end_offset {
                        let dest_offset = offset - start_offset;
                        dest_obj.register_metadata(dest_offset, metadata.clone(), self.gc);
                    }
                }
            }
        };
    }

    fn register_managed_ptr(&self, owner: ManagedPtrOwner<'gc>, ptr: *mut u8, m: ManagedPtr<'gc>) {
        match owner {
            ManagedPtrOwner::Heap(h) => {
                let dest_obj = h.borrow();
                match &dest_obj.storage {
                    HeapStorage::Obj(dest_o) => {
                        register_managed_ptr_helper(dest_o, m, ptr, self.gc);
                    }
                    HeapStorage::Vec(dest_v) => {
                        let base = dest_v.get().as_ptr() as usize;
                        let offset = (ptr as usize).wrapping_sub(base);
                        dest_v.register_metadata(
                            offset,
                            ManagedPtrMetadata::from_managed_ptr(&m),
                            self.gc,
                        );
                    }
                    _ => {}
                }
            }
            ManagedPtrOwner::Stack(s) => {
                let dest_o = unsafe { s.as_ref() };
                register_managed_ptr_helper(dest_o, m, ptr, self.gc);
            }
        }
    }

    fn update_owner_side_table(
        &self,
        src_obj: &ObjectInstance<'gc>,
        dest_owner: ManagedPtrOwner<'gc>,
        ptr: *mut u8,
    ) {
        match dest_owner {
            ManagedPtrOwner::Heap(h) => {
                let mut dest_lock = h.borrow_mut(self.gc);
                match &mut dest_lock.storage {
                    HeapStorage::Obj(dest_o) => {
                        update_owner_side_table_helper(src_obj, dest_o, ptr, self.gc);
                    }
                    HeapStorage::Vec(dest_v) => {
                        let side_table = src_obj.managed_ptr_metadata.borrow();
                        let dest_base = dest_v.get().as_ptr() as usize;
                        let dest_offset = (ptr as usize).wrapping_sub(dest_base);

                        for (&offset, metadata) in &side_table.metadata {
                            dest_v.register_metadata(
                                dest_offset + offset,
                                metadata.clone(),
                                self.gc,
                            );
                        }
                    }
                    _ => {}
                }
            }
            ManagedPtrOwner::Stack(s) => {
                let dest_o = unsafe { s.as_ref() };
                update_owner_side_table_helper(src_obj, dest_o, ptr, self.gc);
            }
        }
    }

    fn check_side_table(
        &self,
        owner: ManagedPtrOwner<'gc>,
        ptr: *const u8,
    ) -> Option<ManagedPtrMetadata<'gc>> {
        match owner {
            ManagedPtrOwner::Heap(h) => {
                let obj = h.borrow();
                match &obj.storage {
                    HeapStorage::Obj(o) => check_side_table_helper(o, ptr),
                    HeapStorage::Vec(v) => {
                        let side_table = v.managed_ptr_metadata.borrow();
                        let base_ptr = v.get().as_ptr() as usize;
                        let offset = (ptr as usize).wrapping_sub(base_ptr);
                        let meta = side_table.get(offset).cloned();
                        if let Some(m) = &meta {
                            m.validate();
                        }
                        meta
                    }
                    _ => None,
                }
            }
            ManagedPtrOwner::Stack(s) => {
                let o = unsafe { s.as_ref() };
                check_side_table_helper(o, ptr)
            }
        }
    }
}

// Standalone helpers adapted from unsafe_ops.rs

fn register_managed_ptr_helper<'gc>(
    obj: &ObjectInstance<'gc>,
    ptr: ManagedPtr<'gc>,
    value: *const u8,
    gc: GCHandle<'gc>,
) {
    obj.register_managed_ptr(
        (value as usize).wrapping_sub(obj.instance_storage.get().as_ptr() as usize),
        &ptr,
        gc,
    );
}

fn update_owner_side_table_helper<'gc>(
    obj: &ObjectInstance<'gc>,
    dest_o: &ObjectInstance<'gc>,
    ptr: *const u8,
    gc: GCHandle<'gc>,
) {
    let side_table = obj.managed_ptr_metadata.borrow();
    let dest_base = dest_o.instance_storage.get().as_ptr() as usize;
    let dest_offset = (ptr as usize).wrapping_sub(dest_base);

    for (&offset, metadata) in &side_table.metadata {
        dest_o.register_metadata(dest_offset + offset, metadata.clone(), gc);
    }
}

fn check_side_table_helper<'gc>(
    obj: &ObjectInstance<'gc>,
    ptr: *const u8,
) -> Option<ManagedPtrMetadata<'gc>> {
    let side_table = obj.managed_ptr_metadata.borrow();
    let base_ptr = obj.instance_storage.get().as_ptr() as usize;
    let offset = (ptr as usize).wrapping_sub(base_ptr);
    let meta = side_table.get(offset).cloned();
    if let Some(m) = &meta {
        m.validate();
    }
    meta
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
            panic!("Heap Corruption: Reading ObjectRef from unmanaged memory is unsafe");
        }
    });
}

pub fn has_ref_at(layout: &LayoutManager, offset: usize) -> bool {
    match layout {
        LayoutManager::Scalar(s) => match s {
            Scalar::ObjectRef | Scalar::ManagedPtr => offset == 0,
            _ => false,
        },
        LayoutManager::FieldLayoutManager(fm) => {
            for f in fm.fields.values() {
                if offset >= f.position && offset < f.position + f.layout.size() {
                    return has_ref_at(&f.layout, offset - f.position);
                }
            }
            false
        }
        LayoutManager::ArrayLayoutManager(am) => {
            let elem_size = am.element_layout.size();
            if elem_size == 0 {
                return false;
            }
            let idx = offset / elem_size;
            if idx >= am.length {
                return false;
            }
            let rel = offset % elem_size;
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
        LayoutManager::FieldLayoutManager(fm) => {
            for f in fm.fields.values() {
                check_refs_in_layout(&f.layout, base + f.position, callback);
            }
        }
        LayoutManager::ArrayLayoutManager(am) => {
            if am.element_layout.is_or_contains_refs() {
                let sz = am.element_layout.size();
                for i in 0..am.length {
                    check_refs_in_layout(&am.element_layout, base + i * sz, callback);
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
        LayoutManager::FieldLayoutManager(fm) => {
            for f in fm.fields.values() {
                let f_start = base_offset + f.position;
                let f_end = f_start + f.layout.size();
                if f_start < range_end && f_end > range_start {
                    validate_ref_integrity(&f.layout, f_start, range_start, range_end, src_layout);
                }
            }
        }
        LayoutManager::ArrayLayoutManager(am) => {
            if am.element_layout.is_or_contains_refs() {
                let elem_size = am.element_layout.size();
                if elem_size == 0 {
                    return;
                }

                let rel_start = range_start.saturating_sub(base_offset);
                let rel_end = range_end.saturating_sub(base_offset);

                let start_idx = rel_start / elem_size;
                let end_idx = rel_end.div_ceil(elem_size);
                let end_idx = end_idx.min(am.length);

                for i in start_idx..end_idx {
                    validate_ref_integrity(
                        &am.element_layout,
                        base_offset + i * elem_size,
                        range_start,
                        range_end,
                        src_layout,
                    );
                }
            }
        }
    }
}
