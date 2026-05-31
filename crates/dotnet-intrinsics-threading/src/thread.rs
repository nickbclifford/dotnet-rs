use crate::ThreadingIntrinsicHost;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_vm_data::StepResult;
use std::time::Duration;

const PROCESSOR_ID_FALLBACK: i32 = 0;

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
