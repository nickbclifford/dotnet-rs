use crate::{
    StepResult,
    instructions::objects::get_ptr_info,
    stack::ops::{
        EvalStackOps, ExceptionOps, MemoryOps, RawMemoryOps, ThreadOps, TypedStackOps, VesBaseOps,
        VesInternals,
    },
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandleType;
use dotnet_value::{
    StackValue,
    layout::{LayoutManager, Scalar},
    object::{HeapStorage, ObjectRef},
    with_string,
};
use dotnet_vm_ops::effects::{
    NativeInvocation, NativeOperandView, NativeResult, NativeSelection, VmEffects,
};

/// System.ArgumentNullException::ThrowIfNull(object, string)
#[dotnet_intrinsic("static void System.ArgumentNullException::ThrowIfNull(object, string)")]
pub fn intrinsic_argument_null_exception_throw_if_null<
    'gc,
    T: TypedStackOps<'gc> + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _param_name = ctx.pop();
    let target = ctx.pop();
    if let StackValue::ObjectRef(obj_ref) = target
        && obj_ref.0.is_none()
    {
        return ctx
            .throw_by_name_with_message("System.ArgumentNullException", "Value cannot be null.");
    }
    StepResult::Continue
}

/// System.Environment::GetEnvironmentVariableCore(string)
#[dotnet_intrinsic("static string System.Environment::GetEnvironmentVariableCore(string)")]
#[dotnet_intrinsic(
    "static string System.Environment::GetEnvironmentVariableCore_NoArrayPool(string)"
)]
pub fn intrinsic_environment_get_variable_core<
    'gc,
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>,
>(
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

/// System.Environment::.cctor()
///
/// CoreLib's managed .cctor can invoke runtime-native entry points that are not
/// available in dotnet-rs yet. Treat it as a no-op bootstrap so Environment
/// APIs can still be used by higher-level libraries like System.Text.Json.
#[dotnet_intrinsic("static void System.Environment::.cctor()")]
pub fn intrinsic_environment_cctor<'gc, T: TypedStackOps<'gc>>(
    _ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    StepResult::Continue
}

/// System.Environment::get_ProcessorCount()
#[dotnet_intrinsic("static int System.Environment::get_ProcessorCount()")]
pub fn intrinsic_environment_processor_count<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // Provide a deterministic non-zero CPU count for libraries sizing
    // pools/buffers from ProcessorCount.
    ctx.push_i32(1);
    StepResult::Continue
}

/// System.Environment::get_CurrentManagedThreadId()
#[dotnet_intrinsic("static int System.Environment::get_CurrentManagedThreadId()")]
pub fn intrinsic_environment_current_managed_thread_id<'gc, T: TypedStackOps<'gc> + ThreadOps>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(ctx.managed_thread_id());
    StepResult::Continue
}

/// System.GC::KeepAlive(object) - Prevents the GC from collecting an object
/// until this method is called.
#[dotnet_intrinsic("static void System.GC::KeepAlive(object)")]
pub fn intrinsic_gc_keep_alive<'gc, T: EvalStackOps<'gc> + TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _obj = ctx.pop();
    StepResult::Continue
}

/// System.GC::SuppressFinalize(object)
#[dotnet_intrinsic("static void System.GC::SuppressFinalize(object)")]
pub fn intrinsic_gc_suppress_finalize<'gc, T: TypedStackOps<'gc> + MemoryOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    if let Some(handle) = obj.0
        && let Some(o) = dotnet_value::ThreadSafeLock::borrow_mut(
            handle,
            &ctx.gc_with_token(&ctx.no_active_borrows_token()),
        )
        .storage
        .as_obj_mut()
    {
        o.finalizer_suppressed = true;
    }
    StepResult::Continue
}

