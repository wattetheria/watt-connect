//! Cursor: global `~/.cursor/mcp.json`. Cursor has no CLI for this, so the
//! file is edited directly (backed up first).

use crate::{
    AgentInstaller, AgentKind, Detection, DirectEntry, InstallError, InstallReport, InstallerEnv,
    json_config, validate_name,
};
use std::path::PathBuf;

pub struct CursorInstaller {
    dir: PathBuf,
}

impl CursorInstaller {
    #[must_use]
    pub fn new(env: &InstallerEnv) -> Self {
        Self {
            dir: env.home.join(".cursor"),
        }
    }

    fn path(&self) -> PathBuf {
        self.dir.join("mcp.json")
    }
}

impl AgentInstaller for CursorInstaller {
    fn kind(&self) -> AgentKind {
        AgentKind::Cursor
    }

    fn detect(&self) -> Detection {
        Detection {
            present: self.dir.is_dir(),
            via_cli: false,
            config_path: self.path(),
        }
    }

    fn install(&self, entry: &DirectEntry) -> Result<InstallReport, InstallError> {
        validate_name(&entry.name)?;
        if !self.dir.is_dir() {
            return Err(InstallError::NotInstalled(AgentKind::Cursor));
        }
        let path = self.path();
        let backup = json_config::upsert(
            &path,
            &entry.name,
            json_config::server_value(&entry.transport, false),
        )?;
        Ok(InstallReport {
            agent: AgentKind::Cursor,
            name: entry.name.clone(),
            location: path.display().to_string(),
            backup,
        })
    }

    fn uninstall(&self, name: &str) -> Result<bool, InstallError> {
        json_config::remove(&self.path(), name)
    }

    fn list(&self) -> Result<Vec<String>, InstallError> {
        json_config::names(&self.path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DirectTransport;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    fn env(home: &std::path::Path) -> InstallerEnv {
        InstallerEnv {
            home: home.to_owned(),
            use_cli: false,
            codex_home: None,
        }
    }

    #[test]
    fn installs_http_entry_and_keeps_other_servers() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".cursor")).unwrap();
        std::fs::write(
            home.path().join(".cursor/mcp.json"),
            r#"{"mcpServers":{"other":{"command":"x"}},"extra":true}"#,
        )
        .unwrap();
        let installer = CursorInstaller::new(&env(home.path()));
        let report = installer
            .install(&DirectEntry {
                name: "mcd-order".into(),
                transport: DirectTransport::Http {
                    url: "https://mcp.mcd.cn".into(),
                    headers: BTreeMap::from([("Authorization".into(), "Bearer t".into())]),
                },
            })
            .unwrap();
        assert!(report.backup.is_some());
        let written: Value = serde_json::from_str(
            &std::fs::read_to_string(home.path().join(".cursor/mcp.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(written["extra"], json!(true));
        assert_eq!(written["mcpServers"]["other"], json!({"command": "x"}));
        assert_eq!(
            written["mcpServers"]["mcd-order"],
            json!({"url": "https://mcp.mcd.cn", "headers": {"Authorization": "Bearer t"}})
        );
        assert_eq!(installer.list().unwrap(), vec!["other", "mcd-order"]);
        assert!(installer.uninstall("mcd-order").unwrap());
        assert!(!installer.uninstall("mcd-order").unwrap());
        assert_eq!(installer.list().unwrap(), vec!["other"]);
    }

    #[test]
    fn refuses_when_cursor_absent() {
        let home = tempfile::tempdir().unwrap();
        let installer = CursorInstaller::new(&env(home.path()));
        assert!(!installer.detect().present);
        let error = installer
            .install(&DirectEntry {
                name: "x".into(),
                transport: DirectTransport::Stdio {
                    command: "npx".into(),
                    args: vec![],
                    env: BTreeMap::new(),
                },
            })
            .unwrap_err();
        assert!(matches!(
            error,
            InstallError::NotInstalled(AgentKind::Cursor)
        ));
    }
}
