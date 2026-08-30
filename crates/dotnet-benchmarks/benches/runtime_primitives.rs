//! Focused Rust-level baselines for runtime hot paths that are hidden by whole-VM fixtures.
//!
//! Keep these separate from `end_to_end`: they deliberately avoid IL dispatch, method
//! resolution, and managed fixture setup so stack and vector changes can be measured directly.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use dotnet_assemblies::AssemblyLoader;
use dotnet_types::{generics::ConcreteType, resolution::ResolutionS};
#[cfg(feature = "memory-validation")]
use dotnet_utils::ArenaId;
use dotnet_utils::{gc::GCHandle, sync::Arc};
use dotnet_value::{
    layout::{ArrayLayoutManager, LayoutManager, Scalar},
    object::Vector,
};
use dotnet_vm::{
    CallStack, EvalStackOps, GCArena,
    dispatch::ExecutionEngine,
    state::{ArenaLocalState, SharedGlobalState},
};
use dotnetdll::prelude::BaseType;
use gc_arena::{Collect, Gc, GcWeak, collect::Trace};

const PRIMITIVE_VECTOR_LENGTH: usize = 64 * 1024;

struct NoopTrace;

impl<'gc> Trace<'gc> for NoopTrace {
    fn trace_gc(&mut self, _gc: Gc<'gc, ()>) {}

    fn trace_gc_weak(&mut self, _gc: GcWeak<'gc, ()>) {}
}

#[allow(
    clippy::arc_with_non_send_sync,
    reason = "the no-MT benchmark confines its one arena to the invoking thread"
)]
fn new_stack_arena() -> GCArena {
    let loader = Arc::new(
        AssemblyLoader::new_bare("runtime-primitives-benchmark".to_owned())
            .expect("benchmark loader must initialize"),
    );
    let shared = Arc::new(SharedGlobalState::new(loader));

    GCArena::new(|_| ExecutionEngine::new(CallStack::new(shared, ArenaLocalState::new())))
}

fn primitive_i32_vector() -> Vector<'static> {
    Vector::new(
        ConcreteType::new(ResolutionS::NULL, BaseType::Int32),
        ArrayLayoutManager {
            element_layout: Arc::new(LayoutManager::Scalar(Scalar::Int32)),
            length: PRIMITIVE_VECTOR_LENGTH,
        },
        vec![0; PRIMITIVE_VECTOR_LENGTH * std::mem::size_of::<i32>()],
        vec![PRIMITIVE_VECTOR_LENGTH],
    )
}

fn runtime_primitives(c: &mut Criterion) {
    c.bench_function("runtime_primitives/eval_stack_push_i32_pop_safe", |b| {
        let mut arena = new_stack_arena();

        b.iter(|| {
            arena.mutate_root(|gc, engine| {
                let gc_handle = GCHandle::new(
                    gc,
                    #[cfg(feature = "multithreading")]
                    // SAFETY: the call stack is rooted in this arena and this benchmark mutates
                    // it only through this arena's `mutate_root` callback.
                    unsafe {
                        engine.stack.arena_inner_gc()
                    },
                    #[cfg(feature = "memory-validation")]
                    ArenaId::INVALID,
                );
                let mut ctx = engine.ves_context(gc_handle);

                ctx.push(dotnet_value::StackValue::Int32(black_box(17)));
                black_box(
                    ctx.pop_safe()
                        .expect("push immediately followed by pop_safe cannot underflow")
                        .as_i32(),
                );
            });
        });
    });

    c.bench_function("runtime_primitives/vector_trace_i32", |b| {
        let vector = primitive_i32_vector();

        b.iter(|| {
            let mut trace = NoopTrace;
            black_box(&vector).trace(&mut trace);
        });
    });
}

criterion_group!(benches, runtime_primitives);
criterion_main!(benches);