/// System.GC::ReRegisterForFinalize(object)
#[dotnet_intrinsic("static void System.GC::ReRegisterForFinalize(object)")]
pub fn intrinsic_gc_reregister_for_finalize<'gc, T: TypedStackOps<'gc> + MemoryOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    if let Some(handle) = obj.0 {
        let is_obj = handle.borrow().storage.as_obj().is_some();
        if is_obj {
            dotnet_value::ThreadSafeLock::borrow_mut(
                handle,
                &ctx.gc_with_token(&ctx.no_active_borrows_token()),
            )
            .storage
            .as_obj_mut()
            .expect("is_obj checked above")
            .finalizer_suppressed = false;

            // Add back to finalization queue if not already present
            let mut queue = ctx.heap().finalization_queue.borrow_mut();
            if !queue.contains(&obj) {
                queue.push(obj);
            }
        }
    }
    StepResult::Continue
}

/// System.GC::Collect()
#[dotnet_intrinsic("static void System.GC::Collect()")]
pub fn intrinsic_gc_collect_0<'gc, T: MemoryOps<'gc> + VesInternals<'gc>>(
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
pub fn intrinsic_gc_collect_1<'gc, T: TypedStackOps<'gc> + MemoryOps<'gc> + VesInternals<'gc>>(
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
pub fn intrinsic_gc_collect_2<'gc, T: TypedStackOps<'gc> + MemoryOps<'gc> + VesInternals<'gc>>(
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

/// A finalizer cannot wait for the work that includes its own completion.
pub fn gc_wait_finalizers_select<'gc, T: MemoryOps<'gc>>(
    ctx: &T,
    _operands: NativeOperandView<'_, 'gc>,
    _method: &MethodDescription,
    _generics: &GenericLookup,
) -> NativeSelection {
    if ctx.heap().processing_finalizer.get() {
        NativeSelection::Fallback
    } else {
        NativeSelection::Async(NativeInvocation {
            start: 0,
            length: 0,
            parameter_count: 0,
        })
    }
}

/// Wait for pending finalizers on legacy dispatch, or return from the current finalizer.
pub fn intrinsic_gc_wait_for_pending_finalizers<'gc, T: MemoryOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    if ctx.heap().processing_finalizer.get() {
        return StepResult::Continue;
    }

    if ctx.has_global_pending_finalizers() || !ctx.heap().pending_finalization.borrow().is_empty() {
        return StepResult::Yield;
    }
    StepResult::Continue
}

#[dotnet_intrinsic("static void System.GC::WaitForPendingFinalizers()", select = gc_wait_finalizers_select, fallback = intrinsic_gc_wait_for_pending_finalizers)]
pub async fn gc_wait_finalizers(mut fx: VmEffects) -> NativeResult {
    fx.wait_finalizers().await
}

#[dotnet_intrinsic("static void System.GC::GetMemoryInfo(System.GCMemoryInfoData, int)")]
pub fn intrinsic_gc_get_memory_info<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _kind = ctx.pop_i32();
    let _data = ctx.pop_obj();
    StepResult::Continue
}

#[dotnet_intrinsic("static T[] System.GC::AllocateUninitializedArray<T>(int, bool)")]
pub fn intrinsic_gc_allocate_uninitialized_array<
    'gc,
    T: TypedStackOps<'gc> + MemoryOps<'gc> + ExceptionOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _pinned = ctx.pop_i32();
    let length = ctx.pop_i32();

    if length < 0 {
        return ctx.throw_by_name_with_message(
            "System.ArgumentOutOfRangeException",
            "Specified argument was out of the range of valid values.",
        );
    }

    let element_type = dotnet_vm_ops::vm_try!(generics.cloned_method_arg(0));
    let vector = dotnet_vm_ops::vm_try!(ctx.new_vector(element_type, length as usize));
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let vector_ref = ctx.alloc_vec_ref(gc, vector);
    ctx.push_obj(vector_ref);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static void System.Gen2GcCallback::Register(System.Func<object, bool>, object)"
)]
pub fn intrinsic_gen2_gc_callback_register<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _target = ctx.pop_obj();
    let _callback = ctx.pop_obj();
    StepResult::Continue
}

