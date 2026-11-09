#[cfg(any(feature = "memory-validation", debug_assertions))]
use crate::ValidationTag;

use crate::{ArenaId, ptr_common::PointerLike};
use dotnet_types::error::ExecutionError;
use dotnet_utils::{
    gc::{GCHandle, ThreadSafeLock},
    sync::get_current_thread_id,
};
use gc_arena::{Collect, Gc, Mutation, collect::Trace};
use std::{
    cmp::Ordering,
    collections::HashSet,
    fmt::{self, Debug, Formatter},
    hash::{Hash, Hasher},
    mem::size_of,
    num::NonZeroUsize,
    ptr::NonNull,
};

#[cfg(feature = "fuzzing")]
use arbitrary::Arbitrary;

#[cfg(feature = "multithreading")]
use dotnet_utils::gc::{get_currently_tracing, record_cross_arena_object, try_acquire_lease};

#[cfg(all(feature = "multithreading", feature = "memory-validation"))]
use dotnet_utils::gc::{is_stw_in_progress, is_valid_cross_arena_ref};

pub mod heap_storage;
pub mod types;

pub use heap_storage::*;
pub use types::*;

#[cfg(feature = "fuzzing")]
static OBJECT_REGISTRY: std::sync::LazyLock<dashmap::DashSet<usize>> =
    std::sync::LazyLock::new(dashmap::DashSet::new);

#[cfg(feature = "fuzzing")]
pub fn register_object_ptr(ptr: usize) {
    OBJECT_REGISTRY.insert(ptr);
}

#[cfg(feature = "fuzzing")]
pub fn is_valid_object_ptr(ptr: usize) -> bool {
    OBJECT_REGISTRY.contains(&ptr)
}

// The compact Tag-5 codec reserves the top 16 address bits and requires a 64-bit pointer.
#[cfg(all(feature = "multithreading", not(target_pointer_width = "64")))]
compile_error!("the multithreaded ObjectRef compact codec requires 64-bit pointers");

#[derive(Debug, PartialEq, Eq)]
enum ObjectCodecError {
    ShortStorage {
        actual: usize,
    },
    #[cfg(feature = "multithreading")]
    InvalidAddress(usize),
    #[cfg(feature = "multithreading")]
    UnrepresentableOwner(ArenaId),
    #[cfg(feature = "multithreading")]
    OwnerUnavailable(ArenaId),
}

#[cfg(feature = "multithreading")]
fn check_compact_address(addr: usize) -> Result<(), ObjectCodecError> {
    if addr == 0 || addr & 7 != 0 || addr >= (1usize << 48) {
        return Err(ObjectCodecError::InvalidAddress(addr));
    }
    Ok(())
}

fn check_object_storage(len: usize) -> Result<(), ObjectCodecError> {
    if len < ObjectRef::SIZE {
        return Err(ObjectCodecError::ShortStorage { actual: len });
    }
    Ok(())
}

fn codec_invariant(error: ObjectCodecError) -> ! {
    panic!("ObjectRef codec host invariant violation: {error:?}")
}

pub const OBJECT_MAGIC: u64 = 0x5AFE_0B1E_C700_0000;
pub type SyncBlockIndex = NonZeroUsize;

#[derive(Collect, Debug)]
#[collect(no_drop)]
pub struct ObjectInner<'gc> {
    #[cfg(any(feature = "memory-validation", debug_assertions))]
    magic: ValidationTag,
    pub storage: HeapStorage<'gc>,
    /// Encoded sync-block index for System.Threading.Monitor.
    /// `0` is the sentinel for "no sync block yet", all non-zero values are valid
    /// `SyncBlockIndex` values.
    sync_block_index: usize,
    /// Arena owner used by:
    /// - write-barrier cross-arena reference recording
    /// - tagged cross-arena serialization (`Tag 5`)
    /// - `memory-validation` arena-consistency checks
    ///
    /// Invariant: immutable after construction and safe for lock-free reads.
    owner_id: ArenaId,
}

impl<'gc> ObjectInner<'gc> {
    #[inline]
    pub fn new(storage: HeapStorage<'gc>, owner_id: ArenaId) -> Self {
        Self {
            #[cfg(any(feature = "memory-validation", debug_assertions))]
            magic: ValidationTag::new(OBJECT_MAGIC),
            storage,
            sync_block_index: 0,
            owner_id,
        }
    }

    #[inline(always)]
    fn sync_block_index(&self) -> Option<SyncBlockIndex> {
        SyncBlockIndex::new(self.sync_block_index)
    }

    #[inline(always)]
    fn set_sync_block_index(&mut self, new_index: SyncBlockIndex) {
        self.sync_block_index = new_index.get();
    }

    #[inline(always)]
    pub fn owner_id(&self) -> ArenaId {
        self.owner_id
    }

    pub fn validate_arena_id(&self) {
        #[cfg(feature = "memory-validation")]
        {
            let owner_id = self.owner_id();
            let current_id = get_current_thread_id();
            if owner_id != current_id && owner_id != ArenaId::INVALID {
                // In multithreading mode, this might be a valid cross-arena reference.
                // But it's unsafe to borrow it directly without coordination.
                #[cfg(not(feature = "multithreading"))]
                panic!(
                    "Arena mismatch: object owned by {:?}, but accessed by {:?}",
                    owner_id, current_id
                );

                #[cfg(feature = "multithreading")]
                {
                    // A lease held by this thread keeps the arena's memory live
                    // even after teardown removed its registry entry, so an
                    // in-flight access is valid; fresh accesses cannot obtain
                    // a lease and still fail here.
                    if !is_valid_cross_arena_ref(owner_id)
                        && !dotnet_utils::gc::current_thread_holds_lease(owner_id)
                    {
                        panic!(
                            "Dangling cross-arena reference: arena {:?} is no longer valid (thread exited?)",
                            owner_id
                        );
                    }

                    if is_stw_in_progress(owner_id) && get_currently_tracing().is_none() {
                        panic!(
                            "Uncoordinated cross-arena access during STW GC: object owned by {:?}, accessed by {:?}",
                            owner_id, current_id
                        );
                    }
                }
            }
        }
    }

    pub fn validate_magic(&self) {
        #[cfg(any(feature = "memory-validation", debug_assertions))]
        self.magic.validate(OBJECT_MAGIC, "ObjectInner");
    }

    pub fn validate_resurrection_invariants(&self) {
        self.validate_magic();
        self.storage.validate_resurrection_invariants();
    }

    #[cfg(test)]
    pub(crate) fn with_magic_for_tests(
        storage: HeapStorage<'gc>,
        owner_id: ArenaId,
        magic: u64,
    ) -> Self {
        #[cfg(any(feature = "memory-validation", debug_assertions))]
        {
            let mut inner = Self::new(storage, owner_id);
            inner.magic = ValidationTag::new(magic);
            inner
        }
        #[cfg(not(any(feature = "memory-validation", debug_assertions)))]
        {
            let _ = magic;
            Self::new(storage, owner_id)
        }
    }
}

#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectPtr(NonNull<ThreadSafeLock<ObjectInner<'static>>>);

