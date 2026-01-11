use crate::{
    types::TypeDescription,
    utils::{is_ptr_aligned_to_field, DebugStr},
    value::{
        layout::{FieldLayoutManager, HasLayout, LayoutManager, Scalar},
        object::ObjectRef,
    },
    vm::{
        context::ResolutionContext,
        sync::{Arc, AtomicU16, AtomicU32, AtomicU64, AtomicU8, Ordering},
    },
};
use gc_arena::{Collect, Collection};
use std::{
    collections::HashSet,
    fmt::{Debug, Formatter},
    ops::Range,
};

mod statics;

pub use statics::*;

#[derive(Clone, PartialEq)]
pub struct FieldStorage {
    layout: Arc<FieldLayoutManager>,
    storage: Vec<u8>,
}

impl Debug for FieldStorage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut fs: Vec<(_, _)> = self
            .layout
            .fields
            .iter()
            .map(|(k, v)| {
                let data = &self.storage[v.as_range()];
                let data_rep = match &*v.layout {
                    LayoutManager::Scalar(Scalar::ObjectRef) => {
                        format!("{:?}", ObjectRef::read(data))
                    }
                    LayoutManager::Scalar(Scalar::ManagedPtr) => {
                        // Skip reading ManagedPtr to avoid transmute issues
                        let bytes: Vec<_> = data.iter().map(|b| format!("{:02x}", b)).collect();
                        format!("ptr({})", bytes.join(" "))
                    }
                    _ => {
                        let bytes: Vec<_> = data.iter().map(|b| format!("{:02x}", b)).collect();
                        bytes.join(" ")
                    }
                };

                (
                    v.position,
                    DebugStr(format!("{} {}: {}", v.layout.type_tag(), k, data_rep)),
                )
            })
            .collect();

        fs.sort_by_key(|(p, _)| *p);

        f.debug_list()
            .entries(fs.into_iter().map(|(_, r)| r))
            .finish()
    }
}

unsafe impl Collect for FieldStorage {
    #[inline]
    fn trace(&self, cc: &Collection) {
        self.layout.trace(&self.storage, cc);
    }
}

impl FieldStorage {
    pub fn new(layout: Arc<FieldLayoutManager>) -> Self {
        let size = layout.size();
        // Defensive check: ensure the total allocation size is reasonable.
        if size > 0x1000_0000 {
            // 256MB limit for a single object instance
            panic!("massive field storage allocation: {} bytes", size);
        }
        Self {
            storage: vec![0; size],
            layout,
        }
    }

    pub fn instance_fields(description: TypeDescription, context: &ResolutionContext) -> Self {
        Self::new(FieldLayoutManager::instance_field_layout_cached(
            description,
            context,
            None,
        ))
    }

    pub fn resurrect<'gc>(&self, fc: &gc_arena::Finalization<'gc>, visited: &mut HashSet<usize>) {
        self.layout.resurrect(&self.storage, fc, visited);
    }

    pub fn static_fields(description: TypeDescription, context: &ResolutionContext) -> Self {
        Self::new(Arc::new(FieldLayoutManager::static_fields(
            description,
            context,
        )))
    }

    pub fn get(&self) -> &[u8] {
        &self.storage
    }

    pub fn get_mut(&mut self) -> &mut [u8] {
        &mut self.storage
    }

    pub fn layout(&self) -> &Arc<FieldLayoutManager> {
        &self.layout
    }

    fn get_field_range(&self, owner: TypeDescription, field: &str) -> Range<usize> {
        match self.layout.get_field(owner, field) {
            None => panic!("field {}::{} not found", owner.type_name(), field),
            Some(l) => l.as_range(),
        }
    }

    pub fn get_field_local(&self, owner: TypeDescription, field: &str) -> &[u8] {
        &self.storage[self.get_field_range(owner, field)]
    }

    pub fn get_field_mut_local(&mut self, owner: TypeDescription, field: &str) -> &mut [u8] {
        let r = self.get_field_range(owner, field);
        &mut self.storage[r]
    }

    pub fn has_field(&self, owner: TypeDescription, field: &str) -> bool {
        self.layout.get_field(owner, field).is_some()
    }

    pub fn get_field_atomic(
        &self,
        owner: TypeDescription,
        field: &str,
        ordering: Ordering,
    ) -> Vec<u8> {
        let layout = self
            .layout
            .get_field(owner, field)
            .unwrap_or_else(|| panic!("field {}::{} not found", owner.type_name(), field));
        let size = layout.layout.size();
        let offset = layout.position;
        let ptr = unsafe { self.storage.as_ptr().add(offset) };

        // Check alignment before attempting atomic operations
        // Unaligned atomic operations are UB in Rust
        if !is_ptr_aligned_to_field(ptr, size) {
            // Fall back to non-atomic read if not aligned
            // This can happen with ExplicitLayout types
            return self.get_field_local(owner, field).to_vec();
        }

        match size {
            1 => vec![unsafe { (*(ptr as *const AtomicU8)).load(ordering) }],
            2 => unsafe { (*(ptr as *const AtomicU16)).load(ordering) }
                .to_ne_bytes()
                .to_vec(),
            4 => unsafe { (*(ptr as *const AtomicU32)).load(ordering) }
                .to_ne_bytes()
                .to_vec(),
            8 => unsafe { (*(ptr as *const AtomicU64)).load(ordering) }
                .to_ne_bytes()
                .to_vec(),
            _ => self.get_field_local(owner, field).to_vec(),
        }
    }

    pub fn set_field_atomic(
        &self,
        owner: TypeDescription,
        field: &str,
        value: &[u8],
        ordering: Ordering,
    ) {
        let layout = self
            .layout
            .get_field(owner, field)
            .unwrap_or_else(|| panic!("field {}::{} not found", owner.type_name(), field));
        let size = layout.layout.size();
        let offset = layout.position;
        let ptr = unsafe { self.storage.as_ptr().add(offset) as *mut u8 };

        if size != value.len() {
            panic!(
                "size mismatch for field {}::{} (expected {}, got {})",
                owner.type_name(),
                field,
                size,
                value.len()
            );
        }

        // Check alignment before attempting atomic operations
        // Unaligned atomic operations are UB in Rust
        if !is_ptr_aligned_to_field(ptr, size) {
            // Fall back to non-atomic write if not aligned
            // This can happen with ExplicitLayout types
            // NOTE: This violates ECMA-335 atomicity requirements, but it's
            // better than UB. A proper implementation would need to lock.
            unsafe {
                std::ptr::copy_nonoverlapping(value.as_ptr(), ptr, size.min(value.len()));
            }
            return;
        }

        match size {
            1 => unsafe { (*(ptr as *const AtomicU8)).store(value[0], ordering) },
            2 => unsafe {
                let val = u16::from_ne_bytes(value.try_into().unwrap());
                (*(ptr as *const AtomicU16)).store(val, ordering)
            },
            4 => unsafe {
                let val = u32::from_ne_bytes(value.try_into().unwrap());
                (*(ptr as *const AtomicU32)).store(val, ordering)
            },
            8 => unsafe {
                let val = u64::from_ne_bytes(value.try_into().unwrap());
                (*(ptr as *const AtomicU64)).store(val, ordering)
            },
            _ => unsafe {
                std::ptr::copy_nonoverlapping(value.as_ptr(), ptr, size);
            },
        }
    }
}
