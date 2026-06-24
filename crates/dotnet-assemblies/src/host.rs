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

#[cfg(test)]
mod tests {
    use super::parse_runtimeconfig;
    use serde_json::Value;
    use std::path::Path;

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
}
