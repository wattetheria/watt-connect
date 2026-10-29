//! One lazily-created rmcp client session per merchant, reused across calls.

use crate::backoff::{BackoffPolicy, parse_retry_after};
use crate::classify::{FailureRules, classify, describe};
use crate::credential;
use crate::http::{ObservedSlot, ObservingClient};
use crate::negotiate::candidate_versions;
use crate::{UpstreamError, UpstreamTarget};
use connect_types::{ConnectErrorCode, ObservedResponse, ServiceId};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, ErrorData,
    Implementation, JsonObject, ProtocolVersion, Tool,
};
use rmcp::service::{ClientInitializeError, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{RoleClient, ServiceError, serve_client};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Upper bound for a single merchant call.
pub const CALL_TIMEOUT: Duration = Duration::from_mins(1);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// A live, initialized MCP session with one merchant.
pub struct UpstreamSession {
    service: RunningService<RoleClient, ClientConfig>,
    observed: ObservedSlot,
    rules: FailureRules,
    protocol_version: String,
}

impl UpstreamSession {
    /// Initialize a session, trying protocol versions newest first.
    pub async fn connect(target: &UpstreamTarget) -> Result<Self, UpstreamError> {
        let mut last_error = None;
        for version in candidate_versions(&target.protocol_versions) {
            match Self::connect_with_version(target, version.clone()).await {
                Ok(session) => return Ok(session),
                Err(error) => {
                    let retryable = matches!(
                        error.code,
                        ConnectErrorCode::ProtocolVersionRejected | ConnectErrorCode::UpstreamError
                    );
                    tracing::debug!(service = %target.service, version = %version, code = %error.code, "initialize failed");
                    if !retryable {
                        return Err(error);
                    }
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            UpstreamError::new(
                ConnectErrorCode::ProtocolVersionRejected,
                "no protocol version to try",
            )
        }))
    }

    async fn connect_with_version(
        target: &UpstreamTarget,
        version: ProtocolVersion,
    ) -> Result<Self, UpstreamError> {
        let endpoint = credential::apply(
            &target.url,
            target
                .credential
                .as_ref()
                .map(|(placement, token)| (placement, token.as_str())),
        )
        .map_err(|error| UpstreamError::new(ConnectErrorCode::Internal, error.to_string()))?;
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent(concat!("watt-connect/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| UpstreamError::new(ConnectErrorCode::Internal, error.to_string()))?;
        let client = ObservingClient::new(http, endpoint.headers);
        let observed = client.observed();
        let transport = StreamableHttpClientTransport::with_client(
            client,
            StreamableHttpClientTransportConfig::with_uri(endpoint.url.as_str()),
        );
        let info = ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("watt-connect", env!("CARGO_PKG_VERSION")),
        )
        .with_protocol_version(version);
        let rules = target.rules();
        let service = match tokio::time::timeout(CALL_TIMEOUT, serve_client(info, transport)).await
        {
            Ok(Ok(service)) => service,
            Ok(Err(error)) => return Err(init_error(&error, &observed, &rules)),
            Err(_) => {
                return Err(UpstreamError::new(
                    ConnectErrorCode::UpstreamUnavailable,
                    "timed out initializing the merchant session",
                ));
            }
        };
        let protocol_version = service
            .peer()
            .peer_info()
            .map(|info| info.protocol_version.to_string())
            .unwrap_or_default();
        Ok(Self {
            service,
            observed,
            rules,
            protocol_version,
        })
    }

    #[must_use]
    pub fn protocol_version(&self) -> &str {
        &self.protocol_version
    }

    pub async fn list_tools(&self) -> Result<Vec<Tool>, UpstreamError> {
        self.take_observed();
        match tokio::time::timeout(CALL_TIMEOUT, self.service.peer().list_all_tools()).await {
            Ok(Ok(tools)) => Ok(tools),
            Ok(Err(error)) => Err(self.service_error(&error)),
            Err(_) => Err(timeout_error()),
        }
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, UpstreamError> {
        self.take_observed();
        let mut params = CallToolRequestParams::new(name.to_owned());
        params.arguments = arguments;
        match tokio::time::timeout(CALL_TIMEOUT, self.service.peer().call_tool(params)).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(error)) => Err(self.service_error(&error)),
            Err(_) => Err(timeout_error()),
        }
    }

    pub async fn close(self) {
        let _ = self.service.cancel().await;
    }

    fn take_observed(&self) -> Option<ObservedResponse> {
        self.observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    fn service_error(&self, error: &ServiceError) -> UpstreamError {
        let observed = match error {
            ServiceError::McpError(data) => Some(rpc_observed(data)),
            _ => self.take_observed(),
        };
        from_observed(observed, error.to_string(), &self.rules)
    }
}

fn timeout_error() -> UpstreamError {
    UpstreamError::new(
        ConnectErrorCode::UpstreamUnavailable,
        "merchant did not answer in time",
    )
}

fn rpc_observed(data: &ErrorData) -> ObservedResponse {
    ObservedResponse {
        jsonrpc_error_code: Some(i64::from(data.code.0)),
        jsonrpc_error_message: Some(data.message.to_string()),
        ..ObservedResponse::default()
    }
}

fn init_error(
    error: &ClientInitializeError,
    slot: &ObservedSlot,
    rules: &FailureRules,
) -> UpstreamError {
    let recorded = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    let observed = match error {
        ClientInitializeError::JsonRpcError(data) => Some(rpc_observed(data)),
        ClientInitializeError::NoCompatibleProtocolVersion { .. } => {
            return UpstreamError::new(
                ConnectErrorCode::ProtocolVersionRejected,
                error.to_string(),
            );
        }
        _ => recorded,
    };
    from_observed(observed, error.to_string(), rules)
}

fn from_observed(
    observed: Option<ObservedResponse>,
    fallback: String,
    rules: &FailureRules,
) -> UpstreamError {
    match observed {
        Some(observed) => {
            let code = classify(&observed, rules);
            UpstreamError {
                code,
                message: describe(&observed),
                retry_after: observed.retry_after.as_deref().and_then(parse_retry_after),
                observed: Some(observed),
            }
        }
        None => UpstreamError::new(ConnectErrorCode::UpstreamUnavailable, fallback),
    }
}

struct PoolEntry {
    fingerprint: u64,
    session: Arc<UpstreamSession>,
}

/// Sessions keyed by service. A changed URL or token replaces the session.
pub struct UpstreamPool {
    sessions: Mutex<HashMap<ServiceId, PoolEntry>>,
    backoff: BackoffPolicy,
}

impl Default for UpstreamPool {
    fn default() -> Self {
        Self::new(BackoffPolicy::default())
    }
}

impl UpstreamPool {
    #[must_use]
    pub fn new(backoff: BackoffPolicy) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            backoff,
        }
    }

    pub async fn session(
        &self,
        target: &UpstreamTarget,
    ) -> Result<Arc<UpstreamSession>, UpstreamError> {
        let fingerprint = fingerprint(target);
        let mut sessions = self.sessions.lock().await;
        if let Some(entry) = sessions.get(&target.service)
            && entry.fingerprint == fingerprint
            && !entry.session.service.is_closed()
        {
            return Ok(entry.session.clone());
        }
        let session = Arc::new(UpstreamSession::connect(target).await?);
        sessions.insert(
            target.service.clone(),
            PoolEntry {
                fingerprint,
                session: session.clone(),
            },
        );
        Ok(session)
    }

    /// Drop the cached session (credential changed, disconnected, broken).
    pub async fn invalidate(&self, service: &ServiceId) {
        let removed = self.sessions.lock().await.remove(service);
        if let Some(entry) = removed
            && let Ok(session) = Arc::try_unwrap(entry.session)
        {
            session.close().await;
        }
    }

    pub async fn list_tools(&self, target: &UpstreamTarget) -> Result<Vec<Tool>, UpstreamError> {
        self.with_backoff(target, |session| async move { session.list_tools().await })
            .await
    }

    /// Call a merchant tool. Only rate-limited calls are retried: the merchant
    /// rejected them, so retrying cannot place an order twice.
    pub async fn call_tool(
        &self,
        target: &UpstreamTarget,
        name: &str,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, UpstreamError> {
        self.with_backoff(target, |session| {
            let arguments = arguments.clone();
            async move { session.call_tool(name, arguments).await }
        })
        .await
    }

    async fn with_backoff<T, F, Fut>(
        &self,
        target: &UpstreamTarget,
        op: F,
    ) -> Result<T, UpstreamError>
    where
        F: Fn(Arc<UpstreamSession>) -> Fut,
        Fut: Future<Output = Result<T, UpstreamError>>,
    {
        let mut attempt = 0;
        loop {
            let result = match self.session(target).await {
                Ok(session) => op(session).await,
                Err(error) => Err(error),
            };
            match result {
                Err(error)
                    if error.code == ConnectErrorCode::RateLimited
                        && attempt < self.backoff.max_retries =>
                {
                    let delay = self.backoff.delay(attempt, error.retry_after);
                    tracing::info!(service = %target.service, ?delay, "rate limited; backing off");
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
                Err(error) => {
                    if matches!(
                        error.code,
                        ConnectErrorCode::UpstreamUnavailable | ConnectErrorCode::CredentialInvalid
                    ) {
                        self.invalidate(&target.service).await;
                    }
                    return Err(error);
                }
                Ok(value) => return Ok(value),
            }
        }
    }
}

fn fingerprint(target: &UpstreamTarget) -> u64 {
    let mut hasher = DefaultHasher::new();
    target.url.hash(&mut hasher);
    if let Some((placement, token)) = &target.credential {
        placement.name.hash(&mut hasher);
        token.as_str().hash(&mut hasher);
    }
    hasher.finish()
}

/// Verify a credential: initialize, list tools, close. Used before a token is
/// stored, so a typo is caught immediately.
pub async fn probe(target: &UpstreamTarget) -> Result<Vec<Tool>, UpstreamError> {
    let session = UpstreamSession::connect(target).await?;
    let tools = session.list_tools().await;
    session.close().await;
    tools
}
