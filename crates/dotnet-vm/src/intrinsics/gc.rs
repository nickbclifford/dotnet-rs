use crate::{
    StepResult,
    stack::ops::{
        CallOps, EvalStackOps, ExceptionOps, LoaderOps, MemoryOps, RawMemoryOps, ResolutionOps,
        ThreadOps, TypedStackOps, VesOps,
    },
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandleType;
use dotnet_value::{
    object::{HeapStorage, ObjectRef},
    with_string,
};

/// System.ArgumentNullException::ThrowIfNull(object, string)
#[dotnet_intrinsic("static void System.ArgumentNullException::ThrowIfNull(object, string)")]
pub fn intrinsic_argument_null_exception_throw_if_null<
    'gc,
    'm: 'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _param_name_obj = ctx.pop_obj();
    let target = ctx.pop_obj();
    if target.0.is_none() {
        return ctx
            .throw_by_name_with_message("System.ArgumentNullException", "Value cannot be null.");
    }
    StepResult::Continue
}

/// System.Environment::GetEnvironmentVariableCore(string)
#[dotnet_intrinsic("static string System.Environment::GetEnvironmentVariableCore(string)")]
pub fn intrinsic_environment_get_variable_core<'gc, 'm: 'gc, T: VesOps<'gc, 'm>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = with_string!(ctx, ctx.pop(), |s| std::env::var(s.as_string()));
    match value.ok() {
        Some(s) => ctx.push_string(s.into()),
        None => ctx.push_obj(ObjectRef(None)),
    }
    StepResult::Continue
}

/// System.GC::KeepAlive(object) - Prevents the GC from collecting an object
/// until this method is called.
#[dotnet_intrinsic("static void System.GC::KeepAlive(object)")]
pub fn intrinsic_gc_keep_alive<'gc, 'm: 'gc, T: EvalStackOps<'gc> + TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _obj = ctx.pop_obj();
    StepResult::Continue
}

/// System.GC::SuppressFinalize(object)
#[dotnet_intrinsic("static void System.GC::SuppressFinalize(object)")]
pub fn intrinsic_gc_suppress_finalize<
    'gc,
    'm: 'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + MemoryOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    if let Some(handle) = obj.0
        && let Some(o) = handle
            .borrow_mut(&ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new()))
            .storage
            .as_obj_mut()
    {
        o.finalizer_suppressed = true;
    }
    StepResult::Continue
}

/// System.GC::ReRegisterForFinalize(object)
#[dotnet_intrinsic("static void System.GC::ReRegisterForFinalize(object)")]
pub fn intrinsic_gc_reregister_for_finalize<
    'gc,
    'm: 'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + MemoryOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    if let Some(handle) = obj.0 {
        let is_obj = handle.borrow().storage.as_obj().is_some();
        if is_obj {
            handle
                .borrow_mut(&ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new()))
                .storage
                .as_obj_mut()
                .unwrap()
                .finalizer_suppressed = false;
        }
    }
    StepResult::Continue
}

/// System.GC::Collect()
#[dotnet_intrinsic("static void System.GC::Collect()")]
pub fn intrinsic_gc_collect_0<'gc, 'm: 'gc, T: VesOps<'gc, 'm>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.heap().needs_full_collect.set(true);
    ctx.increment_ip();
    StepResult::Yield
}

/// System.GC::Collect(int)
#[dotnet_intrinsic("static void System.GC::Collect(int)")]
pub fn intrinsic_gc_collect_1<'gc, 'm: 'gc, T: VesOps<'gc, 'm>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _generation = ctx.pop_i32();
    ctx.heap().needs_full_collect.set(true);
    ctx.increment_ip();
    StepResult::Yield
}

/// System.GC::Collect(int, GCCollectionMode)
#[dotnet_intrinsic("static void System.GC::Collect(int, System.GCCollectionMode)")]
pub fn intrinsic_gc_collect_2<'gc, 'm: 'gc, T: VesOps<'gc, 'm>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _mode = ctx.pop_i32();
    let _generation = ctx.pop_i32();
    ctx.heap().needs_full_collect.set(true);
    ctx.increment_ip();
    StepResult::Yield
}

