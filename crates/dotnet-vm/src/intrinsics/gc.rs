use crate::{CallStack, StepResult};
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
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _param_name_obj = stack.pop_obj(gc);
    let target = stack.pop_obj(gc);
    if target.0.is_none() {
        return stack.throw_by_name(gc, "System.ArgumentNullException");
    }
    StepResult::InstructionStepped
}

/// System.Environment::GetEnvironmentVariableCore(string)
#[dotnet_intrinsic("static string System.Environment::GetEnvironmentVariableCore(string)")]
pub fn intrinsic_environment_get_variable_core<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = with_string!(stack, gc, stack.pop(gc), |s| std::env::var(s.as_string()));
    match value.ok() {
        Some(s) => stack.push_string(gc, s),
        None => stack.push_obj(gc, ObjectRef(None)),
    }
    StepResult::InstructionStepped
}

/// System.GC::KeepAlive(object) - Prevents the GC from collecting an object
/// until this method is called.
#[dotnet_intrinsic("static void System.GC::KeepAlive(object)")]
pub fn intrinsic_gc_keep_alive<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _obj = stack.pop_obj(gc);
    StepResult::InstructionStepped
}

/// System.GC::SuppressFinalize(object)
#[dotnet_intrinsic("static void System.GC::SuppressFinalize(object)")]
pub fn intrinsic_gc_suppress_finalize<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = stack.pop_obj(gc);
    if let Some(handle) = obj.0 {
        if let Some(o) = handle.borrow_mut(gc).storage.as_obj_mut() {
            o.finalizer_suppressed = true;
        }
    }
    StepResult::InstructionStepped
}

/// System.GC::ReRegisterForFinalize(object)
#[dotnet_intrinsic("static void System.GC::ReRegisterForFinalize(object)")]
pub fn intrinsic_gc_reregister_for_finalize<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = stack.pop_obj(gc);
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
    StepResult::InstructionStepped
}

/// System.GC::Collect()
#[dotnet_intrinsic("static void System.GC::Collect()")]
pub fn intrinsic_gc_collect_0<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    stack.heap().needs_full_collect.set(true);
    StepResult::InstructionStepped
}

/// System.GC::Collect(int)
#[dotnet_intrinsic("static void System.GC::Collect(int)")]
pub fn intrinsic_gc_collect_1<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _generation = stack.pop_i32(gc);
    stack.heap().needs_full_collect.set(true);
    StepResult::InstructionStepped
}

/// System.GC::Collect(int, GCCollectionMode)
#[dotnet_intrinsic("static void System.GC::Collect(int, System.GCCollectionMode)")]
pub fn intrinsic_gc_collect_2<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _mode = stack.pop_i32(gc);
    let _generation = stack.pop_i32(gc);
    stack.heap().needs_full_collect.set(true);
    StepResult::InstructionStepped
}

#[dotnet_intrinsic("static void System.GC::WaitForPendingFinalizers()")]
pub fn intrinsic_gc_wait_for_pending_finalizers<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // If there are pending finalizers or we're currently processing one,
    // back up the IP to wait (this effectively spins until finalizers complete)
    if !stack.heap().pending_finalization.borrow().is_empty()
        || stack.heap().processing_finalizer.get()
    {
        stack.current_frame_mut().state.ip -= 1;
        return StepResult::InstructionStepped;
    }
    StepResult::InstructionStepped
}

