use crate::ThreadingIntrinsicHost;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_value::object::{HeapStorage, ObjectRef};
use dotnet_vm_data::StepResult;
use std::time::Duration;

const PROCESSOR_ID_FALLBACK: i32 = 0;
const NULL_THREAD_START_MSG: &str = "Thread(ThreadStart) requires a non-null ThreadStart delegate.";

/// System.Threading.Thread::.ctor(System.Threading.ThreadStart)
///
/// This direct intercept deliberately bypasses CoreLib's QCall initialization. It supports only
/// the `ThreadStart` constructor; overloads remain on their existing unsupported paths.
#[dotnet_intrinsic("void System.Threading.Thread::.ctor(System.Threading.ThreadStart)")]
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

    ctx.threading_new_managed_thread(method.parent.clone(), thread_start)
}

/// System.Threading.Thread::Start()
#[dotnet_intrinsic("void System.Threading.Thread::Start()")]
pub fn intrinsic_thread_start<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let thread = ctx.pop_obj();
    ctx.threading_start_managed_thread(thread)
}

/// System.Threading.Thread::Join()
#[dotnet_intrinsic("void System.Threading.Thread::Join()")]
pub fn intrinsic_thread_join<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    ctx.threading_join_managed_thread()
}

/// System.Threading.Thread::Sleep(int millisecondsTimeout)
///
/// Suspends the current thread for the specified number of milliseconds.
/// A value of 0 yields the time slice without sleeping; negative values are
/// treated as a no-op (the managed side should validate before calling).
#[dotnet_intrinsic("static void System.Threading.Thread::Sleep(int)")]
pub fn intrinsic_thread_sleep<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let ms = ctx.pop_i32();
    if ms > 0 {
        std::thread::sleep(Duration::from_millis(ms as u64));
    } else if ms == 0 {
        std::thread::yield_now();
    }
    // ms < 0 (e.g. Timeout.Infinite = -1): no-op; managed callers are
    // responsible for not passing unbounded sleeps into the VM.
    StepResult::Continue
}

/// System.Threading.Thread::get_CurrentThread()
///
/// The CoreLib implementation is `[Intrinsic]` and typically uses runtime-managed
/// thread state. dotnet-rs does not yet model the current managed `Thread` object's identity,
/// so returning a managed `System.Threading.Thread` instance is sufficient to unblock framework
/// call sites that require a non-null current-thread object.
#[dotnet_intrinsic("static System.Threading.Thread System.Threading.Thread::get_CurrentThread()")]
pub fn intrinsic_thread_get_current_thread<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let thread_obj = dotnet_vm_ops::vm_try!(ctx.new_object(method.parent.clone()));
    ctx.push_obj(ObjectRef::new(
        ctx.gc_with_token(&ctx.no_active_borrows_token()),
        HeapStorage::Obj(Box::new(thread_obj)),
    ));
    StepResult::Continue
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
