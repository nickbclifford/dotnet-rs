use crate::build_support::parser::{InstructionEntry, IntrinsicEntry};
use std::{collections::HashMap, ffi::OsStr, fs, path::Path};

pub fn generate_instruction_table(out_dir: &OsStr, entries: &[InstructionEntry]) {
    let dest_path = Path::new(out_dir).join("instruction_dispatch.rs");
    let mut table_code = String::new();
    table_code.push_str("pub fn dispatch_monomorphic<'gc, T: crate::stack::ops::VesOps<'gc>>(\n");
    table_code.push_str("    ctx: &mut T,\n");
    table_code.push_str("    instr: &Instruction,\n");
    table_code.push_str(") -> crate::StepResult {\n");
    table_code.push_str("    match instr {\n");

    for entry in entries {
        let handler_path: syn::Path = syn::parse_str(&format!(
            "{}::{}",
            entry.mod_path, entry.parsed.handler_name
        ))
        .expect("Failed to parse handler path");

        let extra_arg_info: Vec<_> = entry
            .parsed
            .extra_arg_info
            .iter()
            .map(|(id, is_ref)| (id, *is_ref))
            .collect();

        let arm = entry
            .parsed
            .mapping
            .to_match_arm_path(&handler_path, &extra_arg_info);
        table_code.push_str(&format!("        {},\n", arm));
    }

    table_code.push_str("        _ => crate::StepResult::Error(crate::error::VmError::Execution(crate::error::ExecutionError::NotImplemented(format!(\"{:?}\", instr).into())))\n");
    table_code.push_str("    }\n");
    table_code.push_str("}\n");

    fs::write(dest_path, table_code).unwrap();
}

