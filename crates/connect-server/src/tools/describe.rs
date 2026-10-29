//! `describe_service(service)`.

use super::{lookup, modes_json};
use crate::ConnectServer;
use crate::target::proxy_target;
use connect_catalog::ServiceStatus;
use connect_types::{ConnectError, ConnectErrorCode, ConnectionMode, CredentialStatus, ToolEffect};
use rmcp::model::JsonObject;
use serde_json::{Value, json};

pub(super) async fn run(server: &ConnectServer, args: &JsonObject) -> Result<Value, ConnectError> {
    let entry = lookup(server, args)?;
    let ctx = server.context();
    let meta = entry.connect();
    let record = ctx.connections.get(&entry.name);

    let mut out = json!({
        "service": entry.name,
        "title": entry.display_title(),
        "description": entry.description,
        "status": meta.status.as_str(),
        "category": meta.category,
        "modes": modes_json(&entry),
        "notes": meta.notes,
        "credential_portal": entry.portal_url(),
        "policy": {
            "preview_before": meta.policy.preview_before,
            "rule": "Order tools need a successful call to their preview tool in this session, then the user's explicit confirmation. Payment is always done by the user personally."
        },
        "connection": record.as_ref().map(|record| json!({
            "mode": record.mode.as_str(),
            "credential_status": record.credential_status.as_str(),
            "sandbox": record.sandbox,
        })),
    });

    let proxied = record.as_ref().filter(|record| {
        record.mode == ConnectionMode::Proxy
            && meta.modes.proxy
            && meta.status != ServiceStatus::Revoked
    });
    if let Some(record) = proxied {
        let token = ctx.token(&entry.name).await?;
        let live = match (&meta.credential, token) {
            (Some(_), None) => Err(ConnectError::new(
                ConnectErrorCode::CredentialMissing,
                "no token stored for this service",
            )),
            (_, token) => {
                let target = proxy_target(&entry, record.sandbox, token.as_ref())?;
                ctx.upstream.list_tools(&target).await.map_err(|error| {
                    if error.code == ConnectErrorCode::CredentialInvalid {
                        let _ = ctx.connections.set_credential_status(
                            &entry.name,
                            CredentialStatus::CredentialInvalid,
                        );
                    }
                    ConnectError::new(error.code, connect_vault::redact::redact(&error.message))
                })
            }
        };
        match live {
            Ok(tools) => {
                let tools: Vec<Value> = tools
                    .into_iter()
                    .map(|tool| {
                        let effect = entry.tool_effect(&tool.name).map(ToolEffect::as_str);
                        let preview = meta.policy.preview_before.get(tool.name.as_ref());
                        json!({
                            "name": tool.name,
                            "title": tool.title,
                            "description": tool.description,
                            "inputSchema": tool.input_schema,
                            "annotations": tool.annotations,
                            "effect": effect,
                            "requires_preview": preview,
                        })
                    })
                    .collect();
                out["tools"] = Value::Array(tools);
                out["tools_source"] = json!("live");
                out["how_to_call"] = json!(format!(
                    "call_service(service=\"{}\", tool=<name>, arguments=<object matching inputSchema>)",
                    entry.name
                ));
                return Ok(out);
            }
            Err(error) => {
                out["live_tools_error"] = serde_json::to_value(&error).unwrap_or(Value::Null);
            }
        }
    }

    out["tools"] = serde_json::to_value(&meta.tools).unwrap_or(Value::Null);
    out["tools_source"] = json!("catalog_summary");
    if record.is_none() {
        out["how_to_connect"] = json!(format!(
            "Call connect_service(service=\"{}\") and follow its instructions.",
            entry.name
        ));
    }
    Ok(out)
}
