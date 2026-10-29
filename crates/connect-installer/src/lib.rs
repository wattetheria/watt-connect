//! Direct mode: write a merchant MCP server into an agent's own configuration
//! so the agent talks to the merchant without watt-connect in the middle.
//!
//! Rules: prefer the agent's own CLI when it has one; otherwise edit its
//! config file, backing it up first and touching only our own entry.
//!
//! Config locations and formats (checked 2026-10-29):
//! * Claude Code: `claude mcp add --scope user …`; user scope lives in
//!   `~/.claude.json` under `mcpServers`.
//! * Cursor: `~/.cursor/mcp.json`, `mcpServers.<name>` with `url` + `headers`
//!   or `command` + `args` + `env`.
//! * Codex: `$CODEX_HOME/config.toml` (default `~/.codex`),
//!   `[mcp_servers.<name>]` with `url` + `http_headers` or `command` + `args` + `env`.

pub mod backup;
pub mod claude_code;
pub mod codex;
pub mod cursor;
mod json_config;

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentKind {
    ClaudeCode,
    Cursor,
    Codex,
}

impl AgentKind {
    pub const ALL: [Self; 3] = [Self::ClaudeCode, Self::Cursor, Self::Codex];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Cursor => "cursor",
            Self::Codex => "codex",
        }
    }
}

impl fmt::Display for AgentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AgentKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "claude-code" | "claude" | "claudecode" | "claude_code" => Ok(Self::ClaudeCode),
            "cursor" => Ok(Self::Cursor),
            "codex" => Ok(Self::Codex),
            other => Err(format!(
                "unknown agent `{other}` (supported: claude-code, cursor, codex)"
            )),
        }
    }
}

/// How the agent should reach the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectTransport {
    /// Remote Streamable HTTP server. `url` and `headers` may contain the credential.
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
    /// Local stdio server, e.g. `npx wattetheria mcp-proxy`.
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    },
}

/// One server entry to write. `Debug` is not derived because it may hold a token.
#[derive(Clone, PartialEq, Eq)]
pub struct DirectEntry {
    /// Server name in the agent config, e.g. `lkcoffee-order`.
    pub name: String,
    pub transport: DirectTransport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detection {
    /// The agent appears to be installed (CLI on PATH or config dir present).
    pub present: bool,
    /// The agent's CLI will be used for writes.
    pub via_cli: bool,
    pub config_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReport {
    pub agent: AgentKind,
    pub name: String,
    /// Human-readable description of what was changed.
    pub location: String,
    pub backup: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("{0} is not installed (no CLI on PATH and no config directory)")]
    NotInstalled(AgentKind),
    #[error("cannot read or write {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not valid {format}: {message}")]
    Parse {
        path: PathBuf,
        format: &'static str,
        message: String,
    },
    #[error("`{command}` failed: {message}")]
    Cli { command: String, message: String },
    #[error("server name `{0}` is not allowed")]
    InvalidName(String),
}

/// Where installers look for things. Tests point `home` at a temp dir and
/// disable CLIs.
#[derive(Debug, Clone)]
pub struct InstallerEnv {
    pub home: PathBuf,
    pub use_cli: bool,
    /// `CODEX_HOME`, if set.
    pub codex_home: Option<PathBuf>,
}

impl InstallerEnv {
    /// From the real process environment.
    #[must_use]
    pub fn from_process(home: PathBuf) -> Self {
        Self {
            home,
            use_cli: true,
            codex_home: std::env::var_os("CODEX_HOME").map(PathBuf::from),
        }
    }
}

pub trait AgentInstaller: Send + Sync {
    fn kind(&self) -> AgentKind;
    fn detect(&self) -> Detection;
    /// Add or replace `entry`.
    fn install(&self, entry: &DirectEntry) -> Result<InstallReport, InstallError>;
    /// Remove `name`. Returns whether it existed.
    fn uninstall(&self, name: &str) -> Result<bool, InstallError>;
    /// Names of configured MCP servers.
    fn list(&self) -> Result<Vec<String>, InstallError>;
}

#[must_use]
pub fn installer(kind: AgentKind, env: &InstallerEnv) -> Box<dyn AgentInstaller> {
    match kind {
        AgentKind::ClaudeCode => Box::new(claude_code::ClaudeCodeInstaller::new(env)),
        AgentKind::Cursor => Box::new(cursor::CursorInstaller::new(env)),
        AgentKind::Codex => Box::new(codex::CodexInstaller::new(env)),
    }
}

pub(crate) fn validate_name(name: &str) -> Result<(), InstallError> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(InstallError::InvalidName(name.to_owned()))
    }
}

pub(crate) fn io_error(path: &std::path::Path) -> impl FnOnce(std::io::Error) -> InstallError + '_ {
    move |source| InstallError::Io {
        path: path.to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_names_parse() {
        assert_eq!(
            "claude".parse::<AgentKind>().unwrap(),
            AgentKind::ClaudeCode
        );
        assert_eq!("Cursor".parse::<AgentKind>().unwrap(), AgentKind::Cursor);
        assert!("vim".parse::<AgentKind>().is_err());
    }

    #[test]
    fn names_are_restricted() {
        assert!(validate_name("lkcoffee-order").is_ok());
        assert!(validate_name("a b").is_err());
        assert!(validate_name("a.b").is_err());
        assert!(validate_name("").is_err());
    }
}
