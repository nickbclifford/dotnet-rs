//! # dotnet-macros
//!
//! Procedural macros for the dotnet-rs VM.
//! Includes `#[dotnet_intrinsic]` for BCL methods and `#[dotnet_instruction]` for CIL instructions.
extern crate proc_macro;

use dotnet_macros_core::{
    InstructionMapping, ParsedInstruction, ParsedSignature, expand_trait_aliases,
};
use proc_macro::TokenStream;
use quote::quote;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};
use syn::{ItemFn, LitStr, parse_macro_input};

/// Returns the `BaseType` discriminator used by generated intrinsic filters.
///
/// This intentionally only recognizes primitive/base-shape aliases that can be
/// matched directly from signature metadata. More complex shapes (arrays,
/// generics, refs, user types, etc.) are left to runtime intrinsic handlers.
fn match_primitive(type_name: &str) -> Option<proc_macro2::TokenStream> {
    match type_name {
        "bool" | "Boolean" | "System.Boolean" => Some(quote! { BaseType::<MethodType>::Boolean }),
        "char" | "Char" | "System.Char" => Some(quote! { BaseType::<MethodType>::Char }),
        "sbyte" | "SByte" | "System.SByte" => Some(quote! { BaseType::<MethodType>::Int8 }),
        "byte" | "Byte" | "System.Byte" => Some(quote! { BaseType::<MethodType>::UInt8 }),
        "short" | "Int16" | "System.Int16" => Some(quote! { BaseType::<MethodType>::Int16 }),
        "ushort" | "UInt16" | "System.UInt16" => Some(quote! { BaseType::<MethodType>::UInt16 }),
        "int" | "Int32" | "System.Int32" => Some(quote! { BaseType::<MethodType>::Int32 }),
        "uint" | "UInt32" | "System.UInt32" => Some(quote! { BaseType::<MethodType>::UInt32 }),
        "long" | "Int64" | "System.Int64" => Some(quote! { BaseType::<MethodType>::Int64 }),
        "ulong" | "UInt64" | "System.UInt64" => Some(quote! { BaseType::<MethodType>::UInt64 }),
        "float" | "Single" | "System.Single" | "Float32" => {
            Some(quote! { BaseType::<MethodType>::Float32 })
        }
        "double" | "Double" | "System.Double" | "Float64" => {
            Some(quote! { BaseType::<MethodType>::Float64 })
        }
        "string" | "String" | "System.String" => Some(quote! { BaseType::<MethodType>::String }),
        "object" | "Object" | "System.Object" => Some(quote! { BaseType::<MethodType>::Object }),
        "IntPtr" | "System.IntPtr" => Some(quote! { BaseType::<MethodType>::IntPtr }),
        "UIntPtr" | "System.UIntPtr" => Some(quote! { BaseType::<MethodType>::UIntPtr }),
        _ => None,
    }
}

