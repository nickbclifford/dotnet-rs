use crate::{stack::VesContext, StepResult};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::{GCHandle, GCHandleType};
use dotnet_value::{
    object::{HeapStorage, ObjectRef},
    with_string,
};

/// System.ArgumentNullException::ThrowIfNull(object, string)
#[dotnet_intrinsic("static void System.ArgumentNullException::ThrowIfNull(object, string)")]
pub fn intrinsic_argument_null_exception_throw_if_null<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _param_name_obj = ctx.pop_obj(gc);
    let target = ctx.pop_obj(gc);
    if target.0.is_none() {
        return ctx.throw_by_name(gc, "System.ArgumentNullException");
    }
    StepResult::Continue
}

/// System.Environment::GetEnvironmentVariableCore(string)
#[dotnet_intrinsic("static string System.Environment::GetEnvironmentVariableCore(string)")]
pub fn intrinsic_environment_get_variable_core<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = with_string!(ctx, gc, ctx.pop(gc), |s| std::env::var(s.as_string()));
    match value.ok() {
        Some(s) => ctx.push_string(gc, s),
        None => ctx.push_obj(gc, ObjectRef(None)),
    }
    StepResult::Continue
}

/// System.GC::KeepAlive(object) - Prevents the GC from collecting an object
/// until this method is called.
#[dotnet_intrinsic("static void System.GC::KeepAlive(object)")]
pub fn intrinsic_gc_keep_alive<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _obj = ctx.pop_obj(gc);
    StepResult::Continue
}

/// System.GC::SuppressFinalize(object)
#[dotnet_intrinsic("static void System.GC::SuppressFinalize(object)")]
pub fn intrinsic_gc_suppress_finalize<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj(gc);
    if let Some(handle) = obj.0 {
        if let Some(o) = handle.borrow_mut(gc).storage.as_obj_mut() {
            o.finalizer_suppressed = true;
        }
    }
    StepResult::Continue
}

/// System.GC::ReRegisterForFinalize(object)
#[dotnet_intrinsic("static void System.GC::ReRegisterForFinalize(object)")]
pub fn intrinsic_gc_reregister_for_finalize<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj(gc);
    if let Some(handle) = obj.0 {
        let is_obj = handle.borrow().storage.as_obj().is_some();
        if is_obj {
            handle
                .borrow_mut(gc)
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
pub fn intrinsic_gc_collect_0<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    _gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.heap().needs_full_collect.set(true);
    StepResult::Continue
}

/// System.GC::Collect(int)
#[dotnet_intrinsic("static void System.GC::Collect(int)")]
pub fn intrinsic_gc_collect_1<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _generation = ctx.pop_i32(gc);
    ctx.heap().needs_full_collect.set(true);
    StepResult::Continue
}

/// System.GC::Collect(int, GCCollectionMode)
#[dotnet_intrinsic("static void System.GC::Collect(int, System.GCCollectionMode)")]
pub fn intrinsic_gc_collect_2<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _mode = ctx.pop_i32(gc);
    let _generation = ctx.pop_i32(gc);
    ctx.heap().needs_full_collect.set(true);
    StepResult::Continue
}

#[dotnet_intrinsic("static void System.GC::WaitForPendingFinalizers()")]
pub fn intrinsic_gc_wait_for_pending_finalizers<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    _gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // If there are pending finalizers or we're currently processing one,
    // back up the IP to wait (this effectively spins until finalizers complete)
    if !ctx.heap().pending_finalization.borrow().is_empty() || ctx.heap().processing_finalizer.get()
    {
        ctx.back_up_ip();
        return StepResult::Continue;
    }
    StepResult::Continue
}

