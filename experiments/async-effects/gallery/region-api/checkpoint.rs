//! Region API trial C: awaitable, fallible VM access with an entry checkpoint.
use crate::{InvokeSpec, finish_invoke, prepare_invoke};
use effect_ops::*;

pub async fn substring(
    mut fx: VmEffects,
    source: StringRoot,
    start: i32,
    length: Option<i32>,
) -> Outcome {
    let count = fx
        .checkpoint_vm(|cx| {
            let source = source.get(cx);
            let source = source.borrow();
            let ObjectData::String(chars) = &*source else {
                return Err(Failure::Managed("ArgumentException"));
            };
            if start < 0 || start as usize > chars.len() {
                return Err(Failure::Managed("ArgumentOutOfRangeException:startIndex"));
            }
            let count = length.unwrap_or((chars.len() - start as usize) as i32);
            if count < 0 || count as usize > chars.len() - start as usize {
                return Err(Failure::Managed("ArgumentOutOfRangeException:length"));
            }
            Ok(count as usize)
        })
        .await?;
    let mut result = Vec::with_capacity(count);
    let mut offset = 0;
    const CHUNK_SIZE: usize = 1024 * 64;
    while offset < count {
        let end = (offset + CHUNK_SIZE).min(count);
        fx.checkpoint_vm(|cx| {
            let source = source.get(cx);
            {
                let source = source.borrow();
                let ObjectData::String(chars) = &*source else {
                    unreachable!()
                };
                result.extend_from_slice(&chars[start as usize + offset..start as usize + end]);
            }
            cx.host().copied(end - offset);
            Ok(())
        })
        .await?;
        offset = end;
    }
    fx.checkpoint_vm(|cx| Ok(cx.string(result).0)).await
}

pub async fn wait_finalizers(mut fx: VmEffects) -> Outcome {
    fx.wait_finalizers().await?;
    Ok(Stashed::Void)
}

pub async fn monitor_enter(mut fx: VmEffects, object: Stashed) -> Outcome {
    if matches!(object, Stashed::Null) {
        return Err(Failure::Managed("NullReferenceException"));
    }
    fx.monitor_acquire(object, None).await?;
    Ok(Stashed::Void)
}

pub async fn reflection_invoke(mut fx: VmEffects, spec: InvokeSpec) -> Outcome {
    let call = fx.checkpoint_vm(|cx| prepare_invoke(cx, &spec)).await?;
    let result = fx.call_managed(call).await;
    fx.checkpoint_vm(|cx| finish_invoke(cx, &spec, result))
        .await
}

pub async fn multicast(mut fx: VmEffects, methods: Vec<usize>, args: Vec<Stashed>) -> Outcome {
    let mut last = Stashed::Void;
    for method in methods {
        last = fx
            .call_managed(Call {
                propagation: Propagation::Transparent,
                method,
                args: args.clone(),
                byrefs: vec![],
            })
            .await?;
    }
    Ok(last)
}

pub async fn ensure_static(mut fx: VmEffects, ty: usize) -> Outcome {
    loop {
        match fx
            .checkpoint_vm(|cx| Ok(cx.host().static_action(ty)))
            .await?
        {
            StaticAction::Ready => return Ok(Stashed::Void),
            StaticAction::Failed => return Err(Failure::Managed("TypeInitializationException")),
            StaticAction::Run(method) => {
                fx.call_managed(Call {
                    propagation: Propagation::Transparent,
                    method,
                    args: vec![],
                    byrefs: vec![],
                })
                .await?;
            }
            StaticAction::Wait => {
                fx.wait_static_init(ty).await?;
            }
        }
    }
}

pub async fn heap_slot_after_wait(mut fx: VmEffects, owner: ArrayRoot, index: usize) -> Outcome {
    fx.safe_point().await?;
    fx.checkpoint_vm(|cx| {
        let object = owner.get(cx);
        let object = object.borrow();
        let ObjectData::Array(values) = &*object else {
            return Err(Failure::Managed("ArgumentException"));
        };
        let value = *values
            .get(index)
            .ok_or(Failure::Managed("IndexOutOfRangeException"))?;
        Ok(cx.stash(value))
    })
    .await
}
