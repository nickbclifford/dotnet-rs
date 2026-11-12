//! # dotnet-macros
//!
//! Procedural macros for the dotnet-rs VM.
//! Includes `#[dotnet_intrinsic]` for BCL methods and `#[dotnet_instruction]` for CIL instructions.
extern crate proc_macro;

use dotnet_macros_core::{
    InstructionMapping, IntrinsicArgs, ParsedInstruction, ParsedSignature, expand_trait_aliases,
    validate_intrinsic_options,
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

fn is_named_type(ty: &syn::Type, name: &str) -> bool {
    let syn::Type::Path(path) = ty else {
        return false;
    };
    path.qself.is_none()
        && path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == name)
        && path
            .path
            .segments
            .iter()
            .all(|segment| matches!(segment.arguments, syn::PathArguments::None))
}

fn validate_async_intrinsic_signature(sig: &syn::Signature) -> syn::Result<()> {
    if sig.inputs.len() != 1 {
        return Err(syn::Error::new(
            sig.paren_token.span.join(),
            "async intrinsic requires exactly one `VmEffects` parameter",
        ));
    }
    match &sig.inputs[0] {
        syn::FnArg::Typed(param) if is_named_type(&param.ty, "VmEffects") => {}
        syn::FnArg::Typed(param) => {
            return Err(syn::Error::new_spanned(
                &param.ty,
                "async intrinsic parameter must be `VmEffects`",
            ));
        }
        receiver => {
            return Err(syn::Error::new_spanned(
                receiver,
                "async intrinsic parameter must be `VmEffects`",
            ));
        }
    }
    match &sig.output {
        syn::ReturnType::Type(_, ty) if is_named_type(ty, "NativeResult") => Ok(()),
        syn::ReturnType::Type(_, ty) => Err(syn::Error::new_spanned(
            ty,
            "async intrinsic return type must be `NativeResult`",
        )),
        syn::ReturnType::Default => Err(syn::Error::new_spanned(
            &sig.ident,
            "async intrinsic return type must be `NativeResult`",
        )),
    }
}

