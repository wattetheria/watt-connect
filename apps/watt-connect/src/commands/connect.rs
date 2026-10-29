//! `watt-connect connect <service>`: the only way a proxy-mode token gets in.

use super::{lookup, read_token};
use crate::cli::ConnectArgs;
use anyhow::{Result, bail};
use connect_catalog::{ServerEntry, ServiceStatus};
use connect_server::{ConnectContext, ConnectionRecord, target::proxy_target, tools::now_rfc3339};
use connect_types::{ConnectError, ConnectErrorCode, ConnectionMode, CredentialStatus};
use connect_vault::Secret;
use std::fmt::Write as _;
use std::sync::Arc;

pub async fn run(context: &Arc<ConnectContext>, args: ConnectArgs) -> Result<()> {
    let entry = lookup(context, &args.service)?;
    let meta = entry.connect();
    if meta.status == ServiceStatus::Revoked {
        bail!("{} has been revoked from the catalog", entry.name);
    }
    if !meta.modes.proxy {
        bail!(
            "{} only supports direct mode; run `watt-connect install {} --agent <claude-code|cursor|codex>`",
            entry.name,
            entry.name
        );
    }
    if args.sandbox && meta.sandbox.is_none() {
        bail!("{} has no sandbox endpoint", entry.name);
    }
    for note in &meta.notes {
        eprintln!("note: {note}");
    }
    if args.web {
        let url = crate::connect_web::run_page(
            context.clone(),
            entry,
            args.sandbox,
            !args.token.no_verify,
        )
        .await?;
        eprintln!("Saved. ({url} is now closed.)");
        return Ok(());
    }
    let token = if meta.credential.is_some() {
        Some(read_token(&entry, args.token.token_stdin)?)
    } else {
        None
    };
    let tools = verify_and_store(
        context,
        &entry,
        token.as_ref(),
        args.sandbox,
        !args.token.no_verify,
    )
    .await
    .map_err(|error| anyhow::anyhow!(render_error(&error)))?;
    match tools {
        Some(count) => println!(
            "Connected {} ({}) in proxy mode: token verified, {count} tools available.",
            entry.display_title(),
            entry.name
        ),
        None => println!(
            "Connected {} ({}) in proxy mode (token not verified).",
            entry.display_title(),
            entry.name
        ),
    }
    Ok(())
}

/// Verify the token with `initialize` + `tools/list`, then store it and
/// record the connection. Returns the number of tools when verified.
pub async fn verify_and_store(
    context: &ConnectContext,
    entry: &ServerEntry,
    token: Option<&Secret>,
    sandbox: bool,
    verify: bool,
) -> Result<Option<usize>, ConnectError> {
    let tools = if verify {
        let target = proxy_target(entry, sandbox, token)?;
        let tools = connect_upstream::probe(&target).await.map_err(|error| {
            let message = connect_vault::redact::redact(&error.message);
            match error.code {
                ConnectErrorCode::CredentialInvalid => ConnectError::new(
                    error.code,
                    format!("the merchant rejected this token ({message})"),
                )
                .with_portal(entry.portal_url().map(str::to_owned)),
                code => ConnectError::new(code, format!("could not verify the token: {message}"))
                    .with_hint("Retry later, or pass --no-verify to store it anyway."),
            }
        })?;
        Some(tools.len())
    } else {
        None
    };
    if let Some(token) = token {
        let vault = context.vault.clone();
        let id = entry.name.clone();
        let token = token.clone();
        tokio::task::spawn_blocking(move || vault.set(&id, &token))
            .await
            .map_err(|error| ConnectError::new(ConnectErrorCode::Internal, error.to_string()))?
            .map_err(|error| {
                ConnectError::new(
                    ConnectErrorCode::Internal,
                    format!("credential store: {error}"),
                )
            })?;
    }
    context
        .connections
        .upsert(ConnectionRecord {
            service: entry.name.clone(),
            mode: ConnectionMode::Proxy,
            connected_at: now_rfc3339(),
            credential_status: CredentialStatus::Ok,
            sandbox,
            agents: Vec::new(),
        })
        .map_err(|error| ConnectError::new(ConnectErrorCode::Internal, error))?;
    context.upstream.invalidate(&entry.name).await;
    Ok(tools)
}

pub fn render_error(error: &ConnectError) -> String {
    let mut text = format!("{}: {}", error.code, error.message);
    if let Some(portal) = &error.portal_url {
        let _ = write!(text, "\nGet a new token at: {portal}");
    }
    if let Some(hint) = &error.hint {
        let _ = write!(text, "\n{hint}");
    }
    text
}
