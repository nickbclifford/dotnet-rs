//! # dotnet-rs
//!
//! An experimental .NET runtime interpreter written in Rust.
//!
//! ## Feature Flags
//!
//! - `multithreading`: Enables basic multithreading support, including the .NET `Thread` class
//!   and `Monitor` synchronization primitives. Pulls in `parking_lot`.
//! - `multithreading`: Enables stop-the-world coordinated garbage collection across
//!   multiple thread-local arenas. Depends on `multithreading`.
use clap::Parser;
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

    let loader = match dotnet_assemblies::AssemblyLoader::new(args.assemblies) {
        Ok(l) => Arc::new(l),
        Err(e) => {
            eprintln!("Error initializing assembly loader: {}", e);
            return ExitCode::from(1);
        }
    };

    let resolution = match loader.load_resolution_from_file(args.entrypoint) {
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

    #[allow(clippy::arc_with_non_send_sync)]
    let shared = Arc::new(state::SharedGlobalState::new(loader));
    let mut executor = vm::Executor::new(shared);

    let entrypoint = MethodDescription::new(
        TypeDescription::new(
            resolution,
            &resolution.definition()[entry_method.parent_type()],
            entry_method.parent_type(),
        ),
        resolution,
        &resolution.definition()[entry_method],
    );
    executor.entrypoint(entrypoint);

    let result = executor.run();
    match result {
        ExecutorResult::Exited(i) => {
            std::process::exit(i as i32);
        }
        ExecutorResult::Threw(exc) => {
            eprintln!("{}", exc);
            ExitCode::from(1)
        }
        ExecutorResult::Error(e) => {
            eprintln!("Internal VM error: {}", e);
            ExitCode::from(1)
        }
    }
}