fn expand_intrinsic(
    args: IntrinsicArgs,
    mut func: ItemFn,
) -> syn::Result<proc_macro2::TokenStream> {
    let is_async = func.sig.asyncness.is_some();
    let mut annotations = vec![args];
    if is_async {
        let mut retained = Vec::new();
        for attr in func.attrs {
            if attr.path().is_ident("dotnet_intrinsic") {
                annotations.push(attr.parse_args()?);
            } else {
                retained.push(attr);
            }
        }
        func.attrs = retained;
    }
    validate_intrinsic_options(&annotations, is_async)?;
    if is_async {
        validate_async_intrinsic_signature(&func.sig)?;
    }
    let func_name = &func.sig.ident;
    let filters = annotations
        .iter()
        .map(|attr| {
            let parsed: ParsedSignature = syn::parse_str(&attr.signature.value())?;
            Ok(generate_intrinsic_filter_fn(func_name, &parsed))
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let entry = if is_async {
        let entry_name = syn::Ident::new(&format!("{}_native_entry", func_name), func_name.span());
        let selector_alias = annotations[0].selector.as_ref().map(|selector| {
            let name = syn::Ident::new(&format!("{}_native_selector", func_name), func_name.span());
            quote! { #[doc(hidden)] pub use #selector as #name; }
        });
        let fallback_alias = annotations[0].fallback.as_ref().map(|fallback| {
            let name = syn::Ident::new(&format!("{}_native_fallback", func_name), func_name.span());
            quote! { #[doc(hidden)] pub use #fallback as #name; }
        });
        quote! {
            #selector_alias
            #fallback_alias
            #[doc(hidden)]
            pub fn #entry_name<'gc, T, S>(
                ctx: &mut T,
                method: dotnet_types::members::MethodDescription,
                generics: &dotnet_types::generics::GenericLookup,
                mode: dotnet_vm_ops::effects::NativeDispatchMode,
            ) -> dotnet_vm_data::StepResult
            where
                T: dotnet_vm_ops::effects::NativeIntrinsicHost<'gc>,
                S: dotnet_vm_ops::effects::NativeSelectorProvider<'gc, T>,
            {
                ctx.start_native(method, mode, generics, S::selector(), S::fallback(), |fx| Box::pin(#func_name(fx)))
            }
        }
    } else {
        quote! {}
    };
    Ok(quote! { #func #(#filters)* #entry })
}

/// Registers a synchronous or async native implementation for a managed method.
///
/// Async handlers must take one `VmEffects` argument and return `NativeResult`; they may provide
/// `fallback = path`, `select = path`, both, or neither. The selector runs before operands are
/// consumed and chooses async execution, the registered `fallback` (`NativeSelection::Fallback`,
/// so the selector never names it), or rejection. Without a selector the whole parameter list is
/// the invocation. Admitted threading and Substring bodies validate committed arguments and
/// construct rooted managed exceptions; reflection still excludes its unsupported shapes.
/// Sleep, finalizer wait, and Monitor.Enter/TryEnter/Wait have no fallback. A measured bulk
/// intrinsic may select a synchronous fast path for valid small inputs and use its async body for
/// larger inputs: Substring does this for at most one chunk; invalid ranges reach the body.
/// Inline IL calls include
/// constrained and tail calls, single-cast delegate targets and abstract redirects; non-inline
/// retained non-inline continuations use the optional fallback, or return `NotImplemented`
/// without selecting or consuming operands when it is absent. Synchronous registrations keep
/// their existing convention.
#[proc_macro_attribute]
pub fn dotnet_intrinsic(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as IntrinsicArgs);
    let func = parse_macro_input!(item as ItemFn);
    expand_intrinsic(args, func)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
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
    fn async_intrinsic_expands_once_for_stacked_annotations() {
        let args: IntrinsicArgs = syn::parse_quote!(
            "static void Test.Type::First()",
            select = self::choose,
            fallback = self::sync
        );
        let func: ItemFn = syn::parse_quote! {
            #[dotnet_intrinsic("static void Test.Type::Second(int)", select = self::choose, fallback = self::sync)]
            pub async fn work(mut fx: VmEffects) -> NativeResult { let _ = &mut fx; Ok(()) }
        };
        let file: syn::File = syn::parse2(expand_intrinsic(args, func).unwrap()).unwrap();
        let functions: Vec<_> = file
            .items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Fn(f) => Some(f.sig.ident.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(
            functions
                .iter()
                .filter(|name| name.starts_with("work_filter_"))
                .count(),
            2
        );
        assert_eq!(
            functions
                .iter()
                .filter(|name| *name == "work_native_entry")
                .count(),
            1
        );
        let output = quote!(#file).to_string();
        assert!(output.contains("start_native"));
        assert!(output.contains("work_native_selector"));
        assert!(output.contains("work_native_fallback"));
    }

    #[test]
    fn async_intrinsic_rejects_mismatched_selectors() {
        let args: IntrinsicArgs = syn::parse_quote!(
            "static void Test.Type::First()",
            select = one,
            fallback = sync
        );
        for attr in [
            syn::parse_quote!(#[dotnet_intrinsic("static void Test.Type::Second()", select = two, fallback = sync)]),
            syn::parse_quote!(#[dotnet_intrinsic("static void Test.Type::Second()", fallback = sync)]),
        ] {
            let mut func: ItemFn = syn::parse_quote!(
                async fn work(fx: VmEffects) -> NativeResult {
                    Ok(())
                }
            );
            func.attrs.push(attr);
            assert!(
                expand_intrinsic(
                    syn::parse_quote!(
                        "static void Test.Type::First()",
                        select = one,
                        fallback = sync
                    ),
                    func
                )
                .unwrap_err()
                .to_string()
                .contains("same selector")
            );
        }
        let sync: ItemFn = syn::parse_quote!(
            fn work() {}
        );
        assert!(
            expand_intrinsic(args, sync)
                .unwrap_err()
                .to_string()
                .contains("require an async")
        );
    }

    #[test]
    fn async_intrinsic_body_only_and_selector_only() {
        for args in [
            syn::parse_quote!("static void X::A()"),
            syn::parse_quote!("static void X::A()", select = choose),
        ] {
            let func: ItemFn = syn::parse_quote!(
                pub async fn work(fx: VmEffects) -> NativeResult {
                    Ok(())
                }
            );
            let output = expand_intrinsic(args, func).unwrap().to_string();
            assert!(output.contains("work_native_entry"));
            assert!(!output.contains("work_native_fallback"));
        }
    }

    #[test]
    fn async_intrinsic_requires_consistent_fallbacks() {
        for (other, expected) in [
            (
                syn::parse_quote!(#[dotnet_intrinsic("static void X::B()", fallback = other)]),
                "same fallback",
            ),
            (
                syn::parse_quote!(#[dotnet_intrinsic("static void X::B()")]),
                "same fallback",
            ),
        ] {
            let mut func: ItemFn = syn::parse_quote!(
                async fn work(fx: VmEffects) -> NativeResult {
                    Ok(())
                }
            );
            func.attrs.push(other);
            let args = syn::parse_quote!("static void X::A()", fallback = sync);
            assert!(
                expand_intrinsic(args, func)
                    .unwrap_err()
                    .to_string()
                    .contains(expected)
            );
        }
        let args = syn::parse_quote!("static void X::A()");
        let func: ItemFn = syn::parse_quote!(
            async fn work(fx: VmEffects) -> NativeResult {
                Ok(())
            }
        );
        let output = expand_intrinsic(args, func).unwrap().to_string();
        assert!(!output.contains("work_native_fallback"));
    }

    #[test]
    fn async_intrinsic_without_selector_and_sync_entry_shape() {
        let args: IntrinsicArgs =
            syn::parse_quote!("static void Test.Type::First()", fallback = sync);
        let func: ItemFn = syn::parse_quote!(
            async fn work(fx: VmEffects) -> NativeResult {
                Ok(())
            }
        );
        let output = expand_intrinsic(args, func).unwrap().to_string();
        assert!(output.contains("work_native_entry"));
        assert!(!output.contains("work_native_selector"));
        let args: IntrinsicArgs = syn::parse_quote!("static void Test.Type::First()");
        let func: ItemFn = syn::parse_quote!(
            pub fn work() {}
        );
        let output = expand_intrinsic(args, func).unwrap().to_string();
        assert!(!output.contains("native_entry"));
    }

    fn expand_async_signature(func: ItemFn) -> syn::Result<proc_macro2::TokenStream> {
        expand_intrinsic(
            syn::parse_quote!("static void Test.Type::Work()", fallback = sync),
            func,
        )
    }

    #[test]
    fn async_intrinsic_rejects_wrong_parameter_count() {
        for func in [
            syn::parse_quote!(
                async fn work() -> NativeResult {}
            ),
            syn::parse_quote!(
                async fn work(fx: VmEffects, other: VmEffects) -> NativeResult {}
            ),
        ] {
            let error = expand_async_signature(func).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("exactly one `VmEffects` parameter")
            );
        }
    }

    #[test]
    fn async_intrinsic_rejects_wrong_parameter_type() {
        for func in [
            syn::parse_quote!(
                async fn work(fx: Other) -> NativeResult {}
            ),
            syn::parse_quote!(
                async fn work(fx: &mut VmEffects) -> NativeResult {}
            ),
            syn::parse_quote!(
                async fn work(fx: VmEffects<u8>) -> NativeResult {}
            ),
            syn::parse_quote!(
                async fn work(fx: (VmEffects,)) -> NativeResult {}
            ),
        ] {
            let error = expand_async_signature(func).unwrap_err();
            assert!(error.to_string().contains("parameter must be `VmEffects`"));
        }
    }

    #[test]
    fn async_intrinsic_rejects_wrong_return_type() {
        for func in [
            syn::parse_quote!(
                async fn work(fx: VmEffects) {}
            ),
            syn::parse_quote!(
                async fn work(fx: VmEffects) -> () {}
            ),
            syn::parse_quote!(
                async fn work(fx: VmEffects) -> Other {}
            ),
            syn::parse_quote!(
                async fn work(fx: VmEffects) -> NativeResult<u8> {}
            ),
            syn::parse_quote!(
                async fn work(fx: VmEffects) -> &NativeResult {}
            ),
        ] {
            let error = expand_async_signature(func).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("return type must be `NativeResult`")
            );
        }
    }

    #[test]
    fn async_intrinsic_accepts_qualified_types_and_any_parameter_name() {
        for func in [
            syn::parse_quote!(
                async fn work(mut fx: VmEffects) -> NativeResult {}
            ),
            syn::parse_quote!(
                async fn work(
                    effects: dotnet_vm_ops::effects::VmEffects,
                ) -> dotnet_vm_ops::effects::NativeResult {
                }
            ),
            syn::parse_quote!(
                async fn work(
                    _: ::dotnet_vm_ops::effects::VmEffects,
                ) -> ::dotnet_vm_ops::effects::NativeResult {
                }
            ),
        ] {
            assert!(expand_async_signature(func).is_ok());
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
