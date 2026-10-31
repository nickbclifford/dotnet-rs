//! Compiled gallery bodies: this crate depends only on effect-ops.
use effect_ops::*;
#[cfg(feature = "concrete")]
#[path = "../../../gallery/concrete/ports.rs"]
pub mod concrete;
#[cfg(feature = "gat")]
#[path = "../../../gallery/gat/ports.rs"]
pub mod gat;
#[derive(Clone, Copy)]
pub enum ReturnKind {
    Void,
    Reference,
    Value,
    Constructor,
}
pub struct InvokeSpec {
    pub method: usize,
    pub receiver: Stashed,
    pub parameters: ArrayRoot,
    pub byref_indices: Vec<usize>,
    pub reference_byref_indices: Vec<usize>,
    pub return_kind: ReturnKind,
}
/// Reduced metadata/unboxing model shared by the chosen API and comparisons. Real helper stays in
/// reflection crate. Value byrefs use boxed i64; reference byrefs address array slots, rather than arbitrary CLR layouts.
pub fn prepare_invoke<'gc>(
    c: &mut impl RegionAccess<'gc>,
    spec: &InvokeSpec,
) -> Result<Call, Failure> {
    let owner = spec.parameters.get(c);
    let values = {
        let data = owner.borrow();
        let ObjectData::Array(values) = &*data else {
            return Err(Failure::Managed("ArgumentException"));
        };
        values.clone()
    };
    let mut args = vec![];
    if !matches!(spec.receiver, Stashed::Null) {
        args.push(spec.receiver.clone());
    }
    let mut byrefs = vec![];
    for (index, mut value) in values.into_iter().enumerate() {
        if spec.byref_indices.contains(&index) {
            // Model accepts an unboxed scalar as shorthand for a boxed argument.
            // Existing boxes retain identity. Null value-type refs get zero-filled backing.
            let replacement = match value {
                Value::Int(n) => Some(new_box(c, n)),
                Value::Null => Some(new_box(c, 0)),
                Value::Object(_) => None,
                Value::Void => return Err(Failure::Managed("ArgumentException")),
            };
            if let Some(cell) = replacement {
                value = Value::Object(cell);
                let mut data = owner.borrow_mut(c.mutation());
                let ObjectData::Array(values) = &mut *data else {
                    unreachable!()
                };
                values[index] = value;
            }
            byrefs.push(ByRefArg {
                owner: spec.parameters.0.clone(),
                index,
                kind: ByRefKind::BoxedValue,
            });
        }
        if spec.reference_byref_indices.contains(&index) {
            byrefs.push(ByRefArg {
                owner: spec.parameters.0.clone(),
                index,
                kind: ByRefKind::ObjectSlot,
            });
        }
        // No object lock is held during root-cell allocation.
        args.push(c.stash(value));
    }
    Ok(Call {
        propagation: Propagation::Transparent,
        method: spec.method,
        args,
        byrefs,
    })
}
pub fn finish_invoke<'gc>(
    c: &mut impl RegionAccess<'gc>,
    spec: &InvokeSpec,
    result: Outcome,
) -> Outcome {
    // Mutation was through rooted owner cells, not detached copy-in/copy-out snapshots.
    // Compatibility policy: preserve existing VM exception propagation (no new wrapper).
    let result = result?;
    match spec.return_kind {
        ReturnKind::Void => Ok(Stashed::Null),
        ReturnKind::Reference => Ok(result),
        ReturnKind::Constructor => Ok(spec.receiver.clone()),
        ReturnKind::Value => Ok(c.stash(Value::Object(effect_ops::new_box(c, result.int())))),
    }
}

#[cfg(feature = "concrete")]
#[path = "../../../gallery/region-api/checkpoint.rs"]
pub mod region_checkpoint;
#[cfg(feature = "concrete")]
#[path = "../../../gallery/region-api/vm_await.rs"]
pub mod region_vm_await;
#[cfg(feature = "concrete")]
#[path = "../../../gallery/region-api/with_vm.rs"]
pub mod region_with_vm;

#[cfg(feature = "concrete")]
#[path = "../../../gallery/region-api/raii.rs"]
pub mod region_raii;

pub fn prepare_invoke_guarded(
    access: &mut VmAccess<'_>,
    spec: &InvokeSpec,
) -> Result<Call, Failure> {
    access.with(|cx| prepare_invoke(cx, spec))
}
pub fn finish_invoke_guarded(
    access: &mut VmAccess<'_>,
    spec: &InvokeSpec,
    result: Outcome,
) -> Outcome {
    access.with(|cx| finish_invoke(cx, spec, result))
}

/// Selected native intrinsic implementations, using `VmEffects::vm` and RAII views.
#[cfg(feature = "concrete")]
pub use region_raii as chosen;
