use crate::ThreadingIntrinsicHost;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_value::StackValue;
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::effects::{
    NativeInvocation, NativeOperandView, NativeResult, NativeSelection, VmEffects,
};
use std::time::{Duration, Instant};

const PROCESSOR_ID_FALLBACK: i32 = 0;
const NULL_THREAD_START_MSG: &str = "Thread(ThreadStart) requires a non-null ThreadStart delegate.";

/// System.Threading.Thread::.ctor(System.Threading.ThreadStart)
///
/// This direct intercept bypasses CoreLib's QCall initialization.
#[dotnet_intrinsic("void System.Threading.Thread::.ctor(System.Threading.ThreadStart)")]
#[dotnet_intrinsic(
    "void System.Threading.Thread::.ctor(System.Threading.ParameterizedThreadStart)"
)]
pub fn intrinsic_thread_ctor_thread_start<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let thread_start = ctx.pop_obj();
    if thread_start.0.is_none() {
        return ctx
            .throw_by_name_with_message("System.ArgumentNullException", NULL_THREAD_START_MSG);
    }

    // Both overloads have non-primitive parameters, which intrinsic filters do not distinguish.
    let parameterized = thread_start.as_heap_storage(|storage| match storage {
        dotnet_value::object::HeapStorage::Obj(object) => {
            object.description.definition().type_name()
                == "System.Threading.ParameterizedThreadStart"
        }
        _ => false,
    });
    ctx.threading_new_managed_thread(method.parent.clone(), thread_start, parameterized)
}

/// System.Threading.Thread::Start()
#[dotnet_intrinsic("void System.Threading.Thread::Start()")]
pub fn intrinsic_thread_start<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let thread = ctx.pop_obj();
    ctx.threading_start_managed_thread(thread, dotnet_value::object::ObjectRef(None), false)
}

/// TimerQueue uses UnsafeStart so its background thread does not capture an ExecutionContext.
#[dotnet_intrinsic("void System.Threading.Thread::UnsafeStart()")]
pub fn intrinsic_thread_unsafe_start<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let thread = ctx.pop_obj();
    ctx.threading_start_managed_thread(thread, dotnet_value::object::ObjectRef(None), false)
}

#[dotnet_intrinsic("void System.Threading.Thread::Start(object)")]
pub fn intrinsic_thread_start_with_argument<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let argument = ctx.pop_obj();
    let thread = ctx.pop_obj();
    ctx.threading_start_managed_thread(thread, argument, true)
}

#[dotnet_intrinsic("int System.Threading.Thread::get_ManagedThreadId()")]
pub fn intrinsic_thread_managed_id<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let thread = ctx.pop_obj();
    ctx.threading_managed_id(thread)
}

#[dotnet_intrinsic("bool System.Threading.Thread::get_IsBackground()")]
pub fn intrinsic_thread_get_background<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let thread = ctx.pop_obj();
    ctx.threading_background(thread, None)
}

#[dotnet_intrinsic("void System.Threading.Thread::set_IsBackground(bool)")]
pub fn intrinsic_thread_set_background<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = ctx.pop_i32() != 0;
    let thread = ctx.pop_obj();
    ctx.threading_background(thread, Some(value))
}

/// CoreLib stores the name on the managed Thread object; only its optional OS notification is absent.
#[dotnet_intrinsic("void System.Threading.Thread::ThreadNameChanged(string)")]
pub fn intrinsic_thread_name_changed<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _name = ctx.pop_obj();
    let _thread = ctx.pop_obj();
    StepResult::Continue
}

/// System.Threading.Thread::Join()
#[dotnet_intrinsic("void System.Threading.Thread::Join()")]
pub async fn thread_join(fx: VmEffects) -> NativeResult {
    join_effect(fx, false).await
}

#[dotnet_intrinsic("bool System.Threading.Thread::Join(int)", select = sleep_select)]
pub async fn thread_join_timeout(fx: VmEffects) -> NativeResult {
    join_effect(fx, true).await
}

async fn join_effect(mut fx: VmEffects, timed: bool) -> NativeResult {
    let timeout = if timed {
        fx.invocation().start as u32 as i32
    } else {
        -1
    };
    let deadline = (timeout >= 0).then(|| Instant::now() + Duration::from_millis(timeout as u64));
    let (_receiver, completion) = {
        let mut vm = fx.vm().await?;
        let (receiver, _, _) = vm.prepare_monitor(timed, false)?;
        if timeout < -1 {
            return Err(vm.exception(
                "System.ArgumentOutOfRangeException",
                "The timeout must be greater than or equal to -1.",
            )?);
        }
        if vm.is_null(&receiver)? {
            return Err(vm.exception("System.NullReferenceException", dotnet_vm_ops::NULL_REF_MSG)?);
        }
        let completion = vm.with(|region| region.thread_join(&receiver))??;
        match completion {
            dotnet_vm_ops::effects::NativeThreadJoin::Unstarted => {
                return Err(vm.exception(
                    "System.Threading.ThreadStateException",
                    "Thread has not been started.",
                )?);
            }
            dotnet_vm_ops::effects::NativeThreadJoin::Waiting(completion) => (receiver, completion),
        }
    };
    let outcome = fx.join_thread(completion, deadline).await?;
    let mut vm = fx.vm().await?;
    if let Some(Err(message)) = &outcome {
        return Err(vm.exception("System.InvalidOperationException", message)?);
    }
    if timed {
        vm.push_bool(outcome.is_some())?;
    }
    Ok(())
}

