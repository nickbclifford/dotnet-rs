//! In-mutate native-continuation stack and deliberately small managed interpreter.
//! This models scheduling/root transfer, not the real VM's SEH or OS-thread implementation.
use effect_ops::*;
use gc_arena::{Arena, Collect, Mutation, Rootable};
use std::{cell::Cell, collections::HashMap, task::Poll};

#[derive(Default)]
pub struct Backend {
    pub force_stop: bool,
    pub scopes: Cell<usize>,
    pub copied: usize,
    pub finalizers: usize,
    pub monitor_busy: bool,
    pub statics: HashMap<usize, u8>,
}
impl HostOps<'_> for Backend {
    fn safe_point_requested(&self) -> bool {
        self.force_stop
    }
    fn scopes(&self) -> &Cell<usize> {
        &self.scopes
    }
    fn copied(&mut self, count: usize) {
        self.copied += count;
    }
    fn static_action(&mut self, ty: usize) -> StaticAction {
        match self.statics.get(&ty).copied().unwrap_or(0) {
            0 => {
                self.statics.insert(ty, 1);
                StaticAction::Run(4)
            }
            1 => StaticAction::Wait,
            2 => StaticAction::Ready,
            _ => StaticAction::Failed,
        }
    }
}
#[derive(Collect)]
#[collect(no_drop)]
struct Managed<'gc> {
    #[collect(require_static)]
    propagation: Propagation,
    #[collect(require_static)]
    method: usize,
    args: Vec<Value<'gc>>,
    byrefs: Vec<(Value<'gc>, usize, ByRefKind)>,
    #[collect(require_static)]
    started: bool,
    reply: Option<Result<Value<'gc>, Failure>>,
}
// Failure has only static strings and primitive variants.

