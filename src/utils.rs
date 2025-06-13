use dotnetdll::prelude::*;
use std::{
    fmt::{Debug, Formatter},
    io::Read,
    path::Path,
};
use std::hash::Hash;
use std::ops::Deref;

#[derive(Clone, Copy)]
pub struct ResolutionS(pub &'static Resolution<'static>);
impl ResolutionS {
    pub fn as_raw(self) -> *const Resolution<'static> {
        self.0 as *const _
    }
}
impl Deref for ResolutionS {
    type Target = Resolution<'static>;
    fn deref(&self) -> &'static Self::Target {
        self.0
    }
}
impl Debug for ResolutionS {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "ResolutionS({:#?})", self.as_raw())
    }   
}
impl PartialEq for ResolutionS {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}
impl Eq for ResolutionS {}
impl Hash for ResolutionS {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::ptr::hash(self.0, state);
    }
}

pub fn static_res_from_file(path: impl AsRef<Path>) -> ResolutionS {
    let mut file = std::fs::File::open(&path)
        .unwrap_or_else(|e| panic!("could not open file {} ({:?})", path.as_ref().display(), e));
    let mut buf = vec![];
    file.read_to_end(&mut buf).expect("failed to read file");
    let resolution = Resolution::parse(Box::leak(buf.into_boxed_slice()), ReadOptions::default())
        .expect("failed to parse file as .NET metadata");
    ResolutionS(Box::leak(Box::new(resolution)))
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
