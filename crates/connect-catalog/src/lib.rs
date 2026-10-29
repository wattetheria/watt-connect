//! watt-connect catalog: parse, validate, verify, fetch, cache and search the
//! list of merchant MCP services.
//!
//! The catalog is metadata only. Fetching it never sends a credential.

pub mod bundled;
pub mod cache;
pub mod fetch;
pub mod schema;
pub mod search;
pub mod validate;
pub mod verify;

pub use schema::{
    Catalog, ConnectMeta, CredentialKind, CredentialSpec, Modes, Policy, Remote, ServerEntry,
    ServiceStatus, ToolSummary, compare_versions,
};

use cache::{CachedCatalog, CatalogCache};
use fetch::{FetchError, FetchOutcome};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use url::Url;
use verify::VerifyError;

/// Default published location of the signed catalog (GitHub Pages of this repo).
pub const DEFAULT_CATALOG_URL: &str =
    "https://wattetheria.github.io/watt-connect/catalog/v1/catalog.json";

/// How often a running server re-checks the catalog.
pub const REFRESH_INTERVAL: Duration = Duration::from_hours(6);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogOrigin {
    /// Compiled into the binary.
    Bundled,
    /// Verified copy on disk.
    Cache,
    /// Verified copy just downloaded.
    Remote,
    /// Unsigned local file for development (`WATT_CONNECT_DEV_CATALOG`).
    DevFile,
}

impl CatalogOrigin {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bundled => "bundled",
            Self::Cache => "cache",
            Self::Remote => "remote",
            Self::DevFile => "dev_file",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadedCatalog {
    pub catalog: Catalog,
    pub origin: CatalogOrigin,
}

#[derive(Debug, Clone)]
pub struct CatalogConfig {
    pub url: Url,
    pub trusted_keys: Vec<String>,
    pub cache_dir: PathBuf,
    /// Unsigned catalog file that replaces every other source. Development only.
    pub dev_catalog: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error(transparent)]
    Verify(#[from] VerifyError),
    #[error("catalog is invalid: {0}")]
    Invalid(String),
    #[error("catalog cache error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// Development catalog in use; remote refresh disabled.
    Skipped,
    NotModified,
    /// A verified catalog was downloaded but is not newer than the one in use.
    Unchanged {
        version: String,
    },
    Updated {
        version: String,
    },
}

/// Thread-safe holder of the catalog currently in use.
pub struct CatalogStore {
    config: CatalogConfig,
    cache: CatalogCache,
    http: reqwest::Client,
    current: RwLock<Arc<LoadedCatalog>>,
}

impl CatalogStore {
    /// Load synchronously from the dev file, verified cache, or bundled
    /// snapshot, whichever applies and is newest. Never touches the network.
    pub fn load(config: CatalogConfig) -> Result<Self, CatalogError> {
        let cache = CatalogCache::new(&config.cache_dir);
        let loaded = if let Some(path) = &config.dev_catalog {
            tracing::warn!(path = %path.display(), "using unsigned development catalog");
            let bytes = std::fs::read(path)?;
            LoadedCatalog {
                catalog: parse_checked(&bytes)?,
                origin: CatalogOrigin::DevFile,
            }
        } else {
            let bundled = LoadedCatalog {
                catalog: bundled::bundled_catalog(),
                origin: CatalogOrigin::Bundled,
            };
            match load_verified_cache(&cache, &config.trusted_keys) {
                Some(cached)
                    if compare_versions(&cached.version, &bundled.catalog.version).is_ge() =>
                {
                    LoadedCatalog {
                        catalog: cached,
                        origin: CatalogOrigin::Cache,
                    }
                }
                _ => bundled,
            }
        };
        Ok(Self {
            config,
            cache,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .user_agent(concat!("watt-connect/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|error| CatalogError::Fetch(FetchError::Http(error)))?,
            current: RwLock::new(Arc::new(loaded)),
        })
    }

    /// The catalog in use. Cheap to call; the snapshot is shared.
    #[must_use]
    pub fn current(&self) -> Arc<LoadedCatalog> {
        self.current
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Fetch, verify and, if newer, adopt and cache the remote catalog.
    pub async fn refresh(&self) -> Result<RefreshOutcome, CatalogError> {
        if self.config.dev_catalog.is_some() {
            return Ok(RefreshOutcome::Skipped);
        }
        if self.config.trusted_keys.is_empty() {
            return Err(VerifyError::NoTrustedKeys.into());
        }
        let etag = self.cache.etag();
        let outcome = fetch::fetch_catalog(&self.http, &self.config.url, etag.as_deref()).await?;
        let FetchOutcome::Updated {
            catalog: bytes,
            signature,
            etag,
        } = outcome
        else {
            return Ok(RefreshOutcome::NotModified);
        };
        verify::verify_catalog_bytes(&bytes, &signature, &self.config.trusted_keys)?;
        let catalog = parse_checked(&bytes)?;
        let version = catalog.version.clone();
        let current_version = self.current().catalog.version.clone();
        if compare_versions(&version, &current_version).is_lt() {
            tracing::warn!(%version, %current_version, "ignoring older remote catalog");
            return Ok(RefreshOutcome::Unchanged { version });
        }
        self.cache.store(&CachedCatalog {
            catalog: bytes,
            signature,
            etag,
        })?;
        if compare_versions(&version, &current_version).is_eq() {
            return Ok(RefreshOutcome::Unchanged { version });
        }
        *self
            .current
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(LoadedCatalog {
            catalog,
            origin: CatalogOrigin::Remote,
        });
        Ok(RefreshOutcome::Updated { version })
    }

    /// Refresh now and then every `interval`, logging (never failing) on errors.
    pub fn spawn_refresh_loop(self: Arc<Self>, interval: Duration) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                match self.refresh().await {
                    Ok(outcome) => tracing::debug!(?outcome, "catalog refresh"),
                    Err(CatalogError::Verify(VerifyError::NoTrustedKeys)) => {
                        tracing::debug!("no catalog signing key configured; using bundled catalog");
                        return;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "catalog refresh failed; keeping current catalog");
                    }
                }
                tokio::time::sleep(interval).await;
            }
        })
    }
}

