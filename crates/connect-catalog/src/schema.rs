//! Catalog document types.
//!
//! Each service entry is an MCP registry `server.json` document; everything
//! watt-connect needs beyond that lives under `_meta["watt/connect"]`.

use connect_types::{ConnectionMode, CredentialPlacement, ResponseMatcher, ServiceId, ToolEffect};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Key of the watt-connect extension inside `server.json`'s `_meta`.
pub const META_KEY: &str = "watt/connect";

/// Version of the catalog snapshot format.
pub const CATALOG_SCHEMA_VERSION: u32 = 1;

/// Transport name of remote endpoints watt-connect can proxy.
pub const STREAMABLE_HTTP: &str = "streamable-http";

/// A full catalog snapshot, as published (and signed) by the catalog service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    pub schema_version: u32,
    /// Monotonic snapshot version such as `2026.10.29.1`. Compared numerically
    /// segment by segment.
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
    pub services: Vec<ServerEntry>,
}

/// One MCP server, in MCP registry `server.json` shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerEntry {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub name: ServiceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub description: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remotes: Vec<Remote>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<Package>,
    #[serde(rename = "_meta")]
    pub meta: EntryMeta,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryMeta {
    #[serde(rename = "watt/connect")]
    pub connect: ConnectMeta,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Remote {
    #[serde(rename = "type")]
    pub transport: String,
    pub url: String,
}

/// A locally launched package (e.g. `npx wattetheria mcp-proxy`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Package {
    pub registry_type: String,
    pub identifier: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub transport: PackageTransport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub package_arguments: Vec<PackageArgument>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageTransport {
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageArgument {
    #[serde(rename = "type")]
    pub kind: String,
    pub value: String,
}

/// `_meta["watt/connect"]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectMeta {
    pub status: ServiceStatus,
    #[serde(default)]
    pub category: Vec<String>,
    /// Absent for services that need no credential (e.g. local stdio servers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<CredentialSpec>,
    /// Any match means "the credential is no longer valid".
    #[serde(default)]
    pub auth_failure: Vec<ResponseMatcher>,
    /// Any match means "rate limited". HTTP 429 is always treated as rate limited.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rate_limited: Vec<ResponseMatcher>,
    /// Protocol versions known to work, newest first.
    #[serde(default)]
    pub protocol_versions: Vec<String>,
    pub modes: Modes,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub tools: Vec<ToolSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<Sandbox>,
    /// Short user-facing caveats (e.g. "orders are charged for real").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatus {
    Active,
    Deprecated,
    Revoked,
}

impl ServiceStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Deprecated => "deprecated",
            Self::Revoked => "revoked",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CredentialSpec {
    pub kind: CredentialKind,
    pub placement: CredentialPlacement,
    pub portal_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_hint_days: Option<u32>,
    /// Regexes whose matches are masked in logs and error messages.
    #[serde(default)]
    pub redact_patterns: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    StaticToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Modes {
    pub proxy: bool,
    pub direct: bool,
    /// Mode suggested when the user does not choose. Defaults to proxy when allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ConnectionMode>,
}

impl Modes {
    #[must_use]
    pub fn allows(self, mode: ConnectionMode) -> bool {
        match mode {
            ConnectionMode::Proxy => self.proxy,
            ConnectionMode::Direct => self.direct,
        }
    }

    #[must_use]
    pub fn preferred(self) -> ConnectionMode {
        match self.default {
            Some(mode) if self.allows(mode) => mode,
            _ if self.proxy => ConnectionMode::Proxy,
            _ => ConnectionMode::Direct,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    /// order tool -> preview tool that must succeed first in the same session.
    #[serde(default)]
    pub preview_before: BTreeMap<String, String>,
    /// Field names whose values are payment links in tool results.
    #[serde(default)]
    pub payment_link_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSummary {
    pub name: String,
    pub effect: ToolEffect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sandbox {
    pub url: String,
}

impl Catalog {
    #[must_use]
    pub fn get(&self, id: &ServiceId) -> Option<&ServerEntry> {
        self.services.iter().find(|entry| &entry.name == id)
    }

    /// Parse and structurally check a catalog snapshot.
    pub fn from_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

impl ServerEntry {
    #[must_use]
    pub fn connect(&self) -> &ConnectMeta {
        &self.meta.connect
    }

    #[must_use]
    pub fn display_title(&self) -> &str {
        self.title.as_deref().unwrap_or(self.name.as_str())
    }

    /// The streamable HTTP endpoint watt-connect proxies to.
    #[must_use]
    pub fn http_remote(&self) -> Option<&Remote> {
        self.remotes
            .iter()
            .find(|remote| remote.transport == STREAMABLE_HTTP)
    }

    /// Endpoint URL, or the sandbox endpoint when requested and available.
    #[must_use]
    pub fn endpoint_url(&self, sandbox: bool) -> Option<&str> {
        if sandbox && let Some(sandbox) = &self.connect().sandbox {
            return Some(sandbox.url.as_str());
        }
        self.http_remote().map(|remote| remote.url.as_str())
    }

    #[must_use]
    pub fn tool_effect(&self, tool: &str) -> Option<ToolEffect> {
        self.connect()
            .tools
            .iter()
            .find(|summary| summary.name == tool)
            .map(|summary| summary.effect)
    }

    #[must_use]
    pub fn portal_url(&self) -> Option<&str> {
        self.connect()
            .credential
            .as_ref()
            .map(|credential| credential.portal_url.as_str())
    }
}

/// Compare dotted numeric versions (`2026.10.29.1`). Non-numeric segments
/// compare as strings after all numeric ones.
#[must_use]
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let mut left = a.split(['.', '-']);
    let mut right = b.split(['.', '-']);
    loop {
        match (left.next(), right.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(x), Some(y)) => {
                let ordering = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    (Ok(_), Err(_)) => std::cmp::Ordering::Greater,
                    (Err(_), Ok(_)) => std::cmp::Ordering::Less,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if ordering != std::cmp::Ordering::Equal {
                    return ordering;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(
            compare_versions("2026.10.29.2", "2026.10.29.10"),
            Ordering::Less
        );
        assert_eq!(
            compare_versions("2026.10.30", "2026.10.29.9"),
            Ordering::Greater
        );
        assert_eq!(
            compare_versions("2026.10.29", "2026.10.29"),
            Ordering::Equal
        );
        assert_eq!(
            compare_versions("2026.10.29.1", "2026.10.29"),
            Ordering::Greater
        );
    }

    #[test]
    fn preferred_mode_respects_allowed_modes() {
        let modes = Modes {
            proxy: true,
            direct: true,
            default: Some(ConnectionMode::Direct),
        };
        assert_eq!(modes.preferred(), ConnectionMode::Direct);
        let modes = Modes {
            proxy: false,
            direct: true,
            default: Some(ConnectionMode::Proxy),
        };
        assert_eq!(modes.preferred(), ConnectionMode::Direct);
        let modes = Modes {
            proxy: true,
            direct: false,
            default: None,
        };
        assert_eq!(modes.preferred(), ConnectionMode::Proxy);
    }
}
