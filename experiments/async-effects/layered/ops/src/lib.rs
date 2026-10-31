//! Ops-owned region facade. Intrinsics never depend on the VM crate.
//!
//! The region is an ST-style brand boundary; await performs an effect handled by the VM.
//! These examples must fail because a branded value/borrow cannot leave `enter`.
//!
//! ```compile_fail
//! use effect_ops::*;
//! async fn bad(mut fx: VmEffects, root: Stashed) {
//!     let value = fx.enter(|c| root.get(c));
//!     fx.safe_point().await.unwrap();
//!     drop(value);
//! }
//! ```
//! ```compile_fail
//! use effect_ops::*;
//! fn bad(fx: &mut VmEffects, root: Stashed) {
//!     let guard = fx.enter(|c| root.get(c).object().borrow());
//!     drop(guard);
//! }
//! ```
//! ```compile_fail
//! use effect_ops::*;
//! fn bad(fx: &mut VmEffects) {
//!     let guard = fx.enter(|c| c.scope());
//!     drop(guard);
//! }
//! ```
//! ```compile_fail
//! use effect_ops::*;
//! fn bad(fx: &mut VmEffects, root: Stashed) {
//!     let ptr = fx.enter(|c| ByRef { owner: root.get(c).object(), offset: 0 });
//!     drop(ptr);
//! }
//! ```
//! ```compile_fail
//! use effect_ops::*;
//! fn bad(fx: &mut VmEffects) {
//!     let mutation = fx.enter(|c| c.mc);
//!     drop(mutation);
//! }
//! ```
//! ```compile_fail
//! use effect_ops::*;
//! fn bad(fx: &mut VmEffects, root: Stashed) {
//!     let guard = fx.enter(|c| root.get(c).object().borrow_mut(c.mc));
//!     drop(guard);
//! }
//! ```
//! ```compile_fail
//! use effect_ops::*;
//! fn bad(fx: &mut VmEffects) {
//!     let ctx = fx.enter(|c| c);
//!     drop(ctx);
//! }
//! ```

use gc_arena::{Collect, DynamicRoot, DynamicRootSet, Gc, Mutation, Rootable};
/// A static native future cannot capture a branded input even if it is created in mutate.
/// ```compile_fail
/// use effect_ops::*;
/// fn bad<'gc>(mc: &gc_arena::Mutation<'gc>, value: Value<'gc>) {
///     NativeFrame::new(mc, |mut fx| Box::pin(async move {
///         fx.safe_point().await?;
///         drop(value);
///         Ok(Stashed::Void)
///     }));
/// }
/// ```
pub const STATIC_FUTURE_CONFINEMENT: () = ();

use std::{
    cell::{Cell, RefCell},
    future::Future,
    pin::Pin,
    ptr,
    rc::Rc,
    task::{Context, Poll, Waker},
};