#[cfg(feature = "fuzzing")]
impl<'a> Arbitrary<'a> for ObjectPtr {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let _address_seed: usize = u.arbitrary()?;
        // No arena is available to an unscoped Arbitrary implementation, so it
        // cannot manufacture a valid ObjectPtr from fuzz bytes. A typed dangling
        // sentinel preserves variant-shape coverage without integer-to-pointer
        // reconstruction; targets that dereference must build a live arena fixture.
        Ok(ObjectPtr(NonNull::dangling()))
    }
}

// SAFETY: F11.ObjectPointerThreadSafe — `ObjectPtr` is a raw pointer to a `ThreadSafeLock`. Under
// multithreading, `ThreadSafeLock<T: Send + Sync>` is `Send + Sync` (verified
// by `assert_impl_all!` below). `ObjectInner<'static>: Send + Sync` is
// confirmed by the test assertions in this module.
#[cfg(feature = "multithreading")]
unsafe impl Send for ObjectPtr {}

// SAFETY: F11.ObjectPointerThreadSafe — Same rationale as the `Send` implementation above. These impls are
// gated on `multithreading` because the single-threaded backend uses
// `gc_arena::lock::RefLock`, which is `!Send + !Sync`.
#[cfg(feature = "multithreading")]
unsafe impl Sync for ObjectPtr {}

impl ObjectPtr {
    pub(crate) fn from_handle<'gc>(handle: ObjectHandle<'gc>) -> Self {
        ObjectPtr(NonNull::new(Gc::as_ptr(handle) as *mut _).expect("Gc::as_ptr is never null"))
    }

    /// # Safety
    ///
    /// A non-null pointer must identify a properly aligned, live object allocation
    /// with usable provenance for every protected use. The `'static` parameter
    /// erases the arena brand; it does not extend the allocation's lifetime.
    /// Callers must establish the correct arena, rooting and access discipline
    /// for the use/trace interval, including before any pointee read.
    pub unsafe fn from_raw(ptr: *const ThreadSafeLock<ObjectInner<'static>>) -> Option<Self> {
        NonNull::new(ptr as *mut _).map(ObjectPtr)
    }

    pub fn as_ptr(&self) -> *const ThreadSafeLock<ObjectInner<'static>> {
        self.0.as_ptr()
    }

    /// Erase this pointer for cross-arena transport without exposing its provenance.
    ///
    /// The result is inert: it grants no access, liveness or rooting.
    #[cfg(feature = "multithreading")]
    pub fn to_opaque(self) -> dotnet_utils::gc::OpaqueObjectPtr {
        // SAFETY: F11.ObjectPointerThreadSafe — `self` is a non-null pointer to the
        // runtime's `ThreadSafeLock<ObjectInner>`, the only pointee `OpaqueObjectPtr` admits.
        unsafe { dotnet_utils::gc::OpaqueObjectPtr::from_raw(self.0.as_ptr().cast()) }
    }

    /// Recover an object pointer from inert transport, preserving the same pointer value.
    ///
    /// # Safety
    ///
    /// `ptr` must have been erased from an `ObjectPtr`/`Gc::as_ptr` object allocation.
    /// This performs no access; the caller must separately validate the owner's exact
    /// registration identity, rooting and synchronization before any pointee use.
    #[cfg(feature = "multithreading")]
    pub unsafe fn from_opaque(ptr: dotnet_utils::gc::OpaqueObjectPtr) -> Self {
        ObjectPtr(
            NonNull::new(ptr.as_ptr().cast_mut().cast())
                .expect("OpaqueObjectPtr is constructed from a non-null pointer"),
        )
    }

    #[expect(
        clippy::multiple_unsafe_ops_per_block,
        reason = "the NonNull dereference and lock's interior-pointer access form one immutable owner-id read"
    )]
    pub fn owner_id(&self) -> ArenaId {
        // SAFETY: F10.BorrowedStorageStable — `self.0` is a `NonNull` pointer to a live `ThreadSafeLock`.
        // `as_ref()` yields a shared reference; `as_ptr()` gives a raw pointer
        // to the interior `ObjectInner` without acquiring the lock.  Reading
        // `owner_id` is safe because it is written once at construction and is
        // thereafter immutable.
        unsafe { (*self.0.as_ref().as_ptr()).owner_id() }
    }

    pub fn with_data<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        // SAFETY: F10.BorrowedStorageStable — `self.0` is a valid, non-null `ThreadSafeLock` pointer kept
        // alive by the GC arena for the duration of this call.  `as_ref()` is
        // sound because the pointer is non-null and aligned.  `borrow()` acquires
        // a shared read lock, preventing concurrent mutable access.
        let inner = unsafe { self.0.as_ref().borrow() };
        inner.storage.with_data(f)
    }

    pub fn with_data_mut<T>(&self, _gc: GCHandle<'_>, f: impl FnOnce(&mut [u8]) -> T) -> T {
        // SAFETY: F1.GcHandleRooted — `self.0` is non-null and aligned, and the caller keeps the
        // pointee live and rooted for this access (see `ObjectPtr::from_raw`).
        let lock = unsafe { self.0.as_ref() };
        // SAFETY: F1.GcHandleRooted — the object may belong to another arena, so no `Gc` for
        // this lock exists under our `Mutation` brand and the owner barrier of
        // `ThreadSafeLock::borrow_mut` cannot be applied; the unbarriered borrow is used
        // instead. This relies on the byte closure adopting no new traced `Gc` children
        // without a separately recorded barrier: callers in `dotnet-runtime-memory` record
        // cross-arena stores through `WriteBarrierRecorder` before writing. That caller-side
        // premise is not enforced by this safe signature (docs/plans/05-safe-api-premises.md, item 3).
        let mut inner = unsafe { lock.borrow_mut_unbarriered(_gc.mutation()) };
        inner.storage.with_data_mut(f)
    }

    pub fn as_heap_storage<T>(&self, f: impl for<'a> FnOnce(&HeapStorage<'a>) -> T) -> T {
        // SAFETY: F10.BorrowedStorageStable — `self.0` is a valid, non-null `ThreadSafeLock` pointer.
        // `as_ref()` is sound because the pointer is non-null and aligned.
        // `borrow()` acquires a shared read lock, preventing concurrent writes.
        // The HRTB hides the raw pointer's internal `'static` arena brand: `f`
        // must accept every brand, so `T` cannot carry a storage-derived GC
        // handle whose type depends on that brand.
        let inner = unsafe { self.0.as_ref().borrow() };
        inner.validate_magic();
        inner.validate_arena_id();
        f(&inner.storage)
    }
}

pub type ObjectHandle<'gc> = Gc<'gc, ThreadSafeLock<ObjectInner<'gc>>>;

#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct ObjectRef<'gc>(pub Option<ObjectHandle<'gc>>);

#[cfg(feature = "fuzzing")]
impl<'a, 'gc> Arbitrary<'a> for ObjectRef<'gc> {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let _handle_seed: usize = u.arbitrary()?;
        // A live Gc handle cannot be created without an arena mutation token.
        // Use the valid null representation; GC-backed fuzz targets construct
        // live ObjectRefs in their own arena fixtures.
        Ok(ObjectRef(None))
    }
}

