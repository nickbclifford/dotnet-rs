use crate::ThreadingIntrinsicHost;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{error::ExecutionError, generics::GenericLookup, members::MethodDescription};
use dotnet_value::StackValue;
use dotnet_vm_ops::effects::{
    NativeCall, NativeInvocation, NativeOperandView, NativeResult, NativeSelection, NativeValue,
    RootedValue, VmEffects,
};
use std::time::{Duration, Instant};

fn reject() -> NativeSelection {
    NativeSelection::Reject(ExecutionError::InternalError("invalid event operands".into()).into())
}

pub fn create_select<'gc, T: ThreadingIntrinsicHost<'gc>>(
    _: &T,
    operands: NativeOperandView<'_, 'gc>,
    method: &MethodDescription,
    _: &GenericLookup,
) -> NativeSelection {
    match (operands.peek_at(1), operands.peek_at(0)) {
        (StackValue::Int32(initial), StackValue::Int32(mode)) => {
            NativeSelection::Async(NativeInvocation {
                start: initial as u32 as usize,
                length: mode as u32 as usize,
                parameter_count: method.signature().parameters.len(),
            })
        }
        _ => reject(),
    }
}

fn call(method: MethodDescription, arguments: Vec<RootedValue>) -> NativeCall {
    NativeCall {
        method,
        generics: GenericLookup::default(),
        arguments,
    }
}

#[dotnet_intrinsic("void System.Threading.EventWaitHandle::CreateEventCore(bool, valuetype System.Threading.EventResetMode)", select = create_select)]
pub async fn create_event(mut fx: VmEffects) -> NativeResult {
    let initial = fx.invocation().start != 0;
    let mode = fx.invocation().length as u32 as i32;
    let (receiver, handle, id, zero, methods) = {
        let mut vm = fx.vm().await?;
        vm.discard_scalars(2)?;
        let receiver = vm.pop_object()?;
        if !(0..=1).contains(&mode) {
            return Err(vm.exception("System.ArgumentException", "Invalid event reset mode.")?);
        }
        let (handle, id, zero, methods) = vm.with(|region| {
            let methods = region.event_methods()?;
            let (handle, id) = region.event_create(mode == 1, initial)?;
            let zero = region.root(StackValue::Int32(0))?;
            Ok::<_, dotnet_types::error::VmError>((handle, id, zero, methods))
        })??;
        (receiver, handle, id, zero, methods)
    };
    fx.call(call(methods.constructor, vec![handle.clone(), id, zero]))
        .await?;
    fx.call(call(methods.setter, vec![receiver, handle]))
        .await?;
    Ok(())
}

async fn update_event(mut fx: VmEffects, instance: bool, set: bool) -> NativeResult {
    let (receiver, methods) = {
        let mut vm = fx.vm().await?;
        let object = vm.pop_object()?;
        let methods = vm.with(|region| region.event_methods())??;
        (object, methods)
    };
    let handle = if instance {
        match fx.call(call(methods.getter, vec![receiver])).await? {
            NativeValue::Value(value) => value,
            NativeValue::Void => {
                return Err(dotnet_vm_ops::effects::NativeFailure::Host(
                    ExecutionError::InternalError("event handle getter returned void".into())
                        .into(),
                ));
            }
        }
    } else {
        receiver
    };
    let closed = match fx
        .call(call(methods.is_closed, vec![handle.clone()]))
        .await?
    {
        NativeValue::Value(value) => {
            fx.vm().await?.with(|region| match region.fetch(&value)? {
                StackValue::Int32(value) => Ok::<_, dotnet_types::error::VmError>(value != 0),
                _ => Err(
                    ExecutionError::InternalError("invalid SafeHandle closed state".into()).into(),
                ),
            })??
        }
        NativeValue::Void => {
            return Err(dotnet_vm_ops::effects::NativeFailure::Host(
                ExecutionError::InternalError("SafeHandle state returned void".into()).into(),
            ));
        }
    };
    if closed {
        return Err(fx.vm().await?.exception(
            "System.ObjectDisposedException",
            "Cannot access a disposed object.",
        )?);
    }
    let id = match fx.call(call(methods.get_handle, vec![handle])).await? {
        NativeValue::Value(value) => {
            fx.vm().await?.with(|region| match region.fetch(&value)? {
                StackValue::NativeInt(value) => Ok::<_, dotnet_types::error::VmError>(value),
                _ => Err(ExecutionError::InternalError("invalid SafeHandle value".into()).into()),
            })??
        }
        NativeValue::Void => {
            return Err(dotnet_vm_ops::effects::NativeFailure::Host(
                ExecutionError::InternalError("SafeHandle value returned void".into()).into(),
            ));
        }
    };
    let mut vm = fx.vm().await?;
    vm.with(|region| region.event_update(id, set))??;
    vm.push_bool(true)?;
    Ok(())
}

#[dotnet_intrinsic("bool System.Threading.EventWaitHandle::Set()")]
pub async fn event_set(fx: VmEffects) -> NativeResult {
    update_event(fx, true, true).await
}
#[dotnet_intrinsic("bool System.Threading.EventWaitHandle::Reset()")]
pub async fn event_reset(fx: VmEffects) -> NativeResult {
    update_event(fx, true, false).await
}
#[dotnet_intrinsic(
    "static bool System.Threading.EventWaitHandle::Set(Microsoft.Win32.SafeHandles.SafeWaitHandle)"
)]
pub async fn event_set_safe(fx: VmEffects) -> NativeResult {
    update_event(fx, false, true).await
}

pub fn wait_select<'gc, T: ThreadingIntrinsicHost<'gc>>(
    _: &T,
    operands: NativeOperandView<'_, 'gc>,
    method: &MethodDescription,
    _: &GenericLookup,
) -> NativeSelection {
    match (
        operands.peek_at(2),
        operands.peek_at(1),
        operands.peek_at(0),
    ) {
        (StackValue::NativeInt(id), StackValue::Int32(timeout), StackValue::Int32(_)) => {
            NativeSelection::Async(NativeInvocation {
                start: id as usize,
                length: timeout as u32 as usize,
                parameter_count: method.signature().parameters.len(),
            })
        }
        _ => reject(),
    }
}

#[dotnet_intrinsic("static int System.Threading.WaitHandle::WaitOneCore(nint, int, bool)", select = wait_select)]
pub async fn wait_one(mut fx: VmEffects) -> NativeResult {
    let id = fx.invocation().start as isize;
    let timeout = fx.invocation().length as u32 as i32;
    let deadline = (timeout >= 0).then(|| Instant::now() + Duration::from_millis(timeout as u64));
    fx.vm().await?.discard_scalars(3)?;
    let signaled = fx.event_wait(id, deadline).await?;
    fx.vm()
        .await?
        .with(|region| region.push_int(if signaled { 0 } else { 258 }))??;
    Ok(())
}
