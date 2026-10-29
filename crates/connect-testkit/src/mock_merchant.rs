//! A configurable mock merchant MCP server written with axum.

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Where the mock expects the credential.
#[derive(Debug, Clone)]
pub enum CredentialCheck {
    None,
    Header { name: String, prefix: String },
    Query { name: String },
}

/// How the mock reports a missing or invalid credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthFailureStyle {
    /// Missing: `200 text/plain Missing KEY parameter`; invalid: `200` JSON-RPC `-32002`.
    Didi,
    /// `401` JSON with `WWW-Authenticate` (missing) / `401` JSON (invalid).
    Luckin,
    /// Missing: `403 {"code":"0001"}`; invalid: `401 text/plain`.
    Mcd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitStyle {
    Http429,
    JsonRpc(i64),
}

#[derive(Debug, Clone)]
pub struct MockTool {
    pub name: String,
    /// Returned as `structuredContent` and as JSON text content.
    pub result: Value,
}

impl MockTool {
    pub fn new(name: &str, result: Value) -> Self {
        Self {
            name: name.to_owned(),
            result,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MockConfig {
    pub name: String,
    pub credential: CredentialCheck,
    pub valid_token: Option<String>,
    pub auth_style: AuthFailureStyle,
    /// Empty: accept any version and echo it.
    pub accepted_versions: Vec<String>,
    pub rate_limit_style: RateLimitStyle,
    pub tools: Vec<MockTool>,
    /// Answer requests as SSE streams instead of JSON bodies.
    pub sse: bool,
}

#[derive(Default)]
struct Counters {
    sessions: AtomicU64,
    rate_limit_remaining: AtomicU32,
}

struct Inner {
    config: Mutex<MockConfig>,
    counters: Counters,
    calls: Mutex<Vec<String>>,
    requests: Mutex<Vec<Value>>,
}

type Shared = Arc<Inner>;

/// A running mock. The server stops when the tokio runtime shuts down.
#[derive(Clone)]
pub struct MockMerchant {
    addr: SocketAddr,
    inner: Shared,
}

impl MockMerchant {
    pub async fn start(config: MockConfig) -> Self {
        let inner = Arc::new(Inner {
            config: Mutex::new(config),
            counters: Counters::default(),
            calls: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
        });
        let app = Router::new()
            .route(
                "/mcp",
                post(handle_post).get(handle_get).delete(handle_delete),
            )
            .with_state(inner.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock merchant");
        let addr = listener.local_addr().expect("mock address");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self { addr, inner }
    }

    #[must_use]
    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }

    /// Names of tools successfully called, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<String> {
        lock(&self.inner.calls).clone()
    }

    /// Every JSON-RPC request body received.
    #[must_use]
    pub fn requests(&self) -> Vec<Value> {
        lock(&self.inner.requests).clone()
    }

    /// Simulate the merchant expiring or rotating the token.
    pub fn set_valid_token(&self, token: Option<&str>) {
        lock(&self.inner.config).valid_token = token.map(str::to_owned);
    }

    /// Rate limit the next `count` tool calls.
    pub fn rate_limit_next(&self, count: u32) {
        self.inner
            .counters
            .rate_limit_remaining
            .store(count, Ordering::SeqCst);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

enum CredentialState {
    Ok,
    Missing,
    Invalid,
}

fn check_credential(
    config: &MockConfig,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> CredentialState {
    let presented = match &config.credential {
        CredentialCheck::None => return CredentialState::Ok,
        CredentialCheck::Header { name, prefix } => headers
            .get(name.as_str())
            .and_then(|value| value.to_str().ok())
            .map(|value| {
                value
                    .strip_prefix(prefix.as_str())
                    .unwrap_or(value)
                    .to_owned()
            }),
        CredentialCheck::Query { name } => query.get(name).cloned(),
    };
    match (presented, &config.valid_token) {
        (None, _) => CredentialState::Missing,
        (Some(token), Some(valid)) if &token == valid => CredentialState::Ok,
        (Some(_), _) => CredentialState::Invalid,
    }
}

fn auth_failure(style: AuthFailureStyle, state: &CredentialState, id: &Value) -> Response {
    let missing = matches!(state, CredentialState::Missing);
    match (style, missing) {
        (AuthFailureStyle::Didi, true) => {
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/plain")], "Missing KEY parameter").into_response()
        }
        (AuthFailureStyle::Didi, false) => json_response(
            StatusCode::OK,
            &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32002, "message": "auth failed"}}),
            None,
        ),
        (AuthFailureStyle::Luckin, true) => (
            StatusCode::UNAUTHORIZED,
            [
                (header::CONTENT_TYPE, "application/json"),
                (
                    header::WWW_AUTHENTICATE,
                    "Bearer resource_metadata=\"https://gwmcp.lkcoffee.com/.well-known/oauth-protected-resource\"",
                ),
            ],
            r#"{"message":"oauth token is not found"}"#,
        )
            .into_response(),
        (AuthFailureStyle::Luckin, false) => (
            StatusCode::UNAUTHORIZED,
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"message":"访问令牌无效或已过期"}"#,
        )
            .into_response(),
        (AuthFailureStyle::Mcd, true) => (
            StatusCode::FORBIDDEN,
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"code":"0001","msg":"系统错误"}"#,
        )
            .into_response(),
        (AuthFailureStyle::Mcd, false) => (
            StatusCode::UNAUTHORIZED,
            [(header::CONTENT_TYPE, "text/plain;charset=UTF-8")],
            "当前鉴权码不存在或已失效!",
        )
            .into_response(),
    }
}

