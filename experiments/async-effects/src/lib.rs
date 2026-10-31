//! Prototype: VM effects (GC safe points, managed re-entry) expressed as `.await`
//! on **stable** Rust, over `gc-arena` 0.6 — the same GC foundation as dotnet-rs.
//!
//! The 2026-07-03 investigation found that a future holding `Gc<'gc, _>` across an
//! `.await` cannot work: `'gc` cannot escape `Arena::mutate`, and an `async fn`'s
//! state machine cannot `impl Collect`. Both walls hold. This prototype goes around
//! them instead of through them:
//!
//! 1. **Futures are `'static` and live outside the arena.** They never hold a `Gc`.
//! 2. **GC access is a scoped region**, [`Cx::with`], which is a thin wrapper over
//!    `Arena::mutate_root`. Its closure is `for<'gc>`, so nothing branded `'gc` can be
//!    returned from it — and so nothing branded `'gc` can be live across an `.await`.
//!    The borrow checker enforces that (see the `compile_fail` doctest on [`Cx::with`]).
//! 3. **GC references that must survive an `.await` are stashed** as `DynamicRoot`s
//!    (gc-arena's `'static`, refcounted, traced-from-the-root handles) — see [`Stashed`].
//! 4. **`.await` points are the effects.** A suspended task records *which* effect it
//!    wants in a side channel and returns `Pending`; the driver ([`Vm::run`]) is the
//!    effect handler: it runs a full GC for [`Effect::SafePoint`], or resumes the
//!    interpreter until a managed callee returns for [`Effect::AwaitManagedReturn`].
//!
//! The interpreter's hot dispatch loop ([`Engine::step`]) stays synchronous. Only
//! "slow-path" intrinsics are tasks, and tasks form a stack parallel to the frame
//! stack (a task can call managed code, which can start another task).

