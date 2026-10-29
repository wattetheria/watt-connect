//! Catalog signatures: Ed25519 over the JCS (RFC 8785) canonical form of the
//! catalog JSON, using Watt Credential's detached signature primitives (the
//! same scheme watt-registry uses).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use watt_credential::{DetachedSignature, verify_detached_signature};

pub const SIGNATURE_ALGORITHM: &str = "ed25519";
pub const ENCODING_HEX: &str = "hex";

/// Contents of `catalog.sig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogSignature {
    pub algorithm: String,
    /// Hex Ed25519 public key that produced `signature`.
    pub public_key: String,
    /// Hex Ed25519 signature over the JCS bytes of the catalog.
    pub signature: String,
}

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("catalog is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("catalog could not be canonicalized: {0}")]
    Canonicalize(String),
    #[error("no trusted catalog signing key is configured")]
    NoTrustedKeys,
    #[error("catalog was signed by an untrusted key")]
    UntrustedKey,
    #[error("catalog signature is invalid: {0}")]
    BadSignature(String),
}

/// The exact bytes that are signed: JCS canonical JSON of the catalog.
pub fn signing_bytes(catalog_json: &Value) -> Result<Vec<u8>, VerifyError> {
    serde_jcs::to_vec(catalog_json).map_err(|error| VerifyError::Canonicalize(error.to_string()))
}

/// Verify `catalog_bytes` against `signature`, accepting only `trusted_keys`
/// (hex Ed25519 public keys).
pub fn verify_catalog_bytes(
    catalog_bytes: &[u8],
    signature: &CatalogSignature,
    trusted_keys: &[String],
) -> Result<(), VerifyError> {
    if trusted_keys.is_empty() {
        return Err(VerifyError::NoTrustedKeys);
    }
    let signer = signature.public_key.trim().to_ascii_lowercase();
    if !trusted_keys
        .iter()
        .any(|key| key.trim().eq_ignore_ascii_case(&signer))
    {
        return Err(VerifyError::UntrustedKey);
    }
    let value: Value = serde_json::from_slice(catalog_bytes)?;
    let message = signing_bytes(&value)?;
    verify_detached_signature(
        DetachedSignature {
            algorithm: &signature.algorithm,
            public_key_encoding: ENCODING_HEX,
            public_key: &signer,
            signature_encoding: ENCODING_HEX,
            signature: signature.signature.trim(),
        },
        &message,
    )
    .map_err(|error| VerifyError::BadSignature(error.to_string()))
}

/// Parse a trusted-keys file: one hex public key per line, `#` comments.
#[must_use]
pub fn parse_trusted_keys(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.split('#').next().unwrap_or("").trim())
        .filter(|line| !line.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// Keys compiled into the binary from `catalog/catalog.pub`.
#[must_use]
pub fn builtin_trusted_keys() -> Vec<String> {
    parse_trusted_keys(include_str!("../../../catalog/catalog.pub"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use watt_credential::{SigningKeyMaterial, generate_signing_key, sign_detached};

    fn sign(bytes: &[u8]) -> (CatalogSignature, String) {
        let key = generate_signing_key(SIGNATURE_ALGORITHM).unwrap();
        let value: Value = serde_json::from_slice(bytes).unwrap();
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
            CatalogSignature {
                algorithm: SIGNATURE_ALGORITHM.into(),
                public_key: key.public_key.clone(),
                signature,
            },
            key.public_key,
        )
    }

    #[test]
    fn signature_is_independent_of_formatting() {
        let (signature, public_key) = sign(br#"{"b":1,"a":[1,2]}"#);
        let reformatted = b"{\n  \"a\": [1, 2],\n  \"b\": 1\n}";
        verify_catalog_bytes(reformatted, &signature, &[public_key]).unwrap();
    }

    #[test]
    fn tampering_and_untrusted_keys_are_rejected() {
        let (signature, public_key) = sign(br#"{"a":1}"#);
        assert!(matches!(
            verify_catalog_bytes(br#"{"a":2}"#, &signature, std::slice::from_ref(&public_key)),
            Err(VerifyError::BadSignature(_))
        ));
        assert!(matches!(
            verify_catalog_bytes(br#"{"a":1}"#, &signature, &["00".repeat(32)]),
            Err(VerifyError::UntrustedKey)
        ));
        assert!(matches!(
            verify_catalog_bytes(br#"{"a":1}"#, &signature, &[]),
            Err(VerifyError::NoTrustedKeys)
        ));
    }

    #[test]
    fn trusted_keys_file_ignores_comments() {
        let keys = parse_trusted_keys("# comment\n\nABCD # inline\n");
        assert_eq!(keys, vec!["abcd".to_owned()]);
    }
}
