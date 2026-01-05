use crate::{
    types::{members::MethodDescription, TypeDescription},
    utils::DebugStr,
    value::{
        layout::{FieldLayoutManager, HasLayout, LayoutManager, Scalar},
        object::ObjectRef,
    },
    vm::context::ResolutionContext,
};
use gc_arena::{Collect, Collection};
use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
    marker::PhantomData,
    ops::Range,
    sync::atomic::{AtomicU8, Ordering},
};

#[derive(Clone, PartialEq)]
pub struct FieldStorage<'gc> {
    layout: FieldLayoutManager,
    storage: Vec<u8>,
    _contains_gc: PhantomData<fn(&'gc ()) -> &'gc ()>,
}

impl Debug for FieldStorage<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut fs: Vec<(_, _)> = self
            .layout
            .fields
            .iter()
            .map(|(k, v)| {
                let data = &self.storage[v.as_range()];
                let data_rep = match &v.layout {
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

unsafe impl Collect for FieldStorage<'_> {
    #[inline]
    fn trace(&self, cc: &Collection) {
        self.layout.trace(&self.storage, cc);
    }
}

impl FieldStorage<'_> {
    pub fn new(layout: FieldLayoutManager) -> Self {
        Self {
            storage: vec![0; layout.size()],
            layout,
            _contains_gc: PhantomData,
        }
    }

    pub fn instance_fields(description: TypeDescription, context: &ResolutionContext) -> Self {
        Self::new(FieldLayoutManager::instance_fields(description, context))
    }

    pub fn resurrect<'gc>(
        &self,
        fc: &gc_arena::Finalization<'gc>,
        visited: &mut std::collections::HashSet<usize>,
    ) {
        self.layout.resurrect(&self.storage, fc, visited);
    }

    pub fn static_fields(description: TypeDescription, context: &ResolutionContext) -> Self {
        Self::new(FieldLayoutManager::static_fields(description, context))
    }

    pub fn get(&self) -> &[u8] {
        &self.storage
    }

    pub fn get_mut(&mut self) -> &mut [u8] {
        &mut self.storage
    }

    fn get_field_range(&self, field: &str) -> Range<usize> {
        match self.layout.fields.get(field) {
            None => panic!("field {} not found", field),
            Some(l) => l.as_range(),
        }
    }

    pub fn get_field(&self, field: &str) -> &[u8] {
        &self.storage[self.get_field_range(field)]
    }

    pub fn get_field_mut(&mut self, field: &str) -> &mut [u8] {
        let r = self.get_field_range(field);
        &mut self.storage[r]
    }
}

/// Initialization states for type static constructors (.cctor).
/// This is an atomic state machine for thread-safe type initialization.
pub const INIT_STATE_UNINITIALIZED: u8 = 0;
pub const INIT_STATE_INITIALIZING: u8 = 1;
pub const INIT_STATE_INITIALIZED: u8 = 2;

pub struct StaticStorage<'gc> {
    /// Atomic initialization state for thread-safe .cctor execution.
    /// States: 0=uninitialized, 1=initializing (in progress), 2=initialized (complete)
    init_state: AtomicU8,
    /// The ID of the thread currently initializing this type.
    /// Only valid if init_state is INITIALIZING.
    initializing_thread: u64,
    storage: FieldStorage<'gc>,
}

impl Clone for StaticStorage<'_> {
    fn clone(&self) -> Self {
        Self {
            init_state: AtomicU8::new(self.init_state.load(Ordering::Acquire)),
            initializing_thread: self.initializing_thread,
            storage: self.storage.clone(),
        }
    }
}

unsafe impl Collect for StaticStorage<'_> {
    fn trace(&self, cc: &Collection) {
        self.storage.trace(cc);
    }
}
impl Debug for StaticStorage<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.init_state.load(Ordering::Acquire) {
            INIT_STATE_INITIALIZED => Debug::fmt(&self.storage, f),
            INIT_STATE_INITIALIZING => write!(f, "initializing"),
            _ => write!(f, "uninitialized"),
        }
    }
}

#[derive(Clone)]
pub struct StaticStorageManager<'gc> {
    types: HashMap<TypeDescription, StaticStorage<'gc>>,
}