#[dotnet_intrinsic("static IntPtr System.Runtime.InteropServices.GCHandle::InternalAlloc(object, System.Runtime.InteropServices.GCHandleType)")]
pub fn intrinsic_gchandle_internal_alloc<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle_type = stack.pop_i32(gc);
    let obj = stack.pop_obj(gc);

    let handle_type = GCHandleType::from(handle_type);
    let index = {
        let mut handles = stack.heap().gchandles.borrow_mut();
        if let Some(i) = handles.iter().position(|h| h.is_none()) {
            handles[i] = Some((obj, handle_type));
            i
        } else {
            handles.push(Some((obj, handle_type)));
            handles.len() - 1
        }
    };

    if handle_type == GCHandleType::Pinned {
        stack.heap().pinned_objects.borrow_mut().insert(obj);

        if stack.tracer_enabled() {
            let addr = obj.0.map(|p| gc_arena::Gc::as_ptr(p) as usize).unwrap_or(0);
            stack
                .shared
                .tracer
                .lock()
                .trace_gc_pin(stack.indent(), "PINNED", addr);
        }
    }

    if stack.tracer_enabled() {
        let handle_type_str = format!("{:?}", handle_type);
        let addr = obj.0.map(|p| gc_arena::Gc::as_ptr(p) as usize).unwrap_or(0);
        stack
            .shared
            .tracer
            .lock()
            .trace_gc_handle(stack.indent(), "ALLOC", &handle_type_str, addr);
    }

    stack.push_isize(gc, (index + 1) as isize);
    StepResult::InstructionStepped
}

#[dotnet_intrinsic("static void System.Runtime.InteropServices.GCHandle::InternalFree(IntPtr)")]
pub fn intrinsic_gchandle_internal_free<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = stack.pop_isize(gc);
    if handle != 0 {
        let index = (handle - 1) as usize;
        let mut handles = stack.heap().gchandles.borrow_mut();
        if index < handles.len() {
            if let Some((obj, handle_type)) = handles[index] {
                if handle_type == GCHandleType::Pinned {
                    stack.heap().pinned_objects.borrow_mut().remove(&obj);

                    if stack.tracer_enabled() {
                        let addr = obj.0.map(|p| gc_arena::Gc::as_ptr(p) as usize).unwrap_or(0);
                        stack
                            .shared
                            .tracer
                            .lock()
                            .trace_gc_pin(stack.indent(), "UNPINNED", addr);
                    }
                }

                if stack.tracer_enabled() {
                    let handle_type_str = format!("{:?}", handle_type);
                    let addr = obj.0.map(|p| gc_arena::Gc::as_ptr(p) as usize).unwrap_or(0);
                    stack.shared.tracer.lock().trace_gc_handle(
                        stack.indent(),
                        "FREE",
                        &handle_type_str,
                        addr,
                    );
                }
            }
            handles[index] = None;
        }
    }
    StepResult::InstructionStepped
}

#[dotnet_intrinsic("static object System.Runtime.InteropServices.GCHandle::InternalGet(IntPtr)")]
pub fn intrinsic_gchandle_internal_get<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = stack.pop_isize(gc);
    let result = if handle == 0 {
        ObjectRef(None)
    } else {
        let index = (handle - 1) as usize;
        let handles = stack.heap().gchandles.borrow();
        match handles.get(index) {
            Some(Some((obj, _))) => *obj,
            _ => ObjectRef(None),
        }
    };
    stack.push_obj(gc, result);
    StepResult::InstructionStepped
}

#[dotnet_intrinsic(
    "static void System.Runtime.InteropServices.GCHandle::InternalSet(IntPtr, object)"
)]
pub fn intrinsic_gchandle_internal_set<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = stack.pop_obj(gc);
    let handle = stack.pop_isize(gc);
    if handle != 0 {
        let index = (handle - 1) as usize;
        let mut handles = stack.heap().gchandles.borrow_mut();
        if index < handles.len() {
            if let Some(entry) = &mut handles[index] {
                if entry.1 == GCHandleType::Pinned {
                    let mut pinned = stack.heap().pinned_objects.borrow_mut();
                    pinned.remove(&entry.0);
                    pinned.insert(obj);
                }
                entry.0 = obj;
            }
        }
    }
    StepResult::InstructionStepped
}

#[dotnet_intrinsic(
    "static IntPtr System.Runtime.InteropServices.GCHandle::InternalAddrOfPinnedObject(IntPtr)"
)]
pub fn intrinsic_gchandle_internal_addr_of_pinned_object<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = stack.pop_isize(gc);
    let addr = if handle == 0 {
        0
    } else {
        let index = (handle - 1) as usize;
        let handles = stack.heap().gchandles.borrow();
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
    stack.push_isize(gc, addr);
    StepResult::InstructionStepped
}
