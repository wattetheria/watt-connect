//! Conditional (`ETag`) download of the signed catalog. No credentials or user
//! data are ever sent: the request carries only `If-None-Match`.

use crate::verify::CatalogSignature;
use reqwest::StatusCode;
use reqwest::header::{ETAG, IF_NONE_MATCH};
use url::Url;

#[derive(Debug)]
pub enum FetchOutcome {
    NotModified,
    Updated {
        catalog: Vec<u8>,
        signature: CatalogSignature,
        etag: Option<String>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("catalog request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("catalog server answered HTTP {0}")]
    Status(u16),
    #[error("catalog signature is malformed: {0}")]
    Signature(#[from] serde_json::Error),
    #[error("cannot derive signature URL from {0}")]
    SignatureUrl(String),
}

/// `https://host/path/catalog.json` -> `https://host/path/catalog.sig`.
pub fn signature_url(catalog_url: &Url) -> Result<Url, FetchError> {
    catalog_url
        .join("catalog.sig")
        .map_err(|_| FetchError::SignatureUrl(catalog_url.to_string()))
}

pub async fn fetch_catalog(
    client: &reqwest::Client,
    catalog_url: &Url,
    etag: Option<&str>,
) -> Result<FetchOutcome, FetchError> {
    let mut request = client.get(catalog_url.clone());
    if let Some(etag) = etag {
        request = request.header(IF_NONE_MATCH, etag);
    }
    let response = request.send().await?;
    if response.status() == StatusCode::NOT_MODIFIED {
        return Ok(FetchOutcome::NotModified);
    }
    if !response.status().is_success() {
        return Err(FetchError::Status(response.status().as_u16()));
    }
    let new_etag = response
        .headers()
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let catalog = response.bytes().await?.to_vec();

    let sig_response = client.get(signature_url(catalog_url)?).send().await?;
    if !sig_response.status().is_success() {
        return Err(FetchError::Status(sig_response.status().as_u16()));
    }
    let signature: CatalogSignature = serde_json::from_slice(&sig_response.bytes().await?)?;
    Ok(FetchOutcome::Updated {
        catalog,
        signature,
        etag: new_etag,
    })
}
