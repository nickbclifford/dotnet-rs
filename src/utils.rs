use dotnetdll::prelude::{ReadOptions, Resolution};
use std::io::Read;
use std::path::Path;

pub fn static_res_from_file(path: impl AsRef<Path>) -> &'static Resolution<'static> {
    let mut file = std::fs::File::open(path).expect("could not open file");
    let mut buf = vec![];
    file.read_to_end(&mut buf).expect("failed to read file");
    let resolution = Resolution::parse(Box::leak(buf.into_boxed_slice()), ReadOptions::default())
        .expect("failed to parse file as .NET metadata");
    Box::leak(Box::new(resolution))
}
