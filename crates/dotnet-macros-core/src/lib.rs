//! # dotnet-macros-core
//!
//! Core logic for macro expansion used by `dotnet-macros`.
//! This crate contains the parsing and transformation logic for .NET signatures.
use syn::{
    Ident, Result, Token,
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

pub fn parse_type(input: ParseStream) -> Result<String> {
    let mut segments = Vec::new();
    let mut separators = Vec::new();

    loop {
        let ident: Ident = input.parse()?;
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
    Ok(type_name)
}

pub fn parse_class_name(input: ParseStream) -> Result<String> {
    let mut segments = Vec::new();
    let mut separators = Vec::new();

    loop {
        let ident: Ident = input.parse()?;
        let mut segment = ident.to_string();

        // Generics
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

    let mut res = segments[0].clone();
    for i in 0..separators.len() {
        res.push(separators[i]);
        res.push_str(&segments[i + 1]);
    }
    res.push_str(&suffix);
    Ok(res)
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
}
