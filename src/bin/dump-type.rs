use clap::Parser;
use dotnet_rs::utils::{find_dotnet_sdk_path, static_res_from_file};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Dump the structure and bodies of a specified type"
)]
struct Args {
    /// The assembly to search in (e.g. System.Runtime, or path to a DLL)
    assembly: String,
    /// The full name of the type to dump (e.g. System.Object)
    type_name: String,
}

fn resolve_assembly_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(name);

    // 1. Check if it's a direct path
    if path.exists() && path.is_file() {
        return path;
    }

    // 2. Try adding .dll
    if path.extension().is_none() {
        path.set_extension("dll");
        if path.exists() && path.is_file() {
            return path;
        }
    }

    // 3. Search in .NET SDK
    if let Some(sdk_path) = find_dotnet_sdk_path() {
        let mut sdk_file = sdk_path.clone();
        let name_with_ext = if name.ends_with(".dll") {
            name.to_string()
        } else {
            format!("{}.dll", name)
        };
        sdk_file.push(&name_with_ext);
        if sdk_file.exists() && sdk_file.is_file() {
            return sdk_file;
        }
    }

    // 4. Just return what we have and let static_res_from_file handle the error
    PathBuf::from(name)
}

fn main() {
    let args = Args::parse();

    let mut current_assembly = args.assembly.clone();
    let mut resolution;

    loop {
        let assembly_path = resolve_assembly_path(&current_assembly);
        resolution = static_res_from_file(&assembly_path);

        eprintln!(
            "Assembly: {}, Types: {}, MethodRefs: {}",
            current_assembly,
            resolution.definition().type_definitions.len(),
            resolution.definition().method_references.len()
        );

        // Find the type
        let mut found_type = None;
        for (i, type_def) in resolution.definition().type_definitions.iter().enumerate() {
            let namespace = type_def.namespace.as_deref().unwrap_or("");
            let name = &type_def.name;
            let full_name = if namespace.is_empty() {
                name.to_string()
            } else {
                format!("{}.{}", namespace, name)
            };
            if full_name == args.type_name {
                println!("Found type at index: {}", i);
                found_type = Some(type_def);
                break;
            }
        }

        if let Some(type_def) = found_type {
            print_type_info(resolution, type_def);
            return;
        }

        // Check exported types
        let mut forwarded_to = None;
        for e in &resolution.definition().exported_types {
            let namespace = e.namespace.as_deref().unwrap_or("");
            let name = &e.name;
            let full_name = if namespace.is_empty() {
                name.to_string()
            } else {
                format!("{}.{}", namespace, name)
            };

            if full_name == args.type_name {
                use dotnetdll::prelude::TypeImplementation;
                if let TypeImplementation::TypeForwarder(a) = e.implementation {
                    let forward_to = &resolution.definition()[a];
                    forwarded_to = Some(forward_to.name.to_string());
                }
                break;
            }
        }

        if let Some(next_assembly) = forwarded_to {
            eprintln!(
                "Type '{}' is forwarded from '{}' to '{}'",
                args.type_name, current_assembly, next_assembly
            );
            current_assembly = next_assembly;
            continue;
        }

        eprintln!("Type '{}' not found in assembly", args.type_name);
        println!("\nAvailable types:");
        for t in &resolution.definition().type_definitions {
            let ns = t.namespace.as_deref().unwrap_or("");
            if ns.is_empty() {
                println!("  {}", t.name);
            } else {
                println!("  {}.{}", ns, t.name);
            }
        }
        std::process::exit(1);
    }
}

fn print_type_info(
    _resolution: dotnet_rs::utils::ResolutionS,
    type_def: &dotnetdll::prelude::TypeDefinition,
) {
    println!("\nFields:");
    for field in &type_def.fields {
        println!(
            "  - {}: {:?} {:?}",
            field.name, field.type_modifiers, field.return_type
        );
    }

    println!("\nMethods:");
    for method in &type_def.methods {
        println!("  - {}: {:?}", method.name, method.signature);
        if let Some(body) = &method.body {
            println!("    Instructions:");
            for instr in &body.instructions {
                println!("      {:?}", instr);
            }
        } else {
            println!("    (No body)");
        }
    }

    println!("\nProperties:");
    for prop in &type_def.properties {
        println!(
            "  - {}: getter: {:?}, setter: {:?}",
            prop.name, prop.getter, prop.setter
        );
    }
}
