//! Claude Code. Prefers its CLI (`claude mcp add --scope user …`) so Claude
//! Code owns its own file; falls back to editing `~/.claude.json` only when
//! the CLI is not on PATH.

use crate::{
    AgentInstaller, AgentKind, Detection, DirectEntry, DirectTransport, InstallError,
    InstallReport, InstallerEnv, json_config, validate_name,
};
use std::path::PathBuf;
use std::process::Command;

pub struct ClaudeCodeInstaller {
    home: PathBuf,
    cli: Option<PathBuf>,
}

impl ClaudeCodeInstaller {
    #[must_use]
    pub fn new(env: &InstallerEnv) -> Self {
        Self {
            home: env.home.clone(),
            cli: if env.use_cli {
                which::which("claude").ok()
            } else {
                None
            },
        }
    }

    fn path(&self) -> PathBuf {
        self.home.join(".claude.json")
    }

    fn run(cli: &PathBuf, args: &[String]) -> Result<(), InstallError> {
        let output = Command::new(cli)
            .args(args)
            .output()
            .map_err(|source| InstallError::Io {
                path: cli.clone(),
                source,
            })?;
        if output.status.success() {
            Ok(())
        } else {
            Err(InstallError::Cli {
                // Never echo the arguments: they may contain the credential.
                command: format!("claude mcp {}", args.first().map_or("", String::as_str)),
                message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            })
        }
    }

    /// Arguments after `claude`.
    #[must_use]
    pub fn add_args(entry: &DirectEntry) -> Vec<String> {
        let mut args: Vec<String> = ["mcp", "add", "--scope", "user"].map(String::from).to_vec();
        match &entry.transport {
            DirectTransport::Http { url, headers } => {
                args.extend([
                    "--transport".into(),
                    "http".into(),
                    entry.name.clone(),
                    url.clone(),
                ]);
                for (name, value) in headers {
                    args.extend(["--header".into(), format!("{name}: {value}")]);
                }
            }
            DirectTransport::Stdio {
                command,
                args: cmd_args,
                env,
            } => {
                args.extend(["--transport".into(), "stdio".into()]);
                for (key, value) in env {
                    args.extend(["--env".into(), format!("{key}={value}")]);
                }
                args.extend([entry.name.clone(), "--".into(), command.clone()]);
                args.extend(cmd_args.iter().cloned());
            }
        }
        args
    }
}

impl AgentInstaller for ClaudeCodeInstaller {
    fn kind(&self) -> AgentKind {
        AgentKind::ClaudeCode
    }

    fn detect(&self) -> Detection {
        Detection {
            present: self.cli.is_some()
                || self.path().exists()
                || self.home.join(".claude").is_dir(),
            via_cli: self.cli.is_some(),
            config_path: self.path(),
        }
    }

    fn install(&self, entry: &DirectEntry) -> Result<InstallReport, InstallError> {
        validate_name(&entry.name)?;
        if let Some(cli) = &self.cli {
            // `add` fails if the name exists; replace it.
            let _ = Self::run(
                cli,
                &["mcp", "remove", "--scope", "user", &entry.name].map(String::from),
            );
            Self::run(cli, &Self::add_args(entry))?;
            return Ok(InstallReport {
                agent: AgentKind::ClaudeCode,
                name: entry.name.clone(),
                location: "claude mcp add --scope user".into(),
                backup: None,
            });
        }
        if !self.detect().present {
            return Err(InstallError::NotInstalled(AgentKind::ClaudeCode));
        }
        let path = self.path();
        let backup = json_config::upsert(
            &path,
            &entry.name,
            json_config::server_value(&entry.transport, true),
        )?;
        Ok(InstallReport {
            agent: AgentKind::ClaudeCode,
            name: entry.name.clone(),
            location: path.display().to_string(),
            backup,
        })
    }

    fn uninstall(&self, name: &str) -> Result<bool, InstallError> {
        validate_name(name)?;
        let existed = self.list()?.iter().any(|existing| existing == name);
        if let Some(cli) = &self.cli {
            if existed {
                Self::run(
                    cli,
                    &["mcp", "remove", "--scope", "user", name].map(String::from),
                )?;
            }
            return Ok(existed);
        }
        json_config::remove(&self.path(), name)
    }

    fn list(&self) -> Result<Vec<String>, InstallError> {
        json_config::names(&self.path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    #[test]
    fn cli_arguments() {
        let http = DirectEntry {
            name: "lkcoffee-order".into(),
            transport: DirectTransport::Http {
                url: "https://gwmcp.lkcoffee.com/order/user/mcp".into(),
                headers: BTreeMap::from([("Authorization".into(), "Bearer t".into())]),
            },
        };
        assert_eq!(
            ClaudeCodeInstaller::add_args(&http),
            [
                "mcp",
                "add",
                "--scope",
                "user",
                "--transport",
                "http",
                "lkcoffee-order",
                "https://gwmcp.lkcoffee.com/order/user/mcp",
                "--header",
                "Authorization: Bearer t"
            ]
        );
        let stdio = DirectEntry {
            name: "wattetheria-node".into(),
            transport: DirectTransport::Stdio {
                command: "npx".into(),
                args: vec!["-y".into(), "wattetheria".into(), "mcp-proxy".into()],
                env: BTreeMap::new(),
            },
        };
        assert_eq!(
            ClaudeCodeInstaller::add_args(&stdio),
            [
                "mcp",
                "add",
                "--scope",
                "user",
                "--transport",
                "stdio",
                "wattetheria-node",
                "--",
                "npx",
                "-y",
                "wattetheria",
                "mcp-proxy"
            ]
        );
    }

    #[test]
    fn file_fallback_writes_user_scope() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join(".claude.json"),
            r#"{"numStartups":3,"projects":{}}"#,
        )
        .unwrap();
        let installer = ClaudeCodeInstaller::new(&InstallerEnv {
            home: home.path().to_owned(),
            use_cli: false,
            codex_home: None,
        });
        installer
            .install(&DirectEntry {
                name: "mcd-order".into(),
                transport: DirectTransport::Http {
                    url: "https://mcp.mcd.cn".into(),
                    headers: BTreeMap::from([("Authorization".into(), "Bearer t".into())]),
                },
            })
            .unwrap();
        let written: Value = serde_json::from_str(
            &std::fs::read_to_string(home.path().join(".claude.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(written["numStartups"], json!(3));
        assert_eq!(
            written["mcpServers"]["mcd-order"],
            json!({"type": "http", "url": "https://mcp.mcd.cn", "headers": {"Authorization": "Bearer t"}})
        );
        assert!(installer.uninstall("mcd-order").unwrap());
        assert!(installer.list().unwrap().is_empty());
    }
}