fn generate_intrinsic_filter_fn(
    func_name: &syn::Ident,
    parsed: &ParsedSignature,
) -> proc_macro2::TokenStream {
    // Reconstruct normalized signature string
    // Format: "ReturnType ClassName::MethodName(Param1, Param2)"
    let params_str = parsed.parameters.join(", ");
    let normalized_sig = format!(
        "{} {}::{}({})",
        parsed.return_type, parsed.class_name, parsed.method_name, params_str
    );

    // Calculate hash for unique filter name
    let mut hasher = DefaultHasher::new();
    normalized_sig.hash(&mut hasher);
    let sig_hash = hasher.finish();

    // Generate filter checks
    let param_checks = parsed.parameters.iter().enumerate().map(|(i, p)| {
        if let Some(base_type) = match_primitive(p) {
            quote! {
                // Check if parameter matches Expected Type.
                {
                    use dotnetdll::prelude::*;
                    let Some(param) = method.signature().parameters.get(#i) else { return false; };
                    match &param.1 {
                        ParameterType::Value(MethodType::Base(b)) => {
                            match b.as_ref() {
                                BaseType::ValuePointer(_, _) => {
                                    if !matches!(#base_type, BaseType::IntPtr | BaseType::UIntPtr) { return false; }
                                }
                                BaseType::Type { .. } => {
                                    // IntPtr is a struct in .NET Core
                                    if !matches!(#base_type, BaseType::IntPtr | BaseType::UIntPtr) { return false; }
                                }
                                b_val => if !matches!(b_val, #base_type) { return false; }
                            }
                        }
                        ParameterType::Ref(MethodType::Base(b)) => {
                            if !matches!(b.as_ref(), #base_type) { return false; }
                        }
                        _ => return false,
                    }
                }
            }
        } else {
            // Validation boundary: non-primitive signature shapes intentionally
            // do not emit macro-level checks. Intrinsic handlers/runtime helpers
            // own null/type/generic/byref validation for those arguments.
            quote! {}
        }
    });

    let filter_name = syn::Ident::new(
        &format!("{}_filter_{:x}", func_name, sig_hash),
        func_name.span(),
    );

    quote! {
        pub fn #filter_name(method: &dotnet_types::members::MethodDescription) -> bool {
            #(#param_checks)*
            true
        }
    }
}

#[proc_macro_attribute]
pub fn dotnet_intrinsic(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr_str = parse_macro_input!(attr as LitStr);
    let func = parse_macro_input!(item as ItemFn);
    let func_name = &func.sig.ident;

    let sig_str = attr_str.value();
    let parsed: ParsedSignature = match syn::parse_str(&sig_str) {
        Ok(s) => s,
        Err(e) => return e.to_compile_error().into(),
    };

    let filter_fn = generate_intrinsic_filter_fn(func_name, &parsed);

    let output = quote! {
        #func
        #filter_fn
    };

    output.into()
}

#[proc_macro_attribute]
pub fn dotnet_intrinsic_field(attr: TokenStream, item: TokenStream) -> TokenStream {
    let _attr_str = parse_macro_input!(attr as LitStr);
    let func = parse_macro_input!(item as ItemFn);

    let output = quote! {
        #func
    };

    output.into()
}

#[proc_macro_attribute]
pub fn dotnet_instruction(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mapping = parse_macro_input!(attr as InstructionMapping);
    let func = parse_macro_input!(item as ItemFn);

    let _parsed = match ParsedInstruction::parse(mapping, &func) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };

    let output = quote! {
        #func
    };

    output.into()
}

#[proc_macro]
pub fn trait_alias(input: TokenStream) -> TokenStream {
    match expand_trait_aliases(input.into()) {
        Ok(expanded) => expanded.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotnet_macros_core::FieldMapping;
    use syn::{Ident, punctuated::Punctuated};

    fn expand_intrinsic_to_file(signature: &str, handler_name: &str) -> syn::File {
        let parsed: ParsedSignature =
            syn::parse_str(signature).expect("signature should parse for intrinsic test");
        let ident = Ident::new(handler_name, proc_macro2::Span::call_site());
        let handler_fn: ItemFn = syn::parse_quote! {
            fn #ident() {}
        };
        let filter_fn = generate_intrinsic_filter_fn(&ident, &parsed);

        syn::parse2(quote! {
            #handler_fn
            #filter_fn
        })
        .expect("generated intrinsic expansion should parse as a file")
    }

    fn find_filter_fn<'a>(file: &'a syn::File, handler_name: &str) -> &'a syn::ItemFn {
        let filter_prefix = format!("{}_filter_", handler_name);
        file.items
            .iter()
            .find_map(|item| match item {
                syn::Item::Fn(function)
                    if function.sig.ident.to_string().starts_with(&filter_prefix) =>
                {
                    Some(function)
                }
                _ => None,
            })
            .expect("expanded intrinsic should contain generated filter function")
    }

    fn assert_filter_tail_is_true(filter: &syn::ItemFn) {
        match filter.block.stmts.last() {
            Some(syn::Stmt::Expr(syn::Expr::Lit(expr), _)) => match &expr.lit {
                syn::Lit::Bool(value) => {
                    assert!(value.value, "filter tail should evaluate to true")
                }
                _ => panic!("expected filter tail literal to be a bool"),
            },
            _ => panic!("expected filter tail statement to be a literal expression"),
        }
    }

    #[test]
    fn test_parse_instruction_mapping() {
        let input = "Add";
        let parsed: InstructionMapping = syn::parse_str(input).unwrap();
        match parsed {
            InstructionMapping::Unit(ident) => assert_eq!(ident, "Add"),
            _ => panic!("Expected Unit"),
        }

        let input = "LoadIndirect(param0)";
        let parsed: InstructionMapping = syn::parse_str(input).unwrap();
        match parsed {
            InstructionMapping::Tuple(ident, fields) => {
                assert_eq!(ident, "LoadIndirect");
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0], "param0");
            }
            _ => panic!("Expected Tuple"),
        }

        let input = "LoadField { param0: field }";
        let parsed: InstructionMapping = syn::parse_str(input).unwrap();
        match parsed {
            InstructionMapping::Struct(ident, fields) => {
                assert_eq!(ident, "LoadField");
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].field_name, "param0");
                assert_eq!(fields[0].binding_name, "field");
            }
            _ => panic!("Expected Struct"),
        }

        let input = "LoadField { param0 }";
        let parsed: InstructionMapping = syn::parse_str(input).unwrap();
        match parsed {
            InstructionMapping::Struct(ident, fields) => {
                assert_eq!(ident, "LoadField");
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].field_name, "param0");
                assert_eq!(fields[0].binding_name, "param0");
            }
            _ => panic!("Expected Struct"),
        }
    }

    #[test]
    fn test_instruction_mapping_to_match_arm() {
        let func_name = Ident::new("my_func", proc_macro2::Span::call_site());
        let p0 = Ident::new("p0", proc_macro2::Span::call_site());
        let extra_arg_info = [(&p0, false)];

        // Unit
        let mapping = InstructionMapping::Unit(Ident::new("Nop", proc_macro2::Span::call_site()));
        let arm = mapping.to_match_arm(&func_name, &[]);
        let expected = quote! {
            Instruction::Nop => my_func(ctx)
        };
        assert_eq!(arm.to_string(), expected.to_string());

        // Tuple
        let mapping = InstructionMapping::Tuple(
            Ident::new("LoadIndirect", proc_macro2::Span::call_site()),
            {
                let mut p = Punctuated::new();
                p.push(p0.clone());
                p
            },
        );
        let arm = mapping.to_match_arm(&func_name, &extra_arg_info);
        let expected = quote! {
            Instruction::LoadIndirect(p0) => my_func(ctx, *p0)
        };
        assert_eq!(arm.to_string(), expected.to_string());

        // Struct
        let mapping =
            InstructionMapping::Struct(Ident::new("LoadField", proc_macro2::Span::call_site()), {
                let mut p = Punctuated::new();
                p.push(FieldMapping {
                    field_name: Ident::new("param0", proc_macro2::Span::call_site()),
                    binding_name: p0.clone(),
                });
                p
            });
        let arm = mapping.to_match_arm(&func_name, &extra_arg_info);
        let expected = quote! {
            Instruction::LoadField { param0: p0, .. } => my_func(ctx, *p0)
        };
        assert_eq!(arm.to_string(), expected.to_string());
    }

    #[test]
    fn intrinsic_filter_checks_only_primitive_parameter_shapes() {
        let file = expand_intrinsic_to_file(
            "static void DotnetRs.MacroBoundary::Mixed(int, T, T[], T&)",
            "mixed_boundary_handler",
        );
        let filter = find_filter_fn(&file, "mixed_boundary_handler");

        // One primitive parameter (`int`) emits one check block, and the rest are
        // intentionally left to runtime handler validation.
        assert_eq!(
            filter.block.stmts.len(),
            2,
            "expected one generated primitive check plus final true tail"
        );
        assert_filter_tail_is_true(filter);
    }

    #[test]
    fn intrinsic_filter_is_true_only_when_all_parameters_are_non_primitive_shapes() {
        let file = expand_intrinsic_to_file(
            "static void DotnetRs.MacroBoundary::NonPrimitiveOnly(T, T[], T&, System.Type)",
            "non_primitive_boundary_handler",
        );
        let filter = find_filter_fn(&file, "non_primitive_boundary_handler");

        assert_eq!(
            filter.block.stmts.len(),
            1,
            "non-primitive signature shapes should not emit macro checks"
        );
        assert_filter_tail_is_true(filter);
    }
}
