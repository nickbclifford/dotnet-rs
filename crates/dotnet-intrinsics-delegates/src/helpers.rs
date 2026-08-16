//! Delegate invocation support for runtime-managed delegate methods.
//!
//! Delegates have methods (ctor, Invoke, BeginInvoke, EndInvoke) with no CIL body -
//! they are implemented by the runtime (ECMA-335 §II.14.6).
use crate::{
    BEGIN_END_NOT_SUPPORTED_MSG, DelegateDispatchKind, DelegateInvokeHost, invoke::invoke_delegate,
};
use dotnet_types::{
    TypeDescription, WellKnown, generics::GenericLookup, members::MethodDescription,
};
use dotnet_value::object::ObjectRef;
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::SupportSlotOps;
use dotnet_vm_ops::ops::{DelegateIntrinsicHost, LoaderOps, MemoryOps, ResolutionOps};

/// Check if a type is a delegate type (inherits from System.Delegate or
/// System.MulticastDelegate).
///
/// We intentionally use raw type-hierarchy metadata here (TypeDescription ancestry)
/// instead of `is_a` over `ConcreteType`: delegate `Invoke` methods are declared on
/// open generic delegate types like `System.Func`N, and assignability checks on those
/// open shapes can conservatively fail even though the type is unquestionably in the
/// delegate hierarchy.
pub fn is_delegate_type<T: LoaderOps>(ctx: &T, td: TypeDescription) -> bool {
    is_type_or_ancestor_named(ctx, td.clone(), |canonical| {
        canonical == "System.Delegate" || canonical == "System.MulticastDelegate"
    })
}

fn classify_delegate_dispatch<T: LoaderOps>(
    ctx: &T,
    method: &MethodDescription,
) -> DelegateDispatchKind {
    if !is_delegate_type(ctx, method.parent.clone()) {
        return DelegateDispatchKind::NotDelegate;
    }

    match &*method.method().name {
        "Invoke" => DelegateDispatchKind::Invoke,
        ".ctor" => DelegateDispatchKind::Constructor,
        "BeginInvoke" | "EndInvoke" => DelegateDispatchKind::BeginOrEndInvoke,
        _ => DelegateDispatchKind::OtherDelegateMethod,
    }
}

fn is_type_or_ancestor_named<T, F>(ctx: &T, td: TypeDescription, matches_name: F) -> bool
where
    T: LoaderOps,
    F: Fn(&str) -> bool,
{
    let candidate_matches = |candidate: &TypeDescription| {
        let raw_type_name = candidate.type_name();
        let canonical = ctx.loader().canonical_type_name(&raw_type_name);
        matches_name(canonical)
    };

    if candidate_matches(&td) {
        return true;
    }

    ctx.loader()
        .ancestors(td)
        .any(|(ancestor, _)| candidate_matches(&ancestor))
}

/// Try to dispatch a delegate runtime method. Returns Some(result) if handled.
pub fn try_delegate_dispatch<'gc, T: DelegateIntrinsicHost<'gc> + DelegateInvokeHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    lookup: &GenericLookup,
) -> Option<StepResult> {
    // Quick check: only handle methods without bodies
    if method.body().is_some() {
        return None;
    }

    let dispatch_kind = ctx.delegate_dispatch_kind(&method).unwrap_or_else(|| {
        let kind = classify_delegate_dispatch(ctx, &method);
        ctx.cache_delegate_dispatch_kind(method.clone(), kind);
        kind
    });

    match dispatch_kind {
        DelegateDispatchKind::Invoke => Some(invoke_delegate(ctx, method, lookup)),
        DelegateDispatchKind::Constructor => None, // Constructor is handled by support library stub
        DelegateDispatchKind::BeginOrEndInvoke => Some(ctx.throw_by_name_with_message(
            "System.NotSupportedException",
            BEGIN_END_NOT_SUPPORTED_MSG,
        )),
        DelegateDispatchKind::NotDelegate | DelegateDispatchKind::OtherDelegateMethod => None,
    }
}

