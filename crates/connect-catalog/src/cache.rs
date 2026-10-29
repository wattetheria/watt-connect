//! On-disk cache of the last verified catalog: `catalog.json`, `catalog.sig`
//! and `etag`. The signature is re-verified on every load, so a tampered
//! cache is ignored rather than trusted.

use crate::verify::CatalogSignature;
use std::io;
use std::path::{Path, PathBuf};

pub const CATALOG_FILE: &str = "catalog.json";
pub const SIGNATURE_FILE: &str = "catalog.sig";
pub const ETAG_FILE: &str = "etag";

#[derive(Debug, Clone)]
pub struct CatalogCache {
    dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct CachedCatalog {
    pub catalog: Vec<u8>,
    pub signature: CatalogSignature,
    pub etag: Option<String>,
}

impl CatalogCache {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// `Ok(None)` when nothing (or something unreadable) is cached.
    pub fn load(&self) -> io::Result<Option<CachedCatalog>> {
        let catalog = match std::fs::read(self.dir.join(CATALOG_FILE)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let Ok(signature_bytes) = std::fs::read(self.dir.join(SIGNATURE_FILE)) else {
            return Ok(None);
        };
        let Ok(signature) = serde_json::from_slice(&signature_bytes) else {
            return Ok(None);
        };
        let etag = std::fs::read_to_string(self.dir.join(ETAG_FILE))
            .ok()
            .map(|etag| etag.trim().to_owned())
            .filter(|etag| !etag.is_empty());
        Ok(Some(CachedCatalog {
            catalog,
            signature,
            etag,
        }))
    }

    #[must_use]
    pub fn etag(&self) -> Option<String> {
        std::fs::read_to_string(self.dir.join(ETAG_FILE))
            .ok()
            .map(|etag| etag.trim().to_owned())
            .filter(|etag| !etag.is_empty())
    }

    /// Write atomically (temp file + rename) so a crash never leaves a
    /// catalog paired with the wrong signature for long.
    pub fn store(&self, cached: &CachedCatalog) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        write_atomic(&self.dir.join(CATALOG_FILE), &cached.catalog)?;
        let signature = serde_json::to_vec_pretty(&cached.signature).map_err(io::Error::other)?;
        write_atomic(&self.dir.join(SIGNATURE_FILE), &signature)?;
        match &cached.etag {
            Some(etag) => write_atomic(&self.dir.join(ETAG_FILE), etag.as_bytes())?,
            None => match std::fs::remove_file(self.dir.join(ETAG_FILE)) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            },
        }
        Ok(())
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let cache = CatalogCache::new(dir.path().join("nested"));
        assert!(cache.load().unwrap().is_none());
        let cached = CachedCatalog {
            catalog: b"{}".to_vec(),
            signature: CatalogSignature {
                algorithm: "ed25519".into(),
                public_key: "aa".into(),
                signature: "bb".into(),
            },
            etag: Some("\"v1\"".into()),
        };
        cache.store(&cached).unwrap();
        let loaded = cache.load().unwrap().unwrap();
        assert_eq!(loaded.catalog, b"{}");
        assert_eq!(loaded.etag.as_deref(), Some("\"v1\""));
        assert_eq!(cache.etag().as_deref(), Some("\"v1\""));
    }
}
