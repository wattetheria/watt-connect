//! Meta tool definitions and dispatch.

mod call;
mod connect;
mod describe;
mod install;
mod list;
mod search;

pub(crate) use install::install as install_direct;
pub use install::now_rfc3339;

use crate::ConnectServer;
use connect_catalog::ServerEntry;
use connect_types::{ConnectError, ConnectErrorCode, ServiceId};
use rmcp::model::{CallToolResult, JsonObject, Tool, ToolAnnotations};
use serde_json::{Value, json};
use std::sync::Arc;

pub const SEARCH_SERVICES: &str = "search_services";
pub const DESCRIBE_SERVICE: &str = "describe_service";
pub const CONNECT_SERVICE: &str = "connect_service";
pub const CALL_SERVICE: &str = "call_service";
pub const INSTALL_SERVICE: &str = "install_service";
pub const LIST_CONNECTIONS: &str = "list_connections";

/// What a meta tool produced.
pub enum Output {
    /// Structured JSON, returned as `structuredContent` plus text.
    Json(Value),
    /// A merchant result passed through (possibly annotated).
    Passthrough(CallToolResult),
}

fn schema(value: Value) -> Arc<JsonObject> {
    match value {
        Value::Object(map) => Arc::new(map),
        _ => unreachable!("tool schemas are objects"),
    }
}

/// The six tools, always the same regardless of how many merchants exist.
#[must_use]
pub fn definitions() -> Vec<Tool> {
    let service = json!({
        "type": "string",
        "description": "Catalog service id, e.g. \"cn.lkcoffee/order\". Use search_services to find ids."
    });
    vec![
        Tool::new(
            SEARCH_SERVICES,
            "Search the watt-connect catalog of merchant MCP services (coffee, food, rides, ...). \
             Returns ids, descriptions, and whether each service is already connected.",
            schema(json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Free text, e.g. \"咖啡\" or \"taxi\". Empty lists everything."},
                    "category": {"type": "string", "description": "Optional category filter, e.g. \"food\"."}
                }
            })),
        )
        .annotate(ToolAnnotations::new().read_only(true).open_world(false)),
        Tool::new(
            DESCRIBE_SERVICE,
            "Describe one service. If it is connected in proxy mode, returns the merchant's live tool list \
             with full input schemas (use them with call_service); otherwise returns the catalog summary and \
             how to connect.",
            schema(json!({
                "type": "object",
                "properties": {"service": service},
                "required": ["service"]
            })),
        )
        .annotate(ToolAnnotations::new().read_only(true)),
        Tool::new(
            CONNECT_SERVICE,
            "Start connecting a service. Returns instructions for the USER to enter their token in a terminal \
             or a local web page. Never ask the user to paste a token into the chat.",
            schema(json!({
                "type": "object",
                "properties": {
                    "service": service,
                    "mode": {"type": "string", "enum": ["proxy", "direct"], "description": "Defaults to the service's preferred mode."}
                },
                "required": ["service"]
            })),
        )
        .annotate(ToolAnnotations::new().read_only(true).open_world(false)),
        Tool::new(
            CALL_SERVICE,
            "Call a tool of a connected merchant (proxy mode). Get tool names and argument schemas from \
             describe_service. Order tools require a successful preview call first and the user's explicit \
             confirmation; payment links are returned for the user to pay personally.",
            schema(json!({
                "type": "object",
                "properties": {
                    "service": service,
                    "tool": {"type": "string", "description": "Merchant tool name, e.g. \"previewOrder\"."},
                    "arguments": {"type": "object", "description": "Arguments for the merchant tool.", "additionalProperties": true}
                },
                "required": ["service", "tool"]
            })),
        )
        .annotate(ToolAnnotations::new().destructive(true).open_world(true)),
        Tool::new(
            INSTALL_SERVICE,
            "Direct mode: write the service into an agent's own MCP configuration so the agent talks to the \
             merchant directly. The agent may need a restart afterwards.",
            schema(json!({
                "type": "object",
                "properties": {
                    "service": service,
                    "agent": {"type": "string", "enum": ["claude-code", "cursor", "codex"]}
                },
                "required": ["service", "agent"]
            })),
        )
        .annotate(ToolAnnotations::new().destructive(false).idempotent(true)),
        Tool::new(
            LIST_CONNECTIONS,
            "List connected services, their mode (proxy/direct) and credential status.",
            schema(json!({"type": "object", "properties": {}})),
        )
        .annotate(ToolAnnotations::new().read_only(true).open_world(false)),
    ]
}

pub(crate) async fn dispatch(
    server: &ConnectServer,
    name: &str,
    args: &JsonObject,
) -> Result<Output, ConnectError> {
    if args.keys().any(|key| {
        matches!(
            key.as_str(),
            "token" | "key" | "secret" | "password" | "api_key"
        )
    }) {
        return Err(ConnectError::new(
            ConnectErrorCode::InvalidArguments,
            "watt-connect never accepts credentials through the agent",
        )
        .with_hint("Ask the user to run the terminal command from connect_service instead; tokens must not enter the conversation."));
    }
    match name {
        SEARCH_SERVICES => Ok(Output::Json(search::run(server, args))),
        DESCRIBE_SERVICE => describe::run(server, args).await.map(Output::Json),
        CONNECT_SERVICE => connect::run(server, args).map(Output::Json),
        CALL_SERVICE => call::run(server, args).await,
        INSTALL_SERVICE => install::run(server, args).await.map(Output::Json),
        LIST_CONNECTIONS => list::run(server).await.map(Output::Json),
        other => Err(ConnectError::new(
            ConnectErrorCode::InvalidArguments,
            format!("unknown tool `{other}`"),
        )),
    }
}

pub(crate) fn str_arg<'a>(args: &'a JsonObject, name: &str) -> Result<&'a str, ConnectError> {
    args.get(name).and_then(Value::as_str).ok_or_else(|| {
        ConnectError::new(
            ConnectErrorCode::InvalidArguments,
            format!("`{name}` (string) is required"),
        )
    })
}

pub(crate) fn opt_str_arg<'a>(args: &'a JsonObject, name: &str) -> Option<&'a str> {
    args.get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

/// Look the service up in the current catalog.
pub(crate) fn lookup(
    server: &ConnectServer,
    args: &JsonObject,
) -> Result<ServerEntry, ConnectError> {
    let raw = str_arg(args, "service")?;
    let id = ServiceId::parse(raw).map_err(|error| {
        ConnectError::new(ConnectErrorCode::InvalidArguments, error.to_string())
    })?;
    server
        .context()
        .catalog
        .current()
        .catalog
        .get(&id)
        .cloned()
        .ok_or_else(|| {
            ConnectError::new(
                ConnectErrorCode::ServiceNotFound,
                format!("`{id}` is not in the catalog"),
            )
            .with_hint("Use search_services to find available services.")
        })
}

pub(crate) fn modes_json(entry: &ServerEntry) -> Value {
    let modes = entry.connect().modes;
    json!({"proxy": modes.proxy, "direct": modes.direct, "preferred": modes.preferred().as_str()})
}