use std::{
    cell::RefCell,
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

use gc_arena::{
    Arena, Collect, DynamicRoot, DynamicRootSet, Gc, Mutation, Rootable, lock::RefLock,
};

pub type Arr<'gc> = Gc<'gc, RefLock<Vec<i64>>>;
pub type ArrRoot = DynamicRoot<Rootable![RefLock<Vec<i64>>]>;

/// An evaluation-stack value. Branded with `'gc`: cannot leave a `Cx::with` region.
#[derive(Collect, Clone, Copy)]
#[collect(no_drop)]
pub enum Value<'gc> {
    Int(i64),
    Arr(Arr<'gc>),
}

/// The `'static` mirror of [`Value`]: what a task is allowed to hold across `.await`.
#[derive(Clone)]
pub enum Stashed {
    Int(i64),
    Arr(ArrRoot),
}

impl Stashed {
    pub fn stash<'gc>(v: Value<'gc>, mc: &Mutation<'gc>, roots: &DynamicRootSet<'gc>) -> Self {
        match v {
            Value::Int(i) => Stashed::Int(i),
            Value::Arr(a) => Stashed::Arr(roots.stash::<Rootable![RefLock<Vec<i64>>]>(mc, a)),
        }
    }

    pub fn fetch<'gc>(&self, roots: &DynamicRootSet<'gc>) -> Value<'gc> {
        match self {
            Stashed::Int(i) => Value::Int(*i),
            Stashed::Arr(a) => Value::Arr(roots.fetch(a)),
        }
    }

    pub fn int(&self) -> i64 {
        match self {
            Stashed::Int(i) => *i,
            Stashed::Arr(_) => panic!("expected int"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Intrinsic {
    /// Sums an array in chunks, hitting a GC safe point between chunks.
    ChunkedSum,
    /// Insertion-sorts an array in place, calling a *managed* comparer per comparison.
    SortWith { comparer: usize },
    /// Negates an int after a safe point (used to nest a task inside a managed callback).
    SlowNegate,
}

#[derive(Clone, Debug)]
pub enum Op {
    PushInt(i64),
    NewArr(Vec<i64>),
    LdArg(usize),
    Sub,
    Ret,
    Async(Intrinsic),
    Print,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReturnTo {
    Caller,
    /// Deliver the return value to the innermost suspended task.
    Task,
}

#[derive(Collect)]
#[collect(require_static)]
struct Frame {
    method: usize,
    ip: usize,
    base: usize,
    ret: ReturnTo,
}

/// The arena root (stand-in for dotnet-rs's `ExecutionEngine<'gc>`).
#[derive(Collect)]
#[collect(no_drop)]
pub struct Engine<'gc> {
    pub roots: DynamicRootSet<'gc>,
    pub stack: Vec<Value<'gc>>,
    frames: Vec<Frame>,
    #[collect(require_static)]
    program: Rc<Vec<Vec<Op>>>,
    /// A managed callee's return value, waiting for its task to be re-polled. Traced.
    task_reply: Option<Value<'gc>>,
    #[collect(require_static)]
    pub output: Vec<String>,
}

pub enum Step {
    Continue,
    Spawn(Intrinsic, Vec<Stashed>),
    TaskReply,
    Done,
}

impl<'gc> Engine<'gc> {
    /// One instruction of the synchronous hot loop. Never polls a future.
    pub fn step(&mut self, mc: &Mutation<'gc>) -> Step {
        let f = self.frames.last_mut().expect("no frame");
        let op = self.program[f.method][f.ip].clone();
        f.ip += 1;
        let base = f.base;
        match op {
            Op::PushInt(i) => self.stack.push(Value::Int(i)),
            Op::NewArr(v) => self.stack.push(Value::Arr(Gc::new(mc, RefLock::new(v)))),
            Op::LdArg(i) => self.stack.push(self.stack[base + i]),
            Op::Sub => {
                let (Some(Value::Int(b)), Some(Value::Int(a))) =
                    (self.stack.pop(), self.stack.pop())
                else {
                    panic!("sub: type mismatch")
                };
                self.stack.push(Value::Int(a - b));
            }
            Op::Print => {
                let line = match self.stack.pop().expect("print: empty") {
                    Value::Int(i) => i.to_string(),
                    Value::Arr(a) => format!("{:?}", a.borrow()),
                };
                self.output.push(line);
            }
            Op::Async(i) => {
                let argc = match i {
                    Intrinsic::ChunkedSum | Intrinsic::SortWith { .. } | Intrinsic::SlowNegate => 1,
                };
                let at = self.stack.len() - argc;
                let args = self
                    .stack
                    .drain(at..)
                    .map(|v| Stashed::stash(v, mc, &self.roots))
                    .collect();
                return Step::Spawn(i, args);
            }
            Op::Ret => {
                let v = self.stack.pop().expect("ret: empty");
                let frame = self.frames.pop().unwrap();
                self.stack.truncate(frame.base);
                match frame.ret {
                    ReturnTo::Caller if self.frames.is_empty() => return Step::Done,
                    ReturnTo::Caller => self.stack.push(v),
                    ReturnTo::Task => {
                        self.task_reply = Some(v);
                        return Step::TaskReply;
                    }
                }
            }
        }
        Step::Continue
    }
}

/// What a suspended task is waiting on. The driver is the handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Let the collector run, then resume me.
    SafePoint,
    /// I pushed a managed frame; resume me when it returns (value in `task_reply`).
    AwaitManagedReturn,
}

type EngineArena = Arena<Rootable![Engine<'_>]>;

struct Shared {
    arena: RefCell<EngineArena>,
    effect: RefCell<Option<Effect>>,
}

/// A task's capability to touch the VM. Cheap to clone; `'static`.
#[derive(Clone)]
pub struct Cx(Rc<Shared>);

impl Cx {
    /// Enter a GC region. Nothing branded `'gc` can be returned, so nothing branded
    /// `'gc` can survive past the closure — in particular, not across an `.await`.
    ///
    /// This fails to compile, which is the point:
    ///
    /// ```compile_fail
    /// # use async_effects_proto::*;
    /// async fn bad(cx: Cx) {
    ///     let held = cx.with(|_, e| e.stack[0]); // Value<'gc> escaping the region
    ///     cx.safepoint().await;
    ///     let _ = held;
    /// }
    /// ```
    ///
    /// The sanctioned version stashes first:
    ///
    /// ```
    /// # use async_effects_proto::*;
    /// async fn good(cx: Cx) {
    ///     let held = cx.with(|mc, e| Stashed::stash(e.stack[0], mc, &e.roots));
    ///     cx.safepoint().await;
    ///     cx.with(|_, e| { let _v = held.fetch(&e.roots); });
    /// }
    /// ```
    pub fn with<T>(
        &self,
        f: impl for<'gc> FnOnce(&'gc Mutation<'gc>, &'gc mut Engine<'gc>) -> T,
    ) -> T {
        self.0
            .arena
            .try_borrow_mut()
            .expect("Cx::with re-entered (nested GC regions are not allowed)")
            .mutate_root(f)
    }

    /// Perform an effect: suspend this task until the driver has handled `effect`.
    fn perform(&self, effect: Effect) -> Perform {
        Perform {
            cx: self.clone(),
            effect: Some(effect),
        }
    }

    /// A GC safe point. Everything not stashed is out of reach here by construction.
    pub fn safepoint(&self) -> Perform {
        self.perform(Effect::SafePoint)
    }

    /// Call a managed method and await its return value — straight-line, from Rust.
    pub async fn call_managed(&self, method: usize, args: &[Stashed]) -> Stashed {
        self.with(|_, e| {
            let base = e.stack.len();
            for a in args {
                e.stack.push(a.fetch(&e.roots));
            }
            e.frames.push(Frame {
                method,
                ip: 0,
                base,
                ret: ReturnTo::Task,
            });
        });
        self.perform(Effect::AwaitManagedReturn).await;
        self.with(|mc, e| {
            let v = e.task_reply.take().expect("resumed without a reply");
            Stashed::stash(v, mc, &e.roots)
        })
    }
}

pub struct Perform {
    cx: Cx,
    effect: Option<Effect>,
}

impl Future for Perform {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        match self.effect.take() {
            Some(e) => {
                let prev = self.cx.0.effect.borrow_mut().replace(e);
                debug_assert!(prev.is_none());
                Poll::Pending
            }
            None => Poll::Ready(()),
        }
    }
}

// ---------------------------------------------------------------------------
// Async intrinsics. Note what is *absent*: no `back_up_ip`, no retry flag, no
// continuation enum variant, no re-deriving progress from scratch after a yield.
// ---------------------------------------------------------------------------

pub const CHUNK: usize = 1024;

async fn chunked_sum(cx: Cx, arr: ArrRoot) -> Stashed {
    let len = cx.with(|_, e| e.roots.fetch(&arr).borrow().len());
    let (mut off, mut acc) = (0, 0i64);
    while off < len {
        let end = (off + CHUNK).min(len);
        acc += cx.with(|_, e| e.roots.fetch(&arr).borrow()[off..end].iter().sum::<i64>());
        off = end;
        // `off` and `acc` survive the safe point. Today's string intrinsics instead
        // `return StepResult::Yield` and the instruction restarts from offset 0.
        cx.safepoint().await;
    }
    Stashed::Int(acc)
}

async fn sort_with(cx: Cx, arr: ArrRoot, comparer: usize) -> Stashed {
    let len = cx.with(|_, e| e.roots.fetch(&arr).borrow().len());
    for i in 1..len {
        let mut j = i;
        while j > 0 {
            let (a, b) = cx.with(|_, e| {
                let v = e.roots.fetch(&arr);
                let v = v.borrow();
                (v[j - 1], v[j])
            });
            let c = cx
                .call_managed(comparer, &[Stashed::Int(a), Stashed::Int(b)])
                .await;
            if c.int() <= 0 {
                break;
            }
            cx.with(|mc, e| e.roots.fetch(&arr).borrow_mut(mc).swap(j - 1, j));
            j -= 1;
        }
    }
    Stashed::Arr(arr)
}

async fn slow_negate(cx: Cx, v: i64) -> Stashed {
    cx.safepoint().await;
    Stashed::Int(-v)
}

type Task = Pin<Box<dyn Future<Output = Stashed>>>;

fn spawn(cx: Cx, i: Intrinsic, mut args: Vec<Stashed>) -> Task {
    match i {
        Intrinsic::ChunkedSum => {
            let Some(Stashed::Arr(a)) = args.pop() else {
                panic!("ChunkedSum: want array")
            };
            Box::pin(chunked_sum(cx, a))
        }
        Intrinsic::SortWith { comparer } => {
            let Some(Stashed::Arr(a)) = args.pop() else {
                panic!("SortWith: want array")
            };
            Box::pin(sort_with(cx, a, comparer))
        }
        Intrinsic::SlowNegate => Box::pin(slow_negate(cx, args[0].int())),
    }
}

// ---------------------------------------------------------------------------
// Driver = effect handler (stand-in for `Executor::run`).
// ---------------------------------------------------------------------------

#[derive(Default, Debug)]
pub struct Stats {
    pub instructions: u64,
    pub polls: u64,
    pub safepoints: u64,
    pub collections: u64,
}

pub struct Vm {
    cx: Cx,
    tasks: Vec<Task>,
    /// Run a full collection every N interpreter steps (1 = maximal GC stress).
    pub gc_every: u64,
    pub stats: Stats,
}

impl Vm {
    pub fn new(program: Vec<Vec<Op>>) -> Self {
        let arena = EngineArena::new(|mc| Engine {
            roots: DynamicRootSet::new(mc),
            stack: Vec::new(),
            frames: vec![Frame {
                method: 0,
                ip: 0,
                base: 0,
                ret: ReturnTo::Caller,
            }],
            program: Rc::new(program),
            task_reply: None,
            output: Vec::new(),
        });
        Vm {
            cx: Cx(Rc::new(Shared {
                arena: RefCell::new(arena),
                effect: RefCell::new(None),
            })),
            tasks: Vec::new(),
            gc_every: 1,
            stats: Stats::default(),
        }
    }

    fn full_gc(&mut self) {
        self.cx.0.arena.borrow_mut().finish_cycle();
        self.stats.collections += 1;
    }

    pub fn run(&mut self) -> Vec<String> {
        loop {
            // 1. Resume the innermost task, if any, and handle the effect it performed.
            if let Some(task) = self.tasks.last_mut() {
                self.stats.polls += 1;
                match task.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
                    Poll::Ready(v) => {
                        self.tasks.pop();
                        self.cx.with(|_, e| {
                            let v = v.fetch(&e.roots);
                            e.stack.push(v);
                        });
                        // `v` (and any DynamicRoot it holds) drops here: unrooted.
                    }
                    Poll::Pending => {
                        let effect = self.cx.0.effect.borrow_mut().take();
                        match effect.expect("task returned Pending without performing an effect") {
                            Effect::SafePoint => {
                                self.stats.safepoints += 1;
                                self.full_gc();
                                continue; // re-poll the same task
                            }
                            Effect::AwaitManagedReturn => {} // fall into the interpreter
                        }
                    }
                }
            }

            // 2. Run the synchronous interpreter until something needs a task.
            loop {
                self.stats.instructions += 1;
                if self.stats.instructions.is_multiple_of(self.gc_every) {
                    self.full_gc();
                }
                match self.cx.with(|mc, e| e.step(mc)) {
                    Step::Continue => {}
                    Step::Spawn(i, args) => {
                        self.tasks.push(spawn(self.cx.clone(), i, args));
                        break;
                    }
                    Step::TaskReply => break,
                    Step::Done => {
                        assert!(self.tasks.is_empty(), "program ended with suspended tasks");
                        return self.cx.with(|_, e| std::mem::take(&mut e.output));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CMP: usize = 1; // cmp(a, b) = a - b
    const CMP_NESTED: usize = 2; // cmp(a, b) = SlowNegate(b - a): a task inside a callback

    fn program(main: Vec<Op>) -> Vec<Vec<Op>> {
        vec![
            main,
            vec![Op::LdArg(0), Op::LdArg(1), Op::Sub, Op::Ret],
            vec![
                Op::LdArg(1),
                Op::LdArg(0),
                Op::Sub,
                Op::Async(Intrinsic::SlowNegate),
                Op::Ret,
            ],
        ]
    }

    #[test]
    fn chunked_sum_keeps_progress_across_full_collections() {
        let n = 10 * CHUNK as i64 + 7;
        let mut vm = Vm::new(program(vec![
            Op::NewArr((0..n).collect()),
            Op::Async(Intrinsic::ChunkedSum),
            Op::Print,
            Op::PushInt(0),
            Op::Ret,
        ]));
        assert_eq!(vm.run(), vec![(n * (n - 1) / 2).to_string()]);
        assert_eq!(
            vm.stats.safepoints, 11,
            "one safe point per chunk, no restarts"
        );
    }

    #[test]
    fn managed_callbacks_from_straight_line_rust() {
        let mut vm = Vm::new(program(vec![
            Op::NewArr(vec![5, 3, 9, 1, 7, 2]),
            Op::Async(Intrinsic::SortWith { comparer: CMP }),
            Op::Print,
            Op::NewArr(vec![4, -1, 8, 0]),
            Op::Async(Intrinsic::SortWith {
                comparer: CMP_NESTED,
            }),
            Op::Print,
            Op::PushInt(0),
            Op::Ret,
        ]));
        assert_eq!(vm.run(), vec!["[1, 2, 3, 5, 7, 9]", "[-1, 0, 4, 8]"]);
        assert!(vm.stats.collections > vm.stats.instructions / 2);
    }

    /// Observes collection directly: the sentinel's `Drop` runs when the GC frees it.
    struct Sentinel(Rc<std::cell::Cell<bool>>);
    unsafe impl<'gc> Collect<'gc> for Sentinel {
        const NEEDS_TRACE: bool = false;
    }
    impl Drop for Sentinel {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    #[test]
    fn stash_is_what_keeps_objects_alive() {
        let mut vm = Vm::new(program(vec![Op::PushInt(0), Op::Ret]));
        let freed = Rc::new(std::cell::Cell::new(false));
        let flag = freed.clone();
        let root = vm.cx.with(move |mc, e| {
            e.roots
                .stash::<Rootable![Sentinel]>(mc, Gc::new(mc, Sentinel(flag)))
        });
        vm.full_gc();
        vm.full_gc();
        assert!(!freed.get(), "stashed object must survive collection");
        drop(root);
        vm.full_gc();
        assert!(freed.get(), "unstashed object must be collected");
    }
}

#[cfg(test)]
mod cost {
    use super::*;
    use std::time::Instant;

    /// `cargo test --release --lib cost -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn per_op_overhead() {
        const N: u32 = 2_000_000;
        let vm = Vm::new(vec![vec![Op::PushInt(0), Op::Ret]]);
        let cx = vm.cx.clone();

        let t = Instant::now();
        for i in 0..N {
            cx.with(|_, e| e.stack.push(Value::Int(i as i64)));
            cx.with(|_, e| {
                e.stack.pop();
            });
        }
        let with_ns = t.elapsed().as_nanos() as f64 / (2 * N) as f64;

        let t = Instant::now();
        let waker = Waker::noop();
        for i in 0..N {
            let c = cx.clone();
            let mut task: Task = Box::pin(async move {
                c.with(|_, e| e.stack.push(Value::Int(i as i64)));
                c.perform(Effect::SafePoint).await;
                Stashed::Int(c.with(|_, e| match e.stack.pop() {
                    Some(Value::Int(v)) => v,
                    _ => 0,
                }))
            });
            let mut cxt = Context::from_waker(waker);
            assert!(task.as_mut().poll(&mut cxt).is_pending());
            cx.0.effect.borrow_mut().take();
            assert!(task.as_mut().poll(&mut cxt).is_ready());
        }
        let task_ns = t.elapsed().as_nanos() as f64 / N as f64;
        println!(
            "Cx::with round trip: {with_ns:.1} ns; spawn + 1 suspend + 2 polls + 2 with: {task_ns:.1} ns"
        );
    }
}
