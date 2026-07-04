use crate::error::AssemblyLoadError;
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

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DepsJson {
    #[serde(rename = "runtimeTarget")]
    pub runtime_target: DepsRuntimeTarget,
    #[serde(default)]
    pub targets: BTreeMap<String, BTreeMap<String, TargetLibrary>>,
    #[serde(default)]
    pub libraries: BTreeMap<String, LibraryInfo>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DepsRuntimeTarget {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
pub struct TargetLibrary {
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
    #[serde(default)]
    pub runtime: BTreeMap<String, AssemblyAssetInfo>,
    #[serde(default)]
    pub native: BTreeMap<String, AssemblyAssetInfo>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
pub struct AssemblyAssetInfo {
    #[serde(rename = "assemblyVersion")]
    pub assembly_version: Option<String>,
    #[serde(rename = "fileVersion")]
    pub file_version: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct LibraryInfo {
    #[serde(rename = "type")]
    pub library_type: String,
    pub serviceable: Option<bool>,
    pub sha512: Option<String>,
    pub path: Option<String>,
    #[serde(rename = "hashPath")]
    pub hash_path: Option<String>,
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
    #[error("failed to resolve framework from runtimeconfig '{runtimeconfig_path}'")]
    ResolveFramework { runtimeconfig_path: PathBuf },
    #[error("failed to create assembly loader from framework root '{framework_dir}': {source}")]
    CreateAssemblyLoader {
        framework_dir: PathBuf,
        #[source]
        source: AssemblyLoadError,
    },
    #[error("failed to scan app directory '{app_dir}' for managed assemblies: {source}")]
    AddScanRoot {
        app_dir: PathBuf,
        #[source]
        source: AssemblyLoadError,
    },
    #[error("failed to read deps.json '{path}': {source}")]
    ReadDepsJson {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse deps.json '{path}': {source}")]
    ParseDepsJson {
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

pub fn parse_deps_json(path: &Path) -> Result<DepsJson, HostError> {
    let bytes = fs::read(path).map_err(|source| HostError::ReadDepsJson {
        path: path.to_path_buf(),
        source,
    })?;

    serde_json::from_slice::<DepsJson>(&bytes).map_err(|source| HostError::ParseDepsJson {
        path: path.to_path_buf(),
        source,
    })
}

pub fn derive_managed_probing_paths(
    deps: &DepsJson,
    nuget_global: &Path,
) -> Vec<(String, PathBuf)> {
    let Some(targets) = deps.targets.get(&deps.runtime_target.name) else {
        return Vec::new();
    };

    let mut managed_paths = Vec::new();

    for (library_key, target_library) in targets {
        let Some(library_info) = deps.libraries.get(library_key) else {
            continue;
        };

        if library_info.library_type != "package" {
            continue;
        }

        let Some(package_path) = library_info.path.as_deref() else {
            continue;
        };

        let package_dir = nuget_global.join(package_path);

        for asset_rel_path in target_library.runtime.keys() {
            let asset_rel = Path::new(asset_rel_path);
            let Some(assembly_stem) = asset_rel.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };

            managed_paths.push((assembly_stem.to_string(), package_dir.join(asset_rel)));
        }
    }

    managed_paths
}

pub fn derive_native_search_dirs(deps: &DepsJson, nuget_global: &Path) -> Vec<PathBuf> {
    let Some(targets) = deps.targets.get(&deps.runtime_target.name) else {
        return Vec::new();
    };

    let mut native_dirs = Vec::new();

    for (library_key, target_library) in targets {
        if target_library.native.is_empty() {
            continue;
        }

        let Some(library_info) = deps.libraries.get(library_key) else {
            continue;
        };

        if library_info.library_type != "package" {
            continue;
        }

        let Some(package_path) = library_info.path.as_deref() else {
            continue;
        };

        let package_dir = nuget_global.join(package_path);

        for native_rel_path in target_library.native.keys() {
            let native_rel = Path::new(native_rel_path);
            let native_dir = native_rel
                .parent()
                .map_or_else(|| package_dir.clone(), |parent| package_dir.join(parent));

            if !native_dirs.contains(&native_dir) {
                native_dirs.push(native_dir);
            }
        }
    }

    native_dirs
}

pub fn nuget_global_packages_dir() -> PathBuf {
    if let Some(nuget_packages) = std::env::var_os("NUGET_PACKAGES")
        && !nuget_packages.is_empty()
    {
        return PathBuf::from(nuget_packages);
    }

    if cfg!(target_os = "windows") {
        if let Some(user_profile) = std::env::var_os("USERPROFILE")
            && !user_profile.is_empty()
        {
            return PathBuf::from(user_profile).join(".nuget").join("packages");
        }
    } else if let Some(home) = std::env::var_os("HOME")
        && !home.is_empty()
    {
        return PathBuf::from(home).join(".nuget").join("packages");
    }

    PathBuf::from(".nuget").join("packages")
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

/// Classification of an entry file passed to the runner.
///
/// Only `Managed` images (a PE/PE32+ carrying a CLI/COR header — this includes
/// ReadyToRun, which still embeds IL) can be interpreted. NativeAOT and
/// single-file bundles carry no usable IL and must fail loudly rather than
/// surfacing a confusing downstream parse error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// A managed assembly with a CLI header (incl. ReadyToRun).
    Managed,
    /// A NativeAOT-published native image (ELF/Mach-O, or a PE without a CLI header).
    NativeAot,
    /// A single-file bundle (native host with an appended bundle manifest).
    SingleFileBundle,
}

/// The 16-byte signature the .NET single-file bundler embeds in the host so the
/// runtime can locate the appended bundle manifest. Searched for near the tail.
/// (Matches `bundle_marker` in the dotnet runtime host.)
const SINGLE_FILE_BUNDLE_SIGNATURE: [u8; 16] = [
    0x8b, 0x12, 0x02, 0xb9, 0x6a, 0x61, 0x20, 0x38, 0x72, 0x7b, 0x93, 0x02, 0x14, 0xd7, 0xa0, 0x32,
];

/// Probe an entry file and classify it. On any read/parse uncertainty this
/// returns [`EntryKind::Managed`] (the status-quo path), so the caller's
/// existing loader produces the real error — i.e. we never *misclassify a real
/// managed assembly as native*, only confidently flag the unsupported kinds.
pub fn probe_entry_kind(path: &Path) -> EntryKind {
    match fs::read(path) {
        Ok(bytes) => classify_entry_bytes(&bytes),
        // Let the downstream loader produce the canonical "not found"/IO error.
        Err(_) => EntryKind::Managed,
    }
}

/// Byte-level classifier behind [`probe_entry_kind`] (split out for unit tests).
fn classify_entry_bytes(bytes: &[u8]) -> EntryKind {
    // A single-file bundle is a native host with the bundle signature appended.
    // Check this first: it applies regardless of the host's own binary format.
    if tail_contains_bundle_signature(bytes) {
        return EntryKind::SingleFileBundle;
    }

    // ELF (Linux) / Mach-O (macOS) magic ⇒ a native image. Managed assemblies
    // are always PE, so this is an unambiguous NativeAOT signal on those OSes.
    const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
    const MACHO_MAGICS: [[u8; 4]; 4] = [
        [0xfe, 0xed, 0xfa, 0xce], // MH_MAGIC (32-bit)
        [0xfe, 0xed, 0xfa, 0xcf], // MH_MAGIC_64
        [0xcf, 0xfa, 0xed, 0xfe], // MH_CIGAM_64 (byte-swapped)
        [0xca, 0xfe, 0xba, 0xbe], // FAT_MAGIC (universal)
    ];
    if bytes.len() >= 4 {
        let head = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if head == ELF_MAGIC || MACHO_MAGICS.contains(&head) {
            return EntryKind::NativeAot;
        }
    }

    // PE: only downgrade to NativeAot when we can *confidently* read a zero CLI
    // (COM descriptor, data directory index 14) header. Any parse uncertainty
    // falls through to Managed.
    if bytes.len() >= 2 && &bytes[0..2] == b"MZ" {
        match pe_has_cli_header(bytes) {
            Some(false) => return EntryKind::NativeAot,
            Some(true) | None => return EntryKind::Managed,
        }
    }

    // Unknown/short: be conservative — let the loader speak.
    EntryKind::Managed
}

fn tail_contains_bundle_signature(bytes: &[u8]) -> bool {
    // The signature lives near the end; scan a bounded tail window.
    const TAIL_WINDOW: usize = 4096;
    let start = bytes.len().saturating_sub(TAIL_WINDOW);
    bytes[start..]
        .windows(SINGLE_FILE_BUNDLE_SIGNATURE.len())
        .any(|w| w == SINGLE_FILE_BUNDLE_SIGNATURE)
}

/// Returns `Some(true)` if the PE has a non-empty CLI/COR header (managed),
/// `Some(false)` if it parses as a PE with an empty COM descriptor directory
/// (NativeAOT), or `None` if the headers can't be read with confidence.
fn pe_has_cli_header(bytes: &[u8]) -> Option<bool> {
    let read_u16 = |off: usize| -> Option<u16> {
        bytes
            .get(off..off + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
    };
    let read_u32 = |off: usize| -> Option<u32> {
        bytes
            .get(off..off + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };

    // e_lfanew @ 0x3C → offset of the PE signature.
    let pe_off = read_u32(0x3C)? as usize;
    if bytes.get(pe_off..pe_off + 4)? != b"PE\0\0" {
        return None;
    }

    // COFF header (20 bytes) follows the signature; the optional header follows that.
    let opt_off = pe_off + 4 + 20;
    // Optional header magic: 0x10b = PE32, 0x20b = PE32+.
    let magic = read_u16(opt_off)?;
    // NumberOfRvaAndSizes lives at a magic-dependent offset; the data directory
    // array starts right after it. COM descriptor is directory index 14.
    let (num_dirs_off, data_dir_off) = match magic {
        0x10b => (opt_off + 92, opt_off + 96),   // PE32
        0x20b => (opt_off + 108, opt_off + 112), // PE32+
        _ => return None,
    };
    let num_dirs = read_u32(num_dirs_off)?;
    if num_dirs < 15 {
        // No COM descriptor slot at all ⇒ not a managed image.
        return Some(false);
    }
    let com_off = data_dir_off + 14 * 8; // each IMAGE_DATA_DIRECTORY is 8 bytes
    let com_rva = read_u32(com_off)?;
    let com_size = read_u32(com_off + 4)?;
    Some(com_rva != 0 && com_size != 0)
}

#[cfg(test)]
mod tests {
    use super::{
        EntryKind, FrameworkRef, RollForwardPolicy, SINGLE_FILE_BUNDLE_SIGNATURE,
        classify_entry_bytes, derive_managed_probing_paths, derive_native_search_dirs,
        parse_deps_json, parse_runtimeconfig, resolve_framework_from_runtimeconfig,
        select_framework_version,
    };
    use serde_json::Value;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture_probe_dir() -> PathBuf {
        if let Some(base) = std::env::var_os("DOTNET_FIXTURES_BASE") {
            let path = PathBuf::from(base).join("basic").join("basic_42");
            if path.exists() {
                return path;
            }
        }

        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        if let Some(repo_root) = manifest_dir.parent().and_then(Path::parent) {
            let path = repo_root
                .join("target")
                .join("debug")
                .join("dotnet-fixtures")
                .join("basic")
                .join("basic_42");
            if path.exists() {
                return path;
            }
        }

        PathBuf::from("/tmp/fixture-probe")
    }

    fn fixture_probe_path(file_name: &str) -> PathBuf {
        fixture_probe_dir().join(file_name)
    }

    #[test]
    #[cfg(not(miri))]
    fn parses_fixture_runtimeconfig() {
        let path = fixture_probe_path("SingleFile.runtimeconfig.json");
        assert!(
            path.exists(),
            "missing fixture runtimeconfig at {}; build fixtures first",
            path.display()
        );

        let config = parse_runtimeconfig(&path).expect("runtimeconfig should parse");
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

    #[test]
    #[cfg(not(miri))]
    fn parses_fixture_deps_and_derives_no_nuget_probing_paths() {
        let deps_path = fixture_probe_path("SingleFile.deps.json");
        assert!(
            deps_path.exists(),
            "missing fixture deps.json at {}; build fixtures first",
            deps_path.display()
        );

        let deps = parse_deps_json(&deps_path).expect("deps.json should parse");
        let nuget_global = Path::new("/tmp/fixture-probe-nuget-root");

        assert_eq!(deps.runtime_target.name, ".NETCoreApp,Version=v10.0");
        assert!(derive_managed_probing_paths(&deps, nuget_global).is_empty());
        assert!(derive_native_search_dirs(&deps, nuget_global).is_empty());
    }

    #[test]
    #[cfg(not(miri))]
    fn parses_newtonsoft_deps_and_derives_package_probing_paths() {
        let probe_dir = create_runtime_base(&[]);
        let deps_path = probe_dir.join("App.deps.json");
        fs::write(
            &deps_path,
            r#"{
  "runtimeTarget": {
    "name": ".NETCoreApp,Version=v10.0",
    "signature": ""
  },
  "targets": {
    ".NETCoreApp,Version=v10.0": {
      "App/1.0.0": {
        "dependencies": {
          "Newtonsoft.Json": "13.0.3"
        },
        "runtime": {
          "App.dll": {}
        }
      },
      "Newtonsoft.Json/13.0.3": {
        "runtime": {
          "lib/net6.0/Newtonsoft.Json.dll": {
            "assemblyVersion": "13.0.0.0",
            "fileVersion": "13.0.3.27908"
          }
        }
      }
    }
  },
  "libraries": {
    "App/1.0.0": {
      "type": "project",
      "serviceable": false,
      "sha512": ""
    },
    "Newtonsoft.Json/13.0.3": {
      "type": "package",
      "serviceable": true,
      "sha512": "",
      "path": "newtonsoft.json/13.0.3",
      "hashPath": "newtonsoft.json.13.0.3.nupkg.sha512"
    }
  }
}"#,
        )
        .expect("Newtonsoft probe deps.json should be written");

        let deps = parse_deps_json(&deps_path).expect("deps.json should parse");
        let nuget_global = Path::new("/tmp/nuget-global");

        let managed = derive_managed_probing_paths(&deps, nuget_global);
        assert_eq!(managed.len(), 1);
        assert_eq!(
            managed[0],
            (
                "Newtonsoft.Json".to_string(),
                nuget_global
                    .join("newtonsoft.json/13.0.3")
                    .join("lib/net6.0/Newtonsoft.Json.dll")
            )
        );
        assert!(derive_native_search_dirs(&deps, nuget_global).is_empty());

        fs::remove_dir_all(&probe_dir).expect("Newtonsoft probe dir should be removed");
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
        let path = fixture_probe_path("SingleFile.runtimeconfig.json");
        assert!(
            path.exists(),
            "missing fixture runtimeconfig at {}; build fixtures first",
            path.display()
        );

        let config = parse_runtimeconfig(&path).expect("runtimeconfig should parse");
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

        let path = fixture_probe_path("SingleFile.runtimeconfig.json");
        assert!(
            path.exists(),
            "missing fixture runtimeconfig at {}; build fixtures first",
            path.display()
        );

        let config = parse_runtimeconfig(&path).expect("runtimeconfig should parse");
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

    /// Build a minimal PE (`MZ`…`PE\0\0` + optional header + data dirs) whose
    /// COM-descriptor (dir 14) RVA/size are set per `cli_present`.
    fn synthetic_pe(cli_present: bool) -> Vec<u8> {
        let pe_off: usize = 0x80;
        let opt_off = pe_off + 4 + 20; // after PE sig + COFF header
        let data_dir_off = opt_off + 96; // PE32 data directory array
        let total = data_dir_off + 16 * 8; // 16 directory entries
        let mut buf = vec![0u8; total];
        buf[0] = b'M';
        buf[1] = b'Z';
        buf[0x3C..0x40].copy_from_slice(&(pe_off as u32).to_le_bytes());
        buf[pe_off..pe_off + 4].copy_from_slice(b"PE\0\0");
        buf[opt_off..opt_off + 2].copy_from_slice(&0x10bu16.to_le_bytes()); // PE32
        buf[opt_off + 92..opt_off + 96].copy_from_slice(&16u32.to_le_bytes()); // NumberOfRvaAndSizes
        if cli_present {
            let com_off = data_dir_off + 14 * 8;
            buf[com_off..com_off + 4].copy_from_slice(&0x2000u32.to_le_bytes()); // RVA
            buf[com_off + 4..com_off + 8].copy_from_slice(&0x48u32.to_le_bytes()); // size
        }
        buf
    }

    #[test]
    fn classifies_managed_pe_with_cli_header() {
        assert_eq!(
            classify_entry_bytes(&synthetic_pe(true)),
            EntryKind::Managed
        );
    }

    #[test]
    fn classifies_pe_without_cli_header_as_nativeaot() {
        assert_eq!(
            classify_entry_bytes(&synthetic_pe(false)),
            EntryKind::NativeAot
        );
    }

    #[test]
    fn classifies_elf_as_nativeaot() {
        let mut buf = vec![0u8; 256];
        buf[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        assert_eq!(classify_entry_bytes(&buf), EntryKind::NativeAot);
    }

    #[test]
    fn classifies_bundle_signature_as_single_file() {
        // ELF host with the bundle signature appended near the tail.
        let mut buf = vec![0u8; 8192];
        buf[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        let at = buf.len() - 64;
        buf[at..at + 16].copy_from_slice(&SINGLE_FILE_BUNDLE_SIGNATURE);
        assert_eq!(
            classify_entry_bytes(&buf),
            EntryKind::SingleFileBundle,
            "bundle signature must win even over a native host format"
        );
    }

    #[test]
    fn short_or_unknown_input_defaults_to_managed() {
        // Conservative: never misclassify a real managed assembly as native.
        assert_eq!(classify_entry_bytes(b"hi"), EntryKind::Managed);
        assert_eq!(classify_entry_bytes(&[]), EntryKind::Managed);
    }
}