pub use dotnet_utils::gc::ThreadSafeLock as ObjectLock;
use dotnet_utils::{BorrowScopeOps, GcScopeGuard};
pub type Object<'gc> = Gc<'gc, ObjectLock<ObjectData<'gc>>>;
#[derive(Collect, Clone, Debug)]
#[collect(no_drop)]
pub enum ObjectData<'gc> {
    String(Vec<u16>),
    Array(Vec<Value<'gc>>),
    Boxed(i64),
}
#[derive(Collect, Clone, Copy, Debug)]
#[collect(no_drop)]
pub enum Value<'gc> {
    Void,
    Int(i64),
    Null,
    Object(Object<'gc>),
}
impl<'gc> Value<'gc> {
    pub fn object(self) -> Object<'gc> {
        match self {
            Self::Object(o) => o,
            _ => panic!("expected object"),
        }
    }
}
/// Always allocated in this continuation's arena; its payload is traced normally.
/// Real VM payload uses ObjectRef::trace, including its foreign-arena branch.
#[derive(Collect)]
#[collect(no_drop)]
pub struct RootCell<'gc> {
    value: Value<'gc>,
}
#[derive(Clone)]
pub enum Stashed {
    Void,
    Int(i64),
    Null,
    Object(DynamicRoot<Rootable![RootCell<'_>]>),
}
impl Stashed {
    pub fn get<'gc>(&self, c: &impl RegionAccess<'gc>) -> Value<'gc> {
        match self {
            Self::Void => Value::Void,
            Self::Null => Value::Null,
            Self::Int(n) => Value::Int(*n),
            Self::Object(h) => c.roots().fetch(h).value,
        }
    }
    pub fn int(&self) -> i64 {
        match self {
            Self::Int(n) => *n,
            _ => panic!("expected integer"),
        }
    }
}
#[derive(Clone)]
pub struct StringRoot(pub Stashed);
#[derive(Clone)]
pub struct ArrayRoot(pub Stashed);
impl StringRoot {
    pub fn get<'gc>(&self, c: &impl RegionAccess<'gc>) -> Object<'gc> {
        self.0.get(c).object()
    }
}
impl ArrayRoot {
    pub fn get<'gc>(&self, c: &impl RegionAccess<'gc>) -> Object<'gc> {
        self.0.get(c).object()
    }
}
/// Deliberately branded, and deliberately no Stash implementation for ByRef.
pub struct ByRef<'gc> {
    pub owner: Object<'gc>,
    pub offset: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Managed(&'static str),
    Host(&'static str),
    Aborted,
    Budget,
}
gc_arena::static_collect!(Failure);
pub type Outcome = Result<Stashed, Failure>;
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Propagation {
    Capture,
    Transparent,
}
#[derive(Clone)]
pub struct Call {
    pub propagation: Propagation,
    pub method: usize,
    pub args: Vec<Stashed>,
    pub byrefs: Vec<ByRefArg>,
}
/// Symbolic marshaled heap slot, not a saved address or a general CLI byref.
#[derive(Clone, Copy)]
pub enum ByRefKind {
    BoxedValue,
    ObjectSlot,
}
gc_arena::static_collect!(ByRefKind);
#[derive(Clone)]
pub struct ByRefArg {
    pub owner: Stashed,
    pub index: usize,
    pub kind: ByRefKind,
}
#[derive(Clone)]
pub enum Effect {
    SafePoint,
    CallManaged(Call),
    WaitFinalizers,
    MonitorAcquire {
        object: Stashed,
        deadline: Option<u64>,
    },
    WaitStaticInit {
        ty: usize,
    },
}
/// Backend's object-safe operations; real operations forward to the existing trait bundles.
/// GCHandle is built by the VM, once per poll, never by an intrinsic.
pub trait HostOps<'gc> {
    fn safe_point_requested(&self) -> bool;
    fn scopes(&self) -> &Cell<usize>;
    fn copied(&mut self, count: usize);
    fn static_action(&mut self, ty: usize) -> StaticAction;
}
#[derive(Clone, Copy)]
pub enum StaticAction {
    Ready,
    Run(usize),
    Wait,
    Failed,
}
pub trait RegionAccess<'gc> {
    fn roots(&self) -> &DynamicRootSet<'gc>;
    fn mutation(&self) -> &Mutation<'gc>;
    fn host(&mut self) -> &mut dyn HostOps<'gc>;
    fn requested(&self) -> bool;
    fn stash(&self, v: Value<'gc>) -> Stashed {
        match v {
            Value::Void => Stashed::Void,
            Value::Null => Stashed::Null,
            Value::Int(n) => Stashed::Int(n),
            Value::Object(_) => Stashed::Object(self.roots().stash(
                self.mutation(),
                Gc::new(self.mutation(), RootCell { value: v }),
            )),
        }
    }
    fn string(&self, chars: Vec<u16>) -> StringRoot {
        StringRoot(self.stash(Value::Object(Gc::new(
            self.mutation(),
            ObjectLock::new(ObjectData::String(chars)),
        ))))
    }
    fn array(&self, values: Vec<Value<'gc>>) -> ArrayRoot {
        ArrayRoot(self.stash(Value::Object(Gc::new(
            self.mutation(),
            ObjectLock::new(ObjectData::Array(values)),
        ))))
    }
}
/// Concrete public facade, private backend supplied by the VM.
pub struct Region<'gc, 'a> {
    pub mc: &'a Mutation<'gc>,
    roots: DynamicRootSet<'gc>,
    backend: &'a mut dyn HostOps<'gc>,
}
impl<'gc, 'a> Region<'gc, 'a> {
    pub fn new(
        mc: &'a Mutation<'gc>,
        roots: DynamicRootSet<'gc>,
        backend: &'a mut dyn HostOps<'gc>,
    ) -> Self {
        Self { mc, roots, backend }
    }
    pub fn scope(&self) -> GcScopeGuard<'_> {
        GcScopeGuard::enter(self, self.gc_ready_token())
    }
}
impl BorrowScopeOps for Region<'_, '_> {
    fn enter_gc_scope(&self) {
        let n = self.backend.scopes();
        n.set(n.get() + 1);
    }
    fn exit_gc_scope(&self) {
        let n = self.backend.scopes();
        n.set(n.get() - 1);
    }
    fn active_gc_scope_depth(&self) -> usize {
        self.backend.scopes().get()
    }
}
impl<'gc> RegionAccess<'gc> for Region<'gc, '_> {
    fn roots(&self) -> &DynamicRootSet<'gc> {
        &self.roots
    }
    fn mutation(&self) -> &Mutation<'gc> {
        assert_eq!(
            self.active_gc_scope_depth(),
            0,
            "allocation in borrowed region"
        );
        self.mc
    }
    fn host(&mut self) -> &mut dyn HostOps<'gc> {
        self.backend
    }
    fn requested(&self) -> bool {
        self.backend.safe_point_requested()
    }
}

