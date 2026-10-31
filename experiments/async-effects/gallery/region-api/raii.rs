//! Chosen API: await VM access, then use an RAII capability with rooted views.
use crate::{InvokeSpec, finish_invoke_guarded, prepare_invoke_guarded};
use effect_ops::*;

pub async fn substring(
    mut fx: VmEffects,
    source: StringRoot,
    start: i32,
    length: Option<i32>,
) -> Outcome {
    let count = {
        let mut vm = fx.vm().await?;
        vm.string(&source).substring_len(start, length)?
    };
    let mut result = Vec::with_capacity(count);
    let mut offset = 0;
    const CHUNK_SIZE: usize = 1024 * 64;
    while offset < count {
        let end = (offset + CHUNK_SIZE).min(count);
        {
            let mut vm = fx.vm().await?;
            vm.string(&source).copy_range(
                start as usize + offset,
                start as usize + end,
                &mut result,
            )?;
        }
        offset = end;
    }
    let mut vm = fx.vm().await?;
    Ok(vm.new_string(result))
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
    let call = {
        let mut vm = fx.vm().await?;
        prepare_invoke_guarded(&mut vm, &spec)?
    };
    let result = fx.call_managed(call).await;
    let mut vm = fx.vm().await?;
    finish_invoke_guarded(&mut vm, &spec, result)
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
        let action = {
            let mut vm = fx.vm().await?;
            vm.static_action(ty)
        };
        match action {
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
    let mut vm = fx.vm().await?;
    vm.array_at(&owner, index)
}
