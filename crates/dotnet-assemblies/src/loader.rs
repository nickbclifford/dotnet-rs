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

#[derive(Debug, Clone, Copy, Default)]
pub struct BindingRedirect {
    pub from_version_start: Version,
    pub from_version_end: Version,
    pub to_version: Version,
}

impl BindingRedirect {
    pub fn matches(&self, version: &Version) -> bool {
        version_ge(version, &self.from_version_start) && version_ge(&self.from_version_end, version)
    }
}

pub fn versions_equal(v1: &Version, v2: &Version) -> bool {
    v1.major == v2.major && v1.minor == v2.minor && v1.build == v2.build && v1.revision == v2.revision
}

pub fn version_ge(v1: &Version, v2: &Version) -> bool {
    if v1.major > v2.major {
        return true;
    }
    if v1.major < v2.major {
        return false;
    }
    if v1.minor > v2.minor {
        return true;
    }
    if v1.minor < v2.minor {
        return false;
    }
    if v1.build > v2.build {
        return true;
    }
    if v1.build < v2.build {
        return false;
    }
    v1.revision >= v2.revision
}

pub fn parse_version(s: &str) -> Option<Version> {
    let parts: Vec<_> = s.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    Some(Version {
        major: parts[0].parse().ok()?,
        minor: parts[1].parse().ok()?,
        build: parts[2].parse().ok()?,
        revision: parts[3].parse().ok()?,
    })
}

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
    pub(crate) redirects: DashMap<String, Vec<BindingRedirect>>,
    pub(crate) strict_versioning: bool,
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
            redirects: DashMap::new(),
            strict_versioning: std::env::var("DOTNET_STRICT_VERSIONING").is_ok(),
        };

        this.add_support_library()?;
        this.load_redirects()?;
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

    pub fn add_redirect(&self, name: String, redirect: BindingRedirect) {
        self.redirects.entry(name).or_default().push(redirect);
    }

    pub fn set_strict_versioning(&mut self, strict: bool) {
        self.strict_versioning = strict;
    }

    pub fn load_redirects(&self) -> Result<(), AssemblyLoadError> {
        let mut file = PathBuf::from(&self.assembly_root);
        file.push("redirects.txt");
        if !file.exists() {
            return Ok(());
        }

        let content = fs::read_to_string(file).map_err(|e| {
            AssemblyLoadError::Io(format!("could not read redirects.txt: {}", e))
        })?;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let parts: Vec<_> = line.split_whitespace().collect();
            if parts.len() != 3 {
                continue; // Skip invalid lines
            }

            let name = parts[0];
            let range: Vec<_> = parts[1].split('-').collect();
            if range.len() != 2 {
                continue;
            }

            if let (Some(start), Some(end), Some(to)) = (
                parse_version(range[0]),
                parse_version(range[1]),
                parse_version(parts[2]),
            ) {
                let redirect = BindingRedirect {
                    from_version_start: start,
                    from_version_end: end,
                    to_version: to,
                };
                self.redirects
                    .entry(name.to_string())
                    .or_default()
                    .push(redirect);
            }
        }

        Ok(())
    }

    pub fn get_assembly(&self, name: &str) -> Result<ResolutionS, AssemblyLoadError> {
        self.get_assembly_with_version(name, None)
    }

    pub fn get_assembly_with_version(
        &self,
        name: &str,
        requested_version: Option<Version>,
    ) -> Result<ResolutionS, AssemblyLoadError> {
        let mut requested_version = requested_version;

        // Apply redirects
        if let Some((version, redirects)) = requested_version.zip(self.redirects.get(name)) {
            for redirect in redirects.value() {
                if redirect.matches(&version) {
                    requested_version = Some(redirect.to_version);
                    break;
                }
            }
        }

        let res = { self.external.read().get(name).copied() };
        let resolution = match res {
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
                        if a.name.as_ref() != name {
                            external.insert(name.to_string(), Some(resolution));
                        }
                    }
                }
                resolution
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
                resolution
            }
            Some(Some(res)) => res,
        };

        // Check version compatibility
        if let Some((requested, a)) = requested_version.zip(resolution.assembly.as_ref()) {
            let actual = a.version;
            // ECMA-335 binding: Major versions MUST match exactly.
                // Minor versions intended to be backward compatible.
                // Conforming implementations can be stricter.
                // For now: require same Major/Minor, and Actual >= Requested.
                // This is a reasonable "strong" binding for a VM implementation.
                let mut error = None;
                if actual.major != requested.major || actual.minor != requested.minor {
                    error = Some(format!(
                        "assembly {} has version {}.{}.{}.{}, but {}.{}.{}.{} was requested (and no redirect matched)",
                        name,
                        actual.major, actual.minor, actual.build, actual.revision,
                        requested.major, requested.minor, requested.build, requested.revision
                    ));
                } else if !version_ge(&actual, &requested) {
                    error = Some(format!(
                        "assembly {} has version {}.{}.{}.{}, which is older than the requested {}.{}.{}.{}",
                        name,
                        actual.major, actual.minor, actual.build, actual.revision,
                        requested.major, requested.minor, requested.build, requested.revision
                    ));
                }

                if let Some(msg) = error {
                    if self.strict_versioning {
                        return Err(AssemblyLoadError::InvalidFormat(msg));
                    } else {
                        tracing::warn!("Binding mismatch: {}", msg);
                    }
                }
        }

        Ok(resolution)
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
