//! `watt-connect disconnect <service>`.

use super::lookup;
use crate::cli::DisconnectArgs;
use anyhow::{Result, anyhow};
use connect_installer::{AgentKind, installer};
use connect_server::ConnectContext;

pub async fn run(context: &ConnectContext, args: DisconnectArgs) -> Result<()> {
    let entry = lookup(context, &args.service)?;
    let record = context.connections.get(&entry.name);
    context
        .delete_token(&entry.name)
        .await
        .map_err(|error| anyhow!("{error}"))?;
    context.upstream.invalidate(&entry.name).await;
    if let Some(record) = &record
        && !args.keep_agent_config
    {
        for agent in &record.agents {
            let Ok(kind) = agent.parse::<AgentKind>() else {
                continue;
            };
            match installer(kind, &context.installer_env).uninstall(&entry.name.slug()) {
                Ok(true) => println!("Removed {} from {kind}.", entry.name.slug()),
                Ok(false) => {}
                Err(error) => eprintln!("warning: could not update {kind}: {error}"),
            }
        }
    }
    context
        .connections
        .remove(&entry.name)
        .map_err(|error| anyhow!(error))?;
    println!("Disconnected {} ({}).", entry.display_title(), entry.name);
    Ok(())
}
