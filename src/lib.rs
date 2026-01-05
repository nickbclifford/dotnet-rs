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

use crate::{utils::static_res_from_file, vm::ExecutorResult};
use clap::Parser;
use dotnetdll::prelude::*;
use std::process::ExitCode;

pub mod assemblies;
pub mod types;
pub mod utils;
pub mod value;
#[macro_use]
pub mod vm;

use types::{members::MethodDescription, TypeDescription};

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

    let shared = std::sync::Arc::new(vm::SharedGlobalState::new(loader));
    let mut executor = vm::Executor::new(shared);

    let entrypoint = MethodDescription {
        parent: TypeDescription::new(
            resolution,
            &resolution.definition()[entry_method.parent_type()],
        ),
        method: &resolution.definition()[entry_method],
    };
    executor.entrypoint(entrypoint);

    let result = executor.run();
    match result {
        ExecutorResult::Exited(i) => ExitCode::from(i),
        ExecutorResult::Threw => todo!("pretty output for crashing on unhandled exception"),
    }
}