#[derive(Collect)]
#[collect(no_drop)]
enum Activation<'gc> {
    Native(NativeFrame<'gc>),
    Managed(Managed<'gc>),
}
#[derive(Collect)]
#[collect(no_drop)]
pub struct Root<'gc> {
    stack: Vec<Activation<'gc>>,
    result: Option<Result<Value<'gc>, Failure>>,
    #[collect(require_static)]
    pub backend: Backend,
    #[collect(require_static)]
    pub polls: usize,
    #[collect(require_static)]
    pub effects: usize,
}
impl Root<'_> {
    fn new() -> Self {
        Self {
            stack: vec![],
            result: None,
            backend: Backend::default(),
            polls: 0,
            effects: 0,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Wait {
    SafePoint,
    Finalizers,
    Monitor,
    Static(usize),
}
enum Tick {
    Continue,
    Wait(Wait),
    Done,
}

fn deliver<'gc>(mc: &Mutation<'gc>, root: &mut Root<'gc>, value: Result<Value<'gc>, Failure>) {
    match root.stack.last_mut() {
        Some(Activation::Native(parent)) => {
            let result = parent.region(mc, &mut root.backend, |c| value.map(|v| c.stash(v)));
            parent.reply(result);
        }
        Some(Activation::Managed(parent)) => parent.reply = Some(value),
        None => root.result = Some(value),
    }
}
fn tick<'gc>(mc: &Mutation<'gc>, root: &mut Root<'gc>) -> Tick {
    let Some(top) = root.stack.last_mut() else {
        return Tick::Done;
    };
    match top {
        Activation::Native(frame) => {
            root.polls += 1;
            match frame.poll(mc, &mut root.backend) {
                Poll::Ready(result) => {
                    // Re-brand/transfer before dropping the source's DynamicRootSet.
                    let value = frame.region(mc, &mut root.backend, |c| result.map(|v| v.get(c)));
                    root.stack.pop();
                    deliver(mc, root, value);
                    Tick::Continue
                }
                Poll::Pending => {
                    root.effects += 1;
                    match frame.take_effect() {
                        Effect::CallManaged(call) => {
                            let (args, byrefs) = frame.region(mc, &mut root.backend, |c| {
                                (
                                    call.args.iter().map(|v| v.get(c)).collect(),
                                    call.byrefs
                                        .iter()
                                        .map(|b| (b.owner.get(c), b.index, b.kind))
                                        .collect(),
                                )
                            });
                            root.stack.push(Activation::Managed(Managed {
                                method: call.method,
                                propagation: call.propagation,
                                args,
                                byrefs,
                                started: false,
                                reply: None,
                            }));
                            Tick::Continue
                        }
                        Effect::SafePoint => Tick::Wait(Wait::SafePoint),
                        Effect::WaitFinalizers => Tick::Wait(Wait::Finalizers),
                        Effect::MonitorAcquire { .. } => Tick::Wait(Wait::Monitor),
                        Effect::WaitStaticInit { ty } => Tick::Wait(Wait::Static(ty)),
                    }
                }
            }
        }
        Activation::Managed(frame) => {
            let result = match frame.method {
                0 => Ok(frame.args.first().copied().unwrap_or(Value::Void)),
                1 if !frame.started => {
                    frame.started = true;
                    let child = NativeFrame::new(mc, |mut fx| {
                        Box::pin(async move {
                            fx.safe_point().await?;
                            Ok(Stashed::Int(42))
                        })
                    });
                    root.stack.push(Activation::Native(child));
                    return Tick::Continue;
                }
                1 => frame.reply.take().expect("nested native reply"),
                2 => Err(Failure::Managed("callee failure")),
                3 | 5 => {
                    for (owner, index, kind) in &frame.byrefs {
                        let owner = owner.object();
                        match kind {
                            ByRefKind::BoxedValue => {
                                let data = owner.borrow();
                                let ObjectData::Array(values) = &*data else {
                                    panic!("byref owner is array")
                                };
                                let cell = values[*index].object();
                                drop(data);
                                *cell.borrow_mut(mc) = ObjectData::Boxed(99);
                            }
                            ByRefKind::ObjectSlot => {
                                let replacement = gc_arena::Gc::new(
                                    mc,
                                    ObjectLock::new(ObjectData::String(vec![90])),
                                );
                                let mut data = owner.borrow_mut(mc);
                                let ObjectData::Array(values) = &mut *data else {
                                    panic!("byref owner is array")
                                };
                                values[*index] = Value::Object(replacement);
                            }
                        }
                    }
                    if frame.method == 5 {
                        Err(Failure::Managed("callee failure"))
                    } else {
                        Ok(Value::Int(7))
                    }
                }
                4 => {
                    for state in root.backend.statics.values_mut() {
                        if *state == 1 {
                            *state = 2;
                        }
                    }
                    Ok(Value::Void)
                }
                _ => Err(Failure::Host("unknown method")),
            };
            let transparent = frame.propagation == Propagation::Transparent;
            root.stack.pop();
            if transparent && matches!(result, Err(Failure::Managed(_))) {
                // Model of native-frame retirement during managed unwind: never resume
                // this Rust future with the exception it must transparently propagate.
                assert!(matches!(root.stack.pop(), Some(Activation::Native(_))));
            }
            deliver(mc, root, result);
            Tick::Continue
        }
    }
}

