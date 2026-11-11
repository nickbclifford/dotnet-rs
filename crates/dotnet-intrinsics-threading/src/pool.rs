//! CoreLib ThreadPool management forwards. Queueing remains managed code.
use crate::ThreadingIntrinsicHost;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_value::{
    StackValue,
    layout::{LayoutManager, Scalar},
};
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::ops::RawMemoryOps;

#[dotnet_intrinsic("static void System.Threading.ThreadPool::RequestWorkerThread()")]
pub fn request<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _: MethodDescription,
    _: &GenericLookup,
) -> StepResult {
    ctx.pool_request()
}

#[dotnet_intrinsic(
    "static object System.Threading.ThreadPool::GetOrCreateThreadLocalCompletionCountObject()"
)]
pub fn placeholder<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _: MethodDescription,
    _: &GenericLookup,
) -> StepResult {
    ctx.pool_placeholder()
}

#[dotnet_intrinsic("static bool System.Threading.ThreadPool::NotifyWorkItemComplete(object, int)")]
pub fn complete<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _: MethodDescription,
    _: &GenericLookup,
) -> StepResult {
    let _ = ctx.pop_i32();
    let _ = ctx.pop_obj();
    ctx.pool_complete();
    ctx.push_i32(1);
    StepResult::Continue
}

#[dotnet_intrinsic("static void System.Threading.ThreadPool::NotifyWorkItemProgress()")]
pub fn progress<'gc, T: ThreadingIntrinsicHost<'gc>>(
    _: &mut T,
    _: MethodDescription,
    _: &GenericLookup,
) -> StepResult {
    StepResult::Continue
}

#[dotnet_intrinsic("static void System.Threading.ThreadPool::ReportThreadStatus(bool)")]
pub fn status<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _: MethodDescription,
    _: &GenericLookup,
) -> StepResult {
    let _ = ctx.pop_i32();
    StepResult::Continue
}

#[dotnet_intrinsic("static int System.Threading.ThreadPool::get_ThreadCount()")]
pub fn thread_count<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _: MethodDescription,
    _: &GenericLookup,
) -> StepResult {
    ctx.push_i32(ctx.pool_worker_count());
    StepResult::Continue
}

#[dotnet_intrinsic("static long System.Threading.ThreadPool::get_CompletedWorkItemCount()")]
pub fn completed_count<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _: MethodDescription,
    _: &GenericLookup,
) -> StepResult {
    ctx.push_i64(ctx.pool_completed_count());
    StepResult::Continue
}

#[dotnet_intrinsic("static bool System.Threading.ThreadPool::SetMinThreads(int, int)")]
pub fn set_min<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _: MethodDescription,
    _: &GenericLookup,
) -> StepResult {
    let io = ctx.pop_i32();
    let worker = ctx.pop_i32();
    ctx.push_i32(i32::from(ctx.pool_set_min(worker, io)));
    StepResult::Continue
}
#[dotnet_intrinsic("static bool System.Threading.ThreadPool::SetMaxThreads(int, int)")]
pub fn set_max<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _: MethodDescription,
    _: &GenericLookup,
) -> StepResult {
    let io = ctx.pop_i32();
    let worker = ctx.pop_i32();
    ctx.push_i32(i32::from(ctx.pool_set_max(worker, io)));
    StepResult::Continue
}

fn write_counts<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    counts: (i32, i32),
) -> StepResult {
    let io = ctx.pop_managed_ptr();
    let worker = ctx.pop_managed_ptr();
    let layout = LayoutManager::Scalar(Scalar::Int32);
    for (ptr, value) in [(worker, counts.0), (io, counts.1)] {
        // SAFETY: F10.RawMemoryAccessValid — each pointer is an int32 out parameter;
        // managed origins are range checked by RawMemoryOps and unmanaged origins obey the caller contract.
        if let Err(error) = unsafe {
            RawMemoryOps::write_unaligned(
                ctx,
                ptr.origin().clone(),
                ptr.byte_offset(),
                StackValue::Int32(value),
                &layout,
            )
        } {
            return StepResult::Error(error.into());
        }
    }
    StepResult::Continue
}
#[dotnet_intrinsic("static void System.Threading.ThreadPool::GetMinThreads(int&, int&)")]
pub fn get_min<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _: MethodDescription,
    _: &GenericLookup,
) -> StepResult {
    let counts = ctx.pool_min();
    write_counts(ctx, counts)
}
#[dotnet_intrinsic("static void System.Threading.ThreadPool::GetMaxThreads(int&, int&)")]
pub fn get_max<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _: MethodDescription,
    _: &GenericLookup,
) -> StepResult {
    let counts = ctx.pool_max();
    write_counts(ctx, counts)
}
#[dotnet_intrinsic("static void System.Threading.ThreadPool::GetAvailableThreads(int&, int&)")]
pub fn get_available<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    _: MethodDescription,
    _: &GenericLookup,
) -> StepResult {
    let counts = ctx.pool_available();
    write_counts(ctx, counts)
}