#[derive(Copy, Clone)]
pub(super) struct DelegateIdentity<'gc> {
    pub(super) target: ObjectRef<'gc>,
    pub(super) method_index: usize,
}

pub(super) enum DelegateInvocationListIter<'gc> {
    Single(std::iter::Once<ObjectRef<'gc>>),
    Multicast(std::vec::IntoIter<ObjectRef<'gc>>),
}

impl<'gc> Iterator for DelegateInvocationListIter<'gc> {
    type Item = ObjectRef<'gc>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Single(iter) => iter.next(),
            Self::Multicast(iter) => iter.next(),
        }
    }
}

pub(super) struct DelegateView<'a, 'gc, T: LoaderOps> {
    ctx: &'a T,
    obj: ObjectRef<'gc>,
    delegate_type: TypeDescription,
}

fn delegate_type<T: LoaderOps>(ctx: &T) -> TypeDescription {
    ctx.loader()
        .corlib_wkt(WellKnown::Delegate)
        .expect("System.Delegate must exist")
}

impl<'a, 'gc, T: LoaderOps> DelegateView<'a, 'gc, T> {
    pub(super) fn new(ctx: &'a T, obj: ObjectRef<'gc>) -> Self {
        Self {
            ctx,
            obj,
            delegate_type: delegate_type(ctx),
        }
    }

    pub(super) fn target(&self) -> ObjectRef<'gc> {
        self.obj.as_object(|instance| {
            self.ctx
                .loader()
                .delegate_target_field(&instance.instance_storage, self.delegate_type.clone())
                .expect("validated System.Delegate target support slot")
                .read()
        })
    }

    pub(super) fn method_index(&self) -> usize {
        self.obj.as_object(|instance| {
            self.ctx
                .loader()
                .delegate_method_index_field(&instance.instance_storage, self.delegate_type.clone())
                .expect("validated System.Delegate method-index support slot")
                .read()
        })
    }

    pub(super) fn multicast_targets(&self) -> Option<ObjectRef<'gc>> {
        self.obj.as_object(|instance| {
            if !is_type_or_ancestor_named(self.ctx, instance.description.clone(), |canonical| {
                canonical == "System.MulticastDelegate"
            }) {
                return None;
            }

            let multicast_type = self
                .ctx
                .loader()
                .corlib_wkt(WellKnown::MulticastDelegate)
                .expect("System.MulticastDelegate must exist");
            let targets_ref = self
                .ctx
                .loader()
                .multicast_delegate_targets_field(&instance.instance_storage, multicast_type)
                .expect("validated System.MulticastDelegate targets support slot")
                .read();
            if targets_ref.0.is_some() {
                Some(targets_ref)
            } else {
                None
            }
        })
    }

    pub(super) fn invocation_list_iter(&self) -> DelegateInvocationListIter<'gc>
    where
        T: MemoryOps<'gc>,
    {
        if let Some(targets_ref) = self.multicast_targets() {
            let invocation_list = targets_ref.as_vector(|v| {
                let gc = self.ctx.gc_with_token(&self.ctx.no_active_borrows_token());
                let mut elements = v.object_ref_elements(&gc);
                let mut result = Vec::with_capacity(v.layout.length);
                for _ in 0..v.layout.length {
                    let entry = elements
                        .next()
                        .expect("multicast invocation list storage must match vector length");
                    result.push(entry);
                }
                result
            });
            DelegateInvocationListIter::Multicast(invocation_list.into_iter())
        } else {
            DelegateInvocationListIter::Single(std::iter::once(self.obj))
        }
    }

    pub(super) fn identity(&self) -> DelegateIdentity<'gc> {
        DelegateIdentity {
            target: self.target(),
            method_index: self.method_index(),
        }
    }
}

