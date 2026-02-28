use dotnet_macros_core::{InstructionMapping, ParsedFieldSignature, ParsedInstruction, ParsedSignature};
use std::{
    collections::hash_map::DefaultHasher,
    env, fs,
    hash::{Hash, Hasher},
    path::Path,
};
use syn::{parse::{Parse, ParseStream}, Attribute, Ident, Item, ItemFn, LitStr};
use walkdir::WalkDir;

struct InstructionEntry {
    variant_name: String,
    mod_path: String,
    parsed: ParsedInstruction,
}

// --- Intrinsic registration ---

struct IntrinsicEntry {
    type_name: String,
    member_name: String,
    arity: u16,
    is_static: bool,
    is_field: bool,
    original_handler_path: String,
    filter_name: Option<String>,
    variant_name: String,
}

fn main() {
    let out_dir = env::var_os("OUT_DIR").unwrap();

    // 1. Instruction table generation
    let mut instruction_entries = Vec::new();
    let src_instructions_dir = Path::new("src/instructions");
    for entry in WalkDir::new(src_instructions_dir) {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            process_instruction_file(path, &mut instruction_entries);
        }
    }
    instruction_entries.sort_by(|a, b| a.variant_name.cmp(&b.variant_name));
    generate_instruction_table(&out_dir, &instruction_entries);

    // 2. Intrinsic table generation
    let mut intrinsic_entries = Vec::new();
    let src_intrinsics_dir = Path::new("src/intrinsics");
    for entry in WalkDir::new(src_intrinsics_dir) {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            process_intrinsic_file(path, &mut intrinsic_entries);
        }
    }
    generate_intrinsic_phf(&out_dir, &intrinsic_entries);

    println!("cargo:rerun-if-changed=src/instructions");
    println!("cargo:rerun-if-changed=src/intrinsics");
}

fn get_mod_path(path: &Path) -> String {
    let mut rel_path = path.strip_prefix("src").unwrap().to_path_buf();
    rel_path.set_extension("");
    let components: Vec<_> = rel_path
        .components()
        .map(|c| c.as_os_str().to_str().unwrap())
        .filter(|&c| c != "mod")
        .collect();
    format!("crate::{}", components.join("::"))
}

struct MacroInstruction {
    attrs: Vec<Attribute>,
    name: Ident,
}

impl Parse for MacroInstruction {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let name: Ident = input.parse()?;
        let _ = input.parse::<proc_macro2::TokenStream>()?;
        Ok(Self { attrs, name })
    }
}