#[cfg_attr(
    feature = "multithreading",
    expect(
        clippy::multiple_unsafe_ops_per_block,
        reason = "the tracing-only owner-id read traverses the GC pointer and lock interior as one immutable operation"
    )
)]
// SAFETY: F5.TracesEveryGcRef — A local handle is traced through its `Gc` implementation. During coordinated
// multithreaded collection, a non-local handle is instead recorded as a cross-arena root so its
// owning arena traces it; no reference-bearing variant is skipped. The raw owner-ID read is the
// accepted STW boundary documented under
// `docs/GC_AND_MEMORY_SAFETY.md#accepted-risk-boundaries`.
unsafe impl<'gc> Collect<'gc> for ObjectRef<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        if let Some(h) = self.0 {
            #[cfg(feature = "multithreading")]
            {
                // Check for cross-arena reference
                if let Some(tracing_id) = get_currently_tracing() {
                    let lock_ptr: *const ThreadSafeLock<ObjectInner> = Gc::as_ptr(h);
                    // DANGER: `lock_ptr` may dangle if its owning arena exits. Coordinated STW
                    // must keep every arena alive and every mutator stopped through this read; see the
                    // accepted-risk boundary linked from the `Collect` SAFETY comment above.
                    // SAFETY: F1.StwParked — During STW all mutator threads are at safe points,
                    // so the owning arena cannot be freed. F1.RetiredCollectionExclusion
                    // extends that to retired owners: none frees its arena while any worker is
                    // running or finalizing or a finalizer is pending (abort abandons pending
                    // finalizers), so no collection can trace into a freed arena during shutdown.  `owner_id` is
                    // immutable after construction, so reading it without the
                    // lock is safe even under concurrent observation.
                    let owner_id = unsafe { (*(*lock_ptr).as_ptr()).owner_id() };
                    if owner_id != tracing_id {
                        // SAFETY: F1.StwParked; F11.ObjectPointerThreadSafe — `h` is a live handle
                        // to an object owned by `owner_id`'s registration, which STW keeps alive;
                        // the pointer is erased inert with no exposed provenance.
                        let ptr = unsafe {
                            dotnet_utils::gc::OpaqueObjectPtr::from_raw(Gc::as_ptr(h).cast())
                        };
                        // SAFETY: F1.ArenaGenerationMatch — as above; the identity is captured under a lease just now.
                        unsafe { record_cross_arena_object(owner_id, ptr) };
                        return;
                    }
                }
            }
            h.trace(cc);
        }
    }
}

//noinspection RsAssertEqual
// we assume this type is pointer-sized basically everywhere (for serialization/layout purposes)
// dependency-wise everything guarantees it; this is just a sanity check for the implementation
const _: () = assert!(size_of::<ObjectRef<'static>>() == size_of::<usize>());
const _: () = assert!(size_of::<Option<SyncBlockIndex>>() == size_of::<usize>());

impl PartialEq for ObjectRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self.0, other.0) {
            (Some(l), Some(r)) => Gc::ptr_eq(l, r),
            (None, None) => true,
            _ => false,
        }
    }
}

impl PartialOrd for ObjectRef<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self.0, other.0) {
            (Some(l), Some(r)) => Gc::as_ptr(l).partial_cmp(&Gc::as_ptr(r)),
            (None, None) => Some(Ordering::Equal),
            (None, _) => Some(Ordering::Less),
            (_, None) => Some(Ordering::Greater),
        }
    }
}

impl Eq for ObjectRef<'_> {}

impl Hash for ObjectRef<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self.0 {
            Some(g) => Gc::as_ptr(g).hash(state),
            None => 0usize.hash(state),
        }
    }
}

impl<'gc> ObjectRef<'gc> {
    /// The serialized size when written to memory (always a single pointer/usize).
    /// This is the size used for layout calculations and array element size.
    pub const SIZE: usize = size_of::<usize>();

    pub fn pointer(&self) -> Option<NonNull<u8>> {
        self.0.map(|h| {
            let inner = h.borrow();
            // SAFETY: F1.GcHandleRooted — The active shared object borrow keeps the matched storage allocation live
            // and excludes mutation while its base address is captured.
            NonNull::new(unsafe { inner.storage.raw_data_ptr() })
                .expect("Object storage pointer is null")
        })
    }

    pub fn as_ptr(&self) -> Option<ObjectPtr> {
        self.0.map(|h| ObjectPtr::from_handle(h))
    }

    #[cfg(feature = "multithreading")]
    pub fn as_ptr_info(&self) -> Option<(ObjectPtr, ArenaId)> {
        self.0.map(|h| {
            let ptr = ObjectPtr::from_handle(h);
            let tid = ptr.owner_id();
            (ptr, tid)
        })
    }

    pub fn resurrect(
        &self,
        fc: &'gc gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
        depth: usize,
    ) {
        if depth > 1000 {
            panic!(
                "Resurrection depth exceeded (possible infinite recursion in custom finalizers or corrupt graph)"
            );
        }
        if let Some(handle) = self.0 {
            let ptr = Gc::as_ptr(handle).addr();
            if visited.insert(ptr) {
                let inner = handle.borrow();
                inner.validate_resurrection_invariants();
                drop(inner);

                Gc::resurrect(fc, handle);
                handle.borrow().storage.resurrect(fc, visited, depth + 1);
            }
        }
    }

    pub fn new(gc: GCHandle<'gc>, value: HeapStorage<'gc>) -> Self {
        let owner_id = get_current_thread_id();
        #[cfg(feature = "multithreading")]
        {
            let size = size_of::<ObjectInner>() + value.size_bytes();
            #[cfg(feature = "bench-instrumentation")]
            dotnet_metrics::record_active_allocation_pressure(
                match &value {
                    HeapStorage::Obj(_) | HeapStorage::Boxed(_) => {
                        dotnet_metrics::AllocationPressureSource::Object
                    }
                    HeapStorage::Vec(_) => dotnet_metrics::AllocationPressureSource::Vector,
                    HeapStorage::Str(_) => dotnet_metrics::AllocationPressureSource::String,
                },
                size,
            );
            gc.record_allocation(size);
        }

        let h = Gc::new(&gc, ThreadSafeLock::new(ObjectInner::new(value, owner_id)));

        #[cfg(feature = "fuzzing")]
        register_object_ptr(Gc::as_ptr(h).expose_provenance());

        Self(Some(h))
    }

    /// Reads an ObjectRef from pointer-typed byte storage without lifetime branding.
    ///
    /// # Safety
    /// - `source` must contain at least [`ObjectRef::SIZE`] bytes containing an initialized
    ///   `*mut u8` storage pointer previously produced by [`ObjectRef::to_storage_ptr`], a null
    ///   pointer, or a pointer with an odd non-object tag. The bytes may be unaligned.
    /// - The stored pointer must satisfy the live-allocation, provenance and lifetime
    ///   requirements of [`ObjectRef::from_storage_ptr`]. An arena lease alone does not root it.
    pub unsafe fn read_unchecked(source: &[u8]) -> Self {
        // SAFETY: F10.RawMemoryAccessValid — this function's `# Safety` contract is `try_read_unchecked`'s, which additionally reports a short `source` as an error.
        unsafe { Self::try_read_unchecked(source) }.unwrap_or_else(|error| codec_invariant(error))
    }

