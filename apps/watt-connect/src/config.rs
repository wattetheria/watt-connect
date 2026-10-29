//! Local paths and `config.toml`: which services are connected and how.
//! Tokens are never written here.

use anyhow::{Context, Result};
use connect_server::{ConnectionRecord, ConnectionStore};
use connect_types::ServiceId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Platform directories (via `directories`), or everything under
/// `WATT_CONNECT_HOME` when set.
#[derive(Debug, Clone)]
#[allow(clippy::struct_field_names)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
    pub home_dir: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Result<Self> {
        let base = directories::BaseDirs::new().context("cannot determine the home directory")?;
        let home_dir = base.home_dir().to_owned();
        if let Some(root) = std::env::var_os("WATT_CONNECT_HOME").filter(|value| !value.is_empty())
        {
            let root = PathBuf::from(root);
            return Ok(Self {
                config_dir: root.join("config"),
                cache_dir: root.join("cache"),
                data_dir: root.join("data"),
                log_dir: root.join("logs"),
                home_dir,
            });
        }
        let project = directories::ProjectDirs::from("", "", "watt-connect")
            .context("cannot determine platform directories")?;
        let log_dir = if cfg!(target_os = "macos") {
            home_dir.join("Library/Logs/watt-connect")
        } else {
            project.data_local_dir().join("logs")
        };
        Ok(Self {
            config_dir: project.config_dir().to_owned(),
            cache_dir: project.cache_dir().to_owned(),
            data_dir: project.data_local_dir().to_owned(),
            log_dir,
            home_dir,
        })
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(default, skip_serializing_if = "CatalogSettings::is_empty")]
    pub catalog: CatalogSettings,
    #[serde(default)]
    pub services: BTreeMap<String, ServiceSettings>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CatalogSettings {
    /// Override the catalog URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl CatalogSettings {
    fn is_empty(&self) -> bool {
        self.url.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceSettings {
    pub mode: connect_types::ConnectionMode,
    pub connected_at: String,
    #[serde(default)]
    pub credential_status: connect_types::CredentialStatus,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sandbox: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<String>,
}

pub fn read_config(path: &Path) -> Result<ConfigFile> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            toml::from_str(&text).with_context(|| format!("{} is not valid TOML", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ConfigFile::default()),
        Err(error) => Err(error).with_context(|| format!("cannot read {}", path.display())),
    }
}

fn write_config(path: &Path, config: &ConfigFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = format!(
        "# watt-connect connections. Managed by `watt-connect`; contains no tokens.\n{}",
        toml::to_string_pretty(config)?
    );
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

/// `ConnectionStore` backed by `config.toml`. Re-read on every access so a
/// running server sees `watt-connect connect` done in another terminal.
pub struct TomlConnectionStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl TomlConnectionStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    fn modify(&self, change: impl FnOnce(&mut ConfigFile)) -> Result<(), String> {
        let _guard = self
            .lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut config = read_config(&self.path).map_err(|error| format!("{error:#}"))?;
        change(&mut config);
        write_config(&self.path, &config).map_err(|error| format!("{error:#}"))
    }
}

impl ConnectionStore for TomlConnectionStore {
    fn list(&self) -> Vec<ConnectionRecord> {
        let config = match read_config(&self.path) {
            Ok(config) => config,
            Err(error) => {
                tracing::warn!("{error:#}");
                return Vec::new();
            }
        };
        config
            .services
            .into_iter()
            .filter_map(|(id, settings)| {
                let service = ServiceId::parse(&id).ok()?;
                Some(ConnectionRecord {
                    service,
                    mode: settings.mode,
                    connected_at: settings.connected_at,
                    credential_status: settings.credential_status,
                    sandbox: settings.sandbox,
                    agents: settings.agents,
                })
            })
            .collect()
    }

    fn upsert(&self, record: ConnectionRecord) -> Result<(), String> {
        self.modify(|config| {
            config.services.insert(
                record.service.to_string(),
                ServiceSettings {
                    mode: record.mode,
                    connected_at: record.connected_at,
                    credential_status: record.credential_status,
                    sandbox: record.sandbox,
                    agents: record.agents,
                },
            );
        })
    }

    fn remove(&self, service: &ServiceId) -> Result<(), String> {
        self.modify(|config| {
            config.services.remove(service.as_str());
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use connect_types::{ConnectionMode, CredentialStatus};

    #[test]
    fn round_trips_records_without_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let store = TomlConnectionStore::new(dir.path().join("config.toml"));
        let id = ServiceId::parse("cn.lkcoffee/order").unwrap();
        store
            .upsert(ConnectionRecord {
                service: id.clone(),
                mode: ConnectionMode::Proxy,
                connected_at: "2026-10-29T00:00:00Z".into(),
                credential_status: CredentialStatus::Ok,
                sandbox: false,
                agents: vec![],
            })
            .unwrap();
        let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("[services.\"cn.lkcoffee/order\"]"), "{text}");
        assert!(text.contains("mode = \"proxy\""), "{text}");
        assert_eq!(
            store.get(&id).unwrap().credential_status,
            CredentialStatus::Ok
        );
        store
            .set_credential_status(&id, CredentialStatus::CredentialInvalid)
            .unwrap();
        assert_eq!(
            store.get(&id).unwrap().credential_status,
            CredentialStatus::CredentialInvalid
        );
        store.remove(&id).unwrap();
        assert!(store.list().is_empty());
    }
}