fn parse_checked(bytes: &[u8]) -> Result<Catalog, CatalogError> {
    let catalog =
        Catalog::from_json(bytes).map_err(|error| CatalogError::Invalid(error.to_string()))?;
    let issues = validate::validate_catalog(&catalog);
    if validate::has_errors(&issues) {
        let summary: Vec<String> = issues
            .iter()
            .filter(|issue| issue.severity == validate::Severity::Error)
            .map(ToString::to_string)
            .collect();
        return Err(CatalogError::Invalid(summary.join("; ")));
    }
    Ok(catalog)
}

fn load_verified_cache(cache: &CatalogCache, trusted_keys: &[String]) -> Option<Catalog> {
    let cached = match cache.load() {
        Ok(Some(cached)) => cached,
        Ok(None) => return None,
        Err(error) => {
            tracing::warn!(%error, "cannot read catalog cache");
            return None;
        }
    };
    if let Err(error) =
        verify::verify_catalog_bytes(&cached.catalog, &cached.signature, trusted_keys)
    {
        tracing::warn!(%error, "ignoring catalog cache that fails verification");
        return None;
    }
    match parse_checked(&cached.catalog) {
        Ok(catalog) => Some(catalog),
        Err(error) => {
            tracing::warn!(%error, "ignoring invalid cached catalog");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::http::{HeaderMap, StatusCode, header};
    use axum::response::IntoResponse;
    use axum::routing::get;
    use verify::{CatalogSignature, ENCODING_HEX, SIGNATURE_ALGORITHM, signing_bytes};
    use watt_credential::{SigningKeyMaterial, generate_signing_key, sign_detached};

    fn signed_catalog(version: &str) -> (Vec<u8>, CatalogSignature, String) {
        let mut catalog = bundled::bundled_catalog();
        catalog.version = version.to_owned();
        let bytes = serde_json::to_vec_pretty(&catalog).unwrap();
        let key = generate_signing_key(SIGNATURE_ALGORITHM).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let signature = sign_detached(
            SigningKeyMaterial {
                algorithm: SIGNATURE_ALGORITHM,
                private_key_encoding: ENCODING_HEX,
                private_key: &key.private_key,
            },
            ENCODING_HEX,
            &signing_bytes(&value).unwrap(),
        )
        .unwrap();
        (
            bytes,
            CatalogSignature {
                algorithm: SIGNATURE_ALGORITHM.into(),
                public_key: key.public_key.clone(),
                signature,
            },
            key.public_key,
        )
    }

    async fn serve(bytes: Vec<u8>, signature: CatalogSignature) -> Url {
        let sig = serde_json::to_vec(&signature).unwrap();
        let app = Router::new()
            .route(
                "/v1/catalog.json",
                get(move |headers: HeaderMap| {
                    let bytes = bytes.clone();
                    async move {
                        if headers
                            .get(header::IF_NONE_MATCH)
                            .is_some_and(|v| v == "\"e1\"")
                        {
                            return StatusCode::NOT_MODIFIED.into_response();
                        }
                        ([(header::ETAG, "\"e1\"")], bytes).into_response()
                    }
                }),
            )
            .route("/v1/catalog.sig", get(move || async move { sig.clone() }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Url::parse(&format!("http://{addr}/v1/catalog.json")).unwrap()
    }

    #[tokio::test]
    async fn refresh_verifies_caches_and_uses_etag() {
        let dir = tempfile::tempdir().unwrap();
        let (bytes, signature, public_key) = signed_catalog("2099.1.1");
        let url = serve(bytes, signature).await;
        let config = CatalogConfig {
            url,
            trusted_keys: vec![public_key],
            cache_dir: dir.path().to_owned(),
            dev_catalog: None,
        };
        let store = CatalogStore::load(config.clone()).unwrap();
        assert_eq!(store.current().origin, CatalogOrigin::Bundled);
        assert_eq!(
            store.refresh().await.unwrap(),
            RefreshOutcome::Updated {
                version: "2099.1.1".into()
            }
        );
        assert_eq!(store.current().origin, CatalogOrigin::Remote);
        assert_eq!(store.refresh().await.unwrap(), RefreshOutcome::NotModified);

        // A fresh process picks up the verified cache.
        let reloaded = CatalogStore::load(config.clone()).unwrap();
        assert_eq!(reloaded.current().origin, CatalogOrigin::Cache);
        assert_eq!(reloaded.current().catalog.version, "2099.1.1");

        // A different trusted key rejects the cache and falls back to bundled.
        let untrusted = CatalogStore::load(CatalogConfig {
            trusted_keys: vec!["00".repeat(32)],
            ..config
        })
        .unwrap();
        assert_eq!(untrusted.current().origin, CatalogOrigin::Bundled);
    }

    #[tokio::test]
    async fn refresh_rejects_untrusted_signer() {
        let dir = tempfile::tempdir().unwrap();
        let (bytes, signature, _) = signed_catalog("2099.1.1");
        let url = serve(bytes, signature).await;
        let store = CatalogStore::load(CatalogConfig {
            url,
            trusted_keys: vec!["11".repeat(32)],
            cache_dir: dir.path().to_owned(),
            dev_catalog: None,
        })
        .unwrap();
        assert!(matches!(
            store.refresh().await,
            Err(CatalogError::Verify(VerifyError::UntrustedKey))
        ));
        assert_eq!(store.current().origin, CatalogOrigin::Bundled);
    }

    #[test]
    fn refresh_without_keys_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = CatalogStore::load(CatalogConfig {
            url: Url::parse(DEFAULT_CATALOG_URL).unwrap(),
            trusted_keys: Vec::new(),
            cache_dir: dir.path().to_owned(),
            dev_catalog: None,
        })
        .unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        assert!(matches!(
            runtime.block_on(store.refresh()),
            Err(CatalogError::Verify(VerifyError::NoTrustedKeys))
        ));
    }
}