fn json_response(status: StatusCode, body: &Value, session: Option<&str>) -> Response {
    let mut response = (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response();
    if let Some(session) = session
        && let Ok(value) = HeaderValue::from_str(session)
    {
        response.headers_mut().insert("mcp-session-id", value);
    }
    response
}

fn reply(config: &MockConfig, body: &Value, session: Option<&str>) -> Response {
    if !config.sse {
        return json_response(StatusCode::OK, body, session);
    }
    let payload = format!("event: message\ndata: {body}\n\n");
    let mut response = (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/event-stream")],
        payload,
    )
        .into_response();
    if let Some(session) = session
        && let Ok(value) = HeaderValue::from_str(session)
    {
        response.headers_mut().insert("mcp-session-id", value);
    }
    response
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

async fn handle_post(
    State(inner): State<Shared>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let config = lock(&inner.config).clone();
    let Ok(request) = serde_json::from_slice::<Value>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    lock(&inner.requests).push(request.clone());
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();

    let state = check_credential(&config, &headers, &query);
    if !matches!(state, CredentialState::Ok) {
        return auth_failure(config.auth_style, &state, &id);
    }
    if request.get("id").is_none() {
        // Notification (e.g. notifications/initialized).
        return StatusCode::ACCEPTED.into_response();
    }

    match method {
        "initialize" => {
            let requested = request
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if !config.accepted_versions.is_empty()
                && !config.accepted_versions.contains(&requested)
            {
                return json_response(
                    StatusCode::OK,
                    &rpc_error(
                        &id,
                        -32602,
                        &format!("Unsupported protocol version: {requested}"),
                    ),
                    None,
                );
            }
            let session = format!(
                "mock-session-{}",
                inner.counters.sessions.fetch_add(1, Ordering::SeqCst) + 1
            );
            let result = json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": requested,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": config.name, "version": "0.0.0-mock"}
                }
            });
            reply(&config, &result, Some(&session))
        }
        "ping" => reply(
            &config,
            &json!({"jsonrpc": "2.0", "id": id, "result": {}}),
            None,
        ),
        "tools/list" => {
            let tools: Vec<Value> = config
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "name": tool.name,
                        "description": format!("mock {}", tool.name),
                        "inputSchema": {"type": "object", "properties": {}}
                    })
                })
                .collect();
            reply(
                &config,
                &json!({"jsonrpc": "2.0", "id": id, "result": {"tools": tools}}),
                None,
            )
        }
        "tools/call" => {
            let limited = inner
                .counters
                .rate_limit_remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok();
            if limited {
                return match config.rate_limit_style {
                    RateLimitStyle::Http429 => (
                        StatusCode::TOO_MANY_REQUESTS,
                        [(header::RETRY_AFTER, "0")],
                        "slow down",
                    )
                        .into_response(),
                    RateLimitStyle::JsonRpc(code) => {
                        json_response(StatusCode::OK, &rpc_error(&id, code, "rate limited"), None)
                    }
                };
            }
            let name = request
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let Some(tool) = config.tools.iter().find(|tool| tool.name == name) else {
                return reply(
                    &config,
                    &rpc_error(&id, -32602, &format!("unknown tool {name}")),
                    None,
                );
            };
            lock(&inner.calls).push(tool.name.clone());
            let result = json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{"type": "text", "text": tool.result.to_string()}],
                    "structuredContent": tool.result,
                    "isError": false
                }
            });
            reply(&config, &result, None)
        }
        other => reply(
            &config,
            &rpc_error(&id, -32601, &format!("method not found: {other}")),
            None,
        ),
    }
}

async fn handle_get() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

async fn handle_delete() -> StatusCode {
    StatusCode::OK
}