/// The worker registry owns native handles independently of the managed Thread object.
#[dotnet_intrinsic("void System.Threading.Thread::InternalFinalize()")]
pub fn intrinsic_thread_internal_finalize<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _ = ctx.pop_obj();
    StepResult::Continue
}

pub fn sleep_select<'gc, T: ThreadingIntrinsicHost<'gc>>(
    _ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    method: &MethodDescription,
    _generics: &GenericLookup,
) -> NativeSelection {
    match operands.peek_at(0) {
        StackValue::Int32(ms) => NativeSelection::Async(NativeInvocation {
            start: ms as u32 as usize,
            length: 0,
            parameter_count: method.signature().parameters.len(),
        }),
        _ => NativeSelection::Reject(
            dotnet_types::error::ExecutionError::InternalError(
                "Sleep timeout is not an int32".into(),
            )
            .into(),
        ),
    }
}

#[dotnet_intrinsic("static void System.Threading.Thread::Sleep(int)", select = sleep_select)]
pub async fn thread_sleep(mut fx: VmEffects) -> NativeResult {
    let ms = fx.invocation().start as u32 as i32;
    let deadline = (ms > 0).then(|| Instant::now() + Duration::from_millis(ms as u64));
    let mut vm = fx.vm().await?;
    vm.discard_scalars(1)?;
    if ms < -1 {
        return Err(vm.exception(
            "System.ArgumentOutOfRangeException",
            "The timeout must be greater than or equal to -1.",
        )?);
    }
    drop(vm);
    if ms == 0 {
        std::thread::yield_now();
        fx.yield_now().await
    } else {
        fx.sleep_until(deadline).await
    }
}

#[dotnet_intrinsic("static bool System.Threading.Thread::Yield()")]
pub async fn thread_yield(mut fx: VmEffects) -> NativeResult {
    std::thread::yield_now();
    fx.yield_now().await?;
    fx.vm().await?.push_bool(true)
}

pub fn spin_select<'gc, T: ThreadingIntrinsicHost<'gc>>(
    _ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    method: &MethodDescription,
    _generics: &GenericLookup,
) -> NativeSelection {
    match operands.peek_at(0) {
        StackValue::Int32(count) => NativeSelection::Async(NativeInvocation {
            start: usize::from(count > 0),
            length: 0,
            parameter_count: method.signature().parameters.len(),
        }),
        _ => NativeSelection::Reject(
            dotnet_types::error::ExecutionError::InternalError(
                "SpinWait count is not an int32".into(),
            )
            .into(),
        ),
    }
}

#[dotnet_intrinsic("static void System.Threading.Thread::SpinWait(int)", select = spin_select)]
pub async fn thread_spin_wait(mut fx: VmEffects) -> NativeResult {
    let positive = fx.invocation().start != 0;
    fx.vm().await?.discard_scalars(1)?;
    if positive {
        std::thread::yield_now();
        fx.yield_now().await?;
    }
    Ok(())
}

/// System.Threading.Thread::get_CurrentThread()
///
#[dotnet_intrinsic("static System.Threading.Thread System.Threading.Thread::get_CurrentThread()")]
pub fn intrinsic_thread_get_current_thread<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.threading_current_thread(method.parent.clone())
}

/// System.Threading.Thread::GetCurrentProcessorNumber()
/// System.Threading.Thread::GetCurrentProcessorId()
///
/// These APIs are used by pool partitioning heuristics (e.g., ArrayPool in
/// System.Text.Json dispose paths). Returning a stable fallback keeps managed
/// execution deterministic without requiring native `libSystem.Native` entrypoints.
#[dotnet_intrinsic("static int System.Threading.Thread::GetCurrentProcessorNumber()")]
#[dotnet_intrinsic("static int System.Threading.Thread::GetCurrentProcessorId()")]
pub fn intrinsic_thread_get_current_processor_fallback<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(PROCESSOR_ID_FALLBACK);
    StepResult::Continue
}

/// CoreLib task waits notify the pool before blocking. No compensation worker is available.
#[dotnet_intrinsic("static bool System.Threading.ThreadPool::NotifyThreadBlocked()")]
pub fn intrinsic_threadpool_notify_thread_blocked<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.push_i32(0);
    StepResult::Continue
}

#[dotnet_intrinsic("static void System.Threading.ThreadPool::NotifyThreadUnblocked()")]
pub fn intrinsic_threadpool_notify_thread_unblocked<'gc, T: ThreadingIntrinsicHost<'gc>>(
    _ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    StepResult::Continue
}
