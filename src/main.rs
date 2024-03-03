use std::env::args;

use dotnetdll::prelude::*;

use crate::utils::static_res_from_file;

mod resolve;
mod utils;
mod value;
mod vm;

fn main() {
    let args: Vec<_> = args().collect();
    let input_filename = args
        .get(1)
        .expect("missing input filename (TODO: usage instructions)");

    let resolution = static_res_from_file(input_filename);

    let entry_method = match resolution.entry_point {
        Some(EntryPoint::Method(m)) => m,
        Some(EntryPoint::File(f)) => todo!("find entry point in file {}", resolution[f].name),
        None => panic!("expected input module to have an entry point, received one without"),
    };

    // TODO: process args for external assemblies
    let assemblies = resolve::Assemblies::new(&resolution, std::iter::empty());

    let arena = Box::new(vm::GCArena::new(|gc| vm::CallStack::new(gc, &assemblies)));
    let mut executor = vm::Executor::new(Box::leak(arena));

    executor.entrypoint(&resolution[entry_method]);

    // TODO: collect external assemblies from args
    let assemblies = resolve::Assemblies::new(resolution, std::iter::empty());

    println!("{:#?}", executor.run())
}
