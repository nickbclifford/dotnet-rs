//! `MethodImplOptions.Synchronized` methods (ECMA-335 §II.23.1.11, §I.12.4.1.5).
//!
//! A synchronized method holds the monitor of `this` (instance) or of the declaring type's
//! `Type` object (static) from entry until it exits by any path. Entry reuses the
//! `Monitor.Enter` intrinsic, so a contended acquisition parks through the same native-effect
//! path as an explicit `Monitor.Enter`. The method's instruction stream reserves index 0 for
//! that call (see `MethodInfo::synchronized`); the monitor is released when the frame returns or
//! unwinds.

use crate::{
    StepResult,
    stack::{
        context::VesContext,
        ops::{ArgumentOps, EvalStackOps, ExceptionOps, ReflectionLookupOps, ThreadOps, VmCallOps},
    },
};
use dotnet_types::{
    generics::GenericLookup,
    members::MethodDescription,
    runtime::{RuntimeType, runtime_type_from_concrete},
};
use dotnet_utils::ArgumentIndex;
use dotnet_value::{StackValue, object::ObjectRef};
use dotnet_vm_ops::NULL_REF_MSG;
use dotnetdll::prelude::MethodMemberIndex;

impl<'a, 'gc> VesContext<'a, 'gc> {
    /// The `static void Monitor.Enter(object)` overload.
    fn monitor_enter_method(&self) -> Result<MethodDescription, StepResult> {
        let monitor = self
            .shared
            .loader
            .corlib_type("System.Threading.Monitor")
            .map_err(|error| StepResult::Error(error.into()))?;
        monitor
            .definition()
            .methods
            .iter()
            .enumerate()
            .find_map(|(index, method)| {
                if method.name != "Enter" {
                    return None;
                }
                let desc = MethodDescription::new(
                    monitor.clone(),
                    GenericLookup::default(),
                    monitor.resolution.clone(),
                    MethodMemberIndex::Method(index),
                );
                let signature = desc.signature();
                (!signature.instance && signature.parameters.len() == 1).then_some(desc)
            })
            .ok_or_else(|| StepResult::not_implemented("System.Threading.Monitor.Enter(object)"))
    }

    /// The `Type` object a static synchronized method locks: the closed declaring type.
    fn synchronized_type_object(&mut self) -> Result<ObjectRef<'gc>, StepResult> {
        let frame = self.frame_stack.current_frame();
        let parent = frame.state.info_handle.source.parent.clone();
        let arity = parent.definition().generic_parameters.len();
        let runtime_type = if arity == 0 {
            RuntimeType::Type(parent)
        } else {
            let arguments = frame
                .generic_inst
                .type_generics
                .iter()
                .take(arity)
                .map(|argument| runtime_type_from_concrete(self.shared.loader.as_ref(), argument))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| {
                    StepResult::internal_error("synchronized method on an unresolved generic type")
                })?;
            RuntimeType::Generic(parent, arguments)
        };
        Ok(self.get_runtime_type(runtime_type))
    }

    pub(crate) fn enter_synchronized_method(&mut self) -> StepResult {
        let frame = self.frame_stack.current_frame();
        // Only the reserved slot acquires; a `break` elsewhere is a debugger hook.
        if frame.state.ip != 0 || !frame.state.info_handle.synchronized {
            return StepResult::Continue;
        }
        // A yielded or parked acquisition re-runs this slot with the monitor object still on
        // the stack, so it is pushed once.
        if frame.sync_lock.is_none() {
            let lock = if frame.state.info_handle.signature.instance {
                match self.get_argument(ArgumentIndex::new(0)) {
                    StackValue::ObjectRef(object) => object,
                    // A value-type receiver is a managed pointer with no object identity to
                    // lock; the method runs unsynchronized.
                    _ => return StepResult::Continue,
                }
            } else {
                match self.synchronized_type_object() {
                    Ok(object) => object,
                    Err(step) => return step,
                }
            };
            if lock.0.is_none() {
                return self
                    .throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
            }
            self.frame_stack.current_frame_mut().sync_lock = Some(lock);
            self.push(StackValue::ObjectRef(lock));
        }
        match self.monitor_enter_method() {
            Ok(enter) => self.dispatch_method_inline(enter, GenericLookup::default()),
            Err(step) => step,
        }
    }

    /// Releases the monitor a synchronized frame holds, if any. The lock is released after the
    /// method's own handlers have run, matching a JIT-emitted `try`/`finally` around the body.
    pub(crate) fn release_synchronized_lock(&mut self, lock: Option<ObjectRef<'gc>>) {
        let Some(lock) = lock else { return };
        let thread_id = ThreadOps::thread_id(self);
        if let Some(block) = lock.sync_block_index().and_then(|index| {
            use crate::sync::SyncManagerOps;
            self.shared.sync_blocks.get_sync_block(index)
        }) {
            use crate::sync::SyncBlockOps;
            // Exit fails only if this frame never completed its acquisition (an aborted
            // thread), in which case there is nothing to release.
            let _ = block.exit(thread_id);
        }
    }
}