fn alloc_or_reuse_slot<T>(handles: &mut Vec<Option<T>>, value: T) -> usize {
    if let Some(i) = handles.iter().position(|h| h.is_none()) {
        handles[i] = Some(value);
        i
    } else {
        handles.push(Some(value));
        handles.len() - 1
    }
}

// CoreLib reserves the low two bits for WeakReference and GCHandle tags.
// Each handle-table slot is larger than four bytes, so an allocated Vec's
// byte-size limit also bounds the encoded index within isize::MAX.
fn encode_gc_handle(index: usize) -> isize {
    index
        .checked_add(1)
        .and_then(|value| value.checked_mul(4))
        .and_then(|value| isize::try_from(value).ok())
        .expect("GC handle table exceeds the representable address space")
}

fn decode_gc_handle(handle: isize) -> Option<usize> {
    let value = usize::try_from(handle).ok()?;
    (value != 0 && value % 4 == 0).then(|| value / 4 - 1)
}

#[dotnet_intrinsic(
    "static IntPtr System.Runtime.InteropServices.GCHandle::InternalAlloc(object, System.Runtime.InteropServices.GCHandleType)"
)]
pub fn intrinsic_gchandle_internal_alloc<
    'gc,
    T: TypedStackOps<'gc> + MemoryOps<'gc> + VesBaseOps,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle_type = ctx.pop_i32();
    let obj = ctx.pop_obj();

    let handle_type = GCHandleType::from(handle_type);
    let index = {
        let mut handles = ctx.heap().gchandles.borrow_mut();
        alloc_or_reuse_slot(&mut handles, (obj, handle_type))
    };

    if handle_type == GCHandleType::Pinned {
        ctx.heap().pin_object(obj);

        if ctx.tracer_enabled() {
            let addr = obj.0.map(|p| gc_arena::Gc::as_ptr(p).addr()).unwrap_or(0);
            ctx.tracer().trace_gc_pin(ctx.indent(), "PINNED", addr);
        }
    }

    if ctx.tracer_enabled() {
        let handle_type_str = format!("{:?}", handle_type);
        let addr = obj.0.map(|p| gc_arena::Gc::as_ptr(p).addr()).unwrap_or(0);
        ctx.tracer()
            .trace_gc_handle(ctx.indent(), "ALLOC", &handle_type_str, addr);
    }

    ctx.push_isize(encode_gc_handle(index));
    StepResult::Continue
}

#[dotnet_intrinsic("static void System.Runtime.InteropServices.GCHandle::CheckUninitialized(nint)")]
pub fn intrinsic_gchandle_check_uninitialized<'gc, T: TypedStackOps<'gc> + ExceptionOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = ctx.pop_isize();
    if handle == 0 {
        return ctx.throw_by_name_with_message(
            "System.InvalidOperationException",
            "Handle is not initialized.",
        );
    }

    StepResult::Continue
}

