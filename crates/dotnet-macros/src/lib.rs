//! # dotnet-macros
//!
//! Procedural macros for the dotnet-rs VM.
//! Includes `#[dotnet_intrinsic]` for BCL methods and `#[dotnet_instruction]` for CIL instructions.
extern crate proc_macro;

use dotnet_macros_core::{InstructionMapping, ParsedInstruction, ParsedSignature};
use proc_macro::TokenStream;
use quote::quote;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};
use syn::{ItemFn, LitStr, parse_macro_input};

fn match_primitive(type_name: &str) -> Option<proc_macro2::TokenStream> {
    match type_name {
        "void" | "Void" | "System.Void" => Some(quote! { BaseType::Void }),
        "bool" | "Boolean" | "System.Boolean" => Some(quote! { BaseType::Boolean }),
        "char" | "Char" | "System.Char" => Some(quote! { BaseType::Char }),
        "sbyte" | "SByte" | "System.SByte" => Some(quote! { BaseType::Int8 }),
        "byte" | "Byte" | "System.Byte" => Some(quote! { BaseType::UInt8 }),
        "short" | "Int16" | "System.Int16" => Some(quote! { BaseType::Int16 }),
        "ushort" | "UInt16" | "System.UInt16" => Some(quote! { BaseType::UInt16 }),
        "int" | "Int32" | "System.Int32" => Some(quote! { BaseType::Int32 }),
        "uint" | "UInt32" | "System.UInt32" => Some(quote! { BaseType::UInt32 }),
        "long" | "Int64" | "System.Int64" => Some(quote! { BaseType::Int64 }),
        "ulong" | "UInt64" | "System.UInt64" => Some(quote! { BaseType::UInt64 }),
        "float" | "Single" | "System.Single" | "Float32" => Some(quote! { BaseType::Float32 }),
        "double" | "Double" | "System.Double" | "Float64" => Some(quote! { BaseType::Float64 }),
        "string" | "String" | "System.String" => Some(quote! { BaseType::String }),
        "object" | "Object" | "System.Object" => Some(quote! { BaseType::Object }),
        "IntPtr" | "System.IntPtr" => Some(quote! { BaseType::IntPtr }),
        "UIntPtr" | "System.UIntPtr" => Some(quote! { BaseType::UIntPtr }),
        _ => None,
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
                    let Some(Parameter(_, ParameterType::Value(MethodType::Base(b)))) = method.method.signature.parameters.get(#i) else { return false; };
                    if !matches!(**b, #base_type) { return false; }
                }
            }
        } else {
            quote! {}
        }
    });

    let filter_name = syn::Ident::new(
        &format!("{}_filter_{:x}", func_name, sig_hash),
        func_name.span(),
    );

    let filter_fn = quote! {
        pub(crate) fn #filter_name(method: &dotnet_types::members::MethodDescription) -> bool {
            #(#param_checks)*
            true
        }
    };

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

#[cfg(test)]
mod tests {
    use super::*;
    use dotnet_macros_core::FieldMapping;
    use syn::{Ident, punctuated::Punctuated};

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
}
