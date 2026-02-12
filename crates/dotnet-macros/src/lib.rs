//! # dotnet-macros
//!
//! Procedural macros for the dotnet-rs VM.
//! Includes `#[dotnet_intrinsic]` for BCL methods and `#[dotnet_instruction]` for CIL instructions.

extern crate proc_macro;

use dotnet_macros_core::{ParsedFieldSignature, ParsedSignature};
use proc_macro::TokenStream;
use quote::quote;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};
use syn::{
    Ident, ItemFn, LitStr, Token, braced, parenthesized,
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
};

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

    let is_static = parsed.is_static;

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

    // We assume the macro is used within dotnet-vm, and IntrinsicEntry is available
    // at crate::intrinsics::IntrinsicEntry.
    // Note: 'handler' expects the function itself.
    let _class_name = &parsed.class_name;
    let _method_name = &parsed.method_name;
    let mut _param_count = parsed.parameters.len();
    if !is_static {
        _param_count += 1;
    }

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
    let attr_str = parse_macro_input!(attr as LitStr);
    let func = parse_macro_input!(item as ItemFn);
    let _func_name = &func.sig.ident;

    let sig_str = attr_str.value();
    let parsed: ParsedFieldSignature = match syn::parse_str(&sig_str) {
        Ok(s) => s,
        Err(e) => return e.to_compile_error().into(),
    };

    // We assume the macro is used within dotnet-vm, and IntrinsicFieldEntry is available
    // at crate::intrinsics::IntrinsicFieldEntry.
    let _class_name = &parsed.class_name;
    let _field_name = &parsed.field_name;

    let output = quote! {
        #func
    };

    output.into()
}

enum InstructionMapping {
    Unit(Ident),
    Tuple(Ident, Punctuated<Ident, Token![,]>),
    Struct(Ident, Punctuated<FieldMapping, Token![,]>),
}

impl InstructionMapping {
    fn variant_ident(&self) -> &Ident {
        match self {
            InstructionMapping::Unit(ident) => ident,
            InstructionMapping::Tuple(ident, _) => ident,
            InstructionMapping::Struct(ident, _) => ident,
        }
    }

