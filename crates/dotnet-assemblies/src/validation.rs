use crate::error::AssemblyLoadError;
use dotnetdll::prelude::*;

/// Validates critical ECMA-335 §II.22 metadata table constraints.
pub fn validate_metadata(res: &Resolution) -> Result<(), AssemblyLoadError> {
    // §II.22.30: Module table must have exactly one row.
    // dotnetdll represents this as exactly one 'module' field.
    if res.module.name.is_empty() {
        return Err(AssemblyLoadError::InvalidFormat(
            "Module name is empty".into(),
        ));
    }

    // §II.22.2: Assembly table must have at most one row.
    // In dotnetdll, this is Option<Assembly>.
    if let Some(assembly) = &res.assembly
        && assembly.name.is_empty()
    {
        return Err(AssemblyLoadError::InvalidFormat(
            "Assembly name is empty".into(),
        ));
    }

    // §II.22.37: TypeDef table.
    // At least the <Module> type must exist.
    if res.type_definitions.is_empty() {
        return Err(AssemblyLoadError::InvalidFormat(
            "TypeDef table is empty".into(),
        ));
    }

    for (i, td) in res.type_definitions.iter().enumerate() {
        if td.name.is_empty() {
            return Err(AssemblyLoadError::InvalidFormat(format!(
                "TypeDef[{}] name is empty",
                i
            )));
        }

        // §II.22.37: Direct self-inheritance check.
        if let Some(extends) = &td.extends {
            let base_handle = match extends {
                TypeSource::User(h) => Some(h),
                TypeSource::Generic { base, .. } => Some(base),
            };

            if let Some(UserType::Definition(base_idx)) = base_handle
                && let Some(current_idx) = res.type_definition_index(i)
                && *base_idx == current_idx
            {
                return Err(AssemblyLoadError::InvalidFormat(format!(
                    "TypeDef '{}' (index {:?}) extends itself",
                    td.name, current_idx
                )));
            }
        }

        // Validate fields for empty names
        for (f_idx, field) in td.fields.iter().enumerate() {
            if field.name.is_empty() {
                return Err(AssemblyLoadError::InvalidFormat(format!(
                    "TypeDef '{}' has field at index {} with empty name",
                    td.name, f_idx
                )));
            }
        }

        // Validate methods for empty names
        for (m_idx, method) in td.methods.iter().enumerate() {
            if method.name.is_empty() {
                return Err(AssemblyLoadError::InvalidFormat(format!(
                    "TypeDef '{}' has method at index {} with empty name",
                    td.name, m_idx
                )));
            }
        }
    }

    Ok(())
}
