use crate::sandbox::{DefaultSandbox, PInvokeSandbox};
use dashmap::DashMap;
use dotnet_tracer::Tracer;
use dotnet_types::error::PInvokeError;
use gc_arena::static_collect;
use libffi::middle::CodePtr;
use libloading::{Library, Symbol};
use std::{path::PathBuf, sync::Arc};

pub struct NativeLibraries {
    root: PathBuf,
    search_dirs: Vec<PathBuf>,
    libraries: DashMap<String, Library>,
    sandbox: Arc<dyn PInvokeSandbox>,
}
static_collect!(NativeLibraries);
impl NativeLibraries {
    pub fn new(root: impl AsRef<str>) -> Self {
        Self {
            root: PathBuf::from(root.as_ref()),
            search_dirs: Vec::new(),
            libraries: DashMap::new(),
            sandbox: Arc::new(DefaultSandbox),
        }
    }

    pub fn with_search_dirs(mut self, dirs: impl IntoIterator<Item = PathBuf>) -> Self {
        for dir in dirs {
            if dir == self.root || self.search_dirs.contains(&dir) {
                continue;
            }
            self.search_dirs.push(dir);
        }

        self
    }

    pub fn with_sandbox(mut self, sandbox: Arc<dyn PInvokeSandbox>) -> Self {
        self.sandbox = sandbox;
        self
    }

    fn search_roots(&self) -> impl Iterator<Item = &PathBuf> {
        std::iter::once(&self.root).chain(self.search_dirs.iter())
    }

    fn find_library_path(&self, name: &str) -> Option<PathBuf> {
        // Try with platform extension
        #[cfg(target_os = "linux")]
        let extensions = &[".so", ".dylib", ".dll"];
        #[cfg(target_os = "macos")]
        let extensions = &[".dylib", ".so", ".dll"];
        #[cfg(target_os = "windows")]
        let extensions = &[".dll", ".so", ".dylib"];
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        let extensions = &[".so", ".dll", ".dylib"];

        for root in self.search_roots() {
            let exact = root.join(name);
            if exact.exists() {
                return Some(exact);
            }

            for ext in extensions {
                let path = root.join(format!("{}{}", name, ext));
                if path.exists() {
                    return Some(path);
                }
            }

            // Versioned search
            if let Ok(entries) = root.read_dir() {
                for entry in entries.filter_map(Result::ok) {
                    let path = entry.path();
                    let file_name = entry.file_name();
                    let s = file_name.to_string_lossy();

                    if s.starts_with(name) && (s.contains(".so.") || s.contains(".dylib.")) {
                        return Some(path);
                    }
                }
            }
        }

        None
    }

    pub fn get_library(
        &self,
        name: &str,
        tracer: Option<&Tracer>,
    ) -> Result<dashmap::mapref::one::Ref<'_, String, Library>, PInvokeError> {
        if !self.sandbox.allow_library(name) {
            return Err(PInvokeError::LibraryNotFound(name.into()));
        }

        if let Some(lib) = self.libraries.get(name) {
            return Ok(lib);
        }

        let path = self.find_library_path(name);

        if let Some(t) = tracer {
            t.trace_interop(
                0,
                "RESOLVE",
                &if let Some(p) = &path {
                    format!("Library '{}' found at '{}', now loading", name, p.display())
                } else {
                    format!(
                        "Library '{}' not found in configured search roots, trying system paths",
                        name
                    )
                },
            );
        }

        let mut names_to_try = vec![];
        if let Some(p) = path {
            names_to_try.push(p.to_string_lossy().to_string());
        } else {
            names_to_try.push(name.to_string());
            #[cfg(target_os = "linux")]
            {
                if name == "libc" {
                    names_to_try.push("libc.so.6".to_string());
                } else if name == "libm" {
                    names_to_try.push("libm.so.6".to_string());
                } else if name == "libdl" {
                    names_to_try.push("libdl.so.2".to_string());
                } else if name == "libpthread" {
                    names_to_try.push("libpthread.so.0".to_string());
                }
            }
        }

