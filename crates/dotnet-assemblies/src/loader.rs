use crate::error::AssemblyLoadError;
use dashmap::DashMap;
use dotnet_types::{
    TypeDescription,
    comparer::TypeComparer,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
    resolution::ResolutionS,
};
use dotnet_utils::sync::{AtomicU64, RwLock};
use dotnetdll::prelude::*;
use gc_arena::static_collect;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

/// Owns leaked metadata and byte slices to allow recapture on drop.
pub struct MetadataOwner {
    resolutions: Vec<*mut Resolution<'static>>,
    u64_slices: Vec<*mut [u64]>,
}

impl MetadataOwner {
    pub fn new() -> Self {
        Self {
            resolutions: Vec::new(),
            u64_slices: Vec::new(),
        }
    }
}

impl Default for MetadataOwner {
    fn default() -> Self {
        Self::new()
    }
}

impl MetadataOwner {
    /// # Safety
    ///
    /// The pointer must have been obtained from `Box::into_raw(Box::new(res))`.
    pub unsafe fn add_resolution(&mut self, res: *const Resolution<'static>) {
        self.resolutions.push(res as *mut _);
    }

    /// # Safety
    ///
    /// The pointer must have been obtained from `Box::into_raw(vec.into_boxed_slice())`.
    pub unsafe fn add_u64_slice(&mut self, slice: *mut [u64]) {
        self.u64_slices.push(slice);
    }
}

impl Drop for MetadataOwner {
    fn drop(&mut self) {
        // Drop resolutions first as they may contain references to the slices
        for res in self.resolutions.drain(..) {
            // SAFETY: The pointer was added via add_resolution, which requires it to be from Box::into_raw.
            unsafe {
                drop(Box::from_raw(res));
            }
        }
        for slice in self.u64_slices.drain(..) {
            // SAFETY: The pointer was added via add_u64_slice, which requires it to be from Box::into_raw.
            unsafe {
                drop(Box::from_raw(slice));
            }
        }
    }
}

// SAFETY: MetadataOwner only contains raw pointers to metadata that is not being
// mutated and is owned by this type. Reclaiming it on drop is safe if no references
// exist.
unsafe impl Send for MetadataOwner {}
unsafe impl Sync for MetadataOwner {}

pub(crate) const SUPPORT_LIBRARY: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/support.dll"));
pub const SUPPORT_ASSEMBLY: &str = "__dotnetrs_support";

pub struct AssemblyLoader {
    pub(crate) assembly_root: String,
    pub(crate) external: RwLock<HashMap<String, Option<ResolutionS>>>,
    /// Mapping of canonical BCL names (e.g., "System.Delegate") to their implementation
    /// in the support library (e.g., "DotnetRs.Delegate").
    pub(crate) stubs: HashMap<String, TypeDescription>,
    /// Reverse mapping from support library type names to their canonical BCL names.
    /// Used for name normalization during intrinsic dispatch.
    /// E.g., "DotnetRs.Delegate" → "System.Delegate"
    pub(crate) reverse_stubs: HashMap<String, String>,
    pub(crate) corlib_cache: DashMap<String, TypeDescription>,
    pub(crate) type_cache: DashMap<(ResolutionS, UserType), TypeDescription>,
    pub(crate) method_cache:
        DashMap<(ResolutionS, UserMethod, GenericLookup, Option<ConcreteType>), MethodDescription>,
    pub type_cache_hits: AtomicU64,
    pub type_cache_misses: AtomicU64,
    pub method_cache_hits: AtomicU64,
    pub method_cache_misses: AtomicU64,
    pub(crate) metadata: RwLock<MetadataOwner>,
}
static_collect!(AssemblyLoader);

impl AssemblyLoader {
    pub fn new(assembly_root: String) -> Result<Self, AssemblyLoadError> {
        let resolutions: HashMap<_, _> = fs::read_dir(&assembly_root)
            .map_err(|e| {
                AssemblyLoadError::Io(format!(
                    "could not read assembly root {}: {}",
                    assembly_root, e
                ))
            })?
            .filter_map(|e| {
                let path = e.ok()?.path();
                if path.extension()? == "dll" {
                    Some((path.file_stem()?.to_string_lossy().into_owned(), None))
                } else {
                    None
                }
            })
            .collect();

        Self::new_internal(assembly_root, resolutions)
    }

    /// Creates a new `AssemblyLoader` without scanning the assembly root for DLLs.
    /// This is useful for testing or when all assemblies are registered manually.
    pub fn new_bare(assembly_root: String) -> Result<Self, AssemblyLoadError> {
        Self::new_internal(assembly_root, HashMap::new())
    }

