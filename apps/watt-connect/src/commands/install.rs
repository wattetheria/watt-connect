//! `watt-connect install|uninstall <service> --agent <agent>`: direct mode.

use super::{lookup, read_token};
use crate::cli::{InstallArgs, UninstallArgs};
use crate::commands::connect::render_error;
use anyhow::{Result, anyhow, bail};
use connect_installer::{AgentKind, installer};
use connect_server::target::{direct_mode_exposes_url_credential, proxy_target};
use connect_server::{ConnectContext, ConnectServer};
use connect_types::ConnectionMode;
use std::sync::Arc;

pub async fn run(context: &Arc<ConnectContext>, args: InstallArgs) -> Result<()> {
    let entry = lookup(context, &args.service)?;
    let agent: AgentKind = args.agent.parse().map_err(anyhow::Error::msg)?;
    if !entry.connect().modes.direct {
        bail!("{} does not allow direct mode", entry.name);
    }
    if args.sandbox && entry.connect().sandbox.is_none() {
        bail!("{} has no sandbox endpoint", entry.name);
    }
    let token = if entry.connect().credential.is_some() {
        let stored = context
            .token(&entry.name)
            .await
            .map_err(|error| anyhow!("{error}"))?;
        let token = match stored {
            Some(token) => token,
            None => read_token(&entry, args.token.token_stdin)?,
        };
        if !args.token.no_verify {
            let target = proxy_target(&entry, args.sandbox, Some(&token))
                .map_err(|error| anyhow!("{error}"))?;
            connect_upstream::probe(&target).await.map_err(|error| {
                anyhow!(
                    "could not verify the token: {} {}",
                    error.code,
                    connect_vault::redact::redact(&error.message)
                )
            })?;
        }
        Some(token)
    } else {
        None
    };
    if direct_mode_exposes_url_credential(&entry) {
        eprintln!(
            "warning: {}'s token is part of the server URL and will be stored in plain text in the {agent} config.",
            entry.display_title()
        );
    }
    let server = ConnectServer::new(context.clone());
    let report = server
        .install(&entry, agent, args.sandbox, token.as_ref())
        .await
        .map_err(|error| anyhow!(render_error(&error)))?;
    println!(
        "Installed {} as `{}` in {} ({}).",
        entry.display_title(),
        report.name,
        agent,
        report.location
    );
    if let Some(backup) = report.backup {
        println!("Backup of the previous config: {}", backup.display());
    }
    println!("Restart {agent} to load it. watt-connect keeps no copy of this token.");
    Ok(())
}

pub fn uninstall(context: &Arc<ConnectContext>, args: &UninstallArgs) -> Result<()> {
    let entry = lookup(context, &args.service)?;
    let agent: AgentKind = args.agent.parse().map_err(anyhow::Error::msg)?;
    let removed = installer(agent, &context.installer_env)
        .uninstall(&entry.name.slug())
        .map_err(|error| anyhow!("{error}"))?;
    if let Some(mut record) = context.connections.get(&entry.name)
        && record.mode == ConnectionMode::Direct
    {
        record.agents.retain(|existing| existing != agent.as_str());
        if record.agents.is_empty() {
            context
                .connections
                .remove(&entry.name)
                .map_err(|error| anyhow!(error))?;
        } else {
            context
                .connections
                .upsert(record)
                .map_err(|error| anyhow!(error))?;
        }
    }
    if removed {
        println!("Removed {} from {agent}.", entry.name.slug());
    } else {
        println!("{} was not configured in {agent}.", entry.name.slug());
    }
    Ok(())
}