        let mut lib = None;
        let mut last_error = None;

        for n in &names_to_try {
            // SAFETY: Loading a dynamic library is inherently unsafe because constructors may run
            // and symbol layouts are unchecked. The sandbox gate and curated name list constrain
            // inputs to approved libraries, and failures are surfaced as `PInvokeError::LoadError`.
            match unsafe { Library::new(n) } {
                Ok(l) => {
                    lib = Some(l);
                    break;
                }
                Err(e) => {
                    last_error = Some(e);
                }
            }
        }

        let lib = lib.ok_or_else(|| {
            PInvokeError::LoadError(
                name.into(),
                last_error
                    .map(|e| e.to_string().into_boxed_str())
                    .unwrap_or_else(|| "Unknown error".into()),
            )
        })?;

        if let Some(t) = tracer {
            t.trace_interop(0, "RESOLVE", &format!("Successfully loaded '{}'", name));
        }
        self.libraries.entry(name.to_string()).or_insert(lib);
        Ok(self.libraries.get(name).unwrap())
    }

    pub fn get_function(
        &self,
        library: &str,
        name: &str,
        tracer: Option<&Tracer>,
    ) -> Result<CodePtr, PInvokeError> {
        if !self.sandbox.allow_function(library, name) {
            return Err(PInvokeError::SymbolNotFound(library.into(), name.into()));
        }
        let l = self.get_library(library, tracer)?;
        // SAFETY: We request the raw symbol as an untyped C function pointer and immediately pass
        // it to libffi. Arity/signature validation is handled by metadata-driven marshalling before
        // invocation; lookup failure is converted to `PInvokeError::SymbolNotFound`.
        let sym: Symbol<unsafe extern "C" fn()> = unsafe { l.get(name.as_bytes()) }
            .map_err(|_| PInvokeError::SymbolNotFound(library.into(), name.into()))?;
        Ok(CodePtr::from_fun(*sym))
    }
}

#[cfg(test)]
mod tests {
    use super::NativeLibraries;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock drift")
            .as_nanos();
        std::env::temp_dir().join(format!("dotnet_rs_pinvoke_{label}_{nanos}"))
    }

    #[test]
    fn find_library_path_uses_additional_search_dirs() {
        let root = unique_temp_dir("root");
        let extra = unique_temp_dir("extra");
        fs::create_dir_all(&root).expect("create root");
        fs::create_dir_all(&extra).expect("create extra");

        let target = extra.join("libSystem.Native.so");
        fs::write(&target, b"not an actual shared library").expect("write target");

        let libraries =
            NativeLibraries::new(root.to_string_lossy().as_ref()).with_search_dirs([extra.clone()]);

        assert_eq!(
            libraries.find_library_path("libSystem.Native"),
            Some(target.clone())
        );

        fs::remove_dir_all(&root).expect("cleanup root");
        fs::remove_dir_all(&extra).expect("cleanup extra");
    }

    #[test]
    fn find_library_path_prefers_primary_root_over_additional_dirs() {
        let root = unique_temp_dir("primary");
        let extra = unique_temp_dir("secondary");
        fs::create_dir_all(&root).expect("create root");
        fs::create_dir_all(&extra).expect("create extra");

        let primary = root.join("libSystem.Native.so");
        let secondary = extra.join("libSystem.Native.so");
        fs::write(&primary, b"primary").expect("write primary");
        fs::write(&secondary, b"secondary").expect("write secondary");

        let libraries =
            NativeLibraries::new(root.to_string_lossy().as_ref()).with_search_dirs([extra.clone()]);

        assert_eq!(
            libraries.find_library_path("libSystem.Native"),
            Some(primary.clone())
        );

        fs::remove_dir_all(&root).expect("cleanup root");
        fs::remove_dir_all(&extra).expect("cleanup extra");
    }
}