#[derive(Default)]
struct Slot(Cell<*mut Region<'static, 'static>>);
struct Reset<'a>(&'a Slot);
impl Drop for Reset<'_> {
    fn drop(&mut self) {
        self.0.0.set(ptr::null_mut());
    }
}
impl Slot {
    fn install<R>(&self, c: &mut Region<'_, '_>, f: impl FnOnce() -> R) -> R {
        assert!(self.0.get().is_null(), "VM effects already polling");
        // SAFETY: F6.NoEscapeAcrossArena — lifetime erasure changes no representation.
        // Only enter dereferences this pointer, under an HRTB; Reset clears it on unwind.
        // c stays exclusively borrowed and live until f returns. No collection during poll.
        let erased =
            unsafe { std::mem::transmute::<*mut Region<'_, '_>, *mut Region<'static, 'static>>(c) };
        self.0.set(erased);
        let _reset = Reset(self);
        f()
    }
    fn enter<R>(&self, f: impl for<'gc> FnOnce(&mut Region<'gc, 'gc>) -> R) -> R {
        // Take it out before calling user code: recursive entry through another handle panics.
        let p = self.0.replace(ptr::null_mut());
        assert!(!p.is_null(), "region outside poll or nested region");
        struct Restore<'a>(&'a Slot, *mut Region<'static, 'static>);
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                self.0.0.set(self.1);
            }
        }
        let _restore = Restore(self, p);
        // SAFETY: F6.NoEscapeAcrossArena, F1.GcHandleRooted — install owns the unique
        // live Region for this poll; slot removal excludes aliasing/re-entry. f must work
        // for every brand and cannot return one. No reference survives this invocation.
        // We do not read through a reference with a fabricated static lifetime.
        let p =
            unsafe { std::mem::transmute::<*mut Region<'static, 'static>, *mut Region<'_, '_>>(p) };
        // SAFETY: F8.NoSafepointWhileHeapBorrowed — the unique pointer above is live
        // only in install; all region borrows finish before the future can suspend.
        f(unsafe { &mut *p })
    }
}
const _: () =
    assert!(size_of::<*mut Region<'static, 'static>>() == size_of::<*mut Region<'_, '_>>());

/// Maximum ready access acquisitions before returning to the native handler.
pub const VM_ACCESS_QUANTUM: usize = 32;