pub(super) struct DelegateViewMut<'a, 'gc, T: LoaderOps + MemoryOps<'gc>> {
    ctx: &'a T,
    obj: ObjectRef<'gc>,
    delegate_type: TypeDescription,
}

impl<'a, 'gc, T: LoaderOps + MemoryOps<'gc>> DelegateViewMut<'a, 'gc, T> {
    pub(super) fn new(ctx: &'a T, obj: ObjectRef<'gc>) -> Self {
        Self {
            ctx,
            obj,
            delegate_type: delegate_type(ctx),
        }
    }

    pub(super) fn set_target_method(&self, target: ObjectRef<'gc>, method_index: usize) {
        let gc = self.ctx.gc_with_token(&self.ctx.no_active_borrows_token());
        self.obj.as_object_mut(gc, |instance| {
            self.ctx
                .loader()
                .delegate_target_field(&instance.instance_storage, self.delegate_type.clone())
                .expect("validated System.Delegate target support slot")
                .write(target);
            self.ctx
                .loader()
                .delegate_method_index_field(&instance.instance_storage, self.delegate_type.clone())
                .expect("validated System.Delegate method-index support slot")
                .write(method_index);
        });
    }

    pub(super) fn set_multicast_targets(&self, targets: ObjectRef<'gc>) {
        let gc = self.ctx.gc_with_token(&self.ctx.no_active_borrows_token());
        self.obj.as_object_mut(gc, |instance| {
            let multicast_type = self
                .ctx
                .loader()
                .corlib_wkt(WellKnown::MulticastDelegate)
                .expect("System.MulticastDelegate must exist");
            self.ctx
                .loader()
                .multicast_delegate_targets_field(&instance.instance_storage, multicast_type)
                .expect("validated System.MulticastDelegate targets support slot")
                .write(targets);
        });
    }
}

pub(super) fn get_delegate_info<'gc, T: LoaderOps>(
    ctx: &T,
    obj: ObjectRef<'gc>,
) -> (ObjectRef<'gc>, usize) {
    let identity = DelegateView::new(ctx, obj).identity();
    (identity.target, identity.method_index)
}

pub fn set_delegate_target_method<'gc, T: LoaderOps + MemoryOps<'gc>>(
    ctx: &T,
    obj: ObjectRef<'gc>,
    target: ObjectRef<'gc>,
    method_index: usize,
) {
    DelegateViewMut::new(ctx, obj).set_target_method(target, method_index);
}

pub fn set_delegate_multicast_targets<'gc, T: LoaderOps + MemoryOps<'gc>>(
    ctx: &T,
    obj: ObjectRef<'gc>,
    targets: ObjectRef<'gc>,
) {
    DelegateViewMut::new(ctx, obj).set_multicast_targets(targets);
}

pub(super) fn get_multicast_targets_ref<'gc, T: LoaderOps + ResolutionOps<'gc>>(
    ctx: &T,
    obj: ObjectRef<'gc>,
) -> Option<ObjectRef<'gc>> {
    DelegateView::new(ctx, obj).multicast_targets()
}

pub(super) fn delegates_equal<'gc, T: LoaderOps + ResolutionOps<'gc>>(
    ctx: &T,
    a: ObjectRef<'gc>,
    b: ObjectRef<'gc>,
) -> bool {
    if a == b {
        return true;
    }
    if a.0.is_none() || b.0.is_none() {
        return false;
    }

    let identity_a = DelegateView::new(ctx, a).identity();
    let identity_b = DelegateView::new(ctx, b).identity();

    identity_a.target == identity_b.target && identity_a.method_index == identity_b.method_index
}

pub(super) fn get_invocation_list<'gc, T: LoaderOps + ResolutionOps<'gc> + MemoryOps<'gc>>(
    ctx: &T,
    obj: ObjectRef<'gc>,
) -> Vec<ObjectRef<'gc>> {
    DelegateView::new(ctx, obj).invocation_list_iter().collect()
}
