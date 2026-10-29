use serde::{Deserialize, Serialize};
use std::fmt;

/// Unified error codes. Every merchant-specific failure is mapped onto one of
/// these before it reaches the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConnectErrorCode {
    /// The merchant rejected the stored credential (matched an `auth_failure` rule).
    CredentialInvalid,
    /// No credential stored for this service.
    CredentialMissing,
    /// The merchant is rate limiting us; retried with backoff and still failing.
    RateLimited,
    /// An `order` tool was called without a successful preview in this session.
    PreviewRequired,
    /// The service id is not in the catalog.
    ServiceNotFound,
    /// The service is known but not connected.
    ServiceNotConnected,
    /// The catalog marked the service as revoked.
    ServiceRevoked,
    /// The requested mode is not allowed for this service, or the service is
    /// connected in the other mode.
    ModeNotAllowed,
    /// No protocol version offered by the catalog was accepted.
    ProtocolVersionRejected,
    /// The merchant could not be reached (DNS, TLS, timeout, ...).
    UpstreamUnavailable,
    /// The merchant returned an error that matched no known rule.
    UpstreamError,
    /// Arguments sent to a meta tool were invalid.
    InvalidArguments,
    /// Writing an agent's configuration failed.
    InstallFailed,
    /// Catalog could not be loaded or verified.
    CatalogUnavailable,
    /// Anything else.
    Internal,
}

impl ConnectErrorCode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CredentialInvalid => "CREDENTIAL_INVALID",
            Self::CredentialMissing => "CREDENTIAL_MISSING",
            Self::RateLimited => "RATE_LIMITED",
            Self::PreviewRequired => "PREVIEW_REQUIRED",
            Self::ServiceNotFound => "SERVICE_NOT_FOUND",
            Self::ServiceNotConnected => "SERVICE_NOT_CONNECTED",
            Self::ServiceRevoked => "SERVICE_REVOKED",
            Self::ModeNotAllowed => "MODE_NOT_ALLOWED",
            Self::ProtocolVersionRejected => "PROTOCOL_VERSION_REJECTED",
            Self::UpstreamUnavailable => "UPSTREAM_UNAVAILABLE",
            Self::UpstreamError => "UPSTREAM_ERROR",
            Self::InvalidArguments => "INVALID_ARGUMENTS",
            Self::InstallFailed => "INSTALL_FAILED",
            Self::CatalogUnavailable => "CATALOG_UNAVAILABLE",
            Self::Internal => "INTERNAL",
        }
    }
}

impl fmt::Display for ConnectErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An error as reported to the agent. `message` must already be redacted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ConnectError {
    pub code: ConnectErrorCode,
    pub message: String,
    /// Where the user can obtain a new credential.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portal_url: Option<String>,
    /// What the agent should do next, in plain language.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl ConnectError {
    pub fn new(code: ConnectErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            portal_url: None,
            hint: None,
        }
    }

    #[must_use]
    pub fn with_portal(mut self, portal_url: Option<String>) -> Self {
        self.portal_url = portal_url;
        self
    }

    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}
