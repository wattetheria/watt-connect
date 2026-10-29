//! Catalog signatures: Ed25519 over the JCS (RFC 8785) canonical form of the
//! catalog JSON. Keys and signatures are hex encoded and the algorithm is
//! named `ed25519` — the same format watt-registry uses, implemented here so
//! watt-connect has no dependency on other Watt repositories.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zeroize::Zeroizing;

pub const SIGNATURE_ALGORITHM: &str = "ed25519";

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
    #[error("invalid signing key: {0}")]
    BadKey(String),
}

/// A freshly generated key pair, hex encoded. `Debug` is not derived.
pub struct KeyPair {
    pub private_key: Zeroizing<String>,
    pub public_key: String,
}

/// Generate a new Ed25519 key pair.
#[must_use]
pub fn generate_key_pair() -> KeyPair {
    let mut seed = Zeroizing::new([0_u8; 32]);
    rand::rng().fill_bytes(seed.as_mut());
    let signing_key = SigningKey::from_bytes(&seed);
    KeyPair {
        private_key: Zeroizing::new(hex::encode(seed.as_ref())),
        public_key: hex::encode(signing_key.verifying_key().to_bytes()),
    }
}

fn signing_key(private_key_hex: &str) -> Result<SigningKey, VerifyError> {
    let bytes = Zeroizing::new(
        hex::decode(private_key_hex.trim())
            .map_err(|error| VerifyError::BadKey(error.to_string()))?,
    );
    let seed: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| VerifyError::BadKey("Ed25519 private key must be 32 bytes".into()))?;
    Ok(SigningKey::from_bytes(&seed))
}

/// Hex public key of a hex private key (seed).
pub fn public_key_of(private_key_hex: &str) -> Result<String, VerifyError> {
    Ok(hex::encode(
        signing_key(private_key_hex)?.verifying_key().to_bytes(),
    ))
}

/// The exact bytes that are signed: JCS canonical JSON of the catalog.
pub fn signing_bytes(catalog_json: &Value) -> Result<Vec<u8>, VerifyError> {
    serde_jcs::to_vec(catalog_json).map_err(|error| VerifyError::Canonicalize(error.to_string()))
}

/// Sign `catalog_bytes` (any JSON formatting) with a hex private key.
pub fn sign_catalog_bytes(
    catalog_bytes: &[u8],
    private_key_hex: &str,
) -> Result<CatalogSignature, VerifyError> {
    let key = signing_key(private_key_hex)?;
    let value: Value = serde_json::from_slice(catalog_bytes)?;
    let signature = key.sign(&signing_bytes(&value)?);
    Ok(CatalogSignature {
        algorithm: SIGNATURE_ALGORITHM.into(),
        public_key: hex::encode(key.verifying_key().to_bytes()),
        signature: hex::encode(signature.to_bytes()),
    })
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
    if signature.algorithm != SIGNATURE_ALGORITHM {
        return Err(VerifyError::BadSignature(format!(
            "unsupported algorithm `{}`",
            signature.algorithm
        )));
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
    let public_key: [u8; 32] = decode_fixed(&signer, "public key")?;
    let verifying_key = VerifyingKey::from_bytes(&public_key)
        .map_err(|error| VerifyError::BadSignature(format!("public key: {error}")))?;
    let signature_bytes: [u8; 64] = decode_fixed(signature.signature.trim(), "signature")?;
    verifying_key
        .verify_strict(&message, &Signature::from_bytes(&signature_bytes))
        .map_err(|error| VerifyError::BadSignature(error.to_string()))
}

fn decode_fixed<const N: usize>(hex_value: &str, what: &str) -> Result<[u8; N], VerifyError> {
    hex::decode(hex_value)
        .map_err(|error| VerifyError::BadSignature(format!("{what}: {error}")))?
        .try_into()
        .map_err(|_| VerifyError::BadSignature(format!("{what} must be {N} bytes")))
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

    fn sign(bytes: &[u8]) -> (CatalogSignature, String) {
        let key = generate_key_pair();
        let signature = sign_catalog_bytes(bytes, &key.private_key).unwrap();
        assert_eq!(signature.public_key, key.public_key);
        (signature, key.public_key)
    }

    #[test]
    fn rfc8032_test_vector_1() {
        // RFC 8032 §7.1, TEST 1 (empty message): standard Ed25519, hex encoded.
        let key = signing_key("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
            .unwrap();
        assert_eq!(
            hex::encode(key.verifying_key().to_bytes()),
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
        );
        assert_eq!(
            hex::encode(key.sign(b"").to_bytes()),
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
        );
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
        let other_algorithm = CatalogSignature {
            algorithm: "rsa".into(),
            ..signature
        };
        assert!(matches!(
            verify_catalog_bytes(br#"{"a":1}"#, &other_algorithm, &[public_key]),
            Err(VerifyError::BadSignature(_))
        ));
    }

    #[test]
    fn key_helpers() {
        let key = generate_key_pair();
        assert_eq!(public_key_of(&key.private_key).unwrap(), key.public_key);
        assert!(matches!(public_key_of("abcd"), Err(VerifyError::BadKey(_))));
    }

    #[test]
    fn trusted_keys_file_ignores_comments() {
        let keys = parse_trusted_keys("# comment\n\nABCD # inline\n");
        assert_eq!(keys, vec!["abcd".to_owned()]);
    }
}
