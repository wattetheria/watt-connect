//! `call_service(service, tool, arguments)`: proxy one merchant tool call
//! through the preview gate, then pass payment links through untouched.

use super::{Output, lookup, str_arg};
use crate::target::proxy_target;
use crate::{ConnectServer, now_unix};
use connect_catalog::{ServerEntry, ServiceStatus};
use connect_policy::{GateDecision, ToolPolicy, extract_payment_links, payment_notice};
use connect_types::{ConnectError, ConnectErrorCode, ConnectionMode, CredentialStatus, ToolEffect};
use rmcp::model::{ContentBlock, JsonObject};
use serde_json::{Value, json};
use std::time::Instant;

pub(super) async fn run(server: &ConnectServer, args: &JsonObject) -> Result<Output, ConnectError> {
    let entry = lookup(server, args)?;
    let tool = str_arg(args, "tool")?.to_owned();
    let arguments = match args.get("arguments") {
        None | Some(Value::Null) => None,
        Some(Value::Object(map)) => Some(map.clone()),
        Some(_) => {
            return Err(ConnectError::new(
                ConnectErrorCode::InvalidArguments,
                "`arguments` must be an object",
            ));
        }
    };
    let ctx = server.context();
    let meta = entry.connect();
    let id = entry.name.clone();

    if meta.status == ServiceStatus::Revoked {
        return Err(ConnectError::new(
            ConnectErrorCode::ServiceRevoked,
            format!("{id} has been revoked from the catalog; watt-connect no longer proxies it"),
        ));
    }
    if !meta.modes.proxy {
        return Err(ConnectError::new(
            ConnectErrorCode::ModeNotAllowed,
            format!("{id} can only be used in direct mode"),
        )
        .with_hint(format!(
            "Use install_service(service=\"{id}\", agent=...) instead."
        )));
    }
    let Some(record) = ctx.connections.get(&id) else {
        return Err(ConnectError::new(
            ConnectErrorCode::ServiceNotConnected,
            format!("{id} is not connected"),
        )
        .with_hint(format!("Call connect_service(service=\"{id}\") first.")));
    };
    if record.mode == ConnectionMode::Direct {
        return Err(ConnectError::new(
            ConnectErrorCode::ModeNotAllowed,
            format!("{id} is connected in direct mode; call its tools directly in the agent"),
        ));
    }
    if record.credential_status == CredentialStatus::CredentialInvalid {
        return Err(credential_invalid(server, &entry));
    }
    let token = ctx.token(&id).await?;
    if meta.credential.is_some() && token.is_none() {
        return Err(ConnectError::new(
            ConnectErrorCode::CredentialMissing,
            format!("no token stored for {id}"),
        )
        .with_portal(entry.portal_url().map(str::to_owned))
        .with_hint(format!(
            "Ask the user to run `{}`.",
            ctx.connect_command(&id)
        )));
    }

    let effect = entry.tool_effect(&tool);
    let preview_tool = meta.policy.preview_before.get(&tool).map(String::as_str);
    let now = now_unix();
    let decision = {
        let session = server.policy();
        connect_policy::check(
            &session,
            id.as_str(),
            ToolPolicy {
                effect,
                preview_tool,
            },
            now,
        )
    };
    if let GateDecision::RequirePreview { preview_tool } = decision {
        return Err(ConnectError::new(
            ConnectErrorCode::PreviewRequired,
            format!("`{tool}` places an order and needs a successful `{preview_tool}` call first"),
        )
        .with_hint(format!(
            "Call call_service(service=\"{id}\", tool=\"{preview_tool}\", ...) with the same items, show the user the price, \
             get their explicit confirmation, then call `{tool}` again."
        )));
    }

    let target = proxy_target(&entry, record.sandbox, token.as_ref())?;
    let started = Instant::now();
    let result = ctx.upstream.call_tool(&target, &tool, arguments).await;
    let elapsed_ms = started.elapsed().as_millis();
    let mut result = match result {
        Ok(result) => result,
        Err(error) => {
            tracing::info!(service = %id, %tool, elapsed_ms, outcome = error.code.as_str(), "call_service");
            if error.code == ConnectErrorCode::CredentialInvalid {
                return Err(credential_invalid(server, &entry));
            }
            return Err(ConnectError::new(
                error.code,
                connect_vault::redact::redact(&error.message),
            ));
        }
    };
    let failed = result.is_error == Some(true);
    tracing::info!(
        service = %id,
        %tool,
        elapsed_ms,
        outcome = if failed { "tool_error" } else { "ok" },
        "call_service"
    );
    if record.credential_status != CredentialStatus::Ok {
        let _ = ctx
            .connections
            .set_credential_status(&id, CredentialStatus::Ok);
    }
    if failed {
        return Ok(Output::Passthrough(result));
    }

    let is_preview = meta
        .policy
        .preview_before
        .values()
        .any(|preview| preview == &tool);
    let is_order = effect == Some(ToolEffect::Order) || preview_tool.is_some();
    {
        let mut session = server.policy();
        if is_preview {
            session.record_preview(id.as_str(), &tool, now);
        }
        if let Some(preview) = preview_tool {
            session.consume_preview(id.as_str(), preview);
        }
    }
    if is_order {
        let as_json = serde_json::to_value(&result).unwrap_or(Value::Null);
        let links = extract_payment_links(&as_json, &meta.policy.payment_link_fields);
        let notice = payment_notice(&links);
        // Clients that receive `structuredContent` show it instead of the text
        // blocks, so the notice has to travel inside it as well.
        if let Some(Value::Object(structured)) = result.structured_content.as_mut() {
            structured.insert(
                "watt_connect".into(),
                json!({ "payment_notice": notice, "payment_links": links }),
            );
        }
        result.content.push(ContentBlock::text(notice));
    }
    Ok(Output::Passthrough(result))
}

/// Mark the credential invalid and build the fixed "get a new token" error.
fn credential_invalid(server: &ConnectServer, entry: &ServerEntry) -> ConnectError {
    let ctx = server.context();
    let _ = ctx
        .connections
        .set_credential_status(&entry.name, CredentialStatus::CredentialInvalid);
    ConnectError::new(
        ConnectErrorCode::CredentialInvalid,
        format!(
            "{} rejected the stored token; it has expired or been revoked",
            entry.display_title()
        ),
    )
    .with_portal(entry.portal_url().map(str::to_owned))
    .with_hint(format!(
        "Ask the user to get a new token from the portal and run `{}`.",
        ctx.connect_command(&entry.name)
    ))
}
