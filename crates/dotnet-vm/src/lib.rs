//! # dotnet-vm
//!
//! The core virtual machine implementation for the `dotnet-rs` runtime.
//! This crate implements the Virtual Execution System (VES) as defined by ECMA-335.
//!
//! ## Subsystems
//!
//! - **Stack System** (`stack/`): Evaluation stack, call frames, and operational traits.
//! - **Instruction Set** (`instructions/`): Handlers for CIL instructions.
//! - **Intrinsics** (`intrinsics/`): Native implementations of BCL methods.
//! - **Memory Management** (`memory/`, `gc/`): Heap management and garbage collection.
//! - **Dispatch** (`dispatch/`): Method resolution and execution engine.
//! - **Threading** (`threading/`): Support for multi-threaded execution.
#![allow(clippy::mutable_key_type)]
#![allow(clippy::arc_with_non_send_sync)]
use dotnetdll::prelude::*;
use std::sync::Arc;

#[macro_use]
mod macros;

pub(crate) mod branch_hint;
pub mod context;
pub mod dispatch;
pub mod error;
mod executor;
#[cfg(test)]
mod fault_tests;
#[cfg(feature = "fuzzing")]
pub mod fuzzing;
pub mod gc;
mod instructions;
pub(crate) mod intrinsics;
#[cfg(test)]
mod jmp_tests;
pub mod layout;
pub mod resolution;
pub mod resolver;
mod stack;
pub mod state;
pub mod statics;
pub mod sync;
#[cfg(test)]
mod tail_calls_tests;
pub mod threading;

pub use dotnet_metrics::RuntimeMetricsSnapshot;
pub use dotnet_types::{generics::GenericLookup, members::MethodDescription};
pub use dotnet_utils::{
    ArenaId, ArgumentIndex, ByteOffset, FieldIndex, LocalIndex, StackSlotIndex,
};
pub use dotnet_vm_data::{CollectableMethodDescription, MethodInfo, MethodState, StepResult};
#[cfg(feature = "multithreading")]
pub use executor::ArenaGuard;
pub use executor::{Executor, ExecutorResult};
pub use stack::ops;
pub use stack::{
    ArgumentOps, BasePointer, CallOps, CallStack, EvalStackOps, EvaluationStack, ExceptionContext,
    ExceptionOps, ExceptionState, FrameStack, GCArena, GCArenaRoot, IntrinsicDispatchOps,
    LoaderOps, LocalOps, MemoryOps, PInvokeContext, PinnedLocals, RawMemoryOps,
    ReflectionLookupOps, ReflectionOps, ResolutionOps, StackFrame, StackOps, StaticsOps,
    ThreadContext, ThreadOps, TypedStackOps, VariableOps, VesBaseOps, VesContext, VesInternals,
    VesOps, VmCallOps, VmExceptionContext, VmLoaderOps, VmPInvokeContext, VmRawMemoryOps,
    VmReflectionOps, VmResolutionOps, VmStackOps, VmStaticsOps,
};
pub use state::ReflectionRegistry;

use context::ResolutionContext;
use state::SharedGlobalState;

/// Constructs a [`MethodInfo`] from a resolved method descriptor.
///
/// This factory function lives in `dotnet-vm` rather than `dotnet-vm-ops` because it
/// requires access to [`ResolutionContext`], [`SharedGlobalState`], and
/// [`dotnet_exceptions::parse`] — all of which are internal to this crate.
pub fn build_method_info(
    method: MethodDescription,
    generics: &GenericLookup,
    shared: Arc<SharedGlobalState>,
) -> Result<MethodInfo<'static>, error::TypeResolutionError> {
    let loader = shared.loader.clone();
    let ctx = ResolutionContext::for_method(
        method.clone(),
        loader.clone(),
        generics,
        shared.caches.clone(),
        Some(Arc::downgrade(&shared)),
    );

    if let Some(body) = &method.method().body {
        let mut exceptions: &[body::Exception] = &[];
        for sec in &body.data_sections {
            use body::DataSection::*;
            match sec {
                Unrecognized { .. } => {}
                ExceptionHandlers(e) => {
                    exceptions = e;
                }
            }
        }

        Ok(MethodInfo {
            is_cctor: method.method().runtime_special_name
                && method.method().name == ".cctor"
                && !method.method().signature.instance
                && method.method().signature.parameters.is_empty(),
            signature: &method.method().signature,
            locals: &body.header.local_variables,
            max_stack: body.header.maximum_stack_size,
            exceptions: dotnet_exceptions::parse(exceptions, &ctx)?
                .into_iter()
                .map(Arc::new)
                .collect(),
            instructions: body.instructions.as_slice(),
            source: method,
        })
    } else {
        Ok(MethodInfo {
            is_cctor: false,
            signature: &method.method().signature,
            locals: &[],
            max_stack: 0,
            exceptions: vec![],
            instructions: &[],
            source: method,
        })
    }
}
