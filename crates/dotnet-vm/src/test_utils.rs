use crate::{
    StepResult,
    dispatch::ExecutionEngine,
    stack::{CallStack, GCArena},
    state::{ArenaLocalState, SharedGlobalState},
    sync::Arc,
};
use dotnet_utils::gc::GCHandle;
use dotnetdll::{
    binary::signature::kinds::CallingConvention,
    resolved::{
        signature::{MethodSignature, ParameterType, ReturnType},
        types::{BaseType, MethodType},
    },
};
use gc_arena::Mutation;

/// Constructs a handle for a test arena's rooted engine.
///
/// # Safety
/// The engine must remain the live arena root and its stack must not be replaced
/// for the lifetime of the returned handle. Use it only inside the mutation.
pub(crate) unsafe fn test_gc_handle<'gc>(
    gc: &'gc Mutation<'gc>,
    _engine: &ExecutionEngine<'gc>,
) -> GCHandle<'gc> {
    GCHandle::new(
        gc,
        #[cfg(feature = "multithreading")]
        // SAFETY: F1.GcHandleRooted — the caller guarantees the root and its
        // stack-owned arena stay alive for the returned handle's lifetime.
        unsafe {
            _engine.stack.arena_inner_gc()
        },
        #[cfg(feature = "memory-validation")]
        dotnet_utils::sync::get_current_thread_id(),
    )
}

pub(crate) fn parameterless_i32_signature() -> MethodSignature<CallingConvention, MethodType> {
    MethodSignature {
        instance: false,
        explicit_this: false,
        calling_convention: CallingConvention::Default,
        parameters: vec![],
        return_type: ReturnType(
            vec![],
            Some(ParameterType::Value(MethodType::Base(Box::new(
                BaseType::Int32,
            )))),
        ),
        varargs: None,
    }
}

pub(crate) fn should_break_after_step(step_result: StepResult) -> bool {
    match step_result {
        StepResult::Return => true,
        StepResult::Error(error) => panic!("Execution error: {error}"),
        StepResult::MethodThrew(exception) => {
            panic!("Unexpected managed exception: {exception}")
        }
        _ => false,
    }
}

pub(crate) fn new_test_arena(shared: &Arc<SharedGlobalState>) -> GCArena {
    GCArena::new(|_| {
        let local = ArenaLocalState::new();
        ExecutionEngine::new(CallStack::new(shared.clone(), local))
    })
}