#[dotnet_intrinsic("static IntPtr System.Runtime.InteropServices.GCHandle::InternalAlloc(object, System.Runtime.InteropServices.GCHandleType)")]
pub fn intrinsic_gchandle_internal_alloc<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle_type = ctx.pop_i32(gc);
    let obj = ctx.pop_obj(gc);

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
            ctx.shared
                .tracer
                .lock()
                .trace_gc_pin(ctx.indent(), "PINNED", addr);
        }
    }

    if ctx.tracer_enabled() {
        let handle_type_str = format!("{:?}", handle_type);
        let addr = obj.0.map(|p| gc_arena::Gc::as_ptr(p) as usize).unwrap_or(0);
        ctx.shared
            .tracer
            .lock()
            .trace_gc_handle(ctx.indent(), "ALLOC", &handle_type_str, addr);
    }

    ctx.push_isize(gc, (index + 1) as isize);
    StepResult::Continue
}

#[dotnet_intrinsic("static void System.Runtime.InteropServices.GCHandle::InternalFree(IntPtr)")]
pub fn intrinsic_gchandle_internal_free<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = ctx.pop_isize(gc);
    if handle != 0 {
        let index = (handle - 1) as usize;
        let mut handles = ctx.heap().gchandles.borrow_mut();
        if index < handles.len() {
            if let Some((obj, handle_type)) = handles[index] {
                if handle_type == GCHandleType::Pinned {
                    ctx.heap().pinned_objects.borrow_mut().remove(&obj);

                    if ctx.tracer_enabled() {
                        let addr = obj.0.map(|p| gc_arena::Gc::as_ptr(p) as usize).unwrap_or(0);
                        ctx.shared
                            .tracer
                            .lock()
                            .trace_gc_pin(ctx.indent(), "UNPINNED", addr);
                    }
                }

                if ctx.tracer_enabled() {
                    let handle_type_str = format!("{:?}", handle_type);
                    let addr = obj.0.map(|p| gc_arena::Gc::as_ptr(p) as usize).unwrap_or(0);
                    ctx.shared.tracer.lock().trace_gc_handle(
                        ctx.indent(),
                        "FREE",
                        &handle_type_str,
                        addr,
                    );
                }
            }
            handles[index] = None;
        }
    }
    StepResult::Continue
}

#[dotnet_intrinsic("static object System.Runtime.InteropServices.GCHandle::InternalGet(IntPtr)")]
pub fn intrinsic_gchandle_internal_get<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = ctx.pop_isize(gc);
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
    ctx.push_obj(gc, result);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static void System.Runtime.InteropServices.GCHandle::InternalSet(IntPtr, object)"
)]
pub fn intrinsic_gchandle_internal_set<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj(gc);
    let handle = ctx.pop_isize(gc);
    if handle != 0 {
        let index = (handle - 1) as usize;
        let mut handles = ctx.heap().gchandles.borrow_mut();
        if index < handles.len() {
            if let Some(entry) = &mut handles[index] {
                if entry.1 == GCHandleType::Pinned {
                    let mut pinned = ctx.heap().pinned_objects.borrow_mut();
                    pinned.remove(&entry.0);
                    pinned.insert(obj);
                }
                entry.0 = obj;
            }
        }
    }
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static IntPtr System.Runtime.InteropServices.GCHandle::InternalAddrOfPinnedObject(IntPtr)"
)]
pub fn intrinsic_gchandle_internal_addr_of_pinned_object<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = ctx.pop_isize(gc);
    let addr = if handle == 0 {
        0
    } else {
        let index = (handle - 1) as usize;
        let handles = ctx.heap().gchandles.borrow();
        if index < handles.len() {
            if let Some(entry) = &handles[index] {
                if entry.1 == GCHandleType::Pinned {
                    if let Some(ptr) = entry.0 .0 {
                        match &ptr.borrow().storage {
                            HeapStorage::Obj(_) => unsafe { ptr.as_ptr() as isize },
                            HeapStorage::Vec(v) => v.get().as_ptr() as isize,
                            HeapStorage::Str(s) => s.as_ptr() as isize,
                            _ => 0,
                        }
                    } else {
                        0
                    }
                } else {
                    0
                }
            } else {
                0
            }
        } else {
            0
        }
    };
    ctx.push_isize(gc, addr);
    StepResult::Continue
}