    fn to_match_arm(
        &self,
        func_name: &Ident,
        extra_arg_info: &[(&Ident, bool)],
    ) -> proc_macro2::TokenStream {
        match self {
            InstructionMapping::Unit(variant) => {
                quote! {
                    Instruction::#variant => #func_name(ctx)
                }
            }
            InstructionMapping::Tuple(variant, bindings) => {
                let bindings_iter = bindings.iter();
                let calls = bindings.iter().enumerate().map(|(i, binding)| {
                    if extra_arg_info[i].1 {
                        quote! { #binding }
                    } else {
                        quote! { *#binding }
                    }
                });
                quote! {
                    Instruction::#variant(#(#bindings_iter),*) => #func_name(ctx, #(#calls),*)
                }
            }
            InstructionMapping::Struct(variant, fields) => {
                let patterns = fields.iter().map(|f| {
                    let field_name = &f.field_name;
                    let binding_name = &f.binding_name;
                    if field_name == binding_name {
                        quote! { #field_name }
                    } else {
                        quote! { #field_name: #binding_name }
                    }
                });

                let calls = extra_arg_info.iter().map(|(param_name, is_ref)| {
                    let field = fields
                        .iter()
                        .find(|f| &f.binding_name == *param_name)
                        .expect("Validation should have caught missing binding");
                    let binding = &field.binding_name;
                    if *is_ref {
                        quote! { #binding }
                    } else {
                        quote! { *#binding }
                    }
                });

                quote! {
                    Instruction::#variant { #(#patterns,)* .. } => #func_name(ctx, #(#calls),*)
                }
            }
        }
    }
}

struct FieldMapping {
    field_name: Ident,
    binding_name: Ident,
}

impl Parse for InstructionMapping {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let variant_ident: Ident = input.parse()?;
        if input.peek(syn::token::Paren) {
            let content;
            parenthesized!(content in input);
            let fields = content.parse_terminated(Ident::parse, Token![,])?;
            Ok(InstructionMapping::Tuple(variant_ident, fields))
        } else if input.peek(syn::token::Brace) {
            let content;
            braced!(content in input);
            let fields = content.parse_terminated(FieldMapping::parse, Token![,])?;
            Ok(InstructionMapping::Struct(variant_ident, fields))
        } else {
            Ok(InstructionMapping::Unit(variant_ident))
        }
    }
}

impl Parse for FieldMapping {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let field_name: Ident = input.parse()?;
        if input.peek(Token![:]) {
            input.parse::<Token![:]>()?;
            let binding_name: Ident = input.parse()?;
            Ok(FieldMapping {
                field_name,
                binding_name,
            })
        } else {
            let binding_name = field_name.clone();
            Ok(FieldMapping {
                field_name,
                binding_name,
            })
        }
    }
}

#[proc_macro_attribute]
pub fn dotnet_instruction(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mapping = parse_macro_input!(attr as InstructionMapping);
    let func = parse_macro_input!(item as ItemFn);
    let func_name = &func.sig.ident;
    let variant_ident = mapping.variant_ident();
    let variant_name = variant_ident.to_string();

    let args = &func.sig.inputs;
    let extra_args = args.iter().skip(1);
    let mut extra_arg_info = Vec::new();
    let mut extra_arg_names = Vec::new();
    for arg in extra_args {
        if let syn::FnArg::Typed(pat_type) = arg
            && let syn::Pat::Ident(pat_ident) = &*pat_type.pat
        {
            let name = &pat_ident.ident;
            extra_arg_names.push(name);
            let is_ref = matches!(&*pat_type.ty, syn::Type::Reference(_));
            extra_arg_info.push((name, is_ref));
        } else {
            return syn::Error::new_spanned(arg, "Instruction handler must have named parameters")
                .to_compile_error()
                .into();
        }
    }

    let extra_args_count = extra_arg_names.len();
    if let InstructionMapping::Unit(_) = &mapping {
        if extra_args_count != 0 {
            return syn::Error::new(
                variant_ident.span(),
                format!(
                    "Instruction variant {} expects 0 parameters, but function has {}. Use explicit mapping syntax.",
                    variant_name,
                    extra_args_count
                ),
            )
            .to_compile_error()
            .into();
        }
    } else if let InstructionMapping::Tuple(_, bindings) = &mapping {
        if bindings.len() != extra_args_count {
            return syn::Error::new(
                variant_ident.span(),
                format!(
                    "Instruction variant {} expects {} parameters, but function has {}",
                    variant_name,
                    bindings.len(),
                    extra_args_count
                ),
            )
            .to_compile_error()
            .into();
        }
        for (i, binding) in bindings.iter().enumerate() {
            if binding != extra_arg_names[i] {
                return syn::Error::new(
                    binding.span(),
                    format!(
                        "Binding name '{}' at position {} does not match function parameter '{}'",
                        binding, i, extra_arg_names[i]
                    ),
                )
                .to_compile_error()
                .into();
            }
        }
    } else if let InstructionMapping::Struct(_, fields) = &mapping {
        if fields.len() != extra_args_count {
            return syn::Error::new(
                variant_ident.span(),
                format!(
                    "Instruction variant {} expects {} parameters, but function has {}",
                    variant_name,
                    fields.len(),
                    extra_args_count
                ),
            )
            .to_compile_error()
            .into();
        }
        for field in fields {
            if !extra_arg_names.contains(&&field.binding_name) {
                return syn::Error::new(
                    field.binding_name.span(),
                    format!(
                        "Binding name '{}' not found in function parameters",
                        field.binding_name
                    ),
                )
                .to_compile_error()
                .into();
            }
        }
    }

    let wrapper_name = syn::Ident::new(&format!("{}_wrapper", func_name), func_name.span());
    let match_arm = mapping.to_match_arm(func_name, &extra_arg_info);

    let output = quote! {
        #func

        pub(crate) fn #wrapper_name<'gc, 'm: 'gc>(
            ctx: &mut dyn crate::stack::ops::VesOps<'gc, 'm>,
            instr: &Instruction
        ) -> crate::StepResult {
            match instr {
                #match_arm,
                _ => unreachable!("Instruction wrapper called with wrong variant: expected {}, received {:?}", #variant_name, instr)
            }
        }
    };

    output.into()
}

#[cfg(test)]
mod tests {
    use super::*;

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
