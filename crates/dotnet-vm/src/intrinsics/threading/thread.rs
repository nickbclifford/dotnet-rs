use crate::{
    StepResult,
    stack::ops::TypedStackOps,
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use std::time::Duration;

/// System.Threading.Thread::Sleep(int millisecondsTimeout)
///
/// Suspends the current thread for the specified number of milliseconds.
/// A value of 0 yields the time slice without sleeping; negative values are
/// treated as a no-op (the managed side should validate before calling).
#[dotnet_intrinsic("static void System.Threading.Thread::Sleep(int)")]
pub fn intrinsic_thread_sleep<
    'gc,
     T: TypedStackOps<'gc>,
>(
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
