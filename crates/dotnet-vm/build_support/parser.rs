use crate::build_support::scanner::SourceScanRoot;
use dotnet_build_tools::hash_value_u64;
use dotnet_macros_core::{
    InstructionMapping, ParsedFieldSignature, ParsedInstruction, ParsedSignature,
};
use std::{collections::BTreeMap, fs, path::Path};
use syn::{
    Attribute, Ident, Item, ItemFn, LitStr,
    parse::{Parse, ParseStream},
};

pub struct InstructionEntry {
    pub variant_name: String,
    pub mod_path: String,
    pub source_path: String,
    pub parsed: ParsedInstruction,
}

pub struct IntrinsicEntry {
    pub type_name: String,
    pub member_name: String,
    pub arity: u16,
    pub is_static: bool,
    pub is_field: bool,
    pub original_handler_path: String,
    pub filter_name: Option<String>,
    pub variant_name: String,
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

pub fn process_instruction_file(
    path: &Path,
    root: &SourceScanRoot,
    entries: &mut Vec<InstructionEntry>,
) {
    let content = fs::read_to_string(path).unwrap();
    let mod_path = get_mod_path(path, root);
    let file = syn::parse_file(&content).expect("Failed to parse instruction file");

    for item in file.items {
        match item {
            Item::Fn(item_fn) => process_fn(&item_fn, &mod_path, path, entries),
            Item::Macro(item_macro) => {
                let tokens_clone = item_macro.mac.tokens.clone();
                if let Ok(macro_instr) = syn::parse2::<MacroInstruction>(tokens_clone)
                    && macro_instr
                        .attrs
                        .iter()
                        .any(|attribute| attribute.path().is_ident("dotnet_instruction"))
                {
                    // Manufacture a dummy function that ParsedInstruction::parse can use.
                    // We need to include parameters from the attribute if any.
                    let mut params = quote::quote! {};
                    for attribute in &macro_instr.attrs {
                        #[allow(clippy::collapsible_if)]
                        if attribute.path().is_ident("dotnet_instruction") {
                            if let Ok(mapping) = attribute.parse_args::<InstructionMapping>() {
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
                        pub fn #name<'gc, T: crate::stack::ops::VesOps<'gc> + ?Sized>(ctx: &mut T, #params) -> crate::StepResult {
                            unimplemented!()
                        }
                    };
                    process_fn(&dummy_fn, &mod_path, path, entries);
                }
            }
            _ => {}
        }
    }
}

pub fn process_intrinsic_file(
    path: &Path,
    root: &SourceScanRoot,
    entries: &mut Vec<IntrinsicEntry>,
) {
    let content = fs::read_to_string(path).unwrap();
    let mod_path = get_mod_path(path, root);
    let file = syn::parse_file(&content).expect("Failed to parse file for intrinsics");

    for item in file.items {
        if let Item::Fn(item_fn) = item {
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
                    let sig_hash = hash_value_u64(&normalized_sig);
                    let filter_name = format!("{}_filter_{:x}", func_name, sig_hash);

                    let sanitized_class = parsed.class_name.replace(['.', '+', '`', '\''], "_");
                    let sanitized_method = parsed.method_name.replace(['.', '$'], "_");
                    let variant_name =
                        format!("{}_{}_{:x}", sanitized_class, sanitized_method, sig_hash);
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

                    let sig_hash = hash_value_u64(&sig_str.value());

                    let sanitized_class = parsed.class_name.replace(['.', '+', '`', '\''], "_");
                    let sanitized_field = parsed.field_name.replace(['.', '$'], "_");
                    let variant_name =
                        format!("{}_{}_{:x}", sanitized_class, sanitized_field, sig_hash);
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

pub fn assert_unique_instruction_variants(entries: &[InstructionEntry]) {
    let mut by_variant: BTreeMap<&str, Vec<&InstructionEntry>> = BTreeMap::new();
    for entry in entries {
        by_variant
            .entry(&entry.variant_name)
            .or_default()
            .push(entry);
    }

    let mut duplicates = Vec::new();
    for (variant, registrations) in by_variant {
        if registrations.len() <= 1 {
            continue;
        }
        let handlers: Vec<_> = registrations
            .iter()
            .map(|entry| {
                format!(
                    "{}::{} ({})",
                    entry.mod_path, entry.parsed.handler_name, entry.source_path
                )
            })
            .collect();
        duplicates.push(format!(
            "- `{}` registered by:\n    {}",
            variant,
            handlers.join("\n    ")
        ));
    }

    if !duplicates.is_empty() {
        panic!(
            "Duplicate instruction handler registrations detected before code generation:\n{}",
            duplicates.join("\n")
        );
    }
}

fn process_fn(
    item_fn: &ItemFn,
    mod_path: &str,
    source_path: &Path,
    entries: &mut Vec<InstructionEntry>,
) {
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
                source_path: source_path.display().to_string(),
                parsed,
            });
        }
    }
}

fn get_mod_path(path: &Path, root: &SourceScanRoot) -> String {
    // Extraction assumption: handler module paths are derived from filesystem layout
    // relative to a scan root and then anchored under `module_prefix`.
    let mut rel_path = path
        .strip_prefix(&root.directory)
        .unwrap_or_else(|_| {
            panic!(
                "Handler source `{}` must be under configured root `{}`",
                path.display(),
                root.directory.display()
            )
        })
        .to_path_buf();
    rel_path.set_extension("");
    let components: Vec<_> = rel_path
        .components()
        .map(|component| component.as_os_str().to_str().unwrap())
        .filter(|component| *component != "mod")
        .collect();
    if components.is_empty() {
        root.module_prefix.clone()
    } else {
        format!("{}::{}", root.module_prefix, components.join("::"))
    }
}
