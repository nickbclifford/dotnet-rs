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
    println!("{:#?}", resolution)
}
