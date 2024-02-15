mod value;
mod vm;

use dotnetdll::prelude::*;
use std::{env::args, fs::File, io::prelude::*};

fn main() {
    let args: Vec<_> = args().collect();
    let input_filename = args
        .get(1)
        .expect("missing input filename (TODO: usage instructions)");
    let mut input_file =
        File::open(input_filename).expect(&format!("could not open file {}", input_filename));
    let mut input_buf = vec![];
    input_file
        .read_to_end(&mut input_buf)
        .expect("failed to read input file");

    let resolution = Resolution::parse(
        // TODO: turn all these Box::leaks into lazy statics?
        Box::leak(input_buf.into_boxed_slice()),
        ReadOptions::default(),
    )
    .expect("failed to parse input file as .NET metadata");
    let resolution = Box::leak(Box::new(resolution));

    let entry_method = match resolution.entry_point {
        Some(EntryPoint::Method(m)) => m,
        Some(EntryPoint::File(f)) => todo!("find entry point in file {}", resolution[f].name),
        None => panic!("expected input module to have an entry point, received one without"),
    };

    let arena = Box::new(vm::GCArena::new(|gc| vm::CallStack::new(gc)));

    let mut executor = vm::Executor::new(Box::leak(arena), &resolution[entry_method]);

    let result = executor.run();

    println!("{:#?}", result)
}