pub struct Vm {
    pub arena: Arena<Rootable![Root<'_>]>,
    pub collections: usize,
    pub waits: Vec<Wait>,
}
impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}
impl Vm {
    pub fn new() -> Self {
        Self {
            arena: Arena::new(|_| Root::new()),
            collections: 0,
            waits: vec![],
        }
    }
    pub fn spawn(&mut self, create: impl FnOnce(VmEffects) -> NativeFuture) {
        self.arena.mutate_root(|mc, root| {
            root.stack
                .push(Activation::Native(NativeFrame::new(mc, create)))
        });
    }
    pub fn spawn_string(
        &mut self,
        chars: Vec<u16>,
        start: i32,
        length: Option<i32>,
        generic: bool,
    ) {
        self.arena.mutate_root(|mc, root| {
            let object = gc_arena::Gc::new(mc, ObjectLock::new(ObjectData::String(chars)));
            let frame = NativeFrame::with_args(
                mc,
                vec![Value::Object(object)],
                &mut root.backend,
                |fx, mut args| {
                    let source = StringRoot(args.remove(0));
                    if generic {
                        Box::pin(effect_intrinsics::gat::substring(fx, source, start, length))
                    } else {
                        Box::pin(effect_intrinsics::chosen::substring(
                            fx, source, start, length,
                        ))
                    }
                },
            );
            root.stack.push(Activation::Native(frame));
        });
    }
    pub fn full_gc(&mut self) {
        self.arena.finish_cycle();
        self.collections += 1;
    }
    /// One dispatch boundary. A real spawn shim invokes this poll inline in its current mutate.
    /// No future poll belongs in the 128-instruction dispatch batch.
    pub fn step(&mut self) -> bool {
        match self.arena.mutate_root(tick) {
            Tick::Continue => true,
            Tick::Done => false,
            Tick::Wait(wait) => {
                // Outside mutate and outside all arena/heap locks. Real STW handshake lives here.
                self.waits.push(wait);
                self.full_gc();
                self.arena.mutate_root(|_, root| {
                    match wait {
                        Wait::Finalizers => root.backend.finalizers = 0,
                        Wait::Monitor => root.backend.monitor_busy = false,
                        Wait::Static(ty) => {
                            root.backend.statics.insert(ty, 2);
                        }
                        Wait::SafePoint => {}
                    }
                    let Activation::Native(frame) = root.stack.last().unwrap() else {
                        panic!("waiter missing")
                    };
                    frame.reply(Ok(Stashed::Void));
                });
                true
            }
        }
    }
    pub fn run(&mut self) {
        while self.step() {}
    }
    /// Executor abort/budget paths drop futures and their roots without polling Rust async cleanup.
    pub fn cancel(&mut self, why: Failure) {
        self.arena.mutate_root(|_, root| {
            root.stack.clear();
            root.result = Some(Err(why));
        });
    }
    pub fn result_int(&self) -> Result<i64, Failure> {
        self.arena
            .mutate(|_, root| match root.result.as_ref().expect("completed") {
                Ok(Value::Int(n)) => Ok(*n),
                Err(e) => Err(e.clone()),
                _ => panic!("expected int"),
            })
    }
    pub fn result_string(&self) -> Vec<u16> {
        self.arena.mutate(|_, root| {
            let Value::Object(o) = root.result.as_ref().unwrap().as_ref().unwrap() else {
                panic!("expected string")
            };
            let data = o.borrow();
            let ObjectData::String(chars) = &*data else {
                panic!("expected string")
            };
            chars.clone()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use effect_intrinsics::{InvokeSpec, ReturnKind, chosen, gat};
    use std::{cell::Cell, rc::Rc};

    #[test]
    fn substring_progress_survives_forced_gc_for_chosen_raii_and_gat() {
        for generic in [false, true] {
            let mut vm = Vm::new();
            vm.arena.mutate_root(|_, r| r.backend.force_stop = true);
            let count = 2 * 65536 + 7;
            vm.spawn_string(vec![65; count + 2], 1, Some(count as i32), generic);
            vm.run();
            assert_eq!(vm.result_string(), vec![65; count]);
            assert_eq!(vm.collections, if generic { 2 } else { 5 });
            vm.arena.mutate(|_, r| assert_eq!(r.backend.copied, count));
        }
    }
    #[test]
    fn nested_managed_native_and_exception_reply() {
        let mut vm = Vm::new();
        vm.spawn(|mut fx| {
            Box::pin(async move {
                let result = fx
                    .call_managed(Call {
                        propagation: Propagation::Capture,
                        method: 1,
                        args: vec![],
                        byrefs: vec![],
                    })
                    .await?;
                assert_eq!(result.int(), 42);
                let thrown = fx
                    .call_managed(Call {
                        propagation: Propagation::Capture,
                        method: 2,
                        args: vec![],
                        byrefs: vec![],
                    })
                    .await;
                assert!(matches!(thrown, Err(Failure::Managed("callee failure"))));
                Ok(result)
            })
        });
        vm.run();
        assert_eq!(vm.result_int(), Ok(42));
        assert_eq!(vm.collections, 1);
    }
    #[test]
    fn reflection_boxing_and_byref_mutations_including_throw() {
        for generic in [false, true] {
            for method in [3, 5] {
                let mut vm = Vm::new();
                vm.spawn(move |mut fx| {
                    Box::pin(async move {
                        let parameters = fx.enter(|c| c.array(vec![Value::Int(1)]));
                        let spec = InvokeSpec {
                            method,
                            receiver: Stashed::Null,
                            parameters: parameters.clone(),
                            byref_indices: vec![0],
                            reference_byref_indices: vec![],
                            return_kind: ReturnKind::Value,
                        };
                        // Same continuation owns roots for a direct async subroutine call.
                        let result = if generic {
                            gat::reflection_invoke(fx, spec).await
                        } else {
                            chosen::reflection_invoke(fx, spec).await
                        };
                        // The result is observed by the outer native boundary below, whose region
                        // verifies boxing. Byref owner stays on the managed stack during the call.
                        result
                    })
                });
                vm.run();
                vm.arena.mutate(|_, root| {
                    if method == 5 {
                        assert!(matches!(
                            root.result,
                            Some(Err(Failure::Managed("callee failure")))
                        ));
                    } else {
                        let Some(Ok(Value::Object(o))) = root.result else {
                            panic!("boxed return")
                        };
                        assert!(matches!(*o.borrow(), ObjectData::Boxed(7)));
                    }
                });
            }
        }
    }
    #[test]
    fn marshaled_owner_survives_gc_and_updates_on_exception() {
        let mut vm = Vm::new();
        vm.spawn(|mut fx| {
            Box::pin(async move {
                let owner = fx.enter(|c| c.array(vec![Value::Object(new_box(c, 1))]));
                fx.safe_point().await?;
                let result = fx
                    .call_managed(Call {
                        propagation: Propagation::Capture,
                        method: 5,
                        args: vec![],
                        byrefs: vec![ByRefArg {
                            owner: owner.0.clone(),
                            index: 0,
                            kind: ByRefKind::BoxedValue,
                        }],
                    })
                    .await;
                assert!(result.is_err());
                fx.safe_point().await?;
                let value = fx.enter(|c| {
                    let object = owner.get(c);
                    let data = object.borrow();
                    let ObjectData::Array(values) = &*data else {
                        panic!()
                    };
                    let cell = values[0].object();
                    let ObjectData::Boxed(n) = *cell.borrow() else {
                        panic!()
                    };
                    n
                });
                Ok(Stashed::Int(value))
            })
        });
        vm.run();
        assert_eq!(vm.result_int(), Ok(99));
    }
    #[test]
    fn multicast_stops_at_first_exception() {
        for generic in [false, true] {
            let mut vm = Vm::new();
            vm.spawn(move |fx| {
                if generic {
                    Box::pin(gat::multicast(fx, vec![1, 2, 0], vec![]))
                } else {
                    Box::pin(chosen::multicast(fx, vec![1, 2, 0], vec![]))
                }
            });
            vm.run();
            assert_eq!(vm.result_int(), Err(Failure::Managed("callee failure")));
            vm.arena.mutate(|_, root| assert_eq!(root.effects, 3)); // two calls + nested safe point
        }
    }
    #[test]
    fn reflection_preserves_box_identity_and_reference_slot_assignment() {
        for method in [3, 5] {
            let mut vm = Vm::new();
            vm.spawn(move |mut fx| {
                Box::pin(async move {
                    let (parameters, original) = fx.enter(|c| {
                        let original = new_box(c, 1);
                        (
                            c.array(vec![Value::Object(original), Value::Null]),
                            c.stash(Value::Object(original)),
                        )
                    });
                    let spec = InvokeSpec {
                        method,
                        receiver: Stashed::Null,
                        parameters: parameters.clone(),
                        byref_indices: vec![0],
                        reference_byref_indices: vec![1],
                        return_kind: ReturnKind::Void,
                    };
                    let call = fx.enter(|c| effect_intrinsics::prepare_invoke(c, &spec))?;
                    let result = fx.call_managed(call).await;
                    assert_eq!(result.is_err(), method == 5);
                    fx.safe_point().await?;
                    fx.enter(|c| {
                        let array = parameters.get(c);
                        let data = array.borrow();
                        let ObjectData::Array(values) = &*data else {
                            panic!()
                        };
                        assert!(gc_arena::Gc::ptr_eq(
                            values[0].object(),
                            original.get(c).object()
                        ));
                        assert!(matches!(
                            *values[0].object().borrow(),
                            ObjectData::Boxed(99)
                        ));
                        let replacement = values[1].object();
                        let replacement = replacement.borrow();
                        let ObjectData::String(chars) = &*replacement else {
                            panic!()
                        };
                        assert_eq!(chars, &[90]);
                    });
                    Ok(Stashed::Void)
                })
            });
            vm.run();
        }
    }
    #[test]
    fn transparent_throw_never_resumes_rust_catch_code() {
        let mut vm = Vm::new();
        let resumed = Rc::new(Cell::new(false));
        let flag = resumed.clone();
        vm.spawn(move |mut fx| {
            Box::pin(async move {
                let result = fx
                    .call_managed(Call {
                        propagation: Propagation::Transparent,
                        method: 2,
                        args: vec![],
                        byrefs: vec![],
                    })
                    .await;
                flag.set(true);
                result
            })
        });
        vm.run();
        assert!(!resumed.get());
        assert_eq!(vm.result_int(), Err(Failure::Managed("callee failure")));
    }
    #[test]
    fn wait_effects_and_static_trigger_for_both_candidates() {
        for generic in [false, true] {
            let mut vm = Vm::new();
            vm.spawn(move |fx| {
                if generic {
                    Box::pin(gat::wait_finalizers(fx))
                } else {
                    Box::pin(chosen::wait_finalizers(fx))
                }
            });
            vm.run();
            vm.spawn(move |mut fx| {
                Box::pin(async move {
                    let object = fx.enter(|c| c.string(vec![65]).0);
                    if generic {
                        gat::monitor_enter(fx, object).await
                    } else {
                        chosen::monitor_enter(fx, object).await
                    }
                })
            });
            vm.run();
            vm.spawn(move |fx| {
                if generic {
                    Box::pin(gat::ensure_static(fx, 9))
                } else {
                    Box::pin(chosen::ensure_static(fx, 9))
                }
            });
            vm.run();
            vm.arena
                .mutate(|_, root| assert_eq!(root.backend.statics[&9], 2));
        }
    }
    #[test]
    fn root_transfer_before_parent_resumes_and_source_drops() {
        let mut vm = Vm::new();
        vm.spawn(|mut fx| {
            Box::pin(async move {
                let string = fx.enter(|c| c.string(vec![88]).0);
                let returned = fx
                    .call_managed(Call {
                        propagation: Propagation::Capture,
                        method: 0,
                        args: vec![string],
                        byrefs: vec![],
                    })
                    .await?;
                fx.safe_point().await?;
                Ok(returned)
            })
        });
        vm.run();
        vm.full_gc();
        assert_eq!(vm.result_string(), vec![88]);
    }
    #[test]
    fn abort_and_budget_release_future_and_all_local_roots() {
        struct Watch(Rc<Cell<bool>>);
        impl Drop for Watch {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }
        for reason in [Failure::Aborted, Failure::Budget] {
            let mut vm = Vm::new();
            let before = vm.arena.metrics().total_gc_allocation();
            let dropped = Rc::new(Cell::new(false));
            let watch = Watch(dropped.clone());
            vm.spawn(move |mut fx| {
                Box::pin(async move {
                    let _watch = watch;
                    let root = fx.enter(|c| c.string(vec![65]).0);
                    fx.safe_point().await?;
                    Ok(root)
                })
            });
            assert!(vm.step());
            assert!(!dropped.get());
            vm.cancel(reason);
            assert!(dropped.get());
            vm.full_gc();
            assert_eq!(vm.arena.metrics().total_gc_allocation(), before);
        }
    }
    #[test]
    fn inline_first_poll_and_dyn_trampoline() {
        #[allow(clippy::redundant_closure)]
        // Function item fails HRTB inference on stable 1.95 (E0308).
        let arena = Arena::<Rootable![gc_arena::DynamicRootSet<'_>]>::new(|mc| {
            gc_arena::DynamicRootSet::new(mc)
        });
        arena.mutate(|mc, _| {
            let mut backend = Backend::default();
            let mut frame = NativeFrame::new(mc, |mut fx| {
                Box::pin(async move {
                    let count = enter_erased(&mut fx, |c| {
                        let _scope = c.scope();
                        42
                    });
                    Ok(Stashed::Int(count))
                })
            });
            assert!(matches!(
                frame.poll(mc, &mut backend),
                Poll::Ready(Ok(Stashed::Int(42)))
            ));
        });
    }
    #[test]
    fn region_panic_restores_slot() {
        let mut vm = Vm::new();
        vm.spawn(|mut fx| {
            Box::pin(async move {
                // A nested region is possible through an escaped handle in real misuse; the
                // slot's null state rejects it. Panic recovery here verifies slot restoration.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    fx.enter(|_| panic!("region panic"))
                }));
                assert!(result.is_err());
                assert_eq!(fx.enter(|_| 1), 1);
                fx.safe_point().await?;
                Ok(Stashed::Int(2))
            })
        });
        vm.run();
        assert_eq!(vm.result_int(), Ok(2));
    }
    #[test]
    #[should_panic(expected = "Pending without effect")]
    fn external_pending_is_rejected() {
        let mut vm = Vm::new();
        vm.spawn(|_| {
            Box::pin(async {
                std::future::pending::<()>().await;
                Ok(Stashed::Void)
            })
        });
        vm.run();
    }
    #[test]
    #[should_panic(expected = "borrow leaked from region")]
    fn forgotten_real_scope_is_detected() {
        let mut vm = Vm::new();
        vm.spawn(|mut fx| {
            Box::pin(async move {
                fx.enter(|c| std::mem::forget(c.scope()));
                fx.safe_point().await?;
                Ok(Stashed::Void)
            })
        });
        vm.run();
    }
}