pub fn generate_intrinsic_phf(out_dir: &OsStr, entries: &[IntrinsicEntry]) {
    let phf_path = Path::new(out_dir).join("intrinsics_phf.rs");
    let dispatch_path = Path::new(out_dir).join("intrinsics_dispatch.rs");

    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        let key = if entry.is_field {
            format!(
                "F:{}::{}:{}",
                entry.type_name,
                entry.member_name,
                intrinsic_static_key_segment(entry.is_static)
            )
        } else {
            format!(
                "M:{}::{}#{}:{}",
                entry.type_name,
                entry.member_name,
                entry.arity,
                intrinsic_static_key_segment(entry.is_static)
            )
        };
        groups.entry(key).or_default().push(index);
    }

    let mut sorted_entries = Vec::new();
    let mut group_ranges = Vec::new();
    let mut current_idx = 0;
    let mut keys: Vec<_> = groups.keys().collect();
    keys.sort();

    for key in keys {
        let indices = &groups[key];
        let range = (current_idx, indices.len());
        group_ranges.push((key.clone(), range));
        for &index in indices {
            sorted_entries.push(&entries[index]);
        }
        current_idx += indices.len();
    }

    // 1. Generate IDs and Dispatcher
    let mut dispatch_code = String::new();

    dispatch_code.push_str("#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]\n");
    dispatch_code.push_str("#[allow(non_camel_case_types)]\n");
    dispatch_code.push_str("pub enum MethodIntrinsicId {\n");
    dispatch_code.push_str("    Missing,\n");
    let mut method_variants: Vec<_> = sorted_entries
        .iter()
        .filter(|entry| !entry.is_field)
        .map(|entry| &entry.variant_name)
        .collect();
    method_variants.sort();
    method_variants.dedup();
    for variant in method_variants {
        dispatch_code.push_str(&format!("    {},\n", variant));
    }
    dispatch_code.push_str("}\n\n");

    dispatch_code.push_str("#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]\n");
    dispatch_code.push_str("#[allow(non_camel_case_types)]\n");
    dispatch_code.push_str("pub enum FieldIntrinsicId {\n");
    dispatch_code.push_str("    Missing,\n");
    let mut field_variants: Vec<_> = sorted_entries
        .iter()
        .filter(|entry| entry.is_field)
        .map(|entry| &entry.variant_name)
        .collect();
    field_variants.sort();
    field_variants.dedup();
    for variant in field_variants {
        dispatch_code.push_str(&format!("    {},\n", variant));
    }
    dispatch_code.push_str("}\n\n");

    dispatch_code
        .push_str("pub fn dispatch_method_intrinsic<'gc, T: crate::stack::ops::VesOps<'gc>>(\n");
    dispatch_code.push_str("    id: MethodIntrinsicId,\n");
    dispatch_code.push_str("    ctx: &mut T,\n");
    dispatch_code.push_str("    method: dotnet_types::members::MethodDescription,\n");
    dispatch_code.push_str("    generics: &dotnet_types::generics::GenericLookup,\n");
    dispatch_code.push_str(") -> crate::StepResult {\n");
    dispatch_code.push_str("    match id {\n");
    dispatch_code.push_str("        MethodIntrinsicId::Missing => crate::intrinsics::missing_intrinsic_handler(ctx, method, generics),\n");
    let mut method_variants: Vec<_> = sorted_entries
        .iter()
        .filter(|entry| !entry.is_field)
        .map(|entry| &entry.variant_name)
        .collect();
    method_variants.sort();
    method_variants.dedup();
    for variant in method_variants {
        let entry = sorted_entries
            .iter()
            .find(|entry| !entry.is_field && entry.variant_name == *variant)
            .unwrap();
        dispatch_code.push_str(&format!(
            "        MethodIntrinsicId::{} => {}(ctx, method, generics),\n",
            variant, entry.original_handler_path
        ));
    }
    dispatch_code.push_str("    }\n");
    dispatch_code.push_str("}\n\n");

    dispatch_code
        .push_str("pub fn dispatch_field_intrinsic<'gc, T: crate::stack::ops::VesOps<'gc>>(\n");
    dispatch_code.push_str("    id: FieldIntrinsicId,\n");
    dispatch_code.push_str("    ctx: &mut T,\n");
    dispatch_code.push_str("    field: dotnet_types::members::FieldDescription,\n");
    dispatch_code
        .push_str("    type_generics: std::sync::Arc<[dotnet_types::generics::ConcreteType]>,\n");
    dispatch_code.push_str("    is_address: bool,\n");
    dispatch_code.push_str(") -> crate::StepResult {\n");
    dispatch_code.push_str("    match id {\n");
    dispatch_code.push_str("        FieldIntrinsicId::Missing => crate::StepResult::Error(crate::error::VmError::Execution(crate::error::ExecutionError::NotImplemented(format!(\"Missing field intrinsic: {:?}\", field).into()))),\n");
    let mut field_variants: Vec<_> = sorted_entries
        .iter()
        .filter(|entry| entry.is_field)
        .map(|entry| &entry.variant_name)
        .collect();
    field_variants.sort();
    field_variants.dedup();
    for variant in field_variants {
        let entry = sorted_entries
            .iter()
            .find(|entry| entry.is_field && entry.variant_name == *variant)
            .unwrap();
        dispatch_code.push_str(&format!(
            "        FieldIntrinsicId::{} => {}(ctx, field, type_generics, is_address),\n",
            variant, entry.original_handler_path
        ));
    }
    dispatch_code.push_str("    }\n");
    dispatch_code.push_str("}\n");

    fs::write(dispatch_path, dispatch_code).unwrap();

    // 2. Generate PHF Table
    let mut table_code = String::new();
    table_code.push_str("use crate::intrinsics::static_registry::{StaticIntrinsicEntry, StaticIntrinsicHandler, Range};\n\n");
    table_code.push_str("pub static INTRINSIC_ENTRIES: &[StaticIntrinsicEntry] = &[\n");
    for entry in &sorted_entries {
        let handler_kind = if entry.is_field {
            format!(
                "StaticIntrinsicHandler::Field(FieldIntrinsicId::{})",
                entry.variant_name
            )
        } else {
            format!(
                "StaticIntrinsicHandler::Method(MethodIntrinsicId::{})",
                entry.variant_name
            )
        };
        let filter = match &entry.filter_name {
            Some(filter_name) => format!("Some({})", filter_name),
            None => "None".to_string(),
        };
        table_code.push_str(&format!(
            "    StaticIntrinsicEntry {{ type_name: \"{}\", member_name: \"{}\", arity: {}, is_static: {}, handler: {}, filter: {} }},\n",
            entry.type_name, entry.member_name, entry.arity, entry.is_static, handler_kind, filter
        ));
    }
    table_code.push_str("];\n\n");

    let mut phf_map = phf_codegen::Map::new();
    for (key, (start, len)) in group_ranges {
        phf_map.entry(key, format!("Range {{ start: {}, len: {} }}", start, len));
    }
    table_code.push_str("#[allow(dead_code)]\n");
    table_code.push_str("pub static INTRINSIC_LOOKUP: phf::Map<&'static str, Range> = ");
    table_code.push_str(&phf_map.build().to_string());
    table_code.push_str(";\n");

    fs::write(phf_path, table_code).unwrap();
}

fn intrinsic_static_key_segment(is_static: bool) -> &'static str {
    if is_static { "S" } else { "I" }
}
