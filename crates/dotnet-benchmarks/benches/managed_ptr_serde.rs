//! Managed pointer serialization micro-benchmarks.
//!
//! This isolates the current three-word [`ManagedPtr`] wire format from VM
//! execution. Each origin variant has separate `write` and `read_resolved_unchecked`
//! measurements; `Transient` reads intentionally measure the documented
//! `UnknownSubtag` rejection path.

use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use dotnet_types::{TypeDescription, generics::GenericLookup};
use dotnet_utils::{
    ArenaId, ByteOffset,
    gc::{ArenaHandle, ArenaHandleInner, GCHandle, register_arena, unregister_arena},
    sync::{Arc, AtomicBool, MANAGED_THREAD_ID},
};
use dotnet_value::pointer::{ManagedPtrInfo, ManagedPtrResolver, StaticMetadata};
use dotnet_value::{
    CLRString, HeapStorage, ManagedPtr, Object, ObjectRef, StackSlotIndex,
    layout::{FieldLayoutManager, GcDesc},
    storage::FieldStorage,
};
use dotnet_vm::HeapManagedPtrDecodeCache;
use gc_arena::{Arena, Rootable};
use hashbrown::HashMap;
use std::ptr::NonNull;

const BENCH_ARENA_ID: ArenaId = ArenaId::new(1);

struct ArenaRegistrationGuard {
    arena_id: ArenaId,
}

impl ArenaRegistrationGuard {
    fn register(arena_id: ArenaId) -> Self {
        register_arena(arena_id, Arc::new(AtomicBool::new(false)));
        Self { arena_id }
    }
}

impl Drop for ArenaRegistrationGuard {
    fn drop(&mut self) {
        unregister_arena(self.arena_id);
    }
}

struct ManagedThreadIdGuard {
    previous: Option<ArenaId>,
}

impl ManagedThreadIdGuard {
    fn set(arena_id: ArenaId) -> Self {
        let previous = MANAGED_THREAD_ID.with(|thread_id| {
            let previous = thread_id.get();
            thread_id.set(Some(arena_id));
            previous
        });
        Self { previous }
    }
}

impl Drop for ManagedThreadIdGuard {
    fn drop(&mut self) {
        MANAGED_THREAD_ID.with(|thread_id| thread_id.set(self.previous));
    }
}

fn with_benchmark_gc_context<R>(f: impl for<'gc> FnOnce(GCHandle<'gc>) -> R) -> R {
    type BenchRoot = Rootable![()];

    let arena = Arena::<BenchRoot>::new(|_mc| ());
    let _arena_registration = ArenaRegistrationGuard::register(BENCH_ARENA_ID);
    let _thread_id = ManagedThreadIdGuard::set(BENCH_ARENA_ID);
    let arena_handle_owner = ArenaHandle::new(BENCH_ARENA_ID);
    // SAFETY: F1.GcHandleRooted — `arena_handle_owner` is retained until after `arena.mutate` returns, and the
    // transmuted reference is used only to construct the GCHandle passed within that closure.
    let arena_handle = unsafe {
        std::mem::transmute::<&ArenaHandleInner, &'static ArenaHandleInner>(
            arena_handle_owner.as_inner(),
        )
    };

    arena.mutate(|gc, _root| {
        f(GCHandle::new(
            gc,
            arena_handle,
            #[cfg(feature = "memory-validation")]
            BENCH_ARENA_ID,
        ))
    })
}

fn non_null_at(bytes: &mut [u8], offset: usize) -> NonNull<u8> {
    NonNull::new(bytes.as_mut_ptr().wrapping_add(offset)).expect("slice pointers are non-null")
}

#[derive(Copy, Clone)]
struct BenchmarkManagedPtrResolver {
    base: NonNull<u8>,
}

impl<'gc> ManagedPtrResolver<'gc> for BenchmarkManagedPtrResolver {
    fn stack_slot_base(&self, _slot: StackSlotIndex) -> Option<NonNull<u8>> {
        Some(self.base)
    }

