use dotnet_macros_core::{ParsedFieldSignature, ParsedSignature};
use dotnetdll::prelude::Instruction;
use std::{
    collections::hash_map::DefaultHasher,
    env, fs,
    hash::{Hash, Hasher},
    path::Path,
};
use syn::LitStr;
use walkdir::WalkDir;

// --- Intrinsic registration ---

struct IntrinsicEntry {
    type_name: String,
    member_name: String,
    arity: u16,
    is_static: bool,
    is_field: bool,
    handler_path: String,
    filter_name: Option<String>,
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

fn process_instruction_file(path: &Path, entries: &mut Vec<(String, String)>) {
    let content = fs::read_to_string(path).unwrap();
    let mod_path = get_mod_path(path);

    let mut search_idx = 0;
    while let Some(start_idx) = content[search_idx..].find("#[dotnet_instruction(") {
        let actual_start = search_idx + start_idx;
        let attr_content_start = actual_start + "#[dotnet_instruction(".len();
        if let Some(end_idx) = content[attr_content_start..].find(')') {
            let attr_raw = content[attr_content_start..attr_content_start + end_idx].trim();
            let variant_name = attr_raw.split(['(', '{']).next().unwrap().trim();
            let post_attr_start = attr_content_start + end_idx + 1;
            let post_attr = &content[post_attr_start..];
            let tokens = post_attr
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .filter(|s| !s.is_empty());
            for token in tokens {
                if matches!(
                    token,
                    "pub"
                        | "fn"
                        | "crate"
                        | "extern"
                        | "unsafe"
                        | "mut"
                        | "struct"
                        | "impl"
                        | "enum"
                ) {
                    continue;
                }
                if token == "]" {
                    continue;
                }
                let wrapper_path = format!("{}::{}_wrapper", mod_path, token);
                entries.push((variant_name.to_string(), wrapper_path));
                break;
            }
            search_idx = attr_content_start + end_idx;
        } else {
            search_idx = attr_content_start;
        }
    }
}

fn generate_instruction_table(out_dir: &std::ffi::OsStr, entries: &[(String, String)]) {
    let dest_path = Path::new(out_dir).join("instruction_table.rs");
    let mut table_code = String::new();
    table_code.push_str("pub const INSTRUCTION_TABLE: InstructionTable = {\n");
    table_code.push_str("    let mut table = [None; Instruction::VARIANT_COUNT];\n");
    for (variant_name, wrapper_path) in entries {
        if let Some(opcode) = Instruction::opcode_from_name(variant_name) {
            table_code.push_str(&format!(
                "    table[{}] = Some(unsafe {{ std::mem::transmute::<*const (), crate::dispatch::registry::InstructionHandler>({} as *const ()) }});\n",
                opcode, wrapper_path
            ));
        } else {
            panic!("Unknown instruction variant: {}", variant_name);
        }
    }
    table_code.push_str("    table\n");
    table_code.push_str("};\n");
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

                    entries.push(IntrinsicEntry {
                        type_name: parsed.class_name,
                        member_name: parsed.method_name,
                        arity,
                        is_static: parsed.is_static,
                        is_field: false,
                        handler_path: format!("{}::{}", mod_path, func_name),
                        filter_name: Some(format!("{}::{}", mod_path, filter_name)),
                    });
                } else if attr.path().is_ident("dotnet_intrinsic_field") {
                    let sig_str: LitStr = attr
                        .parse_args()
                        .expect("Failed to parse dotnet_intrinsic_field args");
                    let parsed: ParsedFieldSignature =
                        syn::parse_str(&sig_str.value()).expect("Failed to parse field signature");
                    entries.push(IntrinsicEntry {
                        type_name: parsed.class_name,
                        member_name: parsed.field_name,
                        arity: 0,
                        is_static: parsed.is_static,
                        is_field: true,
                        handler_path: format!("{}::{}", mod_path, func_name),
                        filter_name: None,
                    });
                }
            }
        }
    }
}

fn generate_intrinsic_phf(out_dir: &std::ffi::OsStr, entries: &[IntrinsicEntry]) {
    let dest_path = Path::new(out_dir).join("intrinsics_phf.rs");
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

    let mut table_code = String::new();
    table_code.push_str("use crate::intrinsics::static_registry::{StaticIntrinsicEntry, StaticIntrinsicHandler, Range};\n\n");
    table_code.push_str("pub static INTRINSIC_ENTRIES: &[StaticIntrinsicEntry] = &[\n");
    for entry in sorted_entries {
        let handler_kind = if entry.is_field {
            format!(
                "StaticIntrinsicHandler::Field(unsafe {{ std::mem::transmute::<*const (), IntrinsicFieldHandler>({} as *const ()) }})",
                entry.handler_path
            )
        } else {
            format!(
                "StaticIntrinsicHandler::Method(unsafe {{ std::mem::transmute::<*const (), IntrinsicHandler>({} as *const ()) }})",
                entry.handler_path
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
        phf_map.entry(key, &format!("Range {{ start: {}, len: {} }}", start, len));
    }
    table_code.push_str("#[allow(dead_code)]\n");
    table_code.push_str("pub static INTRINSIC_LOOKUP: phf::Map<&'static str, Range> = ");
    table_code.push_str(&phf_map.build().to_string());
    table_code.push_str(";\n");

    fs::write(dest_path, table_code).unwrap();
}
