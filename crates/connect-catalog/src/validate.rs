//! Semantic validation of catalog entries, run by CI before signing and by
//! watt-connect when loading a catalog.

use crate::schema::{CATALOG_SCHEMA_VERSION, Catalog, ServerEntry, ServiceStatus};
use connect_types::{CredentialPlacement, PlacementLocation, ToolEffect};
use std::collections::BTreeSet;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The catalog must not be published or used.
    Error,
    /// Suspicious but usable.
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    /// Service name, or `catalog` for snapshot-level issues.
    pub subject: String,
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{level}: {}: {}", self.subject, self.message)
    }
}

#[must_use]
pub fn has_errors(issues: &[Issue]) -> bool {
    issues.iter().any(|issue| issue.severity == Severity::Error)
}

#[must_use]
pub fn validate_catalog(catalog: &Catalog) -> Vec<Issue> {
    let mut issues = Vec::new();
    if catalog.schema_version != CATALOG_SCHEMA_VERSION {
        issues.push(Issue {
            severity: Severity::Error,
            subject: "catalog".into(),
            message: format!(
                "unsupported schema_version {} (expected {CATALOG_SCHEMA_VERSION})",
                catalog.schema_version
            ),
        });
    }
    if catalog.version.trim().is_empty() {
        issues.push(Issue {
            severity: Severity::Error,
            subject: "catalog".into(),
            message: "version must not be empty".into(),
        });
    }
    let mut seen = BTreeSet::new();
    for entry in &catalog.services {
        if !seen.insert(entry.name.clone()) {
            issues.push(Issue {
                severity: Severity::Error,
                subject: entry.name.to_string(),
                message: "duplicate service name".into(),
            });
        }
        issues.extend(validate_entry(entry));
    }
    issues
}

#[must_use]
pub fn validate_entry(entry: &ServerEntry) -> Vec<Issue> {
    let mut issues = Vec::new();
    let mut push = |severity, message: String| {
        issues.push(Issue {
            severity,
            subject: entry.name.to_string(),
            message,
        });
    };
    let meta = entry.connect();

    if entry.description.trim().is_empty() {
        push(Severity::Error, "description must not be empty".into());
    }
    if !meta.modes.proxy && !meta.modes.direct {
        push(
            Severity::Error,
            "at least one of modes.proxy / modes.direct must be true".into(),
        );
    }
    if let Some(default) = meta.modes.default
        && !meta.modes.allows(default)
    {
        push(
            Severity::Error,
            format!("modes.default `{default}` is not an allowed mode"),
        );
    }

    match entry.http_remote() {
        Some(remote) => check_url(&remote.url, "remotes[].url", &mut push),
        None if meta.modes.proxy => push(
            Severity::Error,
            "proxy mode requires a `streamable-http` remote".into(),
        ),
        None if entry.packages.is_empty() => push(
            Severity::Error,
            "entry has neither a streamable-http remote nor a package".into(),
        ),
        None => {}
    }
    if let Some(sandbox) = &meta.sandbox {
        check_url(&sandbox.url, "sandbox.url", &mut push);
    }

    match &meta.credential {
        Some(credential) => {
            check_placement(&credential.placement, &mut push);
            check_url(&credential.portal_url, "credential.portal_url", &mut push);
            for pattern in &credential.redact_patterns {
                if let Err(error) = regex::Regex::new(pattern) {
                    push(
                        Severity::Error,
                        format!("redact pattern `{pattern}` is invalid: {error}"),
                    );
                }
            }
            if credential.placement.location == PlacementLocation::Query
                && credential.redact_patterns.is_empty()
            {
                push(
                    Severity::Error,
                    "query-placed credentials need a redact pattern so URLs are masked in logs"
                        .into(),
                );
            }
            if meta.auth_failure.is_empty() {
                push(
                    Severity::Error,
                    "auth_failure must describe how this merchant reports a bad credential".into(),
                );
            }
        }
        None if entry.http_remote().is_some() => push(
            Severity::Warning,
            "remote service without credential spec; calls will be unauthenticated".into(),
        ),
        None => {}
    }
    for (index, matcher) in meta
        .auth_failure
        .iter()
        .chain(&meta.rate_limited)
        .enumerate()
    {
        if matcher.is_empty() {
            push(
                Severity::Error,
                format!("response matcher #{index} has no conditions"),
            );
        }
        if matcher.equals.is_some() && matcher.json_path.is_none() {
            push(
                Severity::Error,
                format!("response matcher #{index} uses `equals` without `json_path`"),
            );
        }
    }

    if meta.modes.proxy && meta.protocol_versions.is_empty() {
        push(
            Severity::Warning,
            "protocol_versions is empty; only the SDK default will be tried".into(),
        );
    }

    let mut tool_names = BTreeSet::new();
    for tool in &meta.tools {
        if !tool_names.insert(tool.name.as_str()) {
            push(
                Severity::Error,
                format!("tool `{}` listed twice", tool.name),
            );
        }
    }
    for (order_tool, preview_tool) in &meta.policy.preview_before {
        match entry.tool_effect(order_tool) {
            Some(ToolEffect::Order) => {}
            Some(other) => push(
                Severity::Error,
                format!(
                    "preview_before key `{order_tool}` has effect `{}`, expected `order`",
                    other.as_str()
                ),
            ),
            None => push(
                Severity::Error,
                format!("preview_before key `{order_tool}` is not in tools"),
            ),
        }
        match entry.tool_effect(preview_tool) {
            Some(ToolEffect::Read) => {}
            Some(other) => push(
                Severity::Error,
                format!(
                    "preview tool `{preview_tool}` has effect `{}`, expected `read`",
                    other.as_str()
                ),
            ),
            None => push(
                Severity::Error,
                format!("preview tool `{preview_tool}` is not in tools"),
            ),
        }
    }
    for tool in &meta.tools {
        if tool.effect == ToolEffect::Order && !meta.policy.preview_before.contains_key(&tool.name)
        {
            push(
                Severity::Error,
                format!("order tool `{}` has no preview_before entry", tool.name),
            );
        }
    }
    if meta.status == ServiceStatus::Revoked && meta.modes.proxy {
        push(
            Severity::Warning,
            "revoked service still allows proxy mode; it will not be proxied".into(),
        );
    }
    issues
}

