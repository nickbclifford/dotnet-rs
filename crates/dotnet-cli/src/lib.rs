//! # dotnet-rs
//!
//! An experimental .NET runtime interpreter written in Rust.
//!
//! ## Feature Flags
//!
//! - `multithreading`: Enables basic multithreading support, including the .NET `Thread` class
//!   and `Monitor` synchronization primitives. Pulls in `parking_lot`.
//! - `multithreaded-gc`: Enables stop-the-world coordinated garbage collection across
//!   multiple thread-local arenas. Depends on `multithreading`.
use clap::Parser;
use dotnet_assemblies::try_static_res_from_file;
use dotnet_types::{TypeDescription, members::MethodDescription};
use dotnet_vm::{self as vm, ExecutorResult};
use dotnetdll::prelude::*;
use std::process::ExitCode;
use vm::{state, sync::Arc};

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "An experimental interpreter for the .NET runtime"
)]
pub struct Args {
    #[arg(short, long, value_name = "FOLDER")]
    pub assemblies: String,
    #[arg(value_name = "DLL")]
    pub entrypoint: String,
}

pub fn run_cli() -> ExitCode {
    let args = Args::parse();

    let resolution = match try_static_res_from_file(args.entrypoint) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error loading entry point: {}", e);
            return ExitCode::from(1);
        }
    };

    let entry_method = match resolution.entry_point {
        Some(EntryPoint::Method(m)) => m,
        Some(EntryPoint::File(f)) => todo!("find entry point in file {}", resolution[f].name),
        None => panic!("expected input module to have an entry point, received one without"),
    };

    let loader = match dotnet_assemblies::AssemblyLoader::new(args.assemblies) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Error initializing assembly loader: {}", e);
            return ExitCode::from(1);
        }
    };
    let loader = Box::leak(Box::new(loader));

    #[allow(clippy::arc_with_non_send_sync)]
    let shared = Arc::new(state::SharedGlobalState::new(loader));
    let mut executor = vm::Executor::new(shared);

    let entrypoint = MethodDescription {
        parent: TypeDescription::new(
            resolution,
            &resolution.definition()[entry_method.parent_type()],
            entry_method.parent_type(),
        ),
        method_resolution: resolution,
        method: &resolution.definition()[entry_method],
    };
    executor.entrypoint(entrypoint);

    let result = executor.run();
    match result {
        ExecutorResult::Exited(i) => ExitCode::from(i),
        ExecutorResult::Threw => todo!("pretty output for crashing on unhandled exception"),
        ExecutorResult::Error(e) => {
            eprintln!("Internal VM error: {}", e);
            ExitCode::from(1)
        }
    }
}
