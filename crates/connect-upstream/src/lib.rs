//! Connections from watt-connect to merchant MCP servers over Streamable HTTP.
//!
//! This crate only knows "service + credential + request". It knows nothing
//! about the meta tools exposed to agents.

pub mod backoff;
pub mod classify;
pub mod credential;
pub mod http;
pub mod negotiate;
pub mod pool;

pub use backoff::BackoffPolicy;
pub use classify::FailureRules;
pub use pool::{UpstreamPool, UpstreamSession, probe};
pub use rmcp::model::{CallToolResult, JsonObject, Tool};

use connect_types::{
    ConnectErrorCode, CredentialPlacement, ObservedResponse, ResponseMatcher, ServiceId,
};
use std::time::Duration;
use zeroize::Zeroizing;

/// Everything needed to reach one merchant.
pub struct UpstreamTarget {
    pub service: ServiceId,
    pub url: String,
    pub credential: Option<(CredentialPlacement, Zeroizing<String>)>,
    /// Newest first.
    pub protocol_versions: Vec<String>,
    pub auth_failure: Vec<ResponseMatcher>,
    pub rate_limited: Vec<ResponseMatcher>,
}

impl UpstreamTarget {
    #[must_use]
    pub fn rules(&self) -> FailureRules {
        FailureRules {
            auth_failure: self.auth_failure.clone(),
            rate_limited: self.rate_limited.clone(),
        }
    }
}

/// A classified failure. `message` is not redacted; callers must redact
/// before showing or logging it.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{code}: {message}")]
pub struct UpstreamError {
    pub code: ConnectErrorCode,
    pub message: String,
    pub observed: Option<ObservedResponse>,
    pub retry_after: Option<Duration>,
}

impl UpstreamError {
    pub fn new(code: ConnectErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            observed: None,
            retry_after: None,
        }
    }
}
