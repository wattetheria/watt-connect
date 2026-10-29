//! Which services the user connected, in which mode. Never holds tokens.

use connect_types::{ConnectionMode, CredentialStatus, ServiceId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionRecord {
    pub service: ServiceId,
    pub mode: ConnectionMode,
    /// RFC 3339.
    pub connected_at: String,
    #[serde(default)]
    pub credential_status: CredentialStatus,
    /// Use the merchant's sandbox endpoint.
    #[serde(default)]
    pub sandbox: bool,
    /// Agents the service was installed into (direct mode).
    #[serde(default)]
    pub agents: Vec<String>,
}

pub trait ConnectionStore: Send + Sync {
    fn list(&self) -> Vec<ConnectionRecord>;
    fn get(&self, service: &ServiceId) -> Option<ConnectionRecord> {
        self.list()
            .into_iter()
            .find(|record| &record.service == service)
    }
    fn upsert(&self, record: ConnectionRecord) -> Result<(), String>;
    fn remove(&self, service: &ServiceId) -> Result<(), String>;

    fn set_credential_status(
        &self,
        service: &ServiceId,
        status: CredentialStatus,
    ) -> Result<(), String> {
        match self.get(service) {
            Some(mut record) if record.credential_status != status => {
                record.credential_status = status;
                self.upsert(record)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Default)]
pub struct MemoryConnectionStore {
    records: Mutex<BTreeMap<ServiceId, ConnectionRecord>>,
}

impl MemoryConnectionStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<ServiceId, ConnectionRecord>> {
        self.records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl ConnectionStore for MemoryConnectionStore {
    fn list(&self) -> Vec<ConnectionRecord> {
        self.lock().values().cloned().collect()
    }

    fn upsert(&self, record: ConnectionRecord) -> Result<(), String> {
        self.lock().insert(record.service.clone(), record);
        Ok(())
    }

    fn remove(&self, service: &ServiceId) -> Result<(), String> {
        self.lock().remove(service);
        Ok(())
    }
}
