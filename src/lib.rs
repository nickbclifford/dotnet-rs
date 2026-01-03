use crate::{
    utils::static_res_from_file
    ,
    vm::ExecutorResult,
};

use clap::Parser;
use dotnetdll::prelude::*;
use std::process::ExitCode;
use value::description::{MethodDescription, TypeDescription};

#[macro_use]
pub mod vm;
pub mod resolve;
pub mod utils;
pub mod value;

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

    let assemblies = resolve::Assemblies::new(args.assemblies);
    let assemblies = Box::leak(Box::new(assemblies));

    let arena = Box::new(vm::GCArena::new(|gc| vm::CallStack::new(gc, assemblies)));
    let mut executor = vm::Executor::new(Box::leak(arena));

    let entrypoint = MethodDescription {
        parent: TypeDescription {
            resolution,
            definition: &resolution.0[entry_method.parent_type()],
        },
        method: &resolution.0[entry_method],
    };
    executor.entrypoint(entrypoint);

    let result = executor.run();
    match result {
        ExecutorResult::Exited(i) => ExitCode::from(i),
        ExecutorResult::Threw => todo!("pretty output for crashing on unhandled exception"),
    }
}
