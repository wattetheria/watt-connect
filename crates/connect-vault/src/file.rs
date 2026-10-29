//! Encrypted file fallback for machines without a usable keychain.
//!
//! `vault.enc` holds `nonce || XChaCha20-Poly1305(json map)`; the 32-byte key
//! is in `vault.key`. Both files are created with mode `0600`. This protects
//! against the vault file being copied or synced on its own; it cannot
//! protect against code running as the same user, which a keychain also
//! cannot fully do.

use crate::{Secret, Vault, VaultError};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use connect_types::ServiceId;
use rand::RngCore;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use zeroize::Zeroizing;

pub const VAULT_FILE: &str = "vault.enc";
pub const KEY_FILE: &str = "vault.key";
const NONCE_LEN: usize = 24;

pub struct FileVault {
    path: PathBuf,
    key: Zeroizing<[u8; 32]>,
    lock: Mutex<()>,
}

impl FileVault {
    pub fn open(dir: &Path) -> Result<Self, VaultError> {
        std::fs::create_dir_all(dir)?;
        let key_path = dir.join(KEY_FILE);
        let key = match std::fs::read(&key_path) {
            Ok(bytes) => {
                let key: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
                    VaultError::Format(format!("{} must contain 32 bytes", key_path.display()))
                })?;
                Zeroizing::new(key)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let mut key = Zeroizing::new([0_u8; 32]);
                rand::rng().fill_bytes(key.as_mut());
                write_private(&key_path, key.as_ref())?;
                key
            }
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            path: dir.join(VAULT_FILE),
            key,
            lock: Mutex::new(()),
        })
    }

    fn read_all(&self) -> Result<BTreeMap<String, Zeroizing<String>>, VaultError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(error) => return Err(error.into()),
        };
        if bytes.len() < NONCE_LEN {
            return Err(VaultError::Decrypt);
        }
        let (nonce, ciphertext) = bytes.split_at(NONCE_LEN);
        let plaintext = Zeroizing::new(
            self.cipher()
                .decrypt(XNonce::from_slice(nonce), ciphertext)
                .map_err(|_| VaultError::Decrypt)?,
        );
        let map: BTreeMap<String, String> = serde_json::from_slice(&plaintext)
            .map_err(|error| VaultError::Format(error.to_string()))?;
        Ok(map
            .into_iter()
            .map(|(k, v)| (k, Zeroizing::new(v)))
            .collect())
    }

    fn write_all(&self, map: &BTreeMap<String, Zeroizing<String>>) -> Result<(), VaultError> {
        let plain: BTreeMap<&str, &str> =
            map.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let plaintext = Zeroizing::new(
            serde_json::to_vec(&plain).map_err(|error| VaultError::Format(error.to_string()))?,
        );
        let mut nonce = [0_u8; NONCE_LEN];
        rand::rng().fill_bytes(&mut nonce);
        let ciphertext = self
            .cipher()
            .encrypt(XNonce::from_slice(&nonce), plaintext.as_slice())
            .map_err(|_| VaultError::Format("encryption failed".into()))?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&ciphertext);
        let tmp = self.path.with_extension("tmp");
        write_private(&tmp, &out)?;
        std::fs::rename(tmp, &self.path)?;
        Ok(())
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(self.key.as_ref().into())
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Vault for FileVault {
    fn get(&self, service: &ServiceId) -> Result<Option<Secret>, VaultError> {
        let _guard = self.guard();
        Ok(self
            .read_all()?
            .get(service.as_str())
            .map(|value| Secret::new(value.as_str())))
    }

    fn set(&self, service: &ServiceId, secret: &Secret) -> Result<(), VaultError> {
        let _guard = self.guard();
        let mut map = self.read_all()?;
        map.insert(
            service.to_string(),
            Zeroizing::new(secret.expose().to_owned()),
        );
        self.write_all(&map)
    }

    fn delete(&self, service: &ServiceId) -> Result<(), VaultError> {
        let _guard = self.guard();
        let mut map = self.read_all()?;
        if map.remove(service.as_str()).is_some() {
            self.write_all(&map)?;
        }
        Ok(())
    }

    fn backend(&self) -> &'static str {
        "encrypted-file"
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_encrypted_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let id = ServiceId::parse("cn.lkcoffee/order").unwrap();
        let vault = FileVault::open(dir.path()).unwrap();
        assert!(vault.get(&id).unwrap().is_none());
        vault.set(&id, &Secret::new("tok-123")).unwrap();
        assert_eq!(vault.get(&id).unwrap().unwrap().expose(), "tok-123");

        let raw = std::fs::read(dir.path().join(VAULT_FILE)).unwrap();
        assert!(!raw.windows(7).any(|window| window == b"tok-123"));

        let reopened = FileVault::open(dir.path()).unwrap();
        assert_eq!(reopened.get(&id).unwrap().unwrap().expose(), "tok-123");
        reopened.delete(&id).unwrap();
        reopened.delete(&id).unwrap();
        assert!(reopened.get(&id).unwrap().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let vault = FileVault::open(dir.path()).unwrap();
        vault
            .set(
                &ServiceId::parse("cn.mcd/order").unwrap(),
                &Secret::new("x"),
            )
            .unwrap();
        for file in [KEY_FILE, VAULT_FILE] {
            let mode = std::fs::metadata(dir.path().join(file))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{file}");
        }
    }

    #[test]
    fn wrong_key_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let id = ServiceId::parse("cn.mcd/order").unwrap();
        FileVault::open(dir.path())
            .unwrap()
            .set(&id, &Secret::new("x"))
            .unwrap();
        std::fs::write(dir.path().join(KEY_FILE), [7_u8; 32]).unwrap();
        let vault = FileVault::open(dir.path()).unwrap();
        assert!(matches!(vault.get(&id), Err(VaultError::Decrypt)));
    }
}
