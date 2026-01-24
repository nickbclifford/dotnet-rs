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
use crate::vm::ExecutorResult;
use clap::Parser;
use dotnet_assemblies::static_res_from_file;
use dotnetdll::prelude::*;
use std::process::ExitCode;

pub use dotnet_assemblies as assemblies;
pub use dotnet_types;
pub use dotnet_utils as utils;
pub use dotnet_value as value;
pub use dotnet_vm as vm;

use dotnet_types::{members::MethodDescription, TypeDescription};
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

    let resolution = static_res_from_file(args.entrypoint);

    let entry_method = match resolution.entry_point {
        Some(EntryPoint::Method(m)) => m,
        Some(EntryPoint::File(f)) => todo!("find entry point in file {}", resolution[f].name),
        None => panic!("expected input module to have an entry point, received one without"),
    };

    let loader = assemblies::AssemblyLoader::new(args.assemblies);
    let loader = Box::leak(Box::new(loader));

    let shared = Arc::new(state::SharedGlobalState::new(loader));
    let mut executor = vm::Executor::new(shared);

    let entrypoint = MethodDescription {
        parent: TypeDescription::new(
            resolution,
            &resolution.definition()[entry_method.parent_type()],
        ),
        method_resolution: resolution,
        method: &resolution.definition()[entry_method],
    };
    executor.entrypoint(entrypoint);

    let result = executor.run();
    match result {
        ExecutorResult::Exited(i) => ExitCode::from(i),
        ExecutorResult::Threw => todo!("pretty output for crashing on unhandled exception"),
    }
}