#[derive(Default)]
struct Shared {
    access_remaining: Cell<usize>,
    slot: Slot,
    effect: RefCell<Option<Effect>>,
    reply: RefCell<Option<Outcome>>,
}
/// Capability for scoped heap access, safe points, managed calls and waits.
/// Thread-affine (!Send); the stored computation is a native continuation.
pub struct VmEffects(Rc<Shared>);
impl VmEffects {
    /// Internal region primitive. Direct access remains for historical gallery comparisons.
    /// Intrinsic bodies use `vm().await?`; helper adapters use `VmAccess::with`.
    #[doc(hidden)]
    pub fn enter<R>(&mut self, f: impl for<'gc> FnOnce(&mut Region<'gc, 'gc>) -> R) -> R {
        self.0.slot.enter(f)
    }
    pub async fn perform(&mut self, effect: Effect) -> Outcome {
        let mut first = Some(effect);
        std::future::poll_fn(|_| {
            if let Some(effect) = first.take() {
                assert!(
                    self.0.effect.borrow_mut().replace(effect).is_none(),
                    "unconsumed effect"
                );
                Poll::Pending
            } else {
                Poll::Ready(
                    self.0
                        .reply
                        .borrow_mut()
                        .take()
                        .expect("resume without reply"),
                )
            }
        })
        .await
    }
    pub async fn safe_point(&mut self) -> Outcome {
        self.perform(Effect::SafePoint).await
    }
    pub async fn call_managed(&mut self, call: Call) -> Outcome {
        self.perform(Effect::CallManaged(call)).await
    }
    pub async fn wait_finalizers(&mut self) -> Outcome {
        self.perform(Effect::WaitFinalizers).await
    }
    pub async fn monitor_acquire(&mut self, object: Stashed, deadline: Option<u64>) -> Outcome {
        self.perform(Effect::MonitorAcquire { object, deadline })
            .await
    }
    pub async fn wait_static_init(&mut self, ty: usize) -> Outcome {
        self.perform(Effect::WaitStaticInit { ty }).await
    }
}
/// Leading alternative: generic host with a GAT. All six bodies compile without annotations.
/// It is intentionally not dyn-compatible; use VmEffects for erased registration.
pub trait NativeHost: 'static {
    type Ctx<'gc>: RegionAccess<'gc>;
    fn enter<R>(&mut self, f: impl for<'gc> FnOnce(&mut Self::Ctx<'gc>) -> R) -> R;
    fn perform(&mut self, effect: Effect) -> impl Future<Output = Outcome>;
}
impl NativeHost for VmEffects {
    type Ctx<'gc> = Region<'gc, 'gc>;
    fn enter<R>(&mut self, f: impl for<'gc> FnOnce(&mut Self::Ctx<'gc>) -> R) -> R {
        self.enter(f)
    }
    async fn perform(&mut self, effect: Effect) -> Outcome {
        self.perform(effect).await
    }
}

pub type NativeFuture = Pin<Box<dyn Future<Output = Outcome>>>;
/// Root-owned wrapper: future is static and roots are traced, as in piccolo.
#[derive(Collect)]
#[collect(no_drop)]
pub struct NativeFrame<'gc> {
    pub roots: DynamicRootSet<'gc>,
    #[collect(require_static)]
    shared: Rc<Shared>,
    #[collect(require_static)]
    future: NativeFuture,
}
impl<'gc> NativeFrame<'gc> {
    pub fn new(mc: &Mutation<'gc>, create: impl FnOnce(VmEffects) -> NativeFuture) -> Self {
        let shared = Rc::new(Shared::default());
        Self {
            roots: DynamicRootSet::new(mc),
            future: create(VmEffects(shared.clone())),
            shared,
        }
    }
    /// Arguments are stashed into this frame before constructing the static future.
    pub fn with_args(
        mc: &Mutation<'gc>,
        args: Vec<Value<'gc>>,
        backend: &mut dyn HostOps<'gc>,
        create: impl FnOnce(VmEffects, Vec<Stashed>) -> NativeFuture,
    ) -> Self {
        let shared = Rc::new(Shared::default());
        let roots = DynamicRootSet::new(mc);
        let c = Region::new(mc, roots, backend);
        let args = args.into_iter().map(|v| c.stash(v)).collect();
        Self {
            roots,
            future: create(VmEffects(shared.clone()), args),
            shared,
        }
    }
    pub fn poll(&mut self, mc: &Mutation<'gc>, backend: &mut dyn HostOps<'gc>) -> Poll<Outcome> {
        assert_eq!(backend.scopes().get(), 0, "borrow active before poll");
        self.shared.access_remaining.set(VM_ACCESS_QUANTUM);
        let mut region = Region::new(mc, self.roots, backend);
        let result = self.shared.slot.install(&mut region, || {
            self.future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
        });
        assert_eq!(
            region.active_gc_scope_depth(),
            0,
            "borrow leaked from region"
        );
        assert_eq!(
            result.is_pending(),
            self.shared.effect.borrow().is_some(),
            "Pending without effect or Ready with effect"
        );
        result
    }
    pub fn take_effect(&self) -> Effect {
        self.shared
            .effect
            .borrow_mut()
            .take()
            .expect("missing effect")
    }
    pub fn reply(&self, result: Outcome) {
        assert!(
            self.shared.reply.borrow_mut().replace(result).is_none(),
            "duplicate reply"
        );
    }
    pub fn region<R>(
        &self,
        mc: &Mutation<'gc>,
        backend: &mut dyn HostOps<'gc>,
        f: impl FnOnce(&mut Region<'gc, '_>) -> R,
    ) -> R {
        f(&mut Region::new(mc, self.roots, backend))
    }
}

