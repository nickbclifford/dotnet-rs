//! Selects and lays out the frame homes of locals and arguments whose address is taken.
//!
//! See [`dotnet_vm_data::frame_homes`] for the storage and its contract.

use crate::{context::ResolutionContext, resolution::TypeResolutionExt, sync::Arc};
use dotnet_runtime_resolver::LayoutFactory;
use dotnet_types::{error::TypeResolutionError, members::MethodDescription};
use dotnet_value::layout::{LayoutManager, Scalar};
use dotnet_vm_data::frame_homes::{FrameHomeLayout, HomeValueType, VariableHome};
use dotnetdll::prelude::*;

/// Lays out a home for every local the method body names in `ldloca` and every argument it
/// names in `ldarga`.
///
/// Byref-typed and `TypedReference` variables, and the `this` byref of a value-type method,
/// keep their evaluation-stack slots: none of them can be encoded in a home block. A
/// value-type variable is encoded with its instance field layout, so loading a home rebuilds
/// the value `init_locals` or the caller supplied.
pub(crate) fn build_frame_homes(
    method: &MethodDescription,
    locals: &[LocalVariable],
    instructions: &[Instruction],
    ctx: &ResolutionContext,
) -> Result<Option<Arc<FrameHomeLayout>>, TypeResolutionError> {
    let signature = method.signature();
    let has_this = signature.instance && !signature.explicit_this;
    let argument_count = usize::from(has_this) + signature.parameters.len();

    let mut addressed_locals = vec![false; locals.len()];
    let mut addressed_arguments = vec![false; argument_count];
    for instruction in instructions {
        let (flags, index) = match instruction {
            Instruction::LoadLocalAddress(index) => (&mut addressed_locals, *index),
            Instruction::LoadArgumentAddress(index) => (&mut addressed_arguments, *index),
            _ => continue,
        };
        if let Some(flag) = flags.get_mut(usize::from(index)) {
            *flag = true;
        }
    }
    if !addressed_locals.contains(&true) && !addressed_arguments.contains(&true) {
        return Ok(None);
    }

    let mut fields: Vec<(String, Arc<LayoutManager>)> = Vec::new();
    let mut local_homes = Vec::with_capacity(locals.len());
    for (index, local) in locals.iter().enumerate() {
        let home = match local {
            LocalVariable::Variable {
                by_ref: false,
                var_type,
                ..
            } if addressed_locals[index] => variable_home(var_type, ctx)?,
            _ => None,
        };
        local_homes.push(home.inspect(|home| {
            fields.push((format!("V_{index}"), home.layout.clone()));
        }));
    }

    let mut argument_homes = Vec::with_capacity(argument_count);
    for (index, addressed) in addressed_arguments.iter().enumerate() {
        let home = if !addressed {
            None
        } else if has_this && index == 0 {
            this_home(method, ctx)?
        } else {
            match &signature.parameters[index - usize::from(has_this)].1 {
                ParameterType::Value(var_type) => variable_home(var_type, ctx)?,
                ParameterType::Ref(_) | ParameterType::TypedReference => None,
            }
        };
        argument_homes.push(home.inspect(|home| {
            fields.push((format!("A_{index}"), home.layout.clone()));
        }));
    }
    if fields.is_empty() {
        return Ok(None);
    }

    let block = LayoutFactory::create_field_layout(
        fields
            .iter()
            .map(|(name, layout)| (method.parent.clone(), name.as_str(), None, layout.clone())),
        Layout::Automatic,
        0,
        1,
    )?;
    let position = |name: String| {
        block
            .fields
            .iter()
            .find(|(key, _)| key.name == name)
            .map(|(_, field)| field.position.as_usize())
            .expect("every home is placed in the block layout")
    };
    for (index, home) in local_homes.iter_mut().enumerate() {
        if let Some(home) = home {
            home.offset = position(format!("V_{index}"));
        }
    }
    for (index, home) in argument_homes.iter_mut().enumerate() {
        if let Some(home) = home {
            home.offset = position(format!("A_{index}"));
        }
    }
    Ok(Some(Arc::new(FrameHomeLayout::new(
        block,
        local_homes.into_boxed_slice(),
        argument_homes.into_boxed_slice(),
    ))))
}

/// The home of a variable of `var_type`, or `None` for a `TypedReference`.
fn variable_home(
    var_type: &MethodType,
    ctx: &ResolutionContext,
) -> Result<Option<VariableHome>, TypeResolutionError> {
    let concrete = ctx.make_concrete(var_type)?;
    let (layout, value_type) = match concrete.get() {
        BaseType::Type { .. } => {
            let description = ctx.loader().find_concrete_type(concrete.clone())?;
            if !description.is_value_type(ctx)? {
                (ctx.resolver().type_layout_cached(concrete, ctx)?, None)
            } else if description.type_name() == "System.TypedReference" {
                return Ok(None);
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
    Ok(Some(VariableHome {
        offset: 0,
        layout,
        value_type,
    }))
}

/// The home of `this`: an object reference for a reference-type method. A value-type method's
/// `this` is a byref and keeps its slot.
fn this_home(
    method: &MethodDescription,
    ctx: &ResolutionContext,
) -> Result<Option<VariableHome>, TypeResolutionError> {
    if method.parent.is_value_type(ctx)? {
        return Ok(None);
    }
    Ok(Some(VariableHome {
        offset: 0,
        layout: Arc::new(LayoutManager::Scalar(Scalar::ObjectRef)),
        value_type: None,
    }))
}
