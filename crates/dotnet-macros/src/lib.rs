extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};
use syn::{parse_macro_input, ItemFn, LitStr};

mod signature;

use signature::{ParsedFieldSignature, ParsedSignature};

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
    let class_name = &parsed.class_name;
    let method_name = &parsed.method_name;
    let mut param_count = parsed.parameters.len();
    if !is_static {
        param_count += 1;
    }

    // Generate filter checks
    let param_checks = parsed.parameters.iter().enumerate().map(|(i, p)| {
        if let Some(base_type) = match_primitive(p) {
            quote! {
                // Check if parameter matches Expected Type.
                {
                    use dotnetdll::prelude::*;
                    if let Some(Parameter(_, ParameterType::Value(MethodType::Base(b)))) =
                        method.method.signature.parameters.get(#i) {
                        if !matches!(**b, #base_type) { return false; }
                    } else {
                        return false;
                    }
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
        fn #filter_name(method: &dotnet_types::members::MethodDescription) -> bool {
            #(#param_checks)*
            true
        }
    };

    let submit = quote! {
        inventory::submit! {
            crate::intrinsics::IntrinsicEntry {
                class_name: #class_name,
                method_name: #method_name,
                signature: #normalized_sig,
                handler: unsafe { std::mem::transmute(#func_name as *const ()) },
                is_static: #is_static,
                param_count: #param_count,
                signature_filter: Some(#filter_name),
            }
        }
    };

    let output = quote! {
        #func
        #filter_fn
        #submit
    };

    output.into()
}

#[proc_macro_attribute]
pub fn dotnet_intrinsic_field(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr_str = parse_macro_input!(attr as LitStr);
    let func = parse_macro_input!(item as ItemFn);
    let func_name = &func.sig.ident;

    let sig_str = attr_str.value();
    let parsed: ParsedFieldSignature = match syn::parse_str(&sig_str) {
        Ok(s) => s,
        Err(e) => return e.to_compile_error().into(),
    };

    // We assume the macro is used within dotnet-vm, and IntrinsicFieldEntry is available
    // at crate::intrinsics::IntrinsicFieldEntry.
    let class_name = &parsed.class_name;
    let field_name = &parsed.field_name;

    let submit = quote! {
        inventory::submit! {
            crate::intrinsics::IntrinsicFieldEntry {
                class_name: #class_name,
                field_name: #field_name,
                handler: unsafe { std::mem::transmute(#func_name as *const ()) },
            }
        }
    };

    let output = quote! {
        #func
        #submit
    };

    output.into()
}

#[proc_macro_attribute]
pub fn dotnet_instruction(attr: TokenStream, item: TokenStream) -> TokenStream {
    let variant_ident = parse_macro_input!(attr as syn::Ident);
    let func = parse_macro_input!(item as ItemFn);
    let func_name = &func.sig.ident;
    let variant_name = variant_ident.to_string();

    let args = &func.sig.inputs;
    let extra_args = args.iter().skip(2);
    let mut extra_arg_names = Vec::new();
    let mut extra_arg_calls = Vec::new();
    for arg in extra_args {
        if let syn::FnArg::Typed(pat_type) = arg {
            if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                let name = &pat_ident.ident;
                extra_arg_names.push(name);
                if let syn::Type::Reference(_) = &*pat_type.ty {
                    extra_arg_calls.push(quote! { #name });
                } else {
                    extra_arg_calls.push(quote! { *#name });
                }
            } else {
                panic!("Instruction handler must have named parameters");
            }
        } else {
            panic!("Instruction handler must have named parameters");
        }
    }

    let wrapper_name = syn::Ident::new(&format!("{}_wrapper", func_name), func_name.span());

    // We generate a match arm.
    let is_likely_struct = variant_name == "Call"
        || variant_name == "CallVirtual"
        || variant_name == "CallIndirect"
        || variant_name == "LoadField"
        || variant_name == "StoreField"
        || variant_name == "LoadStaticField"
        || variant_name == "StoreStaticField"
        || variant_name == "LoadElement"
        || variant_name == "StoreElement"
        || variant_name == "LoadElementAddress"
        || variant_name == "LoadElementPrimitive"
        || variant_name == "StoreElementPrimitive"
        || variant_name == "CastClass"
        || variant_name == "UnboxIntoAddress"
        || variant_name == "LoadObject"
        || variant_name == "StoreObject"
        || variant_name == "CopyMemoryBlock"
        || variant_name == "InitializeMemoryBlock"
        || variant_name == "StoreIndirect"
        || variant_name == "LoadIndirect";

    let match_arm = if is_likely_struct {
        quote! {
            Instruction::#variant_ident { #(#extra_arg_names,)* .. } => #func_name(gc, stack, #(#extra_arg_calls),*)
        }
    } else if extra_arg_names.is_empty() {
        quote! {
            Instruction::#variant_ident => #func_name(gc, stack)
        }
    } else {
        quote! {
            Instruction::#variant_ident(#(#extra_arg_names),*) => #func_name(gc, stack, #(#extra_arg_calls),*)
        }
    };

    let submit = quote! {
        inventory::submit! {
            crate::dispatch::InstructionEntry {
                name: #variant_name,
                handler: unsafe { std::mem::transmute(#wrapper_name as *const ()) },
            }
        }
    };

    let output = quote! {
        #func

        fn #wrapper_name<'gc, 'm: 'gc>(
            gc: dotnet_utils::gc::GCHandle<'gc>,
            stack: &mut crate::CallStack<'gc, 'm>,
            instr: &Instruction
        ) -> crate::StepResult {
            match instr {
                #match_arm,
                _ => unreachable!("Instruction wrapper called with wrong variant: expected {}, received {:?}", #variant_name, instr)
            }
        }

        #submit
    };

    output.into()
}
