//! Map whatever went wrong talking to a merchant onto one `ConnectErrorCode`,
//! using the catalog's per-merchant `auth_failure` and `rate_limited` rules.

use crate::negotiate::is_version_rejection;
use connect_types::{ConnectErrorCode, ObservedResponse, ResponseMatcher, matches_any};

/// JSON-RPC "method not found" etc. are ordinary upstream errors; the rules
/// below only look at what the catalog declares plus HTTP 429.
#[derive(Debug, Clone, Default)]
pub struct FailureRules {
    pub auth_failure: Vec<ResponseMatcher>,
    pub rate_limited: Vec<ResponseMatcher>,
}

#[must_use]
pub fn classify(observed: &ObservedResponse, rules: &FailureRules) -> ConnectErrorCode {
    if matches_any(&rules.auth_failure, observed) {
        return ConnectErrorCode::CredentialInvalid;
    }
    if observed.http_status == Some(429) || matches_any(&rules.rate_limited, observed) {
        return ConnectErrorCode::RateLimited;
    }
    if is_version_rejection(observed) {
        return ConnectErrorCode::ProtocolVersionRejected;
    }
    if observed.http_status.is_none() && observed.jsonrpc_error_code.is_none() {
        return ConnectErrorCode::UpstreamUnavailable;
    }
    ConnectErrorCode::UpstreamError
}

/// Short human-readable summary of a failed exchange (not yet redacted).
#[must_use]
pub fn describe(observed: &ObservedResponse) -> String {
    const MAX: usize = 300;
    let mut parts = Vec::new();
    if let Some(status) = observed.http_status {
        parts.push(format!("HTTP {status}"));
    }
    if let Some(code) = observed.jsonrpc_error_code {
        let message = observed.jsonrpc_error_message.as_deref().unwrap_or("");
        parts.push(format!("JSON-RPC {code} {message}").trim_end().to_owned());
    } else if let Some(body) = observed
        .body
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
    {
        let mut body: String = body.chars().take(MAX).collect();
        if body.len() < observed.body.as_deref().map_or(0, str::len) {
            body.push('…');
        }
        parts.push(body);
    }
    if parts.is_empty() {
        "no response".to_owned()
    } else {
        parts.join(": ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rules(auth: serde_json::Value, rate: serde_json::Value) -> FailureRules {
        FailureRules {
            auth_failure: serde_json::from_value(auth).unwrap(),
            rate_limited: serde_json::from_value(rate).unwrap(),
        }
    }

    #[test]
    fn rate_limit_and_unavailable() {
        let rules = rules(json!([]), json!([{ "jsonrpc_error_code": -32001 }]));
        let http429 = ObservedResponse {
            http_status: Some(429),
            ..ObservedResponse::default()
        };
        assert_eq!(classify(&http429, &rules), ConnectErrorCode::RateLimited);
        let rpc = ObservedResponse {
            jsonrpc_error_code: Some(-32001),
            ..ObservedResponse::default()
        };
        assert_eq!(classify(&rpc, &rules), ConnectErrorCode::RateLimited);
        assert_eq!(
            classify(&ObservedResponse::default(), &rules),
            ConnectErrorCode::UpstreamUnavailable
        );
        let other = ObservedResponse {
            http_status: Some(500),
            ..ObservedResponse::default()
        };
        assert_eq!(classify(&other, &rules), ConnectErrorCode::UpstreamError);
    }

    #[test]
    fn describe_truncates_long_bodies() {
        let observed = ObservedResponse {
            http_status: Some(500),
            body: Some("x".repeat(1000)),
            ..ObservedResponse::default()
        };
        let text = describe(&observed);
        assert!(text.starts_with("HTTP 500: xxx"));
        assert!(text.ends_with('…'));
        assert!(text.chars().count() < 320);
    }
}