#[dotnet_intrinsic("static void System.Runtime.InteropServices.GCHandle::InternalFree(IntPtr)")]
pub fn intrinsic_gchandle_internal_free<
    'gc,
    T: TypedStackOps<'gc> + MemoryOps<'gc> + VesBaseOps,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = ctx.pop_isize();
    if let Some(index) = decode_gc_handle(handle) {
        let mut handles = ctx.heap().gchandles.borrow_mut();
        if index < handles.len() {
            if let Some((obj, handle_type)) = handles[index]
                && handle_type == GCHandleType::Pinned
            {
                ctx.heap().unpin_object(obj);

                if ctx.tracer_enabled() {
                    let addr = obj.0.map(|p| gc_arena::Gc::as_ptr(p).addr()).unwrap_or(0);
                    ctx.tracer().trace_gc_pin(ctx.indent(), "UNPINNED", addr);
                }
            }

            if let Some((obj, handle_type)) = handles[index]
                && ctx.tracer_enabled()
            {
                let handle_type_str = format!("{:?}", handle_type);
                let addr = obj.0.map(|p| gc_arena::Gc::as_ptr(p).addr()).unwrap_or(0);
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
    T: EvalStackOps<'gc> + TypedStackOps<'gc> + MemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = ctx.pop_isize();
    let result = if let Some(index) = decode_gc_handle(handle) {
        let handles = ctx.heap().gchandles.borrow();
        match handles.get(index) {
            Some(Some((obj, _))) => *obj,
            _ => ObjectRef(None),
        }
    } else {
        ObjectRef(None)
    };
    ctx.push_obj(result);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static void System.Runtime.InteropServices.GCHandle::InternalSet(IntPtr, object)"
)]
pub fn intrinsic_gchandle_internal_set<'gc, T: TypedStackOps<'gc> + MemoryOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let handle = ctx.pop_isize();
    if let Some(index) = decode_gc_handle(handle) {
        let mut handles = ctx.heap().gchandles.borrow_mut();
        if index < handles.len()
            && let Some(entry) = &mut handles[index]
        {
            if entry.1 == GCHandleType::Pinned {
                ctx.heap().unpin_object(entry.0);
                ctx.heap().pin_object(obj);
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
    T: TypedStackOps<'gc> + MemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = ctx.pop_isize();
    let addr = if let Some(index) = decode_gc_handle(handle) {
        let handles = ctx.heap().gchandles.borrow();
        if index < handles.len()
            && let Some(entry) = &handles[index]
            && entry.1 == GCHandleType::Pinned
            && let Some(ptr) = entry.0.0
        {
            match &ptr.borrow().storage {
                // SAFETY: F10.BorrowedStorageStable — The validated pinned GCHandle keeps `ptr` and its object lock alive;
                // this branch only exposes that stable address for AddrOfPinnedObject.
                HeapStorage::Obj(_) => unsafe { ptr.as_ptr() }.expose_provenance() as isize,
                HeapStorage::Vec(v) => v.get().as_ptr().expose_provenance() as isize,
                HeapStorage::Str(s) => s.as_ptr().expose_provenance() as isize,
                _ => 0,
            }
        } else {
            0
        }
    } else {
        0
    };
    ctx.push_isize(addr);
    StepResult::Continue
}

#[dotnet_intrinsic("static IntPtr System.Runtime.DependentHandle::InternalAlloc(object, object)")]
pub fn intrinsic_dependent_handle_internal_alloc<'gc, T: TypedStackOps<'gc> + MemoryOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let dependent = ctx.pop_obj();
    let target = ctx.pop_obj();

    let index = {
        let mut handles = ctx.heap().dependent_handles.borrow_mut();
        alloc_or_reuse_slot(&mut handles, (target, dependent))
    };

    ctx.push_isize((index + 1) as isize);
    StepResult::Continue
}

#[dotnet_intrinsic("static object System.Runtime.DependentHandle::InternalGetDependent(IntPtr)")]
pub fn intrinsic_dependent_handle_internal_get_dependent<
    'gc,
    T: TypedStackOps<'gc> + MemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = ctx.pop_isize();
    let dependent = if handle == 0 {
        ObjectRef(None)
    } else {
        let index = (handle - 1) as usize;
        let handles = ctx.heap().dependent_handles.borrow();
        match handles.get(index) {
            Some(Some((_target, dependent))) => *dependent,
            _ => ObjectRef(None),
        }
    };
    ctx.push_obj(dependent);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static object System.Runtime.DependentHandle::InternalGetTargetAndDependent(IntPtr, object&)"
)]
pub fn intrinsic_dependent_handle_internal_get_target_and_dependent<
    'gc,
    T: ExceptionOps<'gc> + RawMemoryOps<'gc> + TypedStackOps<'gc> + MemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let dependent_out_ptr = ctx.pop();
    let handle = ctx.pop_isize();

    let (target, dependent) = if handle == 0 {
        (ObjectRef(None), ObjectRef(None))
    } else {
        let index = (handle - 1) as usize;
        let handles = ctx.heap().dependent_handles.borrow();
        match handles.get(index) {
            Some(Some((target, dependent))) => (*target, *dependent),
            _ => (ObjectRef(None), ObjectRef(None)),
        }
    };

    let layout = LayoutManager::Scalar(Scalar::ObjectRef);
    let (origin, offset) = match get_ptr_info(ctx, &dependent_out_ptr) {
        Ok(v) => v,
        Err(e) => return e,
    };
    // SAFETY: F2.DescriptorMatchesEcmaLayout — `get_ptr_info` decoded the managed ObjectRef out parameter, and `layout` matches
    // the dependent handle value written through that origin and offset.
    dotnet_vm_ops::vm_try!(unsafe {
        ctx.write_unaligned(origin, offset, StackValue::ObjectRef(dependent), &layout)
    });

    ctx.push_obj(target);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static void System.Runtime.DependentHandle::InternalSetDependent(IntPtr, object)"
)]
pub fn intrinsic_dependent_handle_internal_set_dependent<
    'gc,
    T: TypedStackOps<'gc> + MemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let dependent = ctx.pop_obj();
    let handle = ctx.pop_isize();
    if handle != 0 {
        let index = (handle - 1) as usize;
        let mut handles = ctx.heap().dependent_handles.borrow_mut();
        if index < handles.len()
            && let Some(entry) = &mut handles[index]
        {
            entry.1 = dependent;
        }
    }
    StepResult::Continue
}

