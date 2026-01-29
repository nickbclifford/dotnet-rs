use crate::{pop_args, vm_pop, vm_push, CallStack, StepResult};
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::{GCHandle, GCHandleType};
use dotnet_value::{
    object::{HeapStorage, ObjectRef},
    with_string, StackValue,
};

/// System.ArgumentNullException::ThrowIfNull(object, string)
pub fn intrinsic_argument_null_exception_throw_if_null<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ObjectRef(target), ObjectRef(_param_name_obj)]);
    if target.0.is_none() {
        return stack.throw_by_name(gc, "System.ArgumentNullException");
    }
    StepResult::InstructionStepped
}

/// System.Environment::GetEnvironmentVariableCore(string)
pub fn intrinsic_environment_get_variable_core<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = with_string!(stack, gc, vm_pop!(stack, gc), |s| std::env::var(
        s.as_string()
    ));
    match value.ok() {
        Some(s) => vm_push!(stack, gc, string(s)),
        None => vm_push!(stack, gc, StackValue::null()),
    }
    StepResult::InstructionStepped
}

/// System.GC::KeepAlive(object) - Prevents the GC from collecting an object
/// until this method is called.
pub fn intrinsic_gc_keep_alive<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ObjectRef(_obj)]);
    StepResult::InstructionStepped
}

/// System.GC::SuppressFinalize(object)
pub fn intrinsic_gc_suppress_finalize<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ObjectRef(obj)]);
    if let Some(handle) = obj.0 {
        if let Some(o) = handle.borrow_mut(gc).storage.as_obj_mut() {
            o.finalizer_suppressed = true;
        }
    }
    StepResult::InstructionStepped
}

/// System.GC::ReRegisterForFinalize(object)
pub fn intrinsic_gc_reregister_for_finalize<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ObjectRef(obj)]);
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
pub fn intrinsic_gc_collect_1<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [Int32(_generation)]);
    stack.heap().needs_full_collect.set(true);
    StepResult::InstructionStepped
}

/// System.GC::Collect(int, GCCollectionMode)
pub fn intrinsic_gc_collect_2<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [Int32(_generation), Int32(_mode)]);
    stack.heap().needs_full_collect.set(true);
    StepResult::InstructionStepped
}

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

pub fn intrinsic_gchandle_internal_alloc<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ObjectRef(obj), Int32(handle_type)]);

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

    vm_push!(stack, gc, NativeInt((index + 1) as isize));
    StepResult::InstructionStepped
}

pub fn intrinsic_gchandle_internal_free<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [NativeInt(handle)]);
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

pub fn intrinsic_gchandle_internal_get<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [NativeInt(handle)]);
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
    vm_push!(stack, gc, ObjectRef(result));
    StepResult::InstructionStepped
}

pub fn intrinsic_gchandle_internal_set<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [NativeInt(handle), ObjectRef(obj)]);
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

pub fn intrinsic_gchandle_internal_addr_of_pinned_object<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [NativeInt(handle)]);
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
    vm_push!(stack, gc, NativeInt(addr));
    StepResult::InstructionStepped
}