#[cfg(test)]
mod cost {
    use super::*;
    use std::{
        hint::black_box,
        time::{Duration, Instant},
    };
    #[test]
    #[ignore]
    fn native_overhead() {
        let arena = Arena::<Rootable![gc_arena::DynamicRootSet<'_>]>::new(|mc| {
            gc_arena::DynamicRootSet::new(mc)
        });
        let n = 2_000_000;
        let start = Instant::now();
        for _ in 0..n {
            black_box(false);
        }
        println!(
            "baseline {:.3} ns/op",
            start.elapsed().as_nanos() as f64 / n as f64
        );
        arena.mutate(|mc, _| {
            let mut backend = Backend::default();
            let mut frame = NativeFrame::new(mc, |mut fx| {
                Box::pin(async move {
                    let start = Instant::now();
                    for _ in 0..n {
                        fx.enter(|c| black_box(c.requested()));
                    }
                    println!(
                        "region {:.3} ns/op",
                        start.elapsed().as_nanos() as f64 / n as f64
                    );
                    Ok(Stashed::Void)
                })
            });
            assert!(frame.poll(mc, &mut backend).is_ready());
        });
        for suspend in [false, true] {
            let mut vm = Vm::new();
            let mut elapsed = Duration::ZERO;
            let batches = 200;
            let batch_size = 512;
            for _ in 0..batches {
                let start = Instant::now();
                vm.arena.mutate_root(|mc, root| {
                    for _ in 0..batch_size {
                        let mut frame = NativeFrame::new(mc, |mut fx| {
                            Box::pin(async move {
                                if suspend {
                                    fx.safe_point().await?;
                                }
                                Ok(Stashed::Int(black_box(42)))
                            })
                        });
                        if frame.poll(mc, &mut root.backend).is_pending() {
                            black_box(frame.take_effect());
                            frame.reply(Ok(Stashed::Void));
                            assert!(frame.poll(mc, &mut root.backend).is_ready());
                        }
                    }
                });
                elapsed += start.elapsed();
                vm.full_gc(); // outside the measured spawn/poll window
            }
            println!(
                "spawn suspend={suspend} {:.3} ns/op (GC excluded)",
                elapsed.as_nanos() as f64 / (batches * batch_size) as f64
            );
        }
    }
}

