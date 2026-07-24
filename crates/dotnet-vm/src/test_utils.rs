use crate::{
    StepResult,
    dispatch::ExecutionEngine,
    stack::{CallStack, GCArena},
    state::{ArenaLocalState, SharedGlobalState},
    sync::Arc,
};
use dotnetdll::{
    binary::signature::kinds::CallingConvention,
    resolved::{
        signature::{MethodSignature, ParameterType, ReturnType},
        types::{BaseType, MethodType},
    },
};

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
        let local = ArenaLocalState::new(shared.statics.clone());
        ExecutionEngine::new(CallStack::new(shared.clone(), local))
    })
}
