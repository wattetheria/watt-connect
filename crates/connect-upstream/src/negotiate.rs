//! Protocol version negotiation.
//!
//! `initialize` is first sent with the newest version this client supports
//! that still has an `initialize` handshake; the server answers with the
//! version it will use. If the server errors instead, the catalog's
//! `protocol_versions` are tried from newest to oldest.

use connect_types::ObservedResponse;
use rmcp::model::ProtocolVersion;

/// Versions to try, in order, without duplicates. Versions without an
/// `initialize` handshake are skipped: merchants are reached through the
/// session-based lifecycle.
#[must_use]
pub fn candidate_versions(catalog_versions: &[String]) -> Vec<ProtocolVersion> {
    let mut out: Vec<ProtocolVersion> = vec![ProtocolVersion::LATEST_WITH_INITIALIZE];
    for raw in catalog_versions {
        let Ok(version) =
            serde_json::from_value::<ProtocolVersion>(serde_json::Value::String(raw.clone()))
        else {
            continue;
        };
        if version.has_initialize() && !out.contains(&version) {
            out.push(version);
        }
    }
    out
}

/// Whether a failed `initialize` looks like the server refused the version.
#[must_use]
pub fn is_version_rejection(observed: &ObservedResponse) -> bool {
    let mentions_version = |text: &str| {
        let text = text.to_ascii_lowercase();
        text.contains("protocol version")
            || text.contains("protocolversion")
            || text.contains("unsupported version")
    };
    if observed.jsonrpc_error_code == Some(-32602)
        && observed
            .jsonrpc_error_message
            .as_deref()
            .is_some_and(|message| message.to_ascii_lowercase().contains("version"))
    {
        return true;
    }
    observed.http_status == Some(400) && observed.body.as_deref().is_some_and(mentions_version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_start_with_latest_and_deduplicate() {
        let versions = candidate_versions(&[
            "2026-07-28".into(),
            "2025-06-18".into(),
            "2025-11-25".into(),
            "2025-03-26".into(),
        ]);
        let names: Vec<&str> = versions.iter().map(ProtocolVersion::as_str).collect();
        assert_eq!(names, vec!["2025-11-25", "2025-06-18", "2025-03-26"]);
    }

    #[test]
    fn detects_version_rejection() {
        let rpc = ObservedResponse {
            jsonrpc_error_code: Some(-32602),
            jsonrpc_error_message: Some("Unsupported protocol version: 2025-11-25".into()),
            ..ObservedResponse::default()
        };
        assert!(is_version_rejection(&rpc));
        let http = ObservedResponse {
            http_status: Some(400),
            body: Some("Bad Request: Unsupported protocol version".into()),
            ..ObservedResponse::default()
        };
        assert!(is_version_rejection(&http));
        let other = ObservedResponse {
            jsonrpc_error_code: Some(-32602),
            jsonrpc_error_message: Some("missing argument".into()),
            ..ObservedResponse::default()
        };
        assert!(!is_version_rejection(&other));
    }
}
