//! Masking of credentials in URLs and free text.
//!
//! Three layers are applied, in order:
//! 1. exact known secrets (every token watt-connect has loaded),
//! 2. catalog `redact_patterns` (e.g. `key=[^&]+` for query-placed keys),
//! 3. built-in patterns for bearer tokens and common secret query params.
//!
//! A process-wide redactor is used by the log writer so that nothing reaches
//! stderr or log files unmasked, even if a code path forgets to redact.

use regex::{Captures, Regex};
use std::borrow::Cow;
use std::sync::{LazyLock, RwLock};

pub const MASK: &str = "***";

/// Secrets shorter than this are not registered for exact matching, to avoid
/// masking unrelated short strings.
const MIN_SECRET_LEN: usize = 6;

static BUILTIN: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)(bearer\s+)[A-Za-z0-9._~+/=\-]+",
        r#"(?i)((?:access_token|token|key|api_key|apikey)=)[^&\s"']+"#,
    ]
    .iter()
    .map(|pattern| Regex::new(pattern).expect("built-in redact pattern"))
    .collect()
});

#[derive(Debug, Default, Clone)]
pub struct Redactor {
    secrets: Vec<String>,
    patterns: Vec<Regex>,
}

impl Redactor {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_secret(&mut self, secret: &str) {
        if secret.len() >= MIN_SECRET_LEN && !self.secrets.iter().any(|known| known == secret) {
            self.secrets.push(secret.to_owned());
            // Longest first so overlapping secrets are fully masked.
            self.secrets
                .sort_by_key(|known| std::cmp::Reverse(known.len()));
        }
    }

    /// Add a catalog pattern. Invalid patterns are ignored (the catalog
    /// validator rejects them before publishing).
    pub fn add_pattern(&mut self, pattern: &str) {
        if self.patterns.iter().any(|known| known.as_str() == pattern) {
            return;
        }
        match Regex::new(pattern) {
            Ok(regex) => self.patterns.push(regex),
            Err(error) => tracing::warn!(%error, "ignoring invalid redact pattern"),
        }
    }

    #[must_use]
    pub fn redact<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let mut out = Cow::Borrowed(text);
        for secret in &self.secrets {
            if out.contains(secret.as_str()) {
                out = Cow::Owned(out.replace(secret.as_str(), MASK));
            }
        }
        for regex in self.patterns.iter().chain(BUILTIN.iter()) {
            if regex.is_match(&out) {
                out = Cow::Owned(regex.replace_all(&out, mask_match).into_owned());
            }
        }
        out
    }
}

/// Keep a leading capture group (e.g. `Bearer `) or a `name=` prefix; mask the rest.
fn mask_match(caps: &Captures<'_>) -> String {
    if let Some(prefix) = caps.get(1) {
        return format!("{}{MASK}", prefix.as_str());
    }
    let whole = &caps[0];
    match whole.find('=') {
        Some(index) => format!("{}{MASK}", &whole[..=index]),
        None => MASK.to_owned(),
    }
}

static GLOBAL: LazyLock<RwLock<Redactor>> = LazyLock::new(|| RwLock::new(Redactor::new()));

/// Register a secret with the process-wide redactor.
pub fn register_secret(secret: &str) {
    GLOBAL
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .add_secret(secret);
}

/// Register catalog patterns with the process-wide redactor.
pub fn register_patterns<'a>(patterns: impl IntoIterator<Item = &'a str>) {
    let mut global = GLOBAL
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for pattern in patterns {
        global.add_pattern(pattern);
    }
}

/// Redact with the process-wide redactor.
#[must_use]
pub fn redact(text: &str) -> String {
    GLOBAL
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .redact(text)
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_didi_key_in_url_with_catalog_pattern() {
        let mut redactor = Redactor::new();
        redactor.add_pattern("key=[^&\\s\"]+");
        assert_eq!(
            redactor.redact("GET https://mcp.didichuxing.com/mcp-servers?key=abc123&x=1"),
            "GET https://mcp.didichuxing.com/mcp-servers?key=***&x=1"
        );
    }

    #[test]
    fn masks_known_secrets_and_bearer_tokens() {
        let mut redactor = Redactor::new();
        redactor.add_secret("tok-very-secret");
        redactor.add_secret("abc");
        assert_eq!(redactor.redact("got tok-very-secret back"), "got *** back");
        assert_eq!(
            redactor.redact("Authorization: Bearer eyJhbGciOi.x-y_z"),
            "Authorization: Bearer ***"
        );
        assert_eq!(
            redactor.redact("abc is too short to register"),
            "abc is too short to register"
        );
    }

    #[test]
    fn untouched_text_is_borrowed() {
        let redactor = Redactor::new();
        assert!(matches!(redactor.redact("nothing here"), Cow::Borrowed(_)));
    }

    #[test]
    fn global_redactor() {
        register_secret("global-secret-value");
        assert_eq!(redact("x global-secret-value y"), "x *** y");
    }
}
