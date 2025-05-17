use dotnetdll::prelude::*;
use std::{
    fmt::{Debug, Formatter},
    io::Read,
    path::Path,
};

pub type ResolutionS = &'static Resolution<'static>;

pub fn static_res_from_file(path: impl AsRef<Path>) -> ResolutionS {
    let mut file = std::fs::File::open(&path)
        .unwrap_or_else(|e| panic!("could not open file {} ({:?})", path.as_ref().display(), e));
    let mut buf = vec![];
    file.read_to_end(&mut buf).expect("failed to read file");
    let resolution = Resolution::parse(Box::leak(buf.into_boxed_slice()), ReadOptions::default())
        .expect("failed to parse file as .NET metadata");
    Box::leak(Box::new(resolution))
}

pub struct DebugStr(pub String);

impl Debug for DebugStr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub fn decompose_type_source<T: Clone>(t: &TypeSource<T>) -> (UserType, Vec<T>) {
    let mut type_generics: &[T] = &[];
    let ut = match t {
        TypeSource::User(u) => *u,
        TypeSource::Generic { base, parameters } => {
            type_generics = parameters.as_slice();
            *base
        }
    };
    (ut, type_generics.to_vec())
}
