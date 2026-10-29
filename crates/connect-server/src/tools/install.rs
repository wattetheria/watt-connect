//! `install_service(service, agent)`: direct mode.

use super::{lookup, str_arg};
use crate::ConnectServer;
use crate::connections::ConnectionRecord;
use crate::target::{direct_entry, direct_mode_exposes_url_credential};
use connect_catalog::{ServerEntry, ServiceStatus};
use connect_installer::{AgentKind, InstallReport, installer};
use connect_types::{ConnectError, ConnectErrorCode, ConnectionMode, CredentialStatus};
use connect_vault::Secret;
use rmcp::model::JsonObject;
use serde_json::{Value, json};

pub(super) async fn run(server: &ConnectServer, args: &JsonObject) -> Result<Value, ConnectError> {
    let entry = lookup(server, args)?;
    let agent: AgentKind = str_arg(args, "agent")?
        .parse()
        .map_err(|error: String| ConnectError::new(ConnectErrorCode::InvalidArguments, error))?;
    let ctx = server.context();
    let sandbox = ctx
        .connections
        .get(&entry.name)
        .is_some_and(|record| record.sandbox);

    let token = if entry.connect().credential.is_some() {
        match ctx.token(&entry.name).await? {
            Some(token) => Some(token),
            None => {
                return Ok(json!({
                    "service": entry.name,
                    "agent": agent.as_str(),
                    "installed": false,
                    "needs_user_action": true,
                    "terminal_command": ctx.install_command(&entry.name, agent.as_str()),
                    "message": "This service needs the user's token. Ask the user to run the terminal command; \
                                it prompts for the token locally. Do not ask for the token in chat.",
                    "portal_url": entry.portal_url(),
                }));
            }
        }
    } else {
        None
    };
    let report = install(server, &entry, agent, sandbox, token.as_ref()).await?;
    Ok(report_json(&entry, &report))
}

/// Shared with the CLI `install` command.
pub async fn install(
    server: &ConnectServer,
    entry: &ServerEntry,
    agent: AgentKind,
    sandbox: bool,
    token: Option<&Secret>,
) -> Result<InstallReport, ConnectError> {
    let ctx = server.context();
    let meta = entry.connect();
    if meta.status == ServiceStatus::Revoked {
        return Err(ConnectError::new(
            ConnectErrorCode::ServiceRevoked,
            format!("{} has been revoked", entry.name),
        ));
    }
    if !meta.modes.direct {
        return Err(ConnectError::new(
            ConnectErrorCode::ModeNotAllowed,
            format!("{} does not allow direct mode", entry.name),
        ));
    }
    let direct = direct_entry(entry, sandbox, token)?;
    let env = ctx.installer_env.clone();
    let report = tokio::task::spawn_blocking(move || installer(agent, &env).install(&direct))
        .await
        .map_err(|error| crate::internal(error.to_string()))?
        .map_err(|error| {
            ConnectError::new(
                ConnectErrorCode::InstallFailed,
                connect_vault::redact::redact(&error.to_string()),
            )
        })?;

    let mut agents = ctx
        .connections
        .get(&entry.name)
        .filter(|record| record.mode == ConnectionMode::Direct)
        .map(|record| record.agents)
        .unwrap_or_default();
    if !agents.iter().any(|existing| existing == agent.as_str()) {
        agents.push(agent.as_str().to_owned());
    }
    ctx.connections
        .upsert(ConnectionRecord {
            service: entry.name.clone(),
            mode: ConnectionMode::Direct,
            connected_at: now_rfc3339(),
            credential_status: if meta.credential.is_some() {
                CredentialStatus::External
            } else {
                CredentialStatus::Ok
            },
            sandbox,
            agents,
        })
        .map_err(crate::internal)?;
    // The agent manages the token from now on; watt-connect keeps no copy.
    ctx.delete_token(&entry.name).await?;
    ctx.upstream.invalidate(&entry.name).await;
    Ok(report)
}

fn report_json(entry: &ServerEntry, report: &InstallReport) -> Value {
    let mut warnings = Vec::new();
    if direct_mode_exposes_url_credential(entry) {
        warnings.push(
            "The token is part of the server URL and is stored in plain text in the agent's config.".to_owned(),
        );
    }
    json!({
        "service": entry.name,
        "agent": report.agent.as_str(),
        "installed": true,
        "server_name": report.name,
        "location": report.location,
        "backup": report.backup,
        "warnings": warnings,
        "next": "Restart the agent so it loads the new MCP server.",
    })
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