    fn new_internal(
        assembly_root: String,
        resolutions: HashMap<String, Option<ResolutionS>>,
    ) -> Result<Self, AssemblyLoadError> {
        let mut this = Self {
            assembly_root,
            external: RwLock::new(resolutions),
            stubs: HashMap::new(),
            reverse_stubs: HashMap::new(),
            corlib_cache: DashMap::new(),
            type_cache: DashMap::new(),
            method_cache: DashMap::new(),
            type_cache_hits: AtomicU64::new(0),
            type_cache_misses: AtomicU64::new(0),
            method_cache_hits: AtomicU64::new(0),
            method_cache_misses: AtomicU64::new(0),
            metadata: RwLock::new(MetadataOwner::new()),
        };

        this.add_support_library()?;
        Ok(this)
    }

    pub fn get_root(&self) -> &str {
        &self.assembly_root
    }

    /// Returns the canonical (System.*) name for a stubbed type,
    /// or the input unchanged for non-stub types.
    pub fn canonical_type_name<'a>(&'a self, name: &'a str) -> &'a str {
        self.reverse_stubs
            .get(name)
            .map(|s| s.as_str())
            .unwrap_or(name)
    }

    pub fn load_resolution_from_file(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<ResolutionS, AssemblyLoadError> {
        let (res, raw_ptr, slice_ptr) = crate::resolution::load_resolution_core(path)?;
        {
            let mut metadata = self.metadata.write();
            // SAFETY: raw_ptr and slice_ptr were obtained from Box::into_raw in load_resolution_core.
            unsafe {
                metadata.add_resolution(raw_ptr);
                metadata.add_u64_slice(slice_ptr);
            }
        }
        Ok(res)
    }

    /// Returns true if the given name is an alias for a stubbed type.
    pub fn is_stub_alias(&self, name: &str) -> bool {
        self.reverse_stubs.contains_key(name)
    }

    pub fn get_assembly(&self, name: &str) -> Result<ResolutionS, AssemblyLoadError> {
        let res = { self.external.read().get(name).copied() };
        match res {
            None => {
                let mut file = PathBuf::from(&self.assembly_root);
                file.push(format!("{name}.dll"));
                if !file.exists() {
                    return Err(AssemblyLoadError::FileNotFound(format!(
                        "could not find assembly {name} in root {}",
                        self.assembly_root
                    )));
                }
                let resolution = self.load_resolution_from_file(file)?;
                match &resolution.assembly {
                    None => {
                        return Err(AssemblyLoadError::InvalidFormat(
                            "no assembly present in external module".to_string(),
                        ));
                    }
                    Some(a) => {
                        let mut external = self.external.write();
                        external.insert(a.name.to_string(), Some(resolution));
                        // Also cache under the name it was requested as, to prevent redundant loads
                        // when the filename doesn't match the assembly name (e.g. mscorlib -> System.Private.CoreLib)
                        if a.name.as_ref() != name {
                            external.insert(name.to_string(), Some(resolution));
                        }
                    }
                }
                Ok(resolution)
            }
            Some(None) => {
                let mut file = PathBuf::from(&self.assembly_root);
                file.push(format!("{name}.dll"));
                let resolution = self.load_resolution_from_file(file)?;
                match &resolution.assembly {
                    None => {
                        return Err(AssemblyLoadError::InvalidFormat(
                            "no assembly present in external module".to_string(),
                        ));
                    }
                    Some(a) => {
                        let mut external = self.external.write();
                        external.insert(a.name.to_string(), Some(resolution));
                        if a.name.as_ref() != name {
                            external.insert(name.to_string(), Some(resolution));
                        }
                    }
                }
                Ok(resolution)
            }
            Some(Some(res)) => Ok(res),
        }
    }

    pub fn assemblies(&self) -> Vec<ResolutionS> {
        self.external.read().values().flatten().copied().collect()
    }

    pub fn type_cache_size(&self) -> usize {
        self.type_cache.len()
    }

    pub fn method_cache_size(&self) -> usize {
        self.method_cache.len()
    }

    pub fn register_assembly(&self, resolution: ResolutionS) {
        if let Some(a) = &resolution.assembly {
            self.external
                .write()
                .insert(a.name.to_string(), Some(resolution));
        }
    }

    /// Takes ownership of a manually constructed `Resolution` and registers it.
    /// The memory will be reclaimed when this `AssemblyLoader` is dropped.
    pub fn register_owned_assembly(&self, res: Resolution<'static>) -> ResolutionS {
        let res_box = Box::new(res);
        let res_ptr = Box::into_raw(res_box);
        let res_s = ResolutionS::new(res_ptr);

        {
            let mut metadata = self.metadata.write();
            unsafe {
                metadata.add_resolution(res_ptr);
            }
        }

        self.register_assembly(res_s);
        res_s
    }

    pub fn comparer(&self) -> TypeComparer<'_, Self> {
        TypeComparer::new(self)
    }
}