    /// # Safety
    ///
    /// Same as [`Self::read_unchecked`], except that a `source` shorter than
    /// [`ObjectRef::SIZE`] is reported as an error instead of being read.
    unsafe fn try_read_unchecked(source: &[u8]) -> Result<Self, ObjectCodecError> {
        check_object_storage(source.len())?;
        // SAFETY: F10.RawMemoryAccessValid — `check_object_storage` proved `source` holds at least a pointer's bytes, which this function's `# Safety` contract says are an initialized pointer; `read_unaligned` needs no alignment.
        let ptr = unsafe { source.as_ptr().cast::<*mut u8>().read_unaligned() };
        // SAFETY: F1.GcHandleRooted — this function's `# Safety` contract says the stored pointer meets `from_storage_ptr`'s requirements, which are `try_from_storage_ptr`'s.
        unsafe { Self::try_from_storage_ptr(ptr) }
    }

    /// Returns the pointer representation used to store this reference.
    ///
    /// In multithreaded builds, live managed references carry Tag 5 and their owner arena ID in
    /// the pointer's address bits. The pointer's provenance is retained while those bits are
    /// changed, so callers can store the result in pointer-typed atomics or byte storage.
    /// This compact format requires 64-bit pointers, registered owner IDs 0..=65535 and
    /// nonzero 8-byte-aligned addresses below 2^48. INVALID remains untagged. An unavailable
    /// owner or unrepresentable managed object triggers a host invariant panic, not a guest null.
    pub fn to_storage_ptr(&self) -> *mut u8 {
        self.try_to_storage_ptr()
            .unwrap_or_else(|error| codec_invariant(error))
    }

