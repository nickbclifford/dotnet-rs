//! # dotnet-macros-core
//!
//! Core logic for macro expansion used by `dotnet-macros`.
//! This crate contains the parsing and transformation logic for .NET signatures.
use quote::quote;
use syn::{
    Ident, Result, Token, braced,
    ext::IdentExt,
    parenthesized,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

#[derive(Debug)]
pub struct ParsedSignature {
    pub is_static: bool,
    pub return_type: String,
    pub class_name: String,
    pub method_name: String,
    pub parameters: Vec<String>,
}

impl Parse for ParsedSignature {
    fn parse(input: ParseStream) -> Result<Self> {
        let is_static = input.parse::<Token![static]>().is_ok();

        let return_type = parse_type(input)?;

        let class_name = parse_class_name(input)?;

        input.parse::<Token![::]>()?;

        let method_name = if input.peek(Token![.]) {
            input.parse::<Token![.]>()?;
            let id: Ident = input.parse()?;
            format!(".{}", id)
        } else {
            input.parse::<Ident>()?.to_string()
        };

        // Consume method generics if any (e.g. CreateTruncating<T>)
        let _ = parse_generic_args_count(input)?;

        let content;
        syn::parenthesized!(content in input);

        // Handle empty parameters case explicitly if parse_terminated doesn't?
        // parse_terminated handles empty.
        let params_punctuated: Punctuated<String, Token![,]> =
            content.parse_terminated(parse_type, Token![,])?;

        Ok(ParsedSignature {
            is_static,
            return_type,
            class_name,
            method_name,
            parameters: params_punctuated.into_iter().collect(),
        })
    }
}

#[derive(Debug)]
pub struct ParsedFieldSignature {
    pub is_static: bool,
    pub class_name: String,
    pub field_name: String,
}

impl Parse for ParsedFieldSignature {
    fn parse(input: ParseStream) -> Result<Self> {
        let is_static = input.parse::<Token![static]>().is_ok();

        let _field_type = parse_type(input)?;

        let class_name = parse_class_name(input)?;

        input.parse::<Token![::]>()?;

        let field_name = input.parse::<Ident>()?.to_string();

        // Fields do not have parameters/parentheses

        Ok(ParsedFieldSignature {
            is_static,
            class_name,
            field_name,
        })
    }
}

#[derive(Debug, Clone)]
pub enum InstructionMapping {
    Unit(Ident),
    Tuple(Ident, Punctuated<Ident, Token![,]>),
    Struct(Ident, Punctuated<FieldMapping, Token![,]>),
}

impl InstructionMapping {
    pub fn variant_ident(&self) -> &Ident {
        match self {
            InstructionMapping::Unit(ident) => ident,
            InstructionMapping::Tuple(ident, _) => ident,
            InstructionMapping::Struct(ident, _) => ident,
        }
    }

    pub fn to_match_arm(
        &self,
        func_name: &Ident,
        extra_arg_info: &[(&Ident, bool)],
    ) -> proc_macro2::TokenStream {
        self.to_match_arm_path(func_name, extra_arg_info)
    }

    pub fn to_match_arm_path<T: quote::ToTokens>(
        &self,
        func_path: &T,
        extra_arg_info: &[(&Ident, bool)],
    ) -> proc_macro2::TokenStream {
        match self {
            InstructionMapping::Unit(variant) => {
                quote! {
                    Instruction::#variant => #func_path(ctx)
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
                    Instruction::#variant(#(#bindings_iter),*) => #func_path(ctx, #(#calls),*)
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
                    Instruction::#variant { #(#patterns,)* .. } => #func_path(ctx, #(#calls),*)
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct FieldMapping {
    pub field_name: Ident,
    pub binding_name: Ident,
}

#[derive(Debug, Clone)]
pub struct ParsedInstruction {
    pub variant_name: String,
    pub handler_name: Ident,
    pub handler_path: Option<String>,
    pub mapping: InstructionMapping,
    pub extra_arg_info: Vec<(Ident, bool)>,
}

impl ParsedInstruction {
    pub fn parse(attr_mapping: InstructionMapping, func: &syn::ItemFn) -> Result<Self> {
        let handler_name = func.sig.ident.clone();
        let variant_ident = attr_mapping.variant_ident();
        let variant_name = variant_ident.to_string();

        let args = &func.sig.inputs;
        let extra_args = args.iter().skip(1);
        let mut extra_arg_info = Vec::new();
        let mut extra_arg_names = Vec::new();

        for arg in extra_args {
            if let syn::FnArg::Typed(pat_type) = arg {
                if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                    let name = &pat_ident.ident;
                    extra_arg_names.push(name.clone());
                    let is_ref = matches!(&*pat_type.ty, syn::Type::Reference(_));
                    extra_arg_info.push((name.clone(), is_ref));
                } else {
                    return Err(syn::Error::new_spanned(
                        arg,
                        "Instruction handler must have named parameters",
                    ));
                }
            } else {
                return Err(syn::Error::new_spanned(
                    arg,
                    "Instruction handler must have named parameters",
                ));
            }
        }

        let extra_args_count = extra_arg_names.len();
        match &attr_mapping {
            InstructionMapping::Unit(_) => {
                if extra_args_count != 0 {
                    return Err(syn::Error::new(
                        variant_ident.span(),
                        format!(
                            "Instruction variant {} expects 0 parameters, but function has {}. Use explicit mapping syntax.",
                            variant_name, extra_args_count
                        ),
                    ));
                }
            }
            InstructionMapping::Tuple(_, bindings) => {
                if bindings.len() != extra_args_count {
                    return Err(syn::Error::new(
                        variant_ident.span(),
                        format!(
                            "Instruction variant {} expects {} parameters, but function has {}",
                            variant_name,
                            bindings.len(),
                            extra_args_count
                        ),
                    ));
                }
                for (i, binding) in bindings.iter().enumerate() {
                    if binding != &extra_arg_names[i] {
                        return Err(syn::Error::new(
                            binding.span(),
                            format!(
                                "Binding name '{}' at position {} does not match function parameter '{}'",
                                binding, i, extra_arg_names[i]
                            ),
                        ));
                    }
                }
            }
            InstructionMapping::Struct(_, fields) => {
                if fields.len() != extra_args_count {
                    return Err(syn::Error::new(
                        variant_ident.span(),
                        format!(
                            "Instruction variant {} expects {} parameters, but function has {}",
                            variant_name,
                            fields.len(),
                            extra_args_count
                        ),
                    ));
                }
                for field in fields {
                    if !extra_arg_names.contains(&field.binding_name) {
                        return Err(syn::Error::new(
                            field.binding_name.span(),
                            format!(
                                "Binding name '{}' not found in function parameters",
                                field.binding_name
                            ),
                        ));
                    }
                }
            }
        }

        Ok(ParsedInstruction {
            variant_name,
            handler_name,
            handler_path: None,
            mapping: attr_mapping,
            extra_arg_info,
        })
    }
}

impl Parse for InstructionMapping {
    fn parse(input: ParseStream) -> Result<Self> {
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
    fn parse(input: ParseStream) -> Result<Self> {
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

pub fn parse_type(input: ParseStream) -> Result<String> {
    let mut is_ref = false;
    if input.peek(Ident::peek_any) {
        let fork = input.fork();
        let ident: Ident = fork.call(Ident::parse_any)?;
        if ident == "ref" || ident == "out" || ident == "in" {
            input.call(Ident::parse_any)?;
            is_ref = true;
        }
    }

    let (segments, separators) = parse_dotted_segments(input)?;
    let suffix = parse_type_suffix(input)?;

    // Normalize only if all separators were '.'
    let mut type_name = if separators.iter().all(|&c| c == '.') {
        if segments.len() == 1 {
            match segments[0].as_str() {
                "void" => String::from("Void"),
                "bool" => String::from("Boolean"),
                "byte" => String::from("UInt8"),
                "sbyte" => String::from("Int8"),
                "char" => String::from("Char"),
                "short" => String::from("Int16"),
                "ushort" => String::from("UInt16"),
                "int" => String::from("Int32"),
                "uint" => String::from("UInt32"),
                "long" => String::from("Int64"),
                "ulong" => String::from("UInt64"),
                "float" => String::from("Float32"),
                "double" => String::from("Float64"),
                "string" => String::from("String"),
                "object" => String::from("Object"),
                "nint" => String::from("IntPtr"),
                "nuint" => String::from("UIntPtr"),
                _ => segments[0].clone(),
            }
        } else if segments.len() == 2 && segments[0] == "System" {
            match segments[1].as_str() {
                "Void" => String::from("Void"),
                "Boolean" => String::from("Boolean"),
                "Byte" => String::from("UInt8"),
                "SByte" => String::from("Int8"),
                "Char" => String::from("Char"),
                "Int16" => String::from("Int16"),
                "UInt16" => String::from("UInt16"),
                "Int32" => String::from("Int32"),
                "UInt32" => String::from("UInt32"),
                "Int64" => String::from("Int64"),
                "UInt64" => String::from("UInt64"),
                "Single" => String::from("Float32"),
                "Double" => String::from("Float64"),
                "String" => String::from("String"),
                "Object" => String::from("Object"),
                "IntPtr" => String::from("IntPtr"),
                "UIntPtr" => String::from("UIntPtr"),
                _ => segments.join("."),
            }
        } else {
            segments.join(".")
        }
    } else {
        // Build with separators
        let mut res = segments[0].clone();
        for i in 0..separators.len() {
            res.push(separators[i]);
            res.push_str(&segments[i + 1]);
        }
        res
    };

    type_name.push_str(&suffix);
    if is_ref {
        type_name.push('&');
    }
    Ok(type_name)
}

pub fn parse_class_name(input: ParseStream) -> Result<String> {
    let (segments, separators) = parse_dotted_segments(input)?;
    let suffix = parse_type_suffix(input)?;

    let mut res = segments[0].clone();
    for i in 0..separators.len() {
        res.push(separators[i]);
        res.push_str(&segments[i + 1]);
    }
    res.push_str(&suffix);
    Ok(res)
}

fn parse_dotted_segments(input: ParseStream) -> Result<(Vec<String>, Vec<char>)> {
    let mut segments = Vec::new();
    let mut separators = Vec::new();

    loop {
        let ident: Ident = input.call(Ident::parse_any)?;
        let mut segment = ident.to_string();

        // Check for generics <T, U> -> `2
        let count = parse_generic_args_count(input)?;
        if count > 0 {
            segment.push('`');
            segment.push_str(&count.to_string());
        }

        segments.push(segment);

        if input.peek(Token![.]) {
            input.parse::<Token![.]>()?;
            separators.push('.');
            continue;
        } else if input.peek(Token![/]) {
            input.parse::<Token![/]>()?;
            separators.push('+');
            continue;
        } else if input.peek(Token![+]) {
            input.parse::<Token![+]>()?;
            separators.push('+');
            continue;
        } else {
            break;
        }
    }
    Ok((segments, separators))
}

fn parse_type_suffix(input: ParseStream) -> Result<String> {
    let mut suffix = String::new();
    loop {
        if input.peek(syn::token::Bracket) {
            let _content;
            syn::bracketed!(_content in input);
            suffix.push_str("[]");
        } else if input.peek(Token![&]) {
            input.parse::<Token![&]>()?;
            suffix.push('&');
        } else if input.peek(Token![*]) {
            input.parse::<Token![*]>()?;
            suffix.push('*');
        } else {
            break;
        }
    }
    Ok(suffix)
}

pub fn parse_generic_args_count(input: ParseStream) -> Result<usize> {
    if input.peek(Token![<]) {
        input.parse::<Token![<]>()?;
        let mut count = 0;
        loop {
            // consume a type (recursively or just tokens until , or >)
            // simplest: call parse_type, but ignore result
            parse_type(input)?;
            count += 1;

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else {
                break;
            }
        }
        input.parse::<Token![>]>()?;
        Ok(count)
    } else {
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_str;

    #[test]
    fn test_parse_simple_static() {
        let sig: ParsedSignature =
            parse_str("static double System.Math::Min(double, double)").unwrap();
        assert!(sig.is_static);
        assert_eq!(sig.return_type, "Float64");
        assert_eq!(sig.class_name, "System.Math");
        assert_eq!(sig.method_name, "Min");
        assert_eq!(sig.parameters, vec!["Float64", "Float64"]);
    }

    #[test]
    fn test_parse_instance_void() {
        let sig: ParsedSignature = parse_str("void MyClass::DoSomething()").unwrap();
        assert!(!sig.is_static);
        assert_eq!(sig.return_type, "Void");
        assert_eq!(sig.class_name, "MyClass");
        assert_eq!(sig.method_name, "DoSomething");
        assert!(sig.parameters.is_empty());
    }

    #[test]
    fn test_parse_qualified_params() {
        let sig: ParsedSignature = parse_str("System.String System.Console::ReadLine()").unwrap();
        assert_eq!(sig.return_type, "String"); // Normalized

        assert_eq!(sig.class_name, "System.Console");
    }

    #[test]
    fn test_parse_nested_type() {
        let sig: ParsedSignature =
            parse_str("static bool System.Runtime.Intrinsics.X86.Sse2/X64::get_IsSupported()")
                .unwrap();
        assert_eq!(sig.class_name, "System.Runtime.Intrinsics.X86.Sse2+X64");
    }

    #[test]
    fn test_parse_nested_type_parameter() {
        let sig: ParsedSignature =
            parse_str("static void MyClass::Method(Namespace.Parent/Nested)").unwrap();
        assert_eq!(sig.parameters, vec!["Namespace.Parent+Nested"]);
    }

    #[test]
    fn test_parse_field_nested_type() {
        let sig: ParsedFieldSignature =
            parse_str("static int Namespace.Parent/Nested::Field").unwrap();
        assert_eq!(sig.class_name, "Namespace.Parent+Nested");
        assert_eq!(sig.field_name, "Field");
    }

    #[test]
    fn test_parse_ref_params() {
        let sig: ParsedSignature =
            parse_str("static bool System.SpanHelpers::SequenceEqual(ref byte, byte&, int*)")
                .unwrap();
        assert_eq!(sig.parameters, vec!["UInt8&", "UInt8&", "Int32*"]);
    }
}
