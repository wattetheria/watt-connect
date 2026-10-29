//! Codex: `$CODEX_HOME/config.toml` (default `~/.codex/config.toml`),
//! `[mcp_servers.<name>]`. Edited with `toml_edit` so comments and layout of
//! the rest of the file survive. (`codex mcp add` cannot set static headers,
//! so the file is edited directly, backed up first.)

use crate::backup::{backup, write_atomic};
use crate::{
    AgentInstaller, AgentKind, Detection, DirectEntry, DirectTransport, InstallError,
    InstallReport, InstallerEnv, io_error, validate_name,
};
use std::path::{Path, PathBuf};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, value};

const SERVERS: &str = "mcp_servers";

pub struct CodexInstaller {
    dir: PathBuf,
}

impl CodexInstaller {
    #[must_use]
    pub fn new(env: &InstallerEnv) -> Self {
        Self {
            dir: env
                .codex_home
                .clone()
                .unwrap_or_else(|| env.home.join(".codex")),
        }
    }

    fn path(&self) -> PathBuf {
        self.dir.join("config.toml")
    }

    fn read(path: &Path) -> Result<DocumentMut, InstallError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(io_error(path)(error)),
        };
        text.parse::<DocumentMut>()
            .map_err(|error| InstallError::Parse {
                path: path.to_owned(),
                format: "TOML",
                message: error.to_string(),
            })
    }

    fn write(path: &Path, document: &DocumentMut) -> Result<Option<PathBuf>, InstallError> {
        let backup = backup(path)?;
        write_atomic(path, document.to_string().as_bytes())?;
        Ok(backup)
    }

    fn servers<'a>(
        document: &'a mut DocumentMut,
        path: &Path,
    ) -> Result<&'a mut Table, InstallError> {
        let item = document.entry(SERVERS).or_insert_with(|| {
            let mut table = Table::new();
            table.set_implicit(true);
            Item::Table(table)
        });
        item.as_table_mut().ok_or_else(|| InstallError::Parse {
            path: path.to_owned(),
            format: "TOML",
            message: format!("`{SERVERS}` must be a table"),
        })
    }

    fn server_table(transport: &DirectTransport) -> Table {
        let mut table = Table::new();
        let inline = |pairs: &std::collections::BTreeMap<String, String>| {
            let mut inline = InlineTable::new();
            for (key, val) in pairs {
                inline.insert(key, val.as_str().into());
            }
            value(inline)
        };
        match transport {
            DirectTransport::Http { url, headers } => {
                table.insert("url", value(url.as_str()));
                if !headers.is_empty() {
                    table.insert("http_headers", inline(headers));
                }
            }
            DirectTransport::Stdio { command, args, env } => {
                table.insert("command", value(command.as_str()));
                table.insert(
                    "args",
                    value(args.iter().map(String::as_str).collect::<Array>()),
                );
                if !env.is_empty() {
                    table.insert("env", inline(env));
                }
            }
        }
        table
    }
}

impl AgentInstaller for CodexInstaller {
    fn kind(&self) -> AgentKind {
        AgentKind::Codex
    }

    fn detect(&self) -> Detection {
        Detection {
            present: self.dir.is_dir() || which::which("codex").is_ok(),
            via_cli: false,
            config_path: self.path(),
        }
    }

    fn install(&self, entry: &DirectEntry) -> Result<InstallReport, InstallError> {
        validate_name(&entry.name)?;
        if !self.detect().present {
            return Err(InstallError::NotInstalled(AgentKind::Codex));
        }
        let path = self.path();
        let mut document = Self::read(&path)?;
        Self::servers(&mut document, &path)?.insert(
            &entry.name,
            Item::Table(Self::server_table(&entry.transport)),
        );
        let backup = Self::write(&path, &document)?;
        Ok(InstallReport {
            agent: AgentKind::Codex,
            name: entry.name.clone(),
            location: path.display().to_string(),
            backup,
        })
    }

    fn uninstall(&self, name: &str) -> Result<bool, InstallError> {
        let path = self.path();
        let mut document = Self::read(&path)?;
        let removed = document
            .get_mut(SERVERS)
            .and_then(Item::as_table_mut)
            .is_some_and(|servers| servers.remove(name).is_some());
        if removed {
            Self::write(&path, &document)?;
        }
        Ok(removed)
    }

    fn list(&self) -> Result<Vec<String>, InstallError> {
        let document = Self::read(&self.path())?;
        Ok(document
            .get(SERVERS)
            .and_then(Item::as_table)
            .map(|servers| servers.iter().map(|(name, _)| name.to_owned()).collect())
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn writes_http_and_stdio_entries_preserving_comments() {
        let home = tempfile::tempdir().unwrap();
        let codex = home.path().join("codex-home");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::write(codex.join("config.toml"), "# my settings\nmodel = \"o4\"\n").unwrap();
        let installer = CodexInstaller::new(&InstallerEnv {
            home: home.path().to_owned(),
            use_cli: false,
            codex_home: Some(codex.clone()),
        });
        installer
            .install(&DirectEntry {
                name: "lkcoffee-order".into(),
                transport: DirectTransport::Http {
                    url: "https://gwmcp.lkcoffee.com/order/user/mcp".into(),
                    headers: BTreeMap::from([("Authorization".into(), "Bearer t".into())]),
                },
            })
            .unwrap();
        installer
            .install(&DirectEntry {
                name: "wattetheria-node".into(),
                transport: DirectTransport::Stdio {
                    command: "npx".into(),
                    args: vec!["-y".into(), "wattetheria".into(), "mcp-proxy".into()],
                    env: BTreeMap::new(),
                },
            })
            .unwrap();
        let text = std::fs::read_to_string(codex.join("config.toml")).unwrap();
        assert!(
            text.starts_with("# my settings\nmodel = \"o4\"\n"),
            "{text}"
        );
        assert!(text.contains("[mcp_servers.lkcoffee-order]"), "{text}");
        assert!(
            text.contains("url = \"https://gwmcp.lkcoffee.com/order/user/mcp\""),
            "{text}"
        );
        assert!(
            text.contains("http_headers = { Authorization = \"Bearer t\" }"),
            "{text}"
        );
        assert!(
            text.contains("args = [\"-y\", \"wattetheria\", \"mcp-proxy\"]"),
            "{text}"
        );
        assert_eq!(
            installer.list().unwrap(),
            vec!["lkcoffee-order", "wattetheria-node"]
        );
        assert!(installer.uninstall("lkcoffee-order").unwrap());
        assert_eq!(installer.list().unwrap(), vec!["wattetheria-node"]);
    }
}