#[cfg(test)]
mod region_api_tests;

#[cfg(test)]
mod raii_cost {
    use super::*;
    use std::{hint::black_box, time::Instant};

    #[test]
    #[ignore]
    fn chosen_raii_access_cost() {
        let arena = Arena::<Rootable![Root<'_>]>::new(|_| Root::new());
        arena.mutate(|mc, _| {
            let mut backend = Backend::default();
            let mut frame = NativeFrame::new(mc, |mut fx| {
                Box::pin(async move {
                    let iterations = 2_000_000;
                    let start = Instant::now();
                    for _ in 0..iterations {
                        let mut vm = fx.vm().await?;
                        black_box(vm.stop_requested());
                    }
                    println!(
                        "chosen RAII acquisition + scalar read {:.3} ns/op",
                        start.elapsed().as_nanos() as f64 / iterations as f64
                    );
                    let source = {
                        let mut vm = fx.vm().await?;
                        StringRoot(vm.new_string(vec![65; 32]))
                    };
                    let start = Instant::now();
                    for _ in 0..iterations {
                        let mut vm = fx.vm().await?;
                        black_box(vm.string(&source).substring_len(0, None)?);
                    }
                    println!(
                        "chosen RAII acquisition + rooted string read {:.3} ns/op",
                        start.elapsed().as_nanos() as f64 / iterations as f64
                    );
                    Ok(Stashed::Void)
                })
            });
            loop {
                if frame.poll(mc, &mut backend).is_ready() {
                    break;
                }
                assert!(matches!(frame.take_effect(), Effect::SafePoint));
                frame.reply(Ok(Stashed::Void)); // handler round trip; GC excluded here
            }
        });
    }
}
