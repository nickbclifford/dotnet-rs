use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct RuntimeConfig {
    #[serde(rename = "runtimeOptions")]
    pub runtime_options: RuntimeOptions,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct RuntimeOptions {
    pub tfm: Option<String>,
    pub framework: Option<FrameworkRef>,
    #[serde(rename = "rollForward")]
    pub roll_forward: Option<RollForwardPolicy>,
    #[serde(rename = "configProperties", default)]
    pub config_properties: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct FrameworkRef {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "PascalCase")]
pub enum RollForwardPolicy {
    Disable,
    LatestPatch,
    #[default]
    Minor,
    LatestMinor,
    Major,
    LatestMajor,
}

#[derive(Debug, Error)]
pub enum HostError {
    #[error("failed to read runtimeconfig '{path}': {source}")]
    ReadRuntimeConfig {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse runtimeconfig '{path}': {source}")]
    ParseRuntimeConfig {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

pub fn parse_runtimeconfig(path: &Path) -> Result<RuntimeConfig, HostError> {
    let bytes = fs::read(path).map_err(|source| HostError::ReadRuntimeConfig {
        path: path.to_path_buf(),
        source,
    })?;

    serde_json::from_slice::<RuntimeConfig>(&bytes).map_err(|source| {
        HostError::ParseRuntimeConfig {
            path: path.to_path_buf(),
            source,
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct FrameworkVersion {
    major: u32,
    minor: u32,
    patch: u32,
}

#[derive(Debug, Clone)]
struct InstalledFrameworkVersion {
    version: FrameworkVersion,
    path: PathBuf,
}

fn parse_framework_version(version: &str) -> Option<FrameworkVersion> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next()?.parse::<u32>().ok()?;
    let patch = parts.next()?.parse::<u32>().ok()?;

    if parts.next().is_some() {
        return None;
    }

    Some(FrameworkVersion {
        major,
        minor,
        patch,
    })
}

fn discover_installed_framework_versions(base_dir: &Path) -> Vec<InstalledFrameworkVersion> {
    let Ok(entries) = fs::read_dir(base_dir) else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|entry| {
            if !entry.path().is_dir() {
                return None;
            }

            let name = entry.file_name();
            let version_name = name.to_str()?;
            let version = parse_framework_version(version_name)?;

            Some(InstalledFrameworkVersion {
                version,
                path: entry.path(),
            })
        })
        .collect()
}

fn highest_patch_for_major_minor<'a>(
    versions: impl Iterator<Item = &'a InstalledFrameworkVersion>,
) -> Option<&'a InstalledFrameworkVersion> {
    versions.max_by_key(|candidate| candidate.version.patch)
}

fn select_minor_policy_for_major(
    versions: &[InstalledFrameworkVersion],
    major: u32,
    minimum_minor: u32,
) -> Option<&InstalledFrameworkVersion> {
    let target_minor = versions
        .iter()
        .filter(|candidate| {
            candidate.version.major == major && candidate.version.minor >= minimum_minor
        })
        .map(|candidate| candidate.version.minor)
        .min()?;

    highest_patch_for_major_minor(versions.iter().filter(|candidate| {
        candidate.version.major == major && candidate.version.minor == target_minor
    }))
}

fn select_latest_minor_policy_for_major(
    versions: &[InstalledFrameworkVersion],
    major: u32,
    minimum_minor: u32,
) -> Option<&InstalledFrameworkVersion> {
    let target_minor = versions
        .iter()
        .filter(|candidate| {
            candidate.version.major == major && candidate.version.minor >= minimum_minor
        })
        .map(|candidate| candidate.version.minor)
        .max()?;

    highest_patch_for_major_minor(versions.iter().filter(|candidate| {
        candidate.version.major == major && candidate.version.minor == target_minor
    }))
}

pub fn select_framework_version(
    base_dir: &Path,
    requested: &FrameworkRef,
    policy: RollForwardPolicy,
) -> Option<PathBuf> {
    let requested_version = parse_framework_version(&requested.version)?;
    let installed = discover_installed_framework_versions(base_dir);

    let selected = match policy {
        RollForwardPolicy::Disable => installed
            .iter()
            .find(|candidate| candidate.version == requested_version),
        RollForwardPolicy::LatestPatch => {
            highest_patch_for_major_minor(installed.iter().filter(|candidate| {
                candidate.version.major == requested_version.major
                    && candidate.version.minor == requested_version.minor
            }))
        }
        RollForwardPolicy::Minor => select_minor_policy_for_major(
            &installed,
            requested_version.major,
            requested_version.minor,
        ),
        RollForwardPolicy::LatestMinor => select_latest_minor_policy_for_major(
            &installed,
            requested_version.major,
            requested_version.minor,
        ),
        RollForwardPolicy::Major => {
            let target_major = installed
                .iter()
                .filter(|candidate| candidate.version.major >= requested_version.major)
                .map(|candidate| candidate.version.major)
                .min()?;

            let minimum_minor = if target_major == requested_version.major {
                requested_version.minor
            } else {
                0
            };

            select_minor_policy_for_major(&installed, target_major, minimum_minor)
        }
        RollForwardPolicy::LatestMajor => {
            let target_major = installed
                .iter()
                .filter(|candidate| candidate.version.major >= requested_version.major)
                .map(|candidate| candidate.version.major)
                .max()?;

            let minimum_minor = if target_major == requested_version.major {
                requested_version.minor
            } else {
                0
            };

            select_latest_minor_policy_for_major(&installed, target_major, minimum_minor)
        }
    };

    selected.map(|candidate| candidate.path.clone())
}

fn framework_base_candidates(framework_name: &str) -> Vec<PathBuf> {
    let mut base_paths = Vec::new();

    if let Some(dotnet_root) = std::env::var_os("DOTNET_ROOT") {
        base_paths.push(
            PathBuf::from(dotnet_root)
                .join("shared")
                .join(framework_name),
        );
    }

    if cfg!(target_os = "windows") {
        base_paths.push(PathBuf::from("C:\\Program Files\\dotnet\\shared").join(framework_name));
    } else if cfg!(target_os = "macos") {
        base_paths.push(PathBuf::from("/usr/local/share/dotnet/shared").join(framework_name));
    } else {
        base_paths.push(PathBuf::from("/usr/share/dotnet/shared").join(framework_name));
        base_paths.push(PathBuf::from("/usr/lib/dotnet/shared").join(framework_name));
    }

    base_paths
}

pub fn resolve_framework_from_runtimeconfig(
    config: &RuntimeConfig,
    override_base: Option<&Path>,
) -> Option<PathBuf> {
    let framework = config.runtime_options.framework.as_ref()?;
    let policy = config.runtime_options.roll_forward.unwrap_or_default();

    if let Some(base_dir) = override_base {
        return select_framework_version(base_dir, framework, policy);
    }

    framework_base_candidates(&framework.name)
        .into_iter()
        .find_map(|base_dir| select_framework_version(&base_dir, framework, policy))
}

#[cfg(test)]
mod tests {
    use super::{
        FrameworkRef, RollForwardPolicy, parse_runtimeconfig, resolve_framework_from_runtimeconfig,
        select_framework_version,
    };
    use serde_json::Value;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    #[cfg(not(miri))]
    fn parses_fixture_runtimeconfig() {
        let path = Path::new("/tmp/fixture-probe/SingleFile.runtimeconfig.json");
        assert!(
            path.exists(),
            "missing fixture runtimeconfig at {}; build fixtures first",
            path.display()
        );

        let config = parse_runtimeconfig(path).expect("runtimeconfig should parse");
        let options = config.runtime_options;

        assert_eq!(options.tfm.as_deref(), Some("net10.0"));

        let framework = options.framework.expect("framework should be present");
        assert_eq!(framework.name, "Microsoft.NETCore.App");
        assert_eq!(framework.version, "10.0.0");

        assert_eq!(options.roll_forward, None);
        assert_eq!(
            options
                .config_properties
                .get("System.Runtime.Serialization.EnableUnsafeBinaryFormatterSerialization"),
            Some(&Value::Bool(false))
        );
    }

    fn create_runtime_base(version_dirs: &[&str]) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let base_dir = std::env::temp_dir().join(format!(
            "dotnet-rs-host-roll-forward-{}-{}",
            std::process::id(),
            nonce
        ));

        fs::create_dir_all(&base_dir).expect("test runtime base should be created");

        for version in version_dirs {
            fs::create_dir_all(base_dir.join(version))
                .expect("runtime version subdirectory should be created");
        }

        base_dir
    }

    fn framework_request(version: &str) -> FrameworkRef {
        FrameworkRef {
            name: "Microsoft.NETCore.App".to_string(),
            version: version.to_string(),
        }
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    #[cfg(not(miri))]
    fn resolves_framework_from_fixture_runtimeconfig_with_override_base() {
        let path = Path::new("/tmp/fixture-probe/SingleFile.runtimeconfig.json");
        assert!(
            path.exists(),
            "missing fixture runtimeconfig at {}; build fixtures first",
            path.display()
        );

        let config = parse_runtimeconfig(path).expect("runtimeconfig should parse");
        let base_dir = create_runtime_base(&["8.0.28", "10.0.9"]);

        assert_eq!(
            resolve_framework_from_runtimeconfig(&config, Some(&base_dir)),
            Some(base_dir.join("10.0.9"))
        );

        fs::remove_dir_all(&base_dir).expect("test runtime base should be removed");
    }

    #[test]
    #[cfg(not(miri))]
    fn resolves_framework_from_fixture_runtimeconfig_using_dotnet_root_override() {
        let _guard = env_lock()
            .lock()
            .expect("environment lock should not be poisoned");

        let path = Path::new("/tmp/fixture-probe/SingleFile.runtimeconfig.json");
        assert!(
            path.exists(),
            "missing fixture runtimeconfig at {}; build fixtures first",
            path.display()
        );

        let config = parse_runtimeconfig(path).expect("runtimeconfig should parse");
        let dotnet_root = create_runtime_base(&[]);
        let framework_base = dotnet_root.join("shared").join("Microsoft.NETCore.App");
        fs::create_dir_all(framework_base.join("8.0.28"))
            .expect("framework version subdirectory should be created");
        fs::create_dir_all(framework_base.join("10.0.9"))
            .expect("framework version subdirectory should be created");

        let previous_dotnet_root = std::env::var_os("DOTNET_ROOT");
        // SAFETY: Access is serialized by env_lock(), preventing concurrent mutation during this test.
        unsafe {
            std::env::set_var("DOTNET_ROOT", &dotnet_root);
        }

        let resolved = resolve_framework_from_runtimeconfig(&config, None);

        if let Some(previous) = previous_dotnet_root {
            // SAFETY: Access is serialized by env_lock(), preventing concurrent mutation during this test.
            unsafe {
                std::env::set_var("DOTNET_ROOT", previous);
            }
        } else {
            // SAFETY: Access is serialized by env_lock(), preventing concurrent mutation during this test.
            unsafe {
                std::env::remove_var("DOTNET_ROOT");
            }
        }

        assert_eq!(resolved, Some(framework_base.join("10.0.9")));

        fs::remove_dir_all(&dotnet_root).expect("test runtime base should be removed");
    }

    #[test]
    fn select_framework_version_applies_all_roll_forward_policies_for_10_0_0_request() {
        let base_dir = create_runtime_base(&["8.0.28", "10.0.9"]);
        let requested = framework_request("10.0.0");

        assert_eq!(
            select_framework_version(&base_dir, &requested, RollForwardPolicy::Disable),
            None
        );

        for policy in [
            RollForwardPolicy::LatestPatch,
            RollForwardPolicy::Minor,
            RollForwardPolicy::LatestMinor,
            RollForwardPolicy::Major,
            RollForwardPolicy::LatestMajor,
        ] {
            assert_eq!(
                select_framework_version(&base_dir, &requested, policy),
                Some(base_dir.join("10.0.9")),
                "unexpected selected version for {policy:?}"
            );
        }

        fs::remove_dir_all(&base_dir).expect("test runtime base should be removed");
    }

    #[test]
    fn disable_policy_requires_exact_match() {
        let base_dir = create_runtime_base(&["8.0.28", "10.0.0", "10.0.9"]);
        let requested = framework_request("10.0.0");

        assert_eq!(
            select_framework_version(&base_dir, &requested, RollForwardPolicy::Disable),
            Some(base_dir.join("10.0.0"))
        );

        fs::remove_dir_all(&base_dir).expect("test runtime base should be removed");
    }
}
