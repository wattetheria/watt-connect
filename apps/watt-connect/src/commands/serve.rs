//! Default command: serve the meta tools over stdio.

use anyhow::{Context, Result};
use connect_catalog::REFRESH_INTERVAL;
use connect_server::{ConnectContext, ConnectServer};
use rmcp::ServiceExt;
use std::sync::Arc;

pub async fn run(context: Arc<ConnectContext>) -> Result<()> {
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        catalog = %context.catalog.current().catalog.version,
        vault = context.vault.backend(),
        "watt-connect serving on stdio"
    );
    let refresh = context.catalog.clone().spawn_refresh_loop(REFRESH_INTERVAL);
    let server = ConnectServer::new(context);
    let running = Box::pin(server.serve(rmcp::transport::stdio()))
        .await
        .context("MCP handshake with the agent failed")?;
    let reason = running.waiting().await;
    refresh.abort();
    tracing::info!(?reason, "agent disconnected");
    Ok(())
}
