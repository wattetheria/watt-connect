//! Turn a catalog entry (+ token) into an upstream target or a direct-mode
//! agent config entry.

use connect_catalog::ServerEntry;
use connect_installer::{DirectEntry, DirectTransport};
use connect_types::{ConnectError, ConnectErrorCode, PlacementLocation};
use connect_upstream::UpstreamTarget;
use connect_vault::Secret;
use std::collections::BTreeMap;
use zeroize::Zeroizing;

/// Register the entry's redact patterns with the process-wide redactor.
pub fn register_redaction(entry: &ServerEntry) {
    if let Some(credential) = &entry.connect().credential {
        connect_vault::redact::register_patterns(
            credential.redact_patterns.iter().map(String::as_str),
        );
    }
}

pub fn proxy_target(
    entry: &ServerEntry,
    sandbox: bool,
    token: Option<&Secret>,
) -> Result<UpstreamTarget, ConnectError> {
    register_redaction(entry);
    let meta = entry.connect();
    let url = entry.endpoint_url(sandbox).ok_or_else(|| {
        ConnectError::new(
            ConnectErrorCode::ModeNotAllowed,
            format!(
                "{} has no streamable-http endpoint and cannot be proxied",
                entry.name
            ),
        )
    })?;
    let credential = match (&meta.credential, token) {
        (Some(spec), Some(token)) => Some((
            spec.placement.clone(),
            Zeroizing::new(token.expose().to_owned()),
        )),
        _ => None,
    };
    Ok(UpstreamTarget {
        service: entry.name.clone(),
        url: url.to_owned(),
        credential,
        protocol_versions: meta.protocol_versions.clone(),
        auth_failure: meta.auth_failure.clone(),
        rate_limited: meta.rate_limited.clone(),
    })
}

/// Agent config entry for direct mode. Remote services need `token` when the
/// catalog declares a credential.
pub fn direct_entry(
    entry: &ServerEntry,
    sandbox: bool,
    token: Option<&Secret>,
) -> Result<DirectEntry, ConnectError> {
    let name = entry.name.slug();
    if let Some(url) = entry.endpoint_url(sandbox) {
        let mut url = url.to_owned();
        let mut headers = BTreeMap::new();
        if let Some(spec) = &entry.connect().credential {
            let token = token.ok_or_else(|| {
                ConnectError::new(
                    ConnectErrorCode::CredentialMissing,
                    "a token is required for direct mode",
                )
            })?;
            let value = spec.placement.render(token.expose());
            match spec.placement.location {
                PlacementLocation::Header => {
                    headers.insert(spec.placement.name.clone(), value);
                }
                PlacementLocation::Query => {
                    let mut parsed = url::Url::parse(&url).map_err(|error| {
                        ConnectError::new(ConnectErrorCode::Internal, error.to_string())
                    })?;
                    parsed
                        .query_pairs_mut()
                        .append_pair(&spec.placement.name, &value);
                    url = parsed.into();
                }
            }
        }
        return Ok(DirectEntry {
            name,
            transport: DirectTransport::Http { url, headers },
        });
    }
    let package = entry
        .packages
        .iter()
        .find(|package| package.registry_type == "npm" && package.transport.kind == "stdio")
        .ok_or_else(|| {
            ConnectError::new(
                ConnectErrorCode::ModeNotAllowed,
                format!(
                    "{} has no endpoint or npm stdio package to install",
                    entry.name
                ),
            )
        })?;
    let spec = match &package.version {
        Some(version) => format!("{}@{version}", package.identifier),
        None => package.identifier.clone(),
    };
    let mut args = vec!["-y".to_owned(), spec];
    args.extend(
        package
            .package_arguments
            .iter()
            .map(|argument| argument.value.clone()),
    );
    Ok(DirectEntry {
        name,
        transport: DirectTransport::Stdio {
            command: package.runtime_hint.clone().unwrap_or_else(|| "npx".into()),
            args,
            env: BTreeMap::new(),
        },
    })
}

/// Whether installing this entry in direct mode writes the credential into
/// the agent's config as part of a URL (visible in plain text).
#[must_use]
pub fn direct_mode_exposes_url_credential(entry: &ServerEntry) -> bool {
    entry
        .connect()
        .credential
        .as_ref()
        .is_some_and(|spec| spec.placement.location == PlacementLocation::Query)
}

#[cfg(test)]
mod tests {
    use super::*;
    use connect_catalog::bundled::bundled_catalog;
    use connect_types::ServiceId;

    fn entry(id: &str) -> ServerEntry {
        bundled_catalog()
            .get(&ServiceId::parse(id).unwrap())
            .unwrap()
            .clone()
    }

    #[test]
    fn direct_entries_for_each_shape() {
        let token = Secret::new("tok");
        let luckin = direct_entry(&entry("cn.lkcoffee/order"), false, Some(&token)).unwrap();
        assert_eq!(luckin.name, "lkcoffee-order");
        assert!(matches!(
            &luckin.transport,
            DirectTransport::Http { headers, .. } if headers["Authorization"] == "Bearer tok"
        ));

        let didi = direct_entry(&entry("cn.didichuxing/ride"), true, Some(&token)).unwrap();
        assert!(matches!(
            &didi.transport,
            DirectTransport::Http { url, headers } if url == "https://mcp.didichuxing.com/mcp-servers-sandbox?key=tok" && headers.is_empty()
        ));
        assert!(direct_mode_exposes_url_credential(&entry(
            "cn.didichuxing/ride"
        )));

        let node = direct_entry(&entry("io.wattetheria/node"), false, None).unwrap();
        assert_eq!(node.name, "wattetheria-node");
        assert!(matches!(
            &node.transport,
            DirectTransport::Stdio { command, args, .. } if command == "npx" && args == &["-y", "wattetheria", "mcp-proxy"]
        ));

        assert!(direct_entry(&entry("cn.mcd/order"), false, None).is_err());
    }
}