pub fn new_box<'gc>(c: &impl RegionAccess<'gc>, n: i64) -> Object<'gc> {
    Gc::new(c.mutation(), ObjectLock::new(ObjectData::Boxed(n)))
}

/// Object-safe trampoline trial. The generic return value is reconstructed by the caller.
/// VmEffects remains the chosen facade because this adapter adds a second callback layer.
pub trait DynRegionHost {
    fn enter_dyn(&mut self, visitor: &mut dyn for<'gc> FnMut(&mut Region<'gc, 'gc>));
}
impl DynRegionHost for VmEffects {
    fn enter_dyn(&mut self, visitor: &mut dyn for<'gc> FnMut(&mut Region<'gc, 'gc>)) {
        self.enter(|c| visitor(c));
    }
}
pub fn enter_erased<R>(
    host: &mut dyn DynRegionHost,
    f: impl for<'gc> FnOnce(&mut Region<'gc, 'gc>) -> R,
) -> R {
    let mut f = Some(f);
    let mut result = None;
    host.enter_dyn(&mut |c| result = Some(f.take().expect("visitor called twice")(c)));
    result.expect("visitor not called")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Backend(Cell<usize>);
    impl HostOps<'_> for Backend {
        fn safe_point_requested(&self) -> bool {
            false
        }
        fn scopes(&self) -> &Cell<usize> {
            &self.0
        }
        fn copied(&mut self, _: usize) {}
        fn static_action(&mut self, _: usize) -> StaticAction {
            StaticAction::Ready
        }
    }
    #[test]
    fn slot_excludes_recursive_entry_and_clears_on_unwind() {
        #[allow(clippy::redundant_closure)] // Function item fails the Arena HRTB on stable 1.95.
        let arena = ArenaForTest::new(|mc| DynamicRootSet::new(mc));
        arena.mutate(|mc, roots| {
            let slot = Slot::default();
            let mut backend = Backend::default();
            let mut c = Region::new(mc, *roots, &mut backend);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                slot.install(&mut c, || slot.enter(|_| slot.enter(|_| ())));
            }));
            assert!(result.is_err());
            assert!(slot.0.get().is_null());
            slot.install(&mut c, || assert_eq!(slot.enter(|_| 42), 42));
            assert!(slot.0.get().is_null());
        });
    }
    type ArenaForTest = gc_arena::Arena<Rootable![DynamicRootSet<'_>]>;
    #[test]
    #[should_panic(expected = "region outside poll")]
    fn escaped_effects_capability_cannot_enter() {
        VmEffects(Rc::new(Shared::default())).enter(|_| ());
    }
    #[test]
    #[should_panic(expected = "mismatched root set")]
    fn handles_cannot_be_fetched_in_another_continuation() {
        #[allow(clippy::redundant_closure)] // Function item fails the Arena HRTB on stable 1.95.
        let arena = ArenaForTest::new(|mc| DynamicRootSet::new(mc));
        arena.mutate(|mc, roots| {
            let other = DynamicRootSet::new(mc);
            let mut backend = Backend::default();
            let original = Region::new(mc, *roots, &mut backend).string(vec![65]);
            original.get(&Region::new(mc, other, &mut backend));
        });
    }
}

mod region_api;

mod raii_vm;
pub use raii_vm::{VmAccess, VmString};
