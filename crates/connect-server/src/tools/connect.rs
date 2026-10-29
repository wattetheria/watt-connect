//! `connect_service(service, mode?)`: returns instructions only. Tokens are
//! entered by the user in a terminal or a one-time local page, never through
//! the agent.

use super::{lookup, opt_str_arg};
use crate::ConnectServer;
use connect_catalog::ServiceStatus;
use connect_types::{ConnectError, ConnectErrorCode, ConnectionMode, CredentialStatus};
use rmcp::model::JsonObject;
use serde_json::{Value, json};

const DO_NOT_PASTE: &str = "Do not ask the user to paste the token into this conversation; it would end up in the model context and chat history.";

pub(super) fn run(server: &ConnectServer, args: &JsonObject) -> Result<Value, ConnectError> {
    let entry = lookup(server, args)?;
    let ctx = server.context();
    let meta = entry.connect();
    if meta.status == ServiceStatus::Revoked {
        return Err(ConnectError::new(
            ConnectErrorCode::ServiceRevoked,
            format!("{} has been revoked from the catalog", entry.name),
        ));
    }
    let mode = match opt_str_arg(args, "mode") {
        Some(raw) => raw
            .parse::<ConnectionMode>()
            .map_err(|error| ConnectError::new(ConnectErrorCode::InvalidArguments, error))?,
        None => meta.modes.preferred(),
    };
    if !meta.modes.allows(mode) {
        return Err(ConnectError::new(
            ConnectErrorCode::ModeNotAllowed,
            format!("{} does not support {mode} mode", entry.name),
        ));
    }

    if let Some(record) = ctx.connections.get(&entry.name)
        && record.mode == mode
        && matches!(
            record.credential_status,
            CredentialStatus::Ok | CredentialStatus::External
        )
    {
        return Ok(json!({
            "service": entry.name,
            "mode": mode.as_str(),
            "already_connected": true,
            "message": format!("{} is already connected in {mode} mode.", entry.display_title()),
        }));
    }

    match mode {
        ConnectionMode::Proxy => {
            let command = ctx.connect_command(&entry.name);
            let local_url = ctx
                .link_provider
                .as_ref()
                .and_then(|provider| provider.start(&entry.name));
            let mut steps = vec![format!(
                "Get a token from the merchant: {}",
                entry.portal_url().unwrap_or("(see the service notes)")
            )];
            match &local_url {
                Some(url) => steps.push(format!(
                    "Open {url} on this computer and paste the token there (one-time page), or run `{command}` in a terminal."
                )),
                None => steps.push(format!("Run `{command}` in a terminal and paste the token at the prompt.")),
            }
            steps.push("watt-connect verifies the token with the merchant before saving it on this computer.".into());
            steps.push(format!(
                "Then call describe_service(service=\"{}\") to see its tools.",
                entry.name
            ));
            Ok(json!({
                "service": entry.name,
                "mode": "proxy",
                "needs_user_action": true,
                "terminal_command": command,
                "local_url": local_url,
                "portal_url": entry.portal_url(),
                "steps": steps,
                "warning": DO_NOT_PASTE,
                "notes": meta.notes,
            }))
        }
        ConnectionMode::Direct => {
            let needs_token = meta.credential.is_some();
            let mut steps = Vec::new();
            if needs_token {
                steps.push(format!(
                    "Get a token from the merchant: {}",
                    entry.portal_url().unwrap_or("(see the service notes)")
                ));
                steps.push(format!(
                    "Run `{}` in a terminal (agent: claude-code, cursor or codex) and paste the token at the prompt.",
                    ctx.install_command(&entry.name, "<agent>")
                ));
            } else {
                steps.push(format!(
                    "Call install_service(service=\"{}\", agent=<claude-code|cursor|codex>); no token is needed.",
                    entry.name
                ));
            }
            steps.push("Restart the agent so it picks up the new MCP server.".into());
            Ok(json!({
                "service": entry.name,
                "mode": "direct",
                "needs_user_action": needs_token,
                "terminal_command": needs_token.then(|| ctx.install_command(&entry.name, "<agent>")),
                "portal_url": entry.portal_url(),
                "steps": steps,
                "warning": needs_token.then_some(DO_NOT_PASTE),
                "notes": meta.notes,
            }))
        }
    }
}
