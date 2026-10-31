//! Candidate B: GAT host, same handler and region guarantees. Compiled by effect-intrinsics.
use crate::{InvokeSpec, finish_invoke, prepare_invoke};
use effect_ops::*;

// search.rs:60. The spawn shim already removed/stashed the receiver and indices.
pub async fn substring<H: NativeHost>(
    mut fx: H,
    source: StringRoot,
    start: i32,
    length: Option<i32>,
) -> Outcome {
    let count = fx.enter(|c| {
        let source = source.get(c);
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
    })?;
    let mut result = Vec::with_capacity(count);
    let mut offset = 0;
    const CHUNK_SIZE: usize = 1024 * 64;
    while offset < count {
        let end = (offset + CHUNK_SIZE).min(count);
        let requested = fx.enter(|c| {
            let source = source.get(c);
            {
                let source = source.borrow();
                let ObjectData::String(chars) = &*source else {
                    unreachable!()
                };
                result.extend_from_slice(&chars[start as usize + offset..start as usize + end]);
            }
            c.host().copied(end - offset);
            c.requested()
        });
        offset = end;
        if offset < count && requested {
            fx.perform(Effect::SafePoint).await?;
        }
    }
    Ok(fx.enter(|c| c.string(result).0))
}

// gc.rs:199. Production handler probes first; an already-complete wait does not park.
pub async fn wait_finalizers<H: NativeHost>(mut fx: H) -> Outcome {
    fx.perform(Effect::WaitFinalizers).await?;
    Ok(Stashed::Void)
}

// monitor.rs:112. This port is Enter(object); ReliableEnter's arbitrary byref stays legacy.
pub async fn monitor_enter<H: NativeHost>(mut fx: H, object: Stashed) -> Outcome {
    if matches!(object, Stashed::Null) {
        return Err(Failure::Managed("NullReferenceException"));
    }
    fx.perform(Effect::MonitorAcquire {
        object,
        deadline: None,
    })
    .await?;
    Ok(Stashed::Void)
}

// methods.rs:375 + context.rs:382. The args array owns the byref cells.
pub async fn reflection_invoke<H: NativeHost>(mut fx: H, spec: InvokeSpec) -> Outcome {
    let call = fx.enter(|c| prepare_invoke(c, &spec))?;
    let result = fx.perform(Effect::CallManaged(call)).await;
    // Byref writes are through owned cells. A transparent throw unwinds this native frame;
    // normal completion alone reaches return boxing, matching the existing remote hook.
    fx.enter(|c| finish_invoke(c, &spec, result))
}

// invoke.rs:24. Normal delegate method resolution still supplies each prepared target.
pub async fn multicast<H: NativeHost>(
    mut fx: H,
    methods: Vec<usize>,
    args: Vec<Stashed>,
) -> Outcome {
    let mut last = Stashed::Void;
    for method in methods {
        last = fx
            .perform(Effect::CallManaged(Call {
                propagation: Propagation::Transparent,
                method,
                args: args.clone(),
                byrefs: vec![],
            }))
            .await?;
    }
    Ok(last)
}

// statics.rs:476. Deadlock policy lives in the handler; the native future owns no graph guard.
pub async fn ensure_static<H: NativeHost>(mut fx: H, ty: usize) -> Outcome {
    loop {
        match fx.enter(|c| c.host().static_action(ty)) {
            StaticAction::Ready => return Ok(Stashed::Void),
            StaticAction::Failed => return Err(Failure::Managed("TypeInitializationException")),
            StaticAction::Run(method) => {
                // VM marks initialized/failed on .cctor frame retirement, as today.
                fx.perform(Effect::CallManaged(Call {
                    propagation: Propagation::Transparent,
                    method,
                    args: vec![],
                    byrefs: vec![],
                }))
                .await?;
            }
            StaticAction::Wait => {
                fx.perform(Effect::WaitStaticInit { ty }).await?;
            }
        }
    }
}

// Seventh probe: an interior heap slot, recreated from a rooted owner plus checked index.
// This deliberately does not accept ManagedPtr or StackValue::ManagedPtr.
pub async fn heap_slot_after_wait<H: NativeHost>(
    mut fx: H,
    owner: ArrayRoot,
    index: usize,
) -> Outcome {
    fx.perform(Effect::SafePoint).await?;
    fx.enter(|c| {
        let object = owner.get(c);
        let object = object.borrow();
        let ObjectData::Array(values) = &*object else {
            return Err(Failure::Managed("ArgumentException"));
        };
        let value = *values
            .get(index)
            .ok_or(Failure::Managed("IndexOutOfRangeException"))?;
        Ok(c.stash(value))
    })
}
