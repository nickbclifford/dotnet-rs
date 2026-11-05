//! Selects and lays out the frame homes of locals whose address is taken.
//!
//! See [`dotnet_vm_data::frame_homes`] for the storage and its contract.

use crate::{context::ResolutionContext, resolution::TypeResolutionExt, sync::Arc};
use dotnet_runtime_resolver::LayoutFactory;
use dotnet_types::{error::TypeResolutionError, members::MethodDescription};
use dotnet_value::layout::LayoutManager;
use dotnet_vm_data::frame_homes::{HomeValueType, LocalHome, LocalHomeLayout};
use dotnetdll::prelude::*;

/// Lays out a home for every local the method body names in `ldloca`.
///
/// Byref-typed and `TypedReference` locals keep their evaluation-stack slots: neither can be
/// encoded in a home block. A value-type local is encoded with the instance field layout that
/// `init_locals` gives its default value, so loading a home rebuilds the same value.
pub(crate) fn build_local_homes(
    method: &MethodDescription,
    locals: &[LocalVariable],
    instructions: &[Instruction],
    ctx: &ResolutionContext,
) -> Result<Option<Arc<LocalHomeLayout>>, TypeResolutionError> {
    let mut addressed = vec![false; locals.len()];
    for instruction in instructions {
        if let Instruction::LoadLocalAddress(index) = instruction
            && let Some(flag) = addressed.get_mut(usize::from(*index))
        {
            *flag = true;
        }
    }
    if !addressed.contains(&true) {
        return Ok(None);
    }

    let mut homes: Vec<Option<LocalHome>> = Vec::with_capacity(locals.len());
    let mut fields = Vec::new();
    for (index, local) in locals.iter().enumerate() {
        let LocalVariable::Variable {
            by_ref: false,
            var_type,
            ..
        } = local
        else {
            homes.push(None);
            continue;
        };
        if !addressed[index] {
            homes.push(None);
            continue;
        }
        let concrete = ctx.make_concrete(var_type)?;
        let (layout, value_type) = match concrete.get() {
            BaseType::Type { .. } => {
                let description = ctx.loader().find_concrete_type(concrete.clone())?;
                if !description.is_value_type(ctx)? {
                    (ctx.resolver().type_layout_cached(concrete, ctx)?, None)
                } else if description.type_name() == "System.TypedReference" {
                    homes.push(None);
                    continue;
                } else {
                    let mut generics = concrete.make_lookup();
                    generics.set_method_generics(ctx.generics.method_generics.clone());
                    let field_layout = ctx
                        .resolver()
                        .instance_field_layout_cached_with_lookup(description.clone(), &generics)?;
                    (
                        Arc::new(LayoutManager::Field((*field_layout).clone())),
                        Some(HomeValueType {
                            description,
                            generics,
                            fields: field_layout,
                        }),
                    )
                }
            }
            _ => (ctx.resolver().type_layout_cached(concrete, ctx)?, None),
        };
        fields.push((index, layout.clone()));
        homes.push(Some(LocalHome {
            offset: 0,
            layout,
            value_type,
        }));
    }
    if fields.is_empty() {
        return Ok(None);
    }

    let names: Vec<String> = fields
        .iter()
        .map(|(index, _)| format!("V_{index}"))
        .collect();
    let block = LayoutFactory::create_field_layout(
        fields.iter().zip(&names).map(|((_, layout), name)| {
            (method.parent.clone(), name.as_str(), None, layout.clone())
        }),
        Layout::Automatic,
        0,
        1,
    )?;
    for ((local, _), name) in fields.iter().zip(&names) {
        let position = block
            .fields
            .iter()
            .find(|(key, _)| key.name == *name)
            .map(|(_, field)| field.position.as_usize())
            .expect("every home is placed in the block layout");
        homes[*local]
            .as_mut()
            .expect("placed locals have homes")
            .offset = position;
    }
    Ok(Some(Arc::new(LocalHomeLayout::new(
        block,
        homes.into_boxed_slice(),
    ))))
}
