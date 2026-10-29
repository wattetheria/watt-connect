//! The MCP server watt-connect exposes to the user's agent.
//!
//! The agent only ever sees six meta tools, however many merchants exist:
//! `search_services`, `describe_service`, `connect_service`, `call_service`,
//! `install_service`, `list_connections`.

pub mod connections;
mod server;
pub mod target;
pub mod tools;

pub use connections::{ConnectionRecord, ConnectionStore, MemoryConnectionStore};
pub use server::{ConnectServer, INSTRUCTIONS};

use connect_catalog::CatalogStore;
use connect_installer::InstallerEnv;
use connect_types::{ConnectError, ConnectErrorCode, ServiceId};
use connect_upstream::UpstreamPool;
use connect_vault::{Secret, Vault};
use std::sync::Arc;

/// Starts a one-time local web page where the user can paste a token.
pub trait ConnectLinkProvider: Send + Sync {
    /// Returns `http://127.0.0.1:<port>/connect/<nonce>`, or `None` if unavailable.
    fn start(&self, service: &ServiceId) -> Option<String>;
}

/// Everything the meta tools need. Shared by the MCP server and the CLI.
pub struct ConnectContext {
    pub catalog: Arc<CatalogStore>,
    pub vault: Arc<dyn Vault>,
    pub connections: Arc<dyn ConnectionStore>,
    pub upstream: Arc<UpstreamPool>,
    pub installer_env: InstallerEnv,
    pub link_provider: Option<Arc<dyn ConnectLinkProvider>>,
    /// How users invoke the CLI, e.g. `npx watt-connect`.
    pub command: String,
}

impl ConnectContext {
    /// Read a token off the async runtime (keychain calls may block).
    pub async fn token(&self, service: &ServiceId) -> Result<Option<Secret>, ConnectError> {
        let vault = self.vault.clone();
        let id = service.clone();
        let secret = tokio::task::spawn_blocking(move || vault.get(&id))
            .await
            .map_err(|error| internal(error.to_string()))?
            .map_err(|error| internal(format!("credential store: {error}")))?;
        if let Some(secret) = &secret {
            connect_vault::redact::register_secret(secret.expose());
        }
        Ok(secret)
    }

    pub async fn delete_token(&self, service: &ServiceId) -> Result<(), ConnectError> {
        let vault = self.vault.clone();
        let id = service.clone();
        tokio::task::spawn_blocking(move || vault.delete(&id))
            .await
            .map_err(|error| internal(error.to_string()))?
            .map_err(|error| internal(format!("credential store: {error}")))
    }

    #[must_use]
    pub fn connect_command(&self, service: &ServiceId) -> String {
        format!("{} connect {service}", self.command)
    }

    #[must_use]
    pub fn install_command(&self, service: &ServiceId, agent: &str) -> String {
        format!("{} install {service} --agent {agent}", self.command)
    }
}

pub(crate) fn internal(message: impl Into<String>) -> ConnectError {
    ConnectError::new(ConnectErrorCode::Internal, message)
}

pub(crate) fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}
