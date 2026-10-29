//! Shared types for watt-connect. This crate performs no IO.
//!
//! Everything that more than one crate needs to agree on lives here: service
//! identifiers, tool effects, connection modes, the unified error codes that
//! merchant-specific failures are mapped onto, and the declarative response
//! matchers the catalog uses to describe those failures.

mod error;
mod matcher;

pub use error::{ConnectError, ConnectErrorCode};
pub use matcher::{ObservedResponse, ResponseMatcher, matches_any};

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Catalog identifier of a merchant MCP service, for example `cn.lkcoffee/order`.
///
/// The format follows the MCP registry `server.json` naming convention:
/// a reverse-DNS namespace, a slash, and a server name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ServiceId(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid service id `{0}`: expected `<reverse-dns>/<name>`, e.g. `cn.lkcoffee/order`")]
pub struct InvalidServiceId(pub String);

impl ServiceId {
    pub fn parse(value: &str) -> Result<Self, InvalidServiceId> {
        value.parse()
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Namespace part, e.g. `cn.lkcoffee`.
    #[must_use]
    pub fn namespace(&self) -> &str {
        self.0.split_once('/').map_or(&self.0, |(ns, _)| ns)
    }

    /// Server name part, e.g. `order`.
    #[must_use]
    pub fn name(&self) -> &str {
        self.0.split_once('/').map_or("", |(_, name)| name)
    }

    /// A short, config-friendly slug such as `lkcoffee-order`, used as the
    /// server name when writing an agent's own MCP configuration.
    #[must_use]
    pub fn slug(&self) -> String {
        let ns = self.namespace();
        let brand = ns.rsplit('.').next().unwrap_or(ns);
        format!("{brand}-{}", self.name())
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect()
    }
}

impl FromStr for ServiceId {
    type Err = InvalidServiceId;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidServiceId(value.to_owned());
        let (ns, name) = value.split_once('/').ok_or_else(invalid)?;
        let ns_ok = !ns.is_empty()
            && ns.contains('.')
            && ns
                .split('.')
                .all(|label| !label.is_empty() && label.chars().all(is_id_char));
        let name_ok = !name.is_empty() && name.chars().all(|c| is_id_char(c) || c == '.');
        if ns_ok && name_ok {
            Ok(Self(value.to_owned()))
        } else {
            Err(invalid())
        }
    }
}

fn is_id_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'
}

impl TryFrom<String> for ServiceId {
    type Error = InvalidServiceId;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<ServiceId> for String {
    fn from(value: ServiceId) -> Self {
        value.0
    }
}

impl fmt::Display for ServiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What calling a merchant tool does. Only a summary: the authoritative
/// schema always comes from the merchant's live `tools/list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolEffect {
    /// Queries only.
    Read,
    /// Changes state but does not place an order (cancel, edit cart, ...).
    Write,
    /// Places an order or otherwise commits money. Gated by preview policy.
    Order,
}

impl ToolEffect {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Order => "order",
        }
    }
}

/// How a connected service is reached by the user's agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionMode {
    /// agent -> watt-connect (local) -> merchant. Token lives in watt-connect's vault.
    Proxy,
    /// agent -> merchant. Token lives in the agent's own configuration.
    Direct,
}

impl ConnectionMode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proxy => "proxy",
            Self::Direct => "direct",
        }
    }
}

impl FromStr for ConnectionMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "proxy" => Ok(Self::Proxy),
            "direct" => Ok(Self::Direct),
            other => Err(format!(
                "unknown connection mode `{other}` (expected proxy|direct)"
            )),
        }
    }
}

impl fmt::Display for ConnectionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a static credential is placed on each request to the merchant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialPlacement {
    #[serde(rename = "in")]
    pub location: PlacementLocation,
    /// Header name or query parameter name.
    pub name: String,
    /// Value template; `{token}` is replaced with the secret. Defaults to `{token}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementLocation {
    Header,
    Query,
}

impl CredentialPlacement {
    pub const TOKEN_PLACEHOLDER: &'static str = "{token}";

    /// Render the value that carries `token`.
    #[must_use]
    pub fn render(&self, token: &str) -> String {
        self.format
            .as_deref()
            .unwrap_or(Self::TOKEN_PLACEHOLDER)
            .replace(Self::TOKEN_PLACEHOLDER, token)
    }
}

/// Known state of the credential for a proxied service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStatus {
    /// Stored and last known to work.
    Ok,
    /// No credential stored.
    #[default]
    Missing,
    /// The merchant rejected it; the user must obtain a new one.
    CredentialInvalid,
    /// Managed by the agent (direct mode), not by watt-connect.
    External,
}

impl CredentialStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Missing => "missing",
            Self::CredentialInvalid => "credential_invalid",
            Self::External => "external",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_id_parses_registry_style_names() {
        let id: ServiceId = "cn.lkcoffee/order".parse().unwrap();
        assert_eq!(id.namespace(), "cn.lkcoffee");
        assert_eq!(id.name(), "order");
        assert_eq!(id.slug(), "lkcoffee-order");
        assert_eq!(
            ServiceId::parse("cn.didichuxing/ride").unwrap().slug(),
            "didichuxing-ride"
        );
    }

    #[test]
    fn service_id_rejects_malformed_names() {
        for bad in [
            "",
            "order",
            "cn/order",
            "cn.lk/",
            "/order",
            "CN.lk/order",
            "cn..lk/x",
        ] {
            assert!(ServiceId::parse(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn placement_renders_template() {
        let header = CredentialPlacement {
            location: PlacementLocation::Header,
            name: "Authorization".into(),
            format: Some("Bearer {token}".into()),
        };
        assert_eq!(header.render("abc"), "Bearer abc");
        let query = CredentialPlacement {
            location: PlacementLocation::Query,
            name: "key".into(),
            format: None,
        };
        assert_eq!(query.render("abc"), "abc");
    }
}