fn check_url(raw: &str, field: &str, push: &mut impl FnMut(Severity, String)) {
    match url::Url::parse(raw) {
        Ok(parsed) if parsed.scheme() == "https" => {}
        // Local development and test merchants.
        Ok(parsed) if parsed.scheme() == "http" && is_loopback(&parsed) => {}
        Ok(parsed) => push(
            Severity::Error,
            format!("{field} must use https, got `{}`", parsed.scheme()),
        ),
        Err(error) => push(
            Severity::Error,
            format!("{field} `{raw}` is not a URL: {error}"),
        ),
    }
}

fn is_loopback(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(domain)) => domain == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

fn check_placement(placement: &CredentialPlacement, push: &mut impl FnMut(Severity, String)) {
    if placement.name.trim().is_empty() {
        push(
            Severity::Error,
            "credential.placement.name must not be empty".into(),
        );
    }
    if let Some(format) = &placement.format
        && !format.contains(CredentialPlacement::TOKEN_PLACEHOLDER)
    {
        push(
            Severity::Error,
            "credential.placement.format must contain `{token}`".into(),
        );
    }
    if placement.location == PlacementLocation::Header
        && http::HeaderName::from_bytes(placement.name.as_bytes()).is_err()
    {
        push(
            Severity::Error,
            format!("`{}` is not a valid header name", placement.name),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_must_be_https_except_loopback() {
        let errors = |raw: &str| {
            let mut issues = Vec::new();
            check_url(raw, "u", &mut |_: Severity, message: String| {
                issues.push(message);
            });
            issues.len()
        };
        assert_eq!(errors("https://mcp.mcd.cn"), 0);
        assert_eq!(errors("http://127.0.0.1:8080/mcp"), 0);
        assert_eq!(errors("http://localhost/mcp"), 0);
        assert_eq!(errors("http://mcp.mcd.cn"), 1);
    }
}