fn process_instruction_file(path: &Path, entries: &mut Vec<InstructionEntry>) {
    let content = fs::read_to_string(path).unwrap();
    let mod_path = get_mod_path(path);
    let file = syn::parse_file(&content).expect("Failed to parse instruction file");

    for item in file.items {
        match item {
            Item::Fn(item_fn) => {
                process_fn(&item_fn, &mod_path, entries);
            }
            Item::Macro(item_macro) => {
                // Try to extract instruction info from macro tokens
                let tokens_clone = item_macro.mac.tokens.clone();
                match syn::parse2::<MacroInstruction>(tokens_clone) {
                    Ok(macro_instr) => {
                        if macro_instr
                            .attrs
                            .iter()
                            .any(|a| a.path().is_ident("dotnet_instruction"))
                        {
                            // Manufacture a dummy function that ParsedInstruction::parse can use.
                            // We need to include parameters from the attribute if any.
                            let mut params = quote::quote! {};
                            for attr in &macro_instr.attrs {
                                #[allow(clippy::collapsible_if)]
                                if attr.path().is_ident("dotnet_instruction") {
                                    if let Ok(mapping) =
                                        attr.parse_args::<InstructionMapping>()
                                    {
                                        match mapping {
                                            InstructionMapping::Tuple(_, fields) => {
                                                for field in fields {
                                                    params.extend(quote::quote! { #field: u16, });
                                                }
                                            }
                                            InstructionMapping::Struct(_, fields) => {
                                                for field in fields {
                                                    let binding = &field.binding_name;
                                                    params.extend(quote::quote! { #binding: u16, });
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }

                            let name = &macro_instr.name;
                            let attrs = &macro_instr.attrs;
                            let dummy_fn: ItemFn = syn::parse_quote! {
                                #(#attrs)*
                                pub fn #name<'gc, 'm: 'gc, T: crate::stack::ops::VesOps<'gc, 'm> + ?Sized>(ctx: &mut T, #params) -> crate::StepResult {
                                    unimplemented!()
                                }
                            };
                            process_fn(&dummy_fn, &mod_path, entries);
                        }
                    }
                    Err(_e) => {
                        // println!("cargo:warning=Failed to parse macro {}: {}", item_macro.mac.path.segments.last().unwrap().ident, _e);
                    }
                }
            }
            _ => {}
        }
    }
}

fn process_fn(item_fn: &ItemFn, mod_path: &str, entries: &mut Vec<InstructionEntry>) {
    for attr in &item_fn.attrs {
        if attr.path().is_ident("dotnet_instruction") {
            let mapping: InstructionMapping = attr
                .parse_args()
                .expect("Failed to parse dotnet_instruction mapping");
            let parsed = ParsedInstruction::parse(mapping, item_fn)
                .expect("Failed to extract instruction info");

            let variant_name = parsed.variant_name.clone();

            entries.push(InstructionEntry {
                variant_name,
                mod_path: mod_path.to_string(),
                parsed,
            });
        }
    }
}

fn generate_instruction_table(out_dir: &std::ffi::OsStr, entries: &[InstructionEntry]) {
    let dest_path = Path::new(out_dir).join("instruction_dispatch.rs");
    let mut table_code = String::new();

    // New monomorphic dispatcher
    table_code.push_str("pub fn dispatch_monomorphic<'gc, 'm: 'gc, T: crate::stack::ops::VesOps<'gc, 'm>>(\n");
    table_code.push_str("    ctx: &mut T,\n");
    table_code.push_str("    instr: &Instruction,\n");
    table_code.push_str(") -> crate::StepResult {\n");
    table_code.push_str("    match instr {\n");

    for entry in entries {
        let handler_path: syn::Path =
            syn::parse_str(&format!("{}::{}", entry.mod_path, entry.parsed.handler_name))
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

    table_code.push_str("        _ => crate::StepResult::Error(crate::error::VmError::Execution(crate::error::ExecutionError::NotImplemented(format!(\"{:?}\", instr))))\n");
    table_code.push_str("    }\n");
    table_code.push_str("}\n");

    fs::write(dest_path, table_code).unwrap();
}

fn process_intrinsic_file(path: &Path, entries: &mut Vec<IntrinsicEntry>) {
    let content = fs::read_to_string(path).unwrap();
    let mod_path = get_mod_path(path);
    let file = syn::parse_file(&content).expect("Failed to parse file for intrinsics");

    for item in file.items {
        if let syn::Item::Fn(item_fn) = item {
            let func_name = item_fn.sig.ident.to_string();
            for attr in &item_fn.attrs {
                if attr.path().is_ident("dotnet_intrinsic") {
                    let sig_str: LitStr = attr
                        .parse_args()
                        .expect("Failed to parse dotnet_intrinsic args");
                    let parsed: ParsedSignature =
                        syn::parse_str(&sig_str.value()).expect("Failed to parse signature");
                    let mut arity = parsed.parameters.len() as u16;
                    if !parsed.is_static {
                        arity += 1;
                    }
                    let params_str = parsed.parameters.join(", ");
                    let normalized_sig = format!(
                        "{} {}::{}({})",
                        parsed.return_type, parsed.class_name, parsed.method_name, params_str
                    );
                    let mut hasher = DefaultHasher::new();
                    normalized_sig.hash(&mut hasher);
                    let sig_hash = hasher.finish();
                    let filter_name = format!("{}_filter_{:x}", func_name, sig_hash);

                    let sanitized_class = parsed.class_name.replace(['.', '+', '`', '\''], "_");
                    let sanitized_method = parsed.method_name.replace(['.', '$'], "_");
                    let variant_name = format!("{}_{}_{:x}", sanitized_class, sanitized_method, sig_hash);
                    entries.push(IntrinsicEntry {
                        type_name: parsed.class_name,
                        member_name: parsed.method_name,
                        arity,
                        is_static: parsed.is_static,
                        is_field: false,
                        original_handler_path: format!("{}::{}", mod_path, func_name),
                        filter_name: Some(format!("{}::{}", mod_path, filter_name)),
                        variant_name,
                    });
                } else if attr.path().is_ident("dotnet_intrinsic_field") {
                    let sig_str: LitStr = attr
                        .parse_args()
                        .expect("Failed to parse dotnet_intrinsic_field args");
                    let parsed: ParsedFieldSignature =
                        syn::parse_str(&sig_str.value()).expect("Failed to parse field signature");

                    let mut hasher = DefaultHasher::new();
                    sig_str.value().hash(&mut hasher);
                    let sig_hash = hasher.finish();

                    let sanitized_class = parsed.class_name.replace(['.', '+', '`', '\''], "_");
                    let sanitized_field = parsed.field_name.replace(['.', '$'], "_");
                    let variant_name = format!("{}_{}_{:x}", sanitized_class, sanitized_field, sig_hash);
                    entries.push(IntrinsicEntry {
                        type_name: parsed.class_name,
                        member_name: parsed.field_name,
                        arity: 0,
                        is_static: parsed.is_static,
                        is_field: true,
                        original_handler_path: format!("{}::{}", mod_path, func_name),
                        filter_name: None,
                        variant_name,
                    });
                }
            }
        }
    }
}

fn generate_intrinsic_phf(out_dir: &std::ffi::OsStr, entries: &[IntrinsicEntry]) {
    let phf_path = Path::new(out_dir).join("intrinsics_phf.rs");
    let dispatch_path = Path::new(out_dir).join("intrinsics_dispatch.rs");

    let mut groups: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, entry) in entries.iter().enumerate() {
        let key = if entry.is_field {
            format!("F:{}::{}", entry.type_name, entry.member_name)
        } else {
            format!(
                "M:{}::{}#{}",
                entry.type_name, entry.member_name, entry.arity
            )
        };
        groups.entry(key).or_default().push(i);
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
        for &idx in indices {
            sorted_entries.push(&entries[idx]);
        }
        current_idx += indices.len();
    }

    // 1. Generate IDs and Dispatcher
    let mut dispatch_code = String::new();

    dispatch_code.push_str("#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]\n");
    dispatch_code.push_str("#[allow(non_camel_case_types)]\n");
    dispatch_code.push_str("pub enum MethodIntrinsicId {\n");
    dispatch_code.push_str("    Missing,\n");
    let mut method_variants: Vec<_> = sorted_entries.iter().filter(|e| !e.is_field).map(|e| &e.variant_name).collect();
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
    let mut field_variants: Vec<_> = sorted_entries.iter().filter(|e| e.is_field).map(|e| &e.variant_name).collect();
    field_variants.sort();
    field_variants.dedup();
    for variant in field_variants {
        dispatch_code.push_str(&format!("    {},\n", variant));
    }
    dispatch_code.push_str("}\n\n");

    dispatch_code.push_str("pub fn dispatch_method_intrinsic<'gc, 'm: 'gc, T: crate::stack::ops::VesOps<'gc, 'm>>(\n");
    dispatch_code.push_str("    id: MethodIntrinsicId,\n");
    dispatch_code.push_str("    ctx: &mut T,\n");
    dispatch_code.push_str("    method: dotnet_types::members::MethodDescription,\n");
    dispatch_code.push_str("    generics: &dotnet_types::generics::GenericLookup,\n");
    dispatch_code.push_str(") -> crate::StepResult {\n");
    dispatch_code.push_str("    match id {\n");
    dispatch_code.push_str("        MethodIntrinsicId::Missing => crate::intrinsics::missing_intrinsic_handler(ctx, method, generics),\n");
    let mut method_variants: Vec<_> = sorted_entries.iter().filter(|e| !e.is_field).map(|e| &e.variant_name).collect();
    method_variants.sort();
    method_variants.dedup();
    for variant in method_variants {
        let entry = sorted_entries.iter().find(|e| !e.is_field && e.variant_name == *variant).unwrap();
        dispatch_code.push_str(&format!(
            "        MethodIntrinsicId::{} => {}(ctx, method, generics),\n",
            variant, entry.original_handler_path
        ));
    }
    dispatch_code.push_str("    }\n");
    dispatch_code.push_str("}\n\n");

    dispatch_code.push_str("pub fn dispatch_field_intrinsic<'gc, 'm: 'gc, T: crate::stack::ops::VesOps<'gc, 'm>>(\n");
    dispatch_code.push_str("    id: FieldIntrinsicId,\n");
    dispatch_code.push_str("    ctx: &mut T,\n");
    dispatch_code.push_str("    field: dotnet_types::members::FieldDescription,\n");
    dispatch_code.push_str("    type_generics: std::sync::Arc<[dotnet_types::generics::ConcreteType]>,\n");
    dispatch_code.push_str("    is_address: bool,\n");
    dispatch_code.push_str(") -> crate::StepResult {\n");
    dispatch_code.push_str("    match id {\n");
    dispatch_code.push_str("        FieldIntrinsicId::Missing => crate::StepResult::Error(crate::error::VmError::Execution(crate::error::ExecutionError::NotImplemented(format!(\"Missing field intrinsic: {:?}\", field)))),\n");
    let mut field_variants: Vec<_> = sorted_entries.iter().filter(|e| e.is_field).map(|e| &e.variant_name).collect();
    field_variants.sort();
    field_variants.dedup();
    for variant in field_variants {
        let entry = sorted_entries.iter().find(|e| e.is_field && e.variant_name == *variant).unwrap();
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
            Some(f) => format!("Some({})", f),
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
