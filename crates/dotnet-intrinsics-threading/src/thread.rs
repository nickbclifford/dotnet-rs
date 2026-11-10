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

/// System.Threading.Thread::Join()
#[dotnet_intrinsic("void System.Threading.Thread::Join()")]
pub fn intrinsic_thread_join<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.threading_join_managed_thread(None)
}

#[dotnet_intrinsic("bool System.Threading.Thread::Join(int)")]
pub fn intrinsic_thread_join_timeout<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.threading_join_managed_thread(Some(method))
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

/// Legacy dispatch keeps its operand and deadline across retries. Inline calls park outside
/// the mutation through the native effect driver.
pub fn intrinsic_thread_sleep<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let ms = ctx.peek_stack_at(0).as_i32();
    if ms < -1 {
        ctx.monitor_clear_retry();
        let _ = ctx.pop_i32();
        return ctx.throw_by_name_with_message(
            "System.ArgumentOutOfRangeException",
            "The timeout must be greater than or equal to -1.",
        );
    }
    if ms == 0 {
        let _ = ctx.pop_i32();
        std::thread::yield_now();
        return StepResult::Continue;
    }
    let deadline = ctx.monitor_retry_deadline(&method, ms);
    if ms == -1 || Instant::now() < deadline {
        return StepResult::Yield;
    }
    ctx.monitor_clear_retry();
    let _ = ctx.pop_i32();
    StepResult::Continue
}

pub fn sleep_select<'gc, T: ThreadingIntrinsicHost<'gc>>(
    _ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    method: &MethodDescription,
    _generics: &GenericLookup,
) -> NativeSelection {
    match operands.peek_at(0) {
        StackValue::Int32(ms) if ms >= -1 => NativeSelection::Async(NativeInvocation {
            start: ms as u32 as usize,
            length: 0,
            parameter_count: method.signature().parameters.len(),
        }),
        _ => NativeSelection::Fallback,
    }
}

#[dotnet_intrinsic("static void System.Threading.Thread::Sleep(int)", select = sleep_select, fallback = intrinsic_thread_sleep)]
pub async fn thread_sleep(mut fx: VmEffects) -> NativeResult {
    let ms = fx.invocation().start as u32 as i32;
    let deadline = (ms > 0).then(|| Instant::now() + Duration::from_millis(ms as u64));
    fx.vm().await?.discard_scalars(1)?;
    if ms == 0 {
        std::thread::yield_now();
        fx.yield_now().await
    } else {
        fx.sleep_until(deadline).await
    }
}

pub fn intrinsic_thread_yield<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    std::thread::yield_now();
    ctx.push_i32(1);
    StepResult::Continue
}

pub fn intrinsic_thread_spin_wait<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    if ctx.pop_i32() > 0 {
        std::thread::yield_now();
    }
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Threading.Thread::Yield()", fallback = intrinsic_thread_yield)]
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
        _ => NativeSelection::Fallback,
    }
}

#[dotnet_intrinsic("static void System.Threading.Thread::SpinWait(int)", select = spin_select, fallback = intrinsic_thread_spin_wait)]
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

/// System.Threading.ThreadPool::UnsafeQueueUserWorkItem(System.Threading.WaitCallback, object)
///
/// dotnet-rs does not implement general ThreadPool work-item scheduling. Queue requests from DI
/// warm-up paths are therefore treated as accepted no-ops; this is separate from the narrowly
/// supported explicit `Thread(ThreadStart)` lifecycle.
#[dotnet_intrinsic(
    "static bool System.Threading.ThreadPool::UnsafeQueueUserWorkItem(System.Threading.WaitCallback, object)"
)]
pub fn intrinsic_threadpool_unsafe_queue_user_work_item<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _state = ctx.pop();
    let _callback = ctx.pop();
    ctx.push_i32(1);
    StepResult::Continue
}
