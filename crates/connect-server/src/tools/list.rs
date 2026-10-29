//! `list_connections()`.

use crate::ConnectServer;
use connect_types::{ConnectError, ConnectionMode, CredentialStatus};
use serde_json::{Value, json};

pub(super) async fn run(server: &ConnectServer) -> Result<Value, ConnectError> {
    let ctx = server.context();
    let loaded = ctx.catalog.current();
    let mut connections = Vec::new();
    for record in ctx.connections.list() {
        let entry = loaded.catalog.get(&record.service);
        let mut status = record.credential_status;
        if record.mode == ConnectionMode::Proxy
            && entry.is_some_and(|entry| entry.connect().credential.is_some())
            && status != CredentialStatus::CredentialInvalid
            && ctx.token(&record.service).await?.is_none()
        {
            status = CredentialStatus::Missing;
        }
        connections.push(json!({
            "service": record.service,
            "title": entry.map(connect_catalog::ServerEntry::display_title),
            "mode": record.mode.as_str(),
            "credential_status": status.as_str(),
            "catalog_status": entry.map_or("unknown", |entry| entry.connect().status.as_str()),
            "connected_at": record.connected_at,
            "sandbox": record.sandbox,
            "agents": record.agents,
            "portal_url": entry.and_then(connect_catalog::ServerEntry::portal_url),
        }));
    }
    Ok(json!({
        "vault": ctx.vault.backend(),
        "catalog_version": loaded.catalog.version,
        "catalog_source": loaded.origin.as_str(),
        "connections": connections,
    }))
}
