#[cfg(test)]
mod tests {
    #[cfg(not(miri))]
    use crate::loader::AssemblyLoader;
    use crate::loader::{parse_version, versions_equal};
    use dotnetdll::prelude::*;
    #[cfg(not(miri))]
    use std::env;
    #[cfg(not(miri))]
    use std::fs;
    #[cfg(not(miri))]
    use std::sync::atomic::{AtomicU64, Ordering};

    #[cfg(not(miri))]
    static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

    #[cfg(not(miri))]
    struct TestRoot(std::path::PathBuf);

    #[cfg(not(miri))]
    impl TestRoot {
        fn new(label: &str) -> Self {
            loop {
                let id = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
                let path =
                    env::temp_dir().join(format!("dotnet-rs-{label}-{}-{id}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => {
                        panic!("create isolated test directory {}: {error}", path.display())
                    }
                }
            }
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    #[cfg(not(miri))]
    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn test_version_parsing() {
        let v = parse_version("1.2.3.4").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.build, 3);
        assert_eq!(v.revision, 4);
        assert!(parse_version("1.2.3").is_none());
        assert!(parse_version("1.2.3.4.5").is_none());
        assert!(parse_version("a.b.c.d").is_none());
    }
    #[test]
    fn test_version_equality() {
        let v1 = Version {
            major: 1,
            minor: 2,
            build: 3,
            revision: 4,
        };
        let v2 = Version {
            major: 1,
            minor: 2,
            build: 3,
            revision: 4,
        };
        let v3 = Version {
            major: 1,
            minor: 2,
            build: 3,
            revision: 5,
        };
        assert!(versions_equal(&v1, &v2));
        assert!(!versions_equal(&v1, &v3));
    }
    #[test]
    #[cfg(not(miri))]
    fn test_version_binding_basic() {
        let root = TestRoot::new("version-binding");
        let mut loader =
            AssemblyLoader::new_bare(root.path().to_str().unwrap().to_string()).unwrap();
        loader.set_strict_versioning(true);
        let mut res = Resolution::new(Module::new("TestAssembly.dll"));
        let mut assembly = Assembly::new("TestAssembly");
        assembly.version = Version {
            major: 1,
            minor: 0,
            build: 0,
            revision: 0,
        };
        res.assembly = Some(assembly);
        let res_s = loader.register_owned_assembly(res);
        let resolved = loader
            .get_assembly_with_version(
                "TestAssembly",
                Some(Version {
                    major: 1,
                    minor: 0,
                    build: 0,
                    revision: 0,
                }),
            )
            .unwrap();
        assert_eq!(resolved, res_s);
        let err = loader.get_assembly_with_version(
            "TestAssembly",
            Some(Version {
                major: 1,
                minor: 0,
                build: 0,
                revision: 1,
            }),
        );
        assert!(err.is_err());
        let err = loader.get_assembly_with_version(
            "TestAssembly",
            Some(Version {
                major: 2,
                minor: 0,
                build: 0,
                revision: 0,
            }),
        );
        assert!(err.is_err());
    }
    #[test]
    #[cfg(not(miri))]
    fn test_binding_redirect() {
        let root = TestRoot::new("version-redirect");
        let redirects_path = root.path().join("redirects.txt");
        fs::write(&redirects_path, "TestAssembly 2.0.0.0-2.0.0.0 1.0.0.0").unwrap();
        let mut loader = AssemblyLoader::new(root.path().to_str().unwrap().to_string()).unwrap();
        loader.set_strict_versioning(true);
        let mut res = Resolution::new(Module::new("TestAssembly.dll"));
        let mut assembly = Assembly::new("TestAssembly");
        assembly.version = Version {
            major: 1,
            minor: 0,
            build: 0,
            revision: 0,
        };
        res.assembly = Some(assembly);
        let res_s = loader.register_owned_assembly(res);
        let resolved = loader
            .get_assembly_with_version(
                "TestAssembly",
                Some(Version {
                    major: 2,
                    minor: 0,
                    build: 0,
                    revision: 0,
                }),
            )
            .unwrap();
        assert_eq!(resolved, res_s);
    }
}
