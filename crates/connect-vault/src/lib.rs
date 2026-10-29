//! Credential storage and redaction.
//!
//! Merchant tokens never leave the user's machine: they are stored in an
//! encrypted file (macOS, or where no keychain exists) or the system
//! keychain (Windows, Linux), are never
//! written to config files or logs, and are masked wherever text might be
//! logged or returned to the agent.

mod file;
mod keyring;
pub mod redact;

pub use file::FileVault;
pub use keyring::KeyringVault;

use connect_types::ServiceId;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::Mutex;
use zeroize::Zeroizing;

/// Prefix of keychain entries: `watt-connect/<service-name>`.
pub const KEYCHAIN_PREFIX: &str = "watt-connect";

/// A credential. `Debug`/`Display` never print it and memory is wiped on drop.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    /// The raw value. Only for placing it on the outgoing request.
    #[must_use]
    pub fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("keychain error: {0}")]
    Keyring(String),
    #[error("vault file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("vault file is corrupt or was encrypted with another key")]
    Decrypt,
    #[error("vault data error: {0}")]
    Format(String),
}

/// Storage of one token per service.
pub trait Vault: Send + Sync {
    fn get(&self, service: &ServiceId) -> Result<Option<Secret>, VaultError>;
    fn set(&self, service: &ServiceId, secret: &Secret) -> Result<(), VaultError>;
    /// Removing a missing entry is not an error.
    fn delete(&self, service: &ServiceId) -> Result<(), VaultError>;
    /// Short name shown by `watt-connect status`.
    fn backend(&self) -> &'static str;
}

/// Keychain entry name for a service.
#[must_use]
pub fn entry_name(service: &ServiceId) -> String {
    format!("{KEYCHAIN_PREFIX}/{service}")
}

/// Which backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultPreference {
    /// The encrypted file on macOS; elsewhere the keychain when it works,
    /// otherwise the encrypted file.
    Auto,
    Keyring,
    File,
}

impl std::str::FromStr for VaultPreference {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "auto" | "" => Ok(Self::Auto),
            "keyring" | "keychain" => Ok(Self::Keyring),
            "file" => Ok(Self::File),
            other => Err(format!(
                "unknown vault backend `{other}` (expected auto|keyring|file)"
            )),
        }
    }
}

/// Open the vault. `data_dir` holds the file fallback (`vault.enc`, `vault.key`).
pub fn open_vault(
    preference: VaultPreference,
    data_dir: &Path,
) -> Result<Box<dyn Vault>, VaultError> {
    match preference {
        VaultPreference::Keyring => Ok(Box::new(KeyringVault::new())),
        VaultPreference::File => Ok(Box::new(FileVault::open(data_dir)?)),
        // macOS Keychain ties its access list to the binary's code signature;
        // without a Developer ID signature every rebuild or upgrade asks the
        // user for their login password again, once per read.
        VaultPreference::Auto if cfg!(target_os = "macos") => {
            Ok(Box::new(FileVault::open(data_dir)?))
        }
        VaultPreference::Auto => {
            if KeyringVault::available() {
                Ok(Box::new(KeyringVault::new()))
            } else {
                tracing::info!("system keychain unavailable; using encrypted file vault");
                Ok(Box::new(FileVault::open(data_dir)?))
            }
        }
    }
}

/// In-memory vault for tests.
#[derive(Default)]
pub struct MemoryVault {
    entries: Mutex<HashMap<ServiceId, Secret>>,
}

impl Vault for MemoryVault {
    fn get(&self, service: &ServiceId) -> Result<Option<Secret>, VaultError> {
        Ok(self.lock().get(service).cloned())
    }

    fn set(&self, service: &ServiceId, secret: &Secret) -> Result<(), VaultError> {
        self.lock().insert(service.clone(), secret.clone());
        Ok(())
    }

    fn delete(&self, service: &ServiceId) -> Result<(), VaultError> {
        self.lock().remove(service);
        Ok(())
    }

    fn backend(&self) -> &'static str {
        "memory"
    }
}

impl MemoryVault {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ServiceId, Secret>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_is_redacted() {
        let secret = Secret::new("super-secret");
        assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
        assert_eq!(secret.expose(), "super-secret");
    }

    #[test]
    fn entry_names_follow_convention() {
        let id = ServiceId::parse("cn.lkcoffee/order").unwrap();
        assert_eq!(entry_name(&id), "watt-connect/cn.lkcoffee/order");
    }
}