    #[allow(
        clippy::multiple_unsafe_ops_per_block,
        reason = "the immutable owner ID read traverses the live lock and its interior"
    )]
    fn try_to_storage_ptr(&self) -> Result<*mut u8, ObjectCodecError> {
        let Some(handle) = self.0 else {
            return Ok(std::ptr::null_mut());
        };
        let ptr = Gc::as_ptr(handle).cast_mut().cast::<u8>();

        #[cfg(feature = "multithreading")]
        {
            // SAFETY: F1.GcHandleRooted; F11.ObjectPointerThreadSafe — The caller holds a live
            // handle, and owner_id is immutable after initialization, so this read cannot race.
            let owner_id = unsafe { (*(*Gc::as_ptr(handle)).as_ptr()).owner_id() };
            if owner_id != ArenaId::INVALID {
                let owner = u16::try_from(owner_id.as_u64())
                    .map_err(|_| ObjectCodecError::UnrepresentableOwner(owner_id))?;
                check_compact_address(ptr.addr())?;
                let _lease = try_acquire_lease(owner_id)
                    .ok_or(ObjectCodecError::OwnerUnavailable(owner_id))?;
                return Ok(ptr.map_addr(|a| a | 5 | (usize::from(owner) << 48)));
            }
        }
        Ok(ptr)
    }

    /// Reconstructs an ObjectRef from its pointer representation in storage.
    ///
    /// Null decodes to `ObjectRef(None)`. Odd tags other than Tag 5 represent stack, static, or
    /// other non-object values and are returned as `None` without dereferencing the pointer.
    ///
    /// # Safety
    /// - For an untagged, non-null pointer, it must retain provenance from a live
    ///   `ThreadSafeLock<ObjectInner>` allocation containing a valid `Gc` handle, and that handle
    ///   must remain live for the returned lifetime `'gc`.
    /// - For Tag 5, if its owner arena can be leased, the pointer must retain provenance from the
    ///   original live object allocation, its owner ID must describe that allocation, and the
    ///   owner arena must remain valid for the returned reference's lifetime. The lease prevents
    ///   arena teardown during decoding but does not root the object. An unavailable owner is a
    ///   host invariant violation, reported before any pointee read.
    /// - The pointer's tag bits must use the storage format: zero for local references, Tag 5 for
    ///   cross-arena references, or an odd non-object tag. Other odd tags are ignored without
    ///   dereferencing the pointer.
    pub unsafe fn from_storage_ptr(ptr: *mut u8) -> Self {
        // SAFETY: F1.GcHandleRooted — this function's `# Safety` contract is `try_from_storage_ptr`'s.
        unsafe { Self::try_from_storage_ptr(ptr) }.unwrap_or_else(|error| codec_invariant(error))
    }

    /// # Safety
    ///
    /// Same as [`Self::from_storage_ptr`]; a malformed compact address or unavailable owner
    /// is reported as an error before any pointee read.
    #[cfg_attr(
        any(
            feature = "multithreading",
            feature = "memory-validation",
            debug_assertions
        ),
        expect(
            clippy::multiple_unsafe_ops_per_block,
            reason = "the validation reads the live lock and its interior as one operation"
        )
    )]
    unsafe fn try_from_storage_ptr(ptr: *mut u8) -> Result<Self, ObjectCodecError> {
        let addr = ptr.addr();

        #[cfg(feature = "multithreading")]
        if addr & 7 == 5 {
            let owner_id_u16 = (addr >> 48) as u16;
            let owner_id = ArenaId::new(u64::from(owner_id_u16));
            // `map_addr` clears metadata without dropping the provenance carried by `ptr`.
            let real_ptr = ptr.map_addr(|a| a & 0x0000FFFFFFFFFFF8);
            check_compact_address(real_ptr.addr())?;
            let _lease =
                try_acquire_lease(owner_id).ok_or(ObjectCodecError::OwnerUnavailable(owner_id))?;
            // SAFETY: F1.ArenaGenerationMatch — `_lease` pins `owner_id`'s current registration,
            // which is what the tag-5 decode contract treats as the pointer's owner; `real_ptr`
            // keeps its original provenance and is erased without exposure.
            unsafe {
                record_cross_arena_object(
                    owner_id,
                    dotnet_utils::gc::OpaqueObjectPtr::from_raw(real_ptr.cast::<()>()),
                )
            };
            let lock_ptr = real_ptr.cast::<ThreadSafeLock<ObjectInner<'gc>>>();

            #[cfg(any(feature = "memory-validation", debug_assertions))]
            {
                use std::mem::align_of;
                if !real_ptr
                    .addr()
                    .is_multiple_of(align_of::<ThreadSafeLock<ObjectInner<'static>>>())
                {
                    panic!(
                        "ObjectRef::read: Pointer {:#x} is not aligned",
                        real_ptr.addr()
                    );
                }
                // SAFETY: F1.GcHandleRooted — for Tag 5 this function's `# Safety` contract makes `real_ptr` the original live object allocation, `_lease` keeps its arena from tearing down, and alignment was checked above.
                unsafe { (*(*lock_ptr).as_ptr()).validate_magic() };
            }
            // SAFETY: F1.GcHandleRooted — for Tag 5 this function's `# Safety` contract makes `lock_ptr` the original live `Gc` allocation and keeps its owner arena valid for `'gc`.
            return Ok(ObjectRef(Some(unsafe { Gc::from_ptr(lock_ptr) })));
        }

        if addr & 1 != 0 || ptr.is_null() {
            return Ok(ObjectRef(None));
        }

        let lock_ptr = ptr.cast::<ThreadSafeLock<ObjectInner<'gc>>>();
        #[cfg(any(feature = "memory-validation", debug_assertions))]
        {
            use std::mem::align_of;
            if !addr.is_multiple_of(align_of::<ThreadSafeLock<ObjectInner<'static>>>()) {
                panic!("ObjectRef::read: Pointer {:#x} is not aligned", addr);
            }
            // SAFETY: F1.GcHandleRooted — `ptr` is untagged and non-null, so this function's `# Safety` contract makes it a live `ThreadSafeLock<ObjectInner>` allocation; alignment was checked above.
            unsafe { (*(*lock_ptr).as_ptr()).validate_magic() };
        }
        // SAFETY: F1.GcHandleRooted — `ptr` is untagged and non-null, so this function's `# Safety` contract makes it a live `Gc` allocation that stays live for `'gc`.
        Ok(ObjectRef(Some(unsafe { Gc::from_ptr(lock_ptr) })))
    }

    /// Reads an ObjectRef from a byte slice and brands it with the GC lifetime.
    ///
    /// # Safety
    /// - `source` must contain a valid `Gc` pointer.
    /// - The pointer must belong to the arena associated with `gc`.
    pub unsafe fn read_branded(source: &[u8], _gc: &Mutation<'gc>) -> Self {
        // SAFETY: F1.GcHandleRooted — this function's `# Safety` contract (a live `Gc` pointer of `_gc`'s arena) meets `read_unchecked`'s, and `_gc` brands the result with that arena's lifetime.
        unsafe { Self::read_unchecked(source) }
    }

    pub fn write(&self, dest: &mut [u8]) {
        check_object_storage(dest.len()).unwrap_or_else(|error| codec_invariant(error));
        let ptr = self.to_storage_ptr();
        // SAFETY: F10.RawMemoryAccessValid — `check_object_storage` proved `dest` holds at least a pointer's bytes, `&mut` makes them exclusive, and `write_unaligned` needs no alignment.
        unsafe { dest.as_mut_ptr().cast::<*mut u8>().write_unaligned(ptr) };
    }

    pub fn try_as_object<T>(
        &self,
        op: impl FnOnce(&Object<'gc>) -> T,
    ) -> Result<T, ExecutionError> {
        let ObjectRef(Some(o)) = &self else {
            return Err(ExecutionError::NullReference);
        };
        let inner = o.borrow();
        inner.validate_magic();
        let HeapStorage::Obj(instance) = &inner.storage else {
            let variant = match &inner.storage {
                HeapStorage::Vec(_) => "Vec",
                HeapStorage::Obj(_) => "Obj",
                HeapStorage::Str(_) => "Str",
                HeapStorage::Boxed(_) => "Boxed",
            };
            // invariant: try_as_object callers must only use object references whose heap storage
            // kind has already been established; a different variant indicates VM heap corruption.
            panic!(
                "called ObjectRef::try_as_object on non-object heap reference: variant={}",
                variant
            )
        };

        Ok(op(instance))
    }

    pub fn try_as_object_mut<T>(
        &self,
        gc: GCHandle<'gc>,
        op: impl FnOnce(&mut Object<'gc>) -> T,
    ) -> Result<T, ExecutionError> {
        let ObjectRef(Some(o)) = &self else {
            return Err(ExecutionError::NullReference);
        };
        let mut inner = ThreadSafeLock::borrow_mut(*o, &gc);
        inner.validate_magic();
        let HeapStorage::Obj(instance) = &mut inner.storage else {
            let variant = match &inner.storage {
                HeapStorage::Vec(_) => "Vec",
                HeapStorage::Obj(_) => "Obj",
                HeapStorage::Str(_) => "Str",
                HeapStorage::Boxed(_) => "Boxed",
            };
            // invariant: try_as_object_mut callers must only use object references whose heap
            // storage kind has already been established; a different variant is a VM heap bug.
            panic!(
                "called ObjectRef::try_as_object_mut on non-object heap reference: variant={}",
                variant
            )
        };

        Ok(op(instance))
    }

    pub fn try_as_vector<T>(
        &self,
        op: impl FnOnce(&Vector<'gc>) -> T,
    ) -> Result<T, ExecutionError> {
        let ObjectRef(Some(o)) = &self else {
            return Err(ExecutionError::NullReference);
        };
        let inner = o.borrow();
        inner.validate_magic();
        let HeapStorage::Vec(instance) = &inner.storage else {
            // invariant: try_as_vector callers must only use object references whose heap storage
            // kind has already been established; a different variant indicates VM heap corruption.
            panic!("called ObjectRef::try_as_vector on non-vector heap reference")
        };

        Ok(op(instance))
    }

    pub fn try_as_vector_mut<T>(
        &self,
        gc: GCHandle<'gc>,
        op: impl FnOnce(&mut Vector<'gc>) -> T,
    ) -> Result<T, ExecutionError> {
        let ObjectRef(Some(o)) = &self else {
            return Err(ExecutionError::NullReference);
        };
        let mut inner = ThreadSafeLock::borrow_mut(*o, &gc);
        inner.validate_magic();
        let HeapStorage::Vec(instance) = &mut inner.storage else {
            // invariant: try_as_vector_mut callers must only use object references whose heap
            // storage kind has already been established; a different variant is a VM heap bug.
            panic!("called ObjectRef::try_as_vector_mut on non-vector heap reference")
        };

        Ok(op(instance))
    }

    pub fn try_as_heap_storage<T>(
        &self,
        op: impl FnOnce(&HeapStorage<'gc>) -> T,
    ) -> Result<T, ExecutionError> {
        let ObjectRef(Some(o)) = &self else {
            return Err(ExecutionError::NullReference);
        };
        let inner = o.borrow();
        inner.validate_magic();
        inner.validate_arena_id();
        Ok(op(&inner.storage))
    }

    pub fn as_object<T>(&self, op: impl FnOnce(&Object<'gc>) -> T) -> T {
        let ObjectRef(Some(o)) = &self else {
            panic!("NullReferenceException: called ObjectRef::as_object on NULL object reference")
        };
        let inner = o.borrow();
        inner.validate_magic();
        let HeapStorage::Obj(instance) = &inner.storage else {
            let variant = match &inner.storage {
                HeapStorage::Vec(_) => "Vec",
                HeapStorage::Obj(_) => "Obj",
                HeapStorage::Str(_) => "Str",
                HeapStorage::Boxed(_) => "Boxed",
            };
            panic!(
                "called ObjectRef::as_object on non-object heap reference: variant={}",
                variant
            )
        };

        op(instance)
    }

    pub fn as_object_mut<T>(&self, gc: GCHandle<'gc>, op: impl FnOnce(&mut Object<'gc>) -> T) -> T {
        let ObjectRef(Some(o)) = &self else {
            panic!(
                "NullReferenceException: called ObjectRef::as_object_mut on NULL object reference"
            )
        };
        let mut inner = ThreadSafeLock::borrow_mut(*o, &gc);
        inner.validate_magic();
        let HeapStorage::Obj(instance) = &mut inner.storage else {
            let variant = match &inner.storage {
                HeapStorage::Vec(_) => "Vec",
                HeapStorage::Obj(_) => "Obj",
                HeapStorage::Str(_) => "Str",
                HeapStorage::Boxed(_) => "Boxed",
            };
            panic!(
                "called ObjectRef::as_object_mut on non-object heap reference: variant={}",
                variant
            )
        };

        op(instance)
    }

    pub fn as_vector<T>(&self, op: impl FnOnce(&Vector<'gc>) -> T) -> T {
        let ObjectRef(Some(o)) = &self else {
            panic!("NullReferenceException: called ObjectRef::as_vector on NULL object reference")
        };
        let inner = o.borrow();
        inner.validate_magic();
        let HeapStorage::Vec(instance) = &inner.storage else {
            panic!("called ObjectRef::as_vector on non-vector heap reference")
        };

        op(instance)
    }

    pub fn as_vector_mut<T>(&self, gc: GCHandle<'gc>, op: impl FnOnce(&mut Vector<'gc>) -> T) -> T {
        let ObjectRef(Some(o)) = &self else {
            panic!(
                "NullReferenceException: called ObjectRef::as_vector_mut on NULL object reference"
            )
        };
        let mut inner = ThreadSafeLock::borrow_mut(*o, &gc);
        inner.validate_magic();
        let HeapStorage::Vec(instance) = &mut inner.storage else {
            panic!("called ObjectRef::as_vector_mut on non-vector heap reference")
        };

        op(instance)
    }

    pub fn as_heap_storage<T>(&self, op: impl FnOnce(&HeapStorage<'gc>) -> T) -> T {
        let ObjectRef(Some(o)) = &self else {
            panic!(
                "NullReferenceException: called ObjectRef::as_heap_storage on NULL object reference"
            )
        };
        let inner = o.borrow();
        inner.validate_magic();
        inner.validate_arena_id();
        op(&inner.storage)
    }

    pub fn sync_block_index(&self) -> Option<SyncBlockIndex> {
        let ObjectRef(Some(o)) = &self else {
            panic!(
                "NullReferenceException: called ObjectRef::sync_block_index on NULL object reference"
            )
        };
        let inner = o.borrow();
        inner.validate_magic();
        inner.validate_arena_id();
        inner.sync_block_index()
    }

    /// Returns the object's sync block index, assigning the one `assign` creates if it has none.
    ///
    /// An index is assigned at most once and never replaced. The check and the assignment happen
    /// under the object's write lock, so threads racing to lock a fresh object agree on one index
    /// and `assign` runs at most once per object.
    pub fn get_or_assign_sync_block_index(
        &self,
        gc: GCHandle<'gc>,
        assign: impl FnOnce() -> SyncBlockIndex,
    ) -> SyncBlockIndex {
        if let Some(index) = self.sync_block_index() {
            return index;
        }
        let ObjectRef(Some(o)) = &self else {
            panic!(
                "NullReferenceException: called ObjectRef::get_or_assign_sync_block_index on NULL object reference"
            )
        };
        let mut inner = ThreadSafeLock::borrow_mut(*o, &gc);
        inner.validate_magic();
        inner.validate_arena_id();
        if let Some(index) = inner.sync_block_index() {
            return index;
        }
        let index = assign();
        inner.set_sync_block_index(index);
        index
    }

    /// Safely accesses the object's data as a byte slice.
    ///
    /// This method ensures that the object is locked for the duration of the access
    /// and provides a stable view of the underlying data.
    pub fn with_data<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        let ObjectRef(Some(o)) = &self else {
            panic!("NullReferenceException: called ObjectRef::with_data on NULL object reference")
        };
        let inner = o.borrow();
        inner.validate_magic();
        match &inner.storage {
            HeapStorage::Vec(v) => f(&v.storage),
            HeapStorage::Obj(o) => o.instance_storage.with_data(f),
            HeapStorage::Str(s) => {
                // SAFETY: F10.RawMemoryAccessValid — `CLRString` stores UTF-16 `u16` code units.  Casting
                // to `*const u8` is valid (u8 alignment ≤ u16 alignment).
                // `s.len() * 2` gives the correct byte length; the multiplication
                // cannot overflow because `s.len()` is bounded by the allocation
                // size, which fits in `usize`.  The slice lifetime is bounded by
                // the `inner` borrow that holds the lock, preventing data races.
                let bytes =
                    unsafe { std::slice::from_raw_parts(s.as_ptr() as *const u8, s.len() * 2) };
                f(bytes)
            }
            HeapStorage::Boxed(o) => o.instance_storage.with_data(f),
        }
    }

    /// Safely accesses the object's data as a mutable byte slice.
    ///
    /// This method ensures that the object is locked for the duration of the access
    /// and provides a stable, mutable view of the underlying data.
    pub fn with_data_mut<T>(&self, gc: GCHandle<'gc>, f: impl FnOnce(&mut [u8]) -> T) -> T {
        let ObjectRef(Some(o)) = &self else {
            panic!(
                "NullReferenceException: called ObjectRef::with_data_mut on NULL object reference"
            )
        };
        let mut inner = ThreadSafeLock::borrow_mut(*o, &gc);
        inner.validate_magic();
        match &mut inner.storage {
            HeapStorage::Vec(v) => f(&mut v.storage),
            HeapStorage::Obj(o) => o.instance_storage.with_data_mut(f),
            HeapStorage::Boxed(o) => o.instance_storage.with_data_mut(f),
            HeapStorage::Str(s) => {
                // SAFETY: F10.RawMemoryAccessValid — `CLRString` stores UTF-16 `u16` code units. Casting to
                // `*mut u8` is valid (`u8` alignment <= `u16` alignment), and
                // `s.len() * 2` is the correct byte-length view.
                let bytes = unsafe {
                    std::slice::from_raw_parts_mut(
                        s.as_mut_slice().as_mut_ptr() as *mut u8,
                        s.len() * 2,
                    )
                };
                f(bytes)
            }
        }
    }
}

impl Debug for ObjectRef<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.0 {
            None => f.write_str("NULL"),
            Some(gc) => {
                let inner = gc.borrow();
                inner.validate_magic();
                let desc = match &inner.storage {
                    HeapStorage::Obj(o) => o.description.type_name(),
                    HeapStorage::Vec(v) => format!("{:?}[{}]", v.element, v.layout.length),
                    HeapStorage::Str(s) => format!("{:?}", s),
                    HeapStorage::Boxed(o) => format!("boxed {}", o.description.type_name()),
                };
                write!(f, "{} @ {:#?}", desc, Gc::as_ptr(gc))
            }
        }
    }
}

impl<'gc> PointerLike for ObjectRef<'gc> {
    #[inline]
    fn pointer(&self) -> Option<NonNull<u8>> {
        ObjectRef::pointer(self)
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;
    #[cfg(feature = "multithreading")]
    use dotnet_utils::ArenaId;
    #[cfg(feature = "multithreading")]
    use dotnet_utils::gc::{register_arena, unregister_arena};
    #[cfg(feature = "multithreading")]
    use dotnet_utils::sync::MANAGED_THREAD_ID;
    #[cfg(feature = "multithreading")]
    use static_assertions::assert_impl_all;
    #[cfg(not(feature = "multithreading"))]
    use static_assertions::assert_not_impl_all;
    #[cfg(feature = "multithreading")]
    use std::sync::Arc;
    #[cfg(feature = "multithreading")]
    use std::sync::atomic::AtomicBool;
    #[cfg(feature = "multithreading")]
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    #[cfg(feature = "multithreading")]
    static NEXT_TEST_ARENA_ID: AtomicU64 = AtomicU64::new(2000);

    #[cfg(feature = "multithreading")]
    struct EmptyTestRoot;
    #[cfg(feature = "multithreading")]
    gc_arena::static_collect!(EmptyTestRoot);

    #[test]
    fn sync_block_index_encoding_is_compact() {
        assert_eq!(size_of::<Option<SyncBlockIndex>>(), size_of::<usize>());
        assert_eq!(size_of::<SyncBlockIndex>(), size_of::<usize>());
    }

    #[test]
    fn object_inner_sync_block_index_roundtrip() {
        let mut inner = ObjectInner::new(
            HeapStorage::Str(crate::string::CLRString::from("idx")),
            ArenaId::INVALID,
        );
        assert_eq!(inner.sync_block_index(), None);
        let index = SyncBlockIndex::new(17).expect("non-zero index");
        inner.set_sync_block_index(index);
        assert_eq!(inner.sync_block_index(), Some(index));
    }

    #[cfg(feature = "multithreading")]
    assert_impl_all!(ObjectPtr: Send, Sync);
    #[cfg(feature = "multithreading")]
    assert_impl_all!(ObjectInner<'static>: Send, Sync);
    #[cfg(feature = "multithreading")]
    assert_impl_all!(ThreadSafeLock<ObjectInner<'static>>: Send, Sync);
    #[cfg(not(feature = "multithreading"))]
    assert_not_impl_all!(ObjectPtr: Send);
    #[cfg(not(feature = "multithreading"))]
    assert_not_impl_all!(ObjectPtr: Sync);

    #[cfg(feature = "multithreading")]
    fn next_test_arena_id() -> ArenaId {
        ArenaId::new(NEXT_TEST_ARENA_ID.fetch_add(1, AtomicOrdering::Relaxed))
    }

    #[cfg(feature = "multithreading")]
    struct ManagedThreadIdGuard {
        previous: Option<ArenaId>,
    }

    #[cfg(feature = "multithreading")]
    impl ManagedThreadIdGuard {
        fn set(id: ArenaId) -> Self {
            let previous = MANAGED_THREAD_ID.with(|thread_id| {
                let prev = thread_id.get();
                thread_id.set(Some(id));
                prev
            });
            Self { previous }
        }
    }

    #[cfg(feature = "multithreading")]
    impl Drop for ManagedThreadIdGuard {
        fn drop(&mut self) {
            MANAGED_THREAD_ID.with(|thread_id| thread_id.set(self.previous));
        }
    }

    /// Validate addresses without pointers.
    #[cfg(feature = "multithreading")]
    #[test]
    fn codec_address_limits() {
        for addr in [8, (1usize << 48) - 8] {
            assert_eq!(check_compact_address(addr), Ok(()));
        }
        for addr in [0, 1, 9, 1usize << 48, usize::MAX - 7] {
            assert_eq!(
                check_compact_address(addr),
                Err(ObjectCodecError::InvalidAddress(addr))
            );
        }
    }

    /// Short slices fail before memory access.
    #[test]
    fn codec_short_storage() {
        for len in 0..ObjectRef::SIZE {
            assert_eq!(
                check_object_storage(len),
                Err(ObjectCodecError::ShortStorage { actual: len })
            );
            // SAFETY: F10.RawMemoryAccessValid — The checked helper rejects the short slice
            // before attempting to read a pointer, so no initialized pointer is required.
            let result = unsafe { ObjectRef::try_read_unchecked(&[0; ObjectRef::SIZE][..len]) };
            assert!(
                matches!(result, Err(ObjectCodecError::ShortStorage { actual }) if actual == len)
            );
        }
        assert_eq!(check_object_storage(ObjectRef::SIZE), Ok(()));
    }

    #[cfg(feature = "multithreading")]
    #[test]
    fn test_read_unchecked_cross_arena_invalid() {
        let arena_id = ArenaId::new(1000);
        unregister_arena(arena_id);

        // Construct a provenance-carrying Tag 5 pointer for an unregistered arena. Its address
        // bits don't refer to an allocation, but the failed lease must prevent dereferencing it.
        let base = NonNull::<ThreadSafeLock<ObjectInner<'static>>>::dangling()
            .as_ptr()
            .cast::<u8>();
        let tagged_ptr = base.map_addr(|addr| {
            (addr & 0x0000FFFFFFFFFFF8) | 5 | ((arena_id.as_u64() as usize) << 48)
        });

        let mut source = [0u8; 8];
        // SAFETY: F6.NoEscapeAcrossArena — The synthetic Tag 5 value is only checked against the absent arena lease and
        // is never dereferenced.
        unsafe {
            source
                .as_mut_ptr()
                .cast::<*mut u8>()
                .write_unaligned(tagged_ptr)
        };

        // SAFETY: F6.NoEscapeAcrossArena — The absent owner is rejected before the synthetic
        // pointer is dereferenced; the stored pointer itself is initialized.
        let result = unsafe { ObjectRef::try_read_unchecked(&source) };
        assert!(matches!(result, Err(ObjectCodecError::OwnerUnavailable(id)) if id == arena_id));
    }

    /// A real GC allocation
    /// passes through the inert cross-arena wrapper without exposing its pointer.
    #[cfg(feature = "multithreading")]
    #[test]
    fn arena_lifetime_opaque_gc_transport() {
        use dotnet_utils::gc::{CrossArenaRef, OpaqueObjectPtr, try_acquire_lease};
        use gc_arena::{Arena, Rootable};
        let arena = Arena::<Rootable![EmptyTestRoot]>::new(|_| EmptyTestRoot);
        arena.mutate(|mc, _| {
            let id = ArenaId::new(10_103);
            register_arena(id, Arc::new(AtomicBool::new(false)));
            let allocation = Gc::new(
                mc,
                ThreadSafeLock::new(ObjectInner::new(
                    HeapStorage::Str(crate::string::CLRString::from("transport")),
                    id,
                )),
            );
            let lease = try_acquire_lease(id).expect("registered owner");
            // SAFETY: F11.ObjectPointerThreadSafe — This pointer comes directly
            // from Gc::as_ptr for the required ThreadSafeLock<ObjectInner> type.
            let ptr = unsafe { OpaqueObjectPtr::from_raw(Gc::as_ptr(allocation).cast()) };
            // SAFETY: F1.ArenaGenerationMatch — The lease identifies the owner
            // registered just above, which allocated this object in this arena.
            let reference = unsafe { CrossArenaRef::new(lease.identity(), ptr) };
            drop(lease);
            let returned = std::thread::spawn(move || reference).join().unwrap();
            let lease = dotnet_utils::gc::try_acquire_lease_for_identity(returned.identity())
                .expect("same owner registration");
            // SAFETY: F1.GcHandleRooted — No collection can run during this
            // mutation; the live allocation belongs to this arena and the exact
            // owner lease is held. The worker never accesses the pointer.
            let object = unsafe { ObjectPtr::from_raw(returned.ptr().as_ptr().cast()) }.unwrap();
            assert_eq!(object.owner_id(), id);
            assert_eq!(object.as_ptr(), Gc::as_ptr(allocation).cast());
            drop(lease);
            unregister_arena(id);
        });
    }

    #[cfg(feature = "multithreading")]
    #[test]
    fn test_read_unchecked_cross_arena_valid() {
        use gc_arena::{Arena, Rootable};
        let arena = Arena::<Rootable![EmptyTestRoot]>::new(|_mc| EmptyTestRoot);
        arena.mutate(|mc, _root| {
            let arena_id = ArenaId::new(1001);
            register_arena(arena_id, Arc::new(AtomicBool::new(false)));

            // We need a pointer to something that looks like an ObjectInner to pass magic check.
            let inner = ObjectInner::new(
                HeapStorage::Str(crate::string::CLRString::from("test")),
                arena_id,
            );
            let lock = Gc::new(mc, ThreadSafeLock::new(inner));
            let obj = ObjectRef(Some(lock));
            let tagged_ptr = obj.to_storage_ptr();
            assert_eq!(tagged_ptr.addr() & 7, 5);
            assert_eq!((tagged_ptr.addr() >> 48) as u16, arena_id.as_u64() as u16);

            let mut source = [0u8; 8];
            // SAFETY: F10.RawMemoryAccessValid — `source` is an 8-byte local, a pointer's width on the 64-bit targets the compact format requires; `write_unaligned` needs no alignment.
            unsafe {
                source
                    .as_mut_ptr()
                    .cast::<*mut u8>()
                    .write_unaligned(tagged_ptr)
            };

            // SAFETY: F1.GcHandleRooted — `source` holds `tagged_ptr`, encoded from `lock`, which `mc`'s arena keeps live through this mutation, and `arena_id` is still registered.
            let obj_ref = unsafe { ObjectRef::read_unchecked(&source) };

            assert!(
                obj_ref.0.is_some(),
                "Should return Some for registered arena"
            );

            unregister_arena(arena_id);
        });
    }

    #[cfg(feature = "multithreading")]
    #[test]
    fn test_write_encodes_owner_tag_even_on_owner_thread() {
        use gc_arena::{Arena, Rootable};

        let arena_id = next_test_arena_id();
        let _thread_guard = ManagedThreadIdGuard::set(arena_id);
        register_arena(arena_id, Arc::new(AtomicBool::new(false)));

        let arena_handle_owner = dotnet_utils::gc::ArenaHandle::new(arena_id);
        // SAFETY: F1.GcHandleRooted — `arena_handle_owner` is a local that outlives the entire
        // `arena.mutate(...)` closure below.  We extend the lifetime to
        // `'static` only for use within that closure; the closure completes
        // before `arena_handle_owner` is dropped, so the extended lifetime is
        // sound for the duration of this test.
        let arena_handle = unsafe {
            std::mem::transmute::<
                &dotnet_utils::gc::ArenaHandleInner,
                &'static dotnet_utils::gc::ArenaHandleInner,
            >(arena_handle_owner.as_inner())
        };
        let arena = Arena::<Rootable![EmptyTestRoot]>::new(|_mc| EmptyTestRoot);
        arena.mutate(|mc, _root| {
            #[cfg(not(feature = "memory-validation"))]
            let handle = GCHandle::new(mc, arena_handle);
            #[cfg(feature = "memory-validation")]
            let handle = dotnet_utils::gc::GCHandle::new(mc, arena_handle, arena_id);

            let obj = ObjectRef::new(
                handle,
                HeapStorage::Str(crate::string::CLRString::from("x")),
            );

            let mut bytes = [0u8; size_of::<usize>()];
            obj.write(&mut bytes);
            let encoded = usize::from_ne_bytes(bytes);

            assert_eq!(encoded & 7, 5, "ObjectRef::write must persist owner tag");
            assert_eq!(
                (encoded >> 48) as u16 as u64,
                arena_id.as_u64() & 0xFFFF,
                "encoded arena id must match owner"
            );

            // SAFETY: F6.NoEscapeAcrossArena — `bytes` was produced by `ObjectRef::write`, which
            // encodes a valid live `Gc` pointer with a Tag 5 owner marker.
            // The owning arena is still registered at this point
            // (`unregister_arena` is called only after the closure returns),
            // so `read_unchecked` will find a live arena and decode safely.
            let roundtrip = unsafe { ObjectRef::read_unchecked(&bytes) };
            assert!(
                roundtrip.0.is_some(),
                "tagged reference should decode while owner arena is live"
            );
        });

        unregister_arena(arena_id);
    }

    /// Verify that `ObjectPtr::as_heap_storage` panics when the magic number is
    /// corrupt.  `validate_magic` is active under `debug_assertions` (always
    /// true for `cargo test`), so no feature gate is needed here.
    #[test]
    fn test_object_ptr_as_heap_storage_corrupt_magic_panics() {
        use std::panic::{AssertUnwindSafe, catch_unwind};

        // Build an ObjectInner with a deliberately wrong magic value.
        let corrupt = Box::new(ThreadSafeLock::new(ObjectInner::with_magic_for_tests(
            HeapStorage::Str(crate::string::CLRString::from("bad")),
            ArenaId::INVALID,
            0xDEAD_BEEF_1234_5678,
        )));
        let raw = Box::into_raw(corrupt);

        // SAFETY: F10.RawAllocationOwnership — `raw` was produced by `Box::into_raw` immediately above and
        // has not been freed.  The `ObjectPtr` is only used inside this test
        // and is never sent to another thread.
        let ptr = unsafe { ObjectPtr::from_raw(raw).expect("non-null") };

        // validate_magic fires under debug_assertions and panics on mismatch.
        let result = catch_unwind(AssertUnwindSafe(|| ptr.as_heap_storage(|_| ())));

        // Reclaim the allocation regardless of the panic outcome.
        // SAFETY: F10.RawAllocationOwnership — `raw` was produced by `Box::into_raw` above; no other owner
        // of the allocation exists at this point.
        let _ = unsafe { Box::from_raw(raw) };

        assert!(
            result.is_err(),
            "as_heap_storage must panic when magic is corrupt"
        );
    }

    /// Verify that `ObjectPtr::as_heap_storage` panics when the object's
    /// `owner_id` does not match the current arena and the arena is not live.
    /// `validate_arena_id` is only active under `feature = "memory-validation"`.
    #[cfg(feature = "memory-validation")]
    #[test]
    fn test_object_ptr_as_heap_storage_cross_arena_id_panics() {
        use std::panic::{AssertUnwindSafe, catch_unwind};

        // Pick a foreign arena that is definitely not the current thread's and
        // is not registered in the global arena table.
        let foreign_arena = ArenaId::new(0x0000_CAFE_BABE_0001);
        #[cfg(feature = "multithreading")]
        // Ensure the ID is absent from the registry; `unregister_arena` is a
        // no-op when the ID is already absent.
        unregister_arena(foreign_arena);

        // Build an ObjectInner with a valid magic but a foreign owner_id.
        let obj = Box::new(ThreadSafeLock::new(ObjectInner::new(
            HeapStorage::Str(crate::string::CLRString::from("cross")),
            foreign_arena,
        )));
        let raw = Box::into_raw(obj);

        // SAFETY: F10.RawAllocationOwnership — `raw` was produced by `Box::into_raw` immediately above and
        // has not been freed.  The `ObjectPtr` is used only inside this test.
        let ptr = unsafe { ObjectPtr::from_raw(raw).expect("non-null") };

        // validate_arena_id fires because owner_id != current thread id and
        // owner_id != INVALID.  Under not(multithreading) it panics directly;
        // under multithreading it panics because the arena is not registered.
        let result = catch_unwind(AssertUnwindSafe(|| ptr.as_heap_storage(|_| ())));

        // SAFETY: F10.RawAllocationOwnership — `raw` was produced by `Box::into_raw` above; no other owner
        // of the allocation exists at this point.
        let _ = unsafe { Box::from_raw(raw) };

        assert!(
            result.is_err(),
            "as_heap_storage must panic on uncoordinated cross-arena access"
        );
    }
}
