//! HTTP backend for rmcp's Streamable HTTP client.
//!
//! rmcp keeps owning the MCP protocol (session ids, SSE, JSON-RPC
//! correlation). This backend exists because merchants report a bad
//! credential in incompatible ways (HTTP 200 `text/plain`, 401, 403 with a
//! JSON body, ...). The stock reqwest backend discards most of that, so this
//! one records the raw status, content type and (bounded) body of every
//! failed exchange for [`crate::classify`].

use bytes::Bytes;
use connect_types::ObservedResponse;
use futures::StreamExt;
use futures::stream::BoxStream;
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use rmcp::model::{ClientJsonRpcMessage, JsonRpcMessage, ServerJsonRpcMessage};
use rmcp::transport::streamable_http_client::{
    SseError, StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
};
use sse_stream::{Sse, SseStream};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const HEADER_SESSION_ID: &str = "Mcp-Session-Id";
const HEADER_LAST_EVENT_ID: &str = "Last-Event-Id";
const EVENT_STREAM: &str = "text/event-stream";
const JSON: &str = "application/json";
/// Bodies of failed responses are kept up to this size for classification.
const MAX_FAILURE_BODY: usize = 16 * 1024;

/// Last failed exchange seen by a client, shared with its owner.
pub type ObservedSlot = Arc<Mutex<Option<ObservedResponse>>>;

#[derive(Debug, thiserror::Error)]
pub enum HttpFailure {
    #[error("request failed: {0}")]
    Request(#[source] reqwest::Error),
    #[error("merchant answered HTTP {status}")]
    Status { status: u16 },
    #[error("merchant answered with unexpected content type {0:?}")]
    ContentType(Option<String>),
}

#[derive(Clone)]
pub struct ObservingClient {
    http: reqwest::Client,
    /// Credential headers, applied to every request.
    headers: HeaderMap,
    observed: ObservedSlot,
}

impl ObservingClient {
    #[must_use]
    pub fn new(http: reqwest::Client, headers: HeaderMap) -> Self {
        Self {
            http,
            headers,
            observed: Arc::new(Mutex::new(None)),
        }
    }

    #[must_use]
    pub fn observed(&self) -> ObservedSlot {
        self.observed.clone()
    }

    fn record(&self, observed: ObservedResponse) {
        *self
            .observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(observed);
    }

    fn request(
        &self,
        builder: reqwest::RequestBuilder,
        session_id: Option<&str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> reqwest::RequestBuilder {
        let mut builder = builder.headers(self.headers.clone());
        if let Some(auth) = auth_header {
            builder = builder.bearer_auth(auth);
        }
        for (name, value) in custom_headers {
            builder = builder.header(name, value);
        }
        if let Some(session_id) = session_id {
            builder = builder.header(HEADER_SESSION_ID, session_id);
        }
        builder
    }

    /// Read a failed response, remember it, and turn it into an error.
    async fn fail(&self, response: reqwest::Response) -> StreamableHttpError<HttpFailure> {
        let status = response.status().as_u16();
        let content_type = content_type(response.headers());
        let retry_after = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = read_bounded(response).await;
        let jsonrpc = body
            .as_deref()
            .and_then(|body| serde_json::from_str::<ServerJsonRpcMessage>(body).ok())
            .and_then(|message| match message {
                JsonRpcMessage::Error(error) => Some(error.error),
                _ => None,
            });
        self.record(ObservedResponse {
            http_status: Some(status),
            content_type: content_type.clone(),
            body,
            jsonrpc_error_code: jsonrpc.as_ref().map(|error| i64::from(error.code.0)),
            jsonrpc_error_message: jsonrpc.map(|error| error.message.into_owned()),
            retry_after,
        });
        if (200..300).contains(&status) {
            StreamableHttpError::Client(HttpFailure::ContentType(content_type))
        } else {
            StreamableHttpError::Client(HttpFailure::Status { status })
        }
    }
}

fn content_type(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::CONTENT_TYPE)
        .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
}

fn session_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get(HEADER_SESSION_ID)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

async fn read_bounded(response: reqwest::Response) -> Option<String> {
    let bytes = response.bytes().await.ok()?;
    let slice = &bytes[..bytes.len().min(MAX_FAILURE_BODY)];
    Some(String::from_utf8_lossy(slice).into_owned())
}

fn request_error(error: reqwest::Error) -> StreamableHttpError<HttpFailure> {
    // Drop the URL: it may carry a query-placed credential.
    StreamableHttpError::Client(HttpFailure::Request(error.without_url()))
}

fn sse(response: reqwest::Response) -> BoxStream<'static, Result<Sse, SseError>> {
    SseStream::from_bytes_stream(response.bytes_stream()).boxed()
}