    fn static_storage_base(&self, _metadata: &StaticMetadata) -> Option<NonNull<u8>> {
        Some(self.base)
    }
}

/// Decodes one Heap representation through the VM's caller-owned cache type.
/// The cache contains only rooted object handles, so every invocation still
/// borrows current object storage before applying the encoded offset.
fn read_cached_heap_handle<'gc>(
    source: &[u8],
    resolver: &BenchmarkManagedPtrResolver,
    cache: &mut HeapManagedPtrDecodeCache<'gc>,
) -> Option<ManagedPtrInfo<'gc>> {
    // SAFETY: F3.InteriorPointerRebased — The source was produced from a live Heap ManagedPtr and the VM
    // cache type models the traced, collection-epoch-invalidated production owner.
    unsafe { ManagedPtr::read_resolved_with_heap_cache_unchecked(source, resolver, cache).ok() }
}

fn transient_object<'gc>() -> Object<'gc> {
    let layout = Arc::new(FieldLayoutManager {
        fields: HashMap::new(),
        total_size: 0,
        alignment: 1,
        gc_desc: GcDesc::default(),
        has_ref_fields: false,
    });
    Object::new(
        TypeDescription::NULL,
        GenericLookup::default(),
        FieldStorage::new(layout, Vec::new()),
    )
}

