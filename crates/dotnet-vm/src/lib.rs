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
use dotnetdll::prelude::*;
use std::sync::Arc;

#[macro_use]
mod macros;

pub mod context;
pub mod dispatch;
pub mod error;
pub mod exceptions {
    pub use dotnet_exceptions::*;
}
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
pub mod memory;
pub mod metrics;
pub mod pinvoke {
    pub use dotnet_pinvoke::*;
}
pub mod resolution;
pub mod resolver;
mod stack;
pub mod state;
pub mod statics;
pub mod sync;
#[cfg(test)]
mod tail_calls_tests;
pub mod threading;
pub mod tracer;

pub use dotnet_types::{generics::GenericLookup, members::MethodDescription};
pub use dotnet_utils::{
    ArenaId, ArgumentIndex, ByteOffset, FieldIndex, LocalIndex, StackSlotIndex,
};
pub use dotnet_vm_ops::{CollectableMethodDescription, MethodInfo, MethodState, StepResult};
pub use executor::*;
pub use stack::*;
pub use state::ReflectionRegistry;

use context::ResolutionContext;
use state::SharedGlobalState;

/// Constructs a [`MethodInfo`] from a resolved method descriptor.
///
/// This factory function lives in `dotnet-vm` rather than `dotnet-vm-ops` because it
/// requires access to [`ResolutionContext`], [`SharedGlobalState`], and
/// [`exceptions::parse`] — all of which are internal to this crate.
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
            exceptions: exceptions::parse(exceptions, &ctx)?
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
            exceptions: vec![],
            instructions: &[],
            source: method,
        })
    }
}
