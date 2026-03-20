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
        let root = env::temp_dir().join("dotnet_rs_version_test_basic");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let mut loader = AssemblyLoader::new_bare(root.to_str().unwrap().to_string()).unwrap();
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
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    #[cfg(not(miri))]
    fn test_binding_redirect() {
        let root = env::temp_dir().join("dotnet_rs_version_test_redirects");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let redirects_path = root.join("redirects.txt");
        fs::write(&redirects_path, "TestAssembly 2.0.0.0-2.0.0.0 1.0.0.0").unwrap();
        let mut loader = AssemblyLoader::new(root.to_str().unwrap().to_string()).unwrap();
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
        fs::remove_dir_all(&root).unwrap();
    }
}
