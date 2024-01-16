mod vm;
mod value;

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
    let resolution = Resolution::parse(&input_buf, ReadOptions::default())
        .expect("failed to parse input file as .NET metadata");

    let entry_method = match resolution.entry_point {
        Some(EntryPoint::Method(m)) => m,
        Some(EntryPoint::File(f)) => todo!("find entry point in file {}", resolution[f].name),
        None => panic!("expected input module to have an entry point, received one without")
    };

    // https://craftinginterpreters.com/a-bytecode-virtual-machine.html

    println!("{:#?}", resolution)
}