#[dotnet_intrinsic("static void System.GC::WaitForPendingFinalizers()")]
pub fn intrinsic_gc_wait_for_pending_finalizers<
    'gc,
    'm: 'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + MemoryOps<'gc>
        + CallOps<'gc, 'm>
        + ResolutionOps<'gc, 'm>
        + RawMemoryOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps<'m>
        + ThreadOps,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // If there are pending finalizers or we're currently processing one,
    // yield to wait (this effectively spins until finalizers complete)
    if !ctx.heap().pending_finalization.borrow().is_empty() || ctx.heap().processing_finalizer.get()
    {
        return StepResult::Yield;
    }
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static IntPtr System.Runtime.InteropServices.GCHandle::InternalAlloc(object, System.Runtime.InteropServices.GCHandleType)"
)]
pub fn intrinsic_gchandle_internal_alloc<'gc, 'm: 'gc, T: VesOps<'gc, 'm>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle_type = ctx.pop_i32();
    let obj = ctx.pop_obj();

    let handle_type = GCHandleType::from(handle_type);
    let index = {
        let mut handles = ctx.heap().gchandles.borrow_mut();
        if let Some(i) = handles.iter().position(|h| h.is_none()) {
            handles[i] = Some((obj, handle_type));
            i
        } else {
            handles.push(Some((obj, handle_type)));
            handles.len() - 1
        }
    };

    if handle_type == GCHandleType::Pinned {
        ctx.heap().pinned_objects.borrow_mut().insert(obj);

        if ctx.tracer_enabled() {
            let addr = obj.0.map(|p| gc_arena::Gc::as_ptr(p) as usize).unwrap_or(0);
            ctx.tracer().trace_gc_pin(ctx.indent(), "PINNED", addr);
        }
    }

    if ctx.tracer_enabled() {
        let handle_type_str = format!("{:?}", handle_type);
        let addr = obj.0.map(|p| gc_arena::Gc::as_ptr(p) as usize).unwrap_or(0);
        ctx.tracer()
            .trace_gc_handle(ctx.indent(), "ALLOC", &handle_type_str, addr);
    }

    ctx.push_isize((index + 1) as isize);
    StepResult::Continue
}

#[dotnet_intrinsic("static void System.Runtime.InteropServices.GCHandle::InternalFree(IntPtr)")]
pub fn intrinsic_gchandle_internal_free<'gc, 'm: 'gc, T: VesOps<'gc, 'm>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = ctx.pop_isize();
    if handle != 0 {
        let index = (handle - 1) as usize;
        let mut handles = ctx.heap().gchandles.borrow_mut();
        if index < handles.len() {
            if let Some((obj, handle_type)) = handles[index]
                && handle_type == GCHandleType::Pinned
            {
                ctx.heap().pinned_objects.borrow_mut().remove(&obj);

                if ctx.tracer_enabled() {
                    let addr = obj.0.map(|p| gc_arena::Gc::as_ptr(p) as usize).unwrap_or(0);
                    ctx.tracer().trace_gc_pin(ctx.indent(), "UNPINNED", addr);
                }
            }

            if let Some((obj, handle_type)) = handles[index]
                && ctx.tracer_enabled()
            {
                let handle_type_str = format!("{:?}", handle_type);
                let addr = obj.0.map(|p| gc_arena::Gc::as_ptr(p) as usize).unwrap_or(0);
                ctx.tracer()
                    .trace_gc_handle(ctx.indent(), "FREE", &handle_type_str, addr);
            }
            handles[index] = None;
        }
    }
    StepResult::Continue
}

#[dotnet_intrinsic("static object System.Runtime.InteropServices.GCHandle::InternalGet(IntPtr)")]
pub fn intrinsic_gchandle_internal_get<
    'gc,
    'm: 'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + MemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = ctx.pop_isize();
    let result = if handle == 0 {
        ObjectRef(None)
    } else {
        let index = (handle - 1) as usize;
        let handles = ctx.heap().gchandles.borrow();
        match handles.get(index) {
            Some(Some((obj, _))) => *obj,
            _ => ObjectRef(None),
        }
    };
    ctx.push_obj(result);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static void System.Runtime.InteropServices.GCHandle::InternalSet(IntPtr, object)"
)]
pub fn intrinsic_gchandle_internal_set<
    'gc,
    'm: 'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + MemoryOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let handle = ctx.pop_isize();
    if handle != 0 {
        let index = (handle - 1) as usize;
        let mut handles = ctx.heap().gchandles.borrow_mut();
        if index < handles.len()
            && let Some(entry) = &mut handles[index]
        {
            if entry.1 == GCHandleType::Pinned {
                let mut pinned = ctx.heap().pinned_objects.borrow_mut();
                pinned.remove(&entry.0);
                pinned.insert(obj);
            }
            entry.0 = obj;
        }
    }
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static IntPtr System.Runtime.InteropServices.GCHandle::InternalAddrOfPinnedObject(IntPtr)"
)]
pub fn intrinsic_gchandle_internal_addr_of_pinned_object<
    'gc,
    'm: 'gc,
    T: EvalStackOps<'gc>
        + TypedStackOps<'gc>
        + MemoryOps<'gc>
        + CallOps<'gc, 'm>
        + ResolutionOps<'gc, 'm>
        + RawMemoryOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps<'m>
        + ThreadOps,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = ctx.pop_isize();
    let addr = if handle == 0 {
        0
    } else {
        let index = (handle - 1) as usize;
        let handles = ctx.heap().gchandles.borrow();
        if index < handles.len()
            && let Some(entry) = &handles[index]
            && entry.1 == GCHandleType::Pinned
            && let Some(ptr) = entry.0.0
        {
            match &ptr.borrow().storage {
                HeapStorage::Obj(_) => unsafe { ptr.as_ptr() as isize },
                HeapStorage::Vec(v) => v.get().as_ptr() as isize,
                HeapStorage::Str(s) => s.as_ptr() as isize,
                _ => 0,
            }
        } else {
            0
        }
    };
    ctx.push_isize(addr);
    StepResult::Continue
}