fn bench_managed_ptr_serde(c: &mut Criterion) {
    with_benchmark_gc_context(|gc| {
        dotnet_value::pointer::reset_static_registry();

        let mut backing = vec![0u8; 64];
        let resolver = BenchmarkManagedPtrResolver {
            base: non_null_at(&mut backing, 0),
        };
        let unmanaged_ptr = non_null_at(&mut backing, 0);
        let stack_ptr = non_null_at(&mut backing, 8);
        let static_ptr = non_null_at(&mut backing, 16);
        let transient_ptr = non_null_at(&mut backing, 24);

        let object = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("managed ptr bench")));
        let heap_offset = ByteOffset::new(2);
        let heap_ptr = object.with_data(|data| {
            NonNull::new(
                data.as_ptr()
                    .wrapping_add(heap_offset.as_usize())
                    .cast_mut(),
            )
            .expect("object storage pointer is non-null")
        });
        let (cross_object, cross_arena_id) = object
            .as_ptr_info()
            .expect("benchmark object has a cross-arena pointer");
        let cross_offset = ByteOffset::new(4);
        let cross_ptr = object.with_data(|data| {
            NonNull::new(
                data.as_ptr()
                    .wrapping_add(cross_offset.as_usize())
                    .cast_mut(),
            )
            .expect("object storage pointer is non-null")
        });

        let cases = vec![
            (
                "unmanaged",
                ManagedPtr::new(
                    Some(unmanaged_ptr),
                    TypeDescription::NULL,
                    None,
                    false,
                    None,
                ),
            ),
            (
                "stack",
                ManagedPtr::new(
                    Some(stack_ptr),
                    TypeDescription::NULL,
                    None,
                    false,
                    Some(ByteOffset::new(8)),
                )
                .with_stack_origin(StackSlotIndex::new(7)),
            ),
            (
                "heap",
                ManagedPtr::new(
                    Some(heap_ptr),
                    TypeDescription::NULL,
                    Some(object),
                    false,
                    Some(heap_offset),
                ),
            ),
            (
                "static",
                ManagedPtr::new_static(
                    Some(static_ptr),
                    TypeDescription::NULL,
                    TypeDescription::NULL,
                    GenericLookup::default(),
                    false,
                    ByteOffset::new(16),
                ),
            ),
            (
                "cross_arena",
                ManagedPtr::new_cross_arena(
                    Some(cross_ptr),
                    TypeDescription::NULL,
                    cross_object,
                    cross_arena_id,
                    cross_offset,
                ),
            ),
            (
                "transient",
                ManagedPtr::new_transient(
                    Some(transient_ptr),
                    TypeDescription::NULL,
                    transient_object(),
                    ByteOffset::new(24),
                ),
            ),
        ];

        let mut write_group = c.benchmark_group("ManagedPtr::write");
        for (origin, ptr) in &cases {
            let mut dest = ManagedPtr::serialization_buffer();
            write_group.bench_function(*origin, |b| {
                b.iter(|| ptr.write(black_box(&mut dest)));
            });
        }
        write_group.finish();

        let mut read_group = c.benchmark_group("ManagedPtr::read_resolved_unchecked");
        for (origin, ptr) in &cases {
            let mut source = ManagedPtr::serialization_buffer();
            ptr.write(&mut source);
            read_group.bench_function(*origin, |b| {
                b.iter(|| {
                    // SAFETY: F3.InteriorPointerRebased — `source` was produced by `ManagedPtr::write` from a live benchmark
                    // case and remains unchanged for the duration of this benchmark iteration; the
                    // resolver supplies the live Stack and Static bases.
                    black_box(unsafe {
                        ManagedPtr::read_resolved_unchecked(black_box(&source), &resolver)
                    })
                });
            });
        }
        read_group.finish();

        let mut heap_source = ManagedPtr::serialization_buffer();
        cases
            .iter()
            .find(|(origin, _)| *origin == "heap")
            .expect("benchmark cases include Heap")
            .1
            .write(&mut heap_source);
        let heap_handle_word = usize::from_ne_bytes(
            heap_source[..ObjectRef::SIZE]
                .try_into()
                .expect("ManagedPtr word 0 has pointer width"),
        );
        assert_ne!(heap_handle_word, 0, "Heap wire encoding is non-null");

        let heap_miss_object = ObjectRef::new(
            gc,
            HeapStorage::Str(CLRString::from("managed ptr bench cache miss")),
        );
        let heap_miss_ptr = heap_miss_object.with_data(|data| {
            NonNull::new(
                data.as_ptr()
                    .wrapping_add(heap_offset.as_usize())
                    .cast_mut(),
            )
            .expect("object storage pointer is non-null")
        });
        let heap_miss_ptr = ManagedPtr::new(
            Some(heap_miss_ptr),
            TypeDescription::NULL,
            Some(heap_miss_object),
            false,
            Some(heap_offset),
        );
        let mut heap_miss_source = ManagedPtr::serialization_buffer();
        heap_miss_ptr.write(&mut heap_miss_source);

        let mut cache_hit = HeapManagedPtrDecodeCache::default();
        assert!(read_cached_heap_handle(&heap_source, &resolver, &mut cache_hit).is_some());

        c.bench_function(
            "ManagedPtr::read_resolved_with_heap_cache_unchecked/heap_hit",
            |b| {
                b.iter(|| {
                    black_box(read_cached_heap_handle(
                        black_box(&heap_source),
                        black_box(&resolver),
                        black_box(&mut cache_hit),
                    ))
                });
            },
        );

        c.bench_function(
            "ManagedPtr::read_resolved_with_heap_cache_unchecked/heap_miss",
            |b| {
                b.iter_batched(
                    HeapManagedPtrDecodeCache::default,
                    |mut cache| {
                        black_box(read_cached_heap_handle(&heap_source, &resolver, &mut cache))
                    },
                    BatchSize::SmallInput,
                );
            },
        );

        c.bench_function(
            "ManagedPtr::read_resolved_with_heap_cache_unchecked/heap_mixed_hit_miss_pair",
            |b| {
                b.iter_batched(
                    || {
                        let mut cache = HeapManagedPtrDecodeCache::default();
                        assert!(
                            read_cached_heap_handle(&heap_source, &resolver, &mut cache).is_some()
                        );
                        cache
                    },
                    |mut cache| {
                        black_box((
                            read_cached_heap_handle(&heap_source, &resolver, &mut cache),
                            read_cached_heap_handle(&heap_miss_source, &resolver, &mut cache),
                        ))
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    });
}

criterion_group! {
    name = managed_ptr_serde_group;
    config = Criterion::default().configure_from_args();
    targets = bench_managed_ptr_serde
}
criterion_main!(managed_ptr_serde_group);