fn is_reply_or_notification(message: &ClientJsonRpcMessage) -> bool {
    matches!(
        message,
        JsonRpcMessage::Notification(_) | JsonRpcMessage::Response(_) | JsonRpcMessage::Error(_)
    )
}

impl StreamableHttpClient for ObservingClient {
    type Error = HttpFailure;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        let builder = self
            .http
            .post(uri.as_ref())
            .header(header::ACCEPT, format!("{JSON}, {EVENT_STREAM}"));
        let response = self
            .request(builder, session_id.as_deref(), auth_header, custom_headers)
            .json(&message)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if matches!(status, StatusCode::ACCEPTED | StatusCode::NO_CONTENT) {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        if status == StatusCode::NOT_FOUND && session_id.is_some() {
            return Err(StreamableHttpError::SessionExpired);
        }
        if !status.is_success() {
            return Err(self.fail(response).await);
        }
        let new_session = self::session_id(response.headers());
        let content_type = content_type(response.headers()).unwrap_or_default();
        if content_type.starts_with(EVENT_STREAM) {
            return Ok(StreamableHttpPostResponse::Sse(sse(response), new_session));
        }
        if content_type.starts_with(JSON) {
            let bytes: Bytes = response.bytes().await.map_err(request_error)?;
            if bytes.is_empty() && is_reply_or_notification(&message) {
                return Ok(StreamableHttpPostResponse::Accepted);
            }
            return match serde_json::from_slice::<ServerJsonRpcMessage>(&bytes) {
                Ok(parsed) => Ok(StreamableHttpPostResponse::Json(parsed, new_session)),
                Err(_) if is_reply_or_notification(&message) => {
                    Ok(StreamableHttpPostResponse::Accepted)
                }
                Err(error) => {
                    self.record(ObservedResponse {
                        http_status: Some(status.as_u16()),
                        content_type: Some(content_type),
                        body: Some(
                            String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_FAILURE_BODY)])
                                .into_owned(),
                        ),
                        ..ObservedResponse::default()
                    });
                    Err(StreamableHttpError::Deserialize(error))
                }
            };
        }
        if response.content_length() == Some(0) && is_reply_or_notification(&message) {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        // e.g. Didi's `200 text/plain: Missing KEY parameter`.
        Err(self.fail(response).await)
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        let builder = self.http.delete(uri.as_ref());
        let response = self
            .request(builder, Some(&session_id), auth_header, custom_headers)
            .send()
            .await
            .map_err(request_error)?;
        if response.status().is_success() || response.status() == StatusCode::METHOD_NOT_ALLOWED {
            Ok(())
        } else {
            Err(StreamableHttpError::Client(HttpFailure::Status {
                status: response.status().as_u16(),
            }))
        }
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        let mut builder = self
            .http
            .get(uri.as_ref())
            .header(header::ACCEPT, EVENT_STREAM);
        if let Some(last_event_id) = last_event_id {
            builder = builder.header(HEADER_LAST_EVENT_ID, last_event_id);
        }
        let response = self
            .request(builder, session_id.as_deref(), auth_header, custom_headers)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if status == StatusCode::METHOD_NOT_ALLOWED {
            return Err(StreamableHttpError::ServerDoesNotSupportSse);
        }
        if !status.is_success() {
            return Err(self.fail(response).await);
        }
        match content_type(response.headers()) {
            Some(ct) if ct.starts_with(EVENT_STREAM) => Ok(sse(response)),
            other => Err(StreamableHttpError::UnexpectedContentType(other)),
        }
    }
}