#[dotnet_intrinsic("static void System.Runtime.DependentHandle::InternalSetTargetToNull(IntPtr)")]
pub fn intrinsic_dependent_handle_internal_set_target_to_null<
    'gc,
    T: TypedStackOps<'gc> + MemoryOps<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = ctx.pop_isize();
    if handle != 0 {
        let index = (handle - 1) as usize;
        let mut handles = ctx.heap().dependent_handles.borrow_mut();
        if index < handles.len()
            && let Some(entry) = &mut handles[index]
        {
            entry.0 = ObjectRef(None);
            entry.1 = ObjectRef(None);
        }
    }
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Runtime.DependentHandle::InternalFree(IntPtr)")]
pub fn intrinsic_dependent_handle_internal_free<'gc, T: TypedStackOps<'gc> + MemoryOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let handle = ctx.pop_isize();
    if handle != 0 {
        let index = (handle - 1) as usize;
        let mut handles = ctx.heap().dependent_handles.borrow_mut();
        if index < handles.len() {
            handles[index] = None;
        }
    }
    ctx.push_i32(1);
    StepResult::Continue
}

/// RuntimeHelpers.ObjectHasComponentSize — returns true for arrays and strings.
///
/// In the CLR, "component-sized" objects (arrays, strings) store a per-element
/// size field in their object header.  ReadOnlyMemory<T>.Span uses this to
/// distinguish array-backed memory from MemoryManager-backed memory.
#[dotnet_intrinsic(
    "static bool System.Runtime.CompilerServices.RuntimeHelpers::ObjectHasComponentSize(object)"
)]
pub fn intrinsic_object_has_component_size<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop_obj();
    let has_component_size = obj.0.as_ref().is_some_and(|handle| {
        let storage = handle.borrow();
        matches!(storage.storage, HeapStorage::Vec(_) | HeapStorage::Str(_))
    });
    ctx.push_i32(has_component_size as i32);
    StepResult::Continue
}

#[cfg(test)]
mod handle_tests {
    use super::{decode_gc_handle, encode_gc_handle};

    #[test]
    fn gc_handle_slots_reserve_tag_bits_and_reject_invalid_values() {
        for index in [0, 1, 2, 17, 1024] {
            let handle = encode_gc_handle(index);
            assert_eq!(handle & 3, 0);
            assert_eq!(decode_gc_handle(handle), Some(index));
        }
        for invalid in [0, -4, 1, 2, 3, 5, 6, 7] {
            assert_eq!(decode_gc_handle(invalid), None);
        }
    }
}