unsafe impl Collect for StaticStorageManager<'_> {
    fn trace(&self, cc: &Collection) {
        for v in self.types.values() {
            v.trace(cc);
        }
    }
}
impl Debug for StaticStorageManager<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_map()
            .entries(self.types.iter().map(|(k, v)| (DebugStr(k.type_name()), v)))
            .finish()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum StaticInitResult {
    /// This thread must execute the static constructor.
    Execute(MethodDescription),
    /// The type is already fully initialized.
    Initialized,
    /// This is a recursive call on the same thread; proceed as if initialized.
    Recursive,
    /// Another thread is currently initializing this type.
    Waiting,
}

impl<'gc> StaticStorageManager<'gc> {
    pub fn new() -> Self {
        Self {
            types: HashMap::new(),
        }
    }
}

impl<'gc> Default for StaticStorageManager<'gc> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'gc> StaticStorageManager<'gc> {
    pub fn get(&self, description: TypeDescription) -> &FieldStorage<'gc> {
        &self
            .types
            .get(&description)
            .expect("missing type in static storage")
            .storage
    }

    pub fn get_mut(&mut self, description: TypeDescription) -> &mut FieldStorage<'gc> {
        &mut self
            .types
            .get_mut(&description)
            .expect("missing type in static storage")
            .storage
    }

    /// Get the current initialization state of a type.
    pub fn get_init_state(&self, description: TypeDescription) -> u8 {
        self.types
            .get(&description)
            .map(|s| s.init_state.load(Ordering::Acquire))
            .unwrap_or(INIT_STATE_UNINITIALIZED)
    }

    /// Initialize static storage for a type and determine if a .cctor needs to run.
    /// This implementation ensures that a type's static constructor is only assigned
    /// to exactly one thread for execution, even if multiple threads race to initialize
    /// the same type simultaneously.
    ///
    /// Returns a `StaticInitResult` indicating what the calling thread should do.
    #[must_use]
    pub fn init(
        &mut self,
        description: TypeDescription,
        context: &ResolutionContext,
        thread_id: u64,
    ) -> StaticInitResult {
        // Ensure the storage exists
        self.types
            .entry(description)
            .or_insert_with(|| StaticStorage {
                init_state: AtomicU8::new(INIT_STATE_UNINITIALIZED),
                initializing_thread: 0,
                storage: FieldStorage::static_fields(description, context),
            });

        // Check for recursion on same thread
        let storage = self
            .types
            .get(&description)
            .expect("missing type in static storage");
        let state = storage.init_state.load(Ordering::Acquire);

        if state == INIT_STATE_INITIALIZED {
            return StaticInitResult::Initialized;
        }

        if state == INIT_STATE_INITIALIZING && storage.initializing_thread == thread_id {
            return StaticInitResult::Recursive;
        }

        // Check if there's a static initializer
        let cctor = description.static_initializer();

        if cctor.is_none() {
            // No .cctor, so mark as initialized if it was uninitialized
            storage
                .init_state
                .compare_exchange(
                    INIT_STATE_UNINITIALIZED,
                    INIT_STATE_INITIALIZED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .ok();
            return StaticInitResult::Initialized;
        }

        let cctor = cctor.unwrap();

        match storage.init_state.compare_exchange(
            INIT_STATE_UNINITIALIZED,
            INIT_STATE_INITIALIZING,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                // We won the race - this thread should execute the .cctor
                let storage_mut = self.types.get_mut(&description).unwrap();
                storage_mut.initializing_thread = thread_id;
                StaticInitResult::Execute(cctor)
            }
            Err(INIT_STATE_INITIALIZED) => {
                // Already initialized by another thread
                StaticInitResult::Initialized
            }
            Err(INIT_STATE_INITIALIZING) => {
                // Another thread is currently initializing
                StaticInitResult::Waiting
            }
            Err(_) => unreachable!("Invalid initialization state"),
        }
    }

    /// Mark a type as fully initialized after its .cctor completes.
    /// This must be called after the .cctor returned by `init()` has finished executing.
    pub fn mark_initialized(&mut self, description: TypeDescription) {
        if let Some(storage) = self.types.get(&description) {
            storage
                .init_state
                .store(INIT_STATE_INITIALIZED, Ordering::Release);
        }
    }
}
