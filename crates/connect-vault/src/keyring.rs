//! System keychain backend: macOS Keychain, Windows Credential Manager, or
//! Linux Secret Service, via the `keyring` crate.

use crate::{Secret, Vault, VaultError, entry_name};
use connect_types::ServiceId;
use keyring::{Entry, Error};

/// Account name stored alongside each keychain entry.
const ACCOUNT: &str = "token";

#[derive(Debug, Default)]
pub struct KeyringVault;

impl KeyringVault {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Whether the platform keychain can actually be used (e.g. a headless
    /// Linux box usually has no Secret Service running).
    #[must_use]
    pub fn available() -> bool {
        if Entry::store_status().is_err() {
            return false;
        }
        let Ok(entry) = Entry::new(&format!("{}/.probe", crate::KEYCHAIN_PREFIX), ACCOUNT) else {
            return false;
        };
        matches!(entry.get_password(), Ok(_) | Err(Error::NoEntry))
    }

    fn entry(service: &ServiceId) -> Result<Entry, VaultError> {
        Entry::new(&entry_name(service), ACCOUNT)
            .map_err(|error| VaultError::Keyring(error.to_string()))
    }
}

impl Vault for KeyringVault {
    fn get(&self, service: &ServiceId) -> Result<Option<Secret>, VaultError> {
        match Self::entry(service)?.get_password() {
            Ok(value) => Ok(Some(Secret::new(value))),
            Err(Error::NoEntry) => Ok(None),
            Err(error) => Err(VaultError::Keyring(error.to_string())),
        }
    }

    fn set(&self, service: &ServiceId, secret: &Secret) -> Result<(), VaultError> {
        Self::entry(service)?
            .set_password(secret.expose())
            .map_err(|error| VaultError::Keyring(error.to_string()))
    }

    fn delete(&self, service: &ServiceId) -> Result<(), VaultError> {
        match Self::entry(service)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(error) => Err(VaultError::Keyring(error.to_string())),
        }
    }

    fn backend(&self) -> &'static str {
        "keychain"
    }
}
