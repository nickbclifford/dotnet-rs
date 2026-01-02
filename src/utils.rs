use dotnetdll::prelude::*;
use std::{
    fmt::{Debug, Formatter},
    hash::Hash,
    io::Read,
    ops::Deref,
    path::{Path, PathBuf},
};

use gc_arena::{Collect, unsafe_empty_collect};

#[derive(Clone, Copy)]
pub struct ResolutionS(pub &'static Resolution<'static>);
unsafe_empty_collect!(ResolutionS);
impl ResolutionS {
    pub fn as_raw(self) -> *const Resolution<'static> {
        self.0 as *const _
    }

    /// # Safety
    ///
    /// The `data` slice must contain a valid pointer to a `Resolution<'static>` in native endianness.
    pub unsafe fn from_raw(data: &[u8]) -> Self {
        let mut res_data = [0u8; size_of::<usize>()];
        res_data.copy_from_slice(data);
        let res = &*(usize::from_ne_bytes(res_data) as *const _);
        Self(res)
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
        write!(
            f,
            "ResolutionS({} @ {:#?})",
            self.0.assembly.as_ref().unwrap().name,
            self.as_raw()
        )
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

pub fn find_dotnet_sdk_path() -> Option<PathBuf> {
    let mut search_paths = vec![PathBuf::from(
        "/usr/share/dotnet/shared/Microsoft.NETCore.App",
    )];

    if let Ok(home) = std::env::var("HOME") {
        search_paths.push(PathBuf::from(home).join(".dotnet/shared/Microsoft.NETCore.App"));
    }

    if let Ok(dotnet_root) = std::env::var("DOTNET_ROOT") {
        search_paths.insert(
            0,
            PathBuf::from(dotnet_root).join("shared/Microsoft.NETCore.App"),
        );
    }

    for base_path in search_paths {
        if !base_path.exists() {
            continue;
        }

        let mut versions: Vec<_> = std::fs::read_dir(base_path)
            .ok()?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.path())
            .collect();

        versions.sort();
        if let Some(latest) = versions.pop() {
            return Some(latest);
        }
    }

    None
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
