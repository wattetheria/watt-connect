use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A declarative description of a merchant response, as written in the
/// catalog's `auth_failure` / `rate_limited` lists.
///
/// Every field that is present must match (logical AND). A list of matchers
/// matches when any one of them does (logical OR). A matcher with no fields
/// never matches, so an accidentally empty `{}` cannot classify everything.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseMatcher {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    /// Case-sensitive substring of the raw response body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_contains: Option<String>,
    /// JSON-RPC `error.code`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jsonrpc_error_code: Option<i64>,
    /// Simple dotted path into a JSON body, e.g. `$.code` or `$.error.type`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_path: Option<String>,
    /// Expected value at `json_path`. Strings and numbers compare by their
    /// textual form, so `"0001"` does not equal `1`. Without `equals`, the
    /// path only has to exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<Value>,
}

/// What was actually seen from the merchant for a single failed exchange.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObservedResponse {
    pub http_status: Option<u16>,
    pub content_type: Option<String>,
    pub body: Option<String>,
    pub jsonrpc_error_code: Option<i64>,
    pub jsonrpc_error_message: Option<String>,
    /// Raw `Retry-After` header, if any.
    pub retry_after: Option<String>,
}

impl ResponseMatcher {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.http_status.is_none()
            && self.body_contains.is_none()
            && self.jsonrpc_error_code.is_none()
            && self.json_path.is_none()
    }

    #[must_use]
    pub fn matches(&self, observed: &ObservedResponse) -> bool {
        if self.is_empty() {
            return false;
        }
        if let Some(status) = self.http_status
            && observed.http_status != Some(status)
        {
            return false;
        }
        if let Some(needle) = &self.body_contains
            && !observed
                .body
                .as_deref()
                .is_some_and(|body| body.contains(needle.as_str()))
        {
            return false;
        }
        if let Some(code) = self.jsonrpc_error_code
            && observed.jsonrpc_error_code != Some(code)
        {
            return false;
        }
        if let Some(path) = &self.json_path {
            let Some(found) = observed
                .body
                .as_deref()
                .and_then(|body| serde_json::from_str::<Value>(body).ok())
                .and_then(|json| lookup_path(&json, path).cloned())
            else {
                return false;
            };
            if let Some(expected) = &self.equals
                && !values_equal(&found, expected)
            {
                return false;
            }
        }
        true
    }
}

#[must_use]
pub fn matches_any(matchers: &[ResponseMatcher], observed: &ObservedResponse) -> bool {
    matchers.iter().any(|matcher| matcher.matches(observed))
}

/// Resolve `$.a.b.0.c` style paths. Only object keys and array indices.
fn lookup_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let trimmed = path.strip_prefix('$').unwrap_or(path);
    let trimmed = trimmed.strip_prefix('.').unwrap_or(trimmed);
    if trimmed.is_empty() {
        return Some(value);
    }
    trimmed
        .split('.')
        .try_fold(value, |current, segment| match current {
            Value::Object(map) => map.get(segment),
            Value::Array(items) => segment.parse::<usize>().ok().and_then(|i| items.get(i)),
            _ => None,
        })
}

fn values_equal(found: &Value, expected: &Value) -> bool {
    match (found, expected) {
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => a == b,
        (Value::String(a), Value::Number(b)) | (Value::Number(b), Value::String(a)) => {
            *a == b.to_string()
        }
        (a, b) => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn observed(status: u16, body: &str) -> ObservedResponse {
        ObservedResponse {
            http_status: Some(status),
            body: Some(body.to_owned()),
            ..ObservedResponse::default()
        }
    }

    #[test]
    fn empty_matcher_never_matches() {
        assert!(!ResponseMatcher::default().matches(&observed(401, "")));
    }

    #[test]
    fn status_and_body_are_anded() {
        let rule = ResponseMatcher {
            http_status: Some(200),
            body_contains: Some("Missing KEY parameter".into()),
            ..ResponseMatcher::default()
        };
        assert!(rule.matches(&observed(200, "Missing KEY parameter")));
        assert!(!rule.matches(&observed(200, "ok")));
        assert!(!rule.matches(&observed(500, "Missing KEY parameter")));
    }

    #[test]
    fn json_path_compares_text_form() {
        let rule = ResponseMatcher {
            http_status: Some(403),
            json_path: Some("$.code".into()),
            equals: Some(json!("0001")),
            ..ResponseMatcher::default()
        };
        assert!(rule.matches(&observed(403, r#"{"code":"0001","msg":"系统错误"}"#)));
        assert!(!rule.matches(&observed(403, r#"{"code":"0002"}"#)));
        assert!(!rule.matches(&observed(403, "not json")));
        assert!(!rule.matches(&observed(403, r#"{"code":1}"#)));
    }

    #[test]
    fn jsonrpc_code_matches_without_status() {
        let rule = ResponseMatcher {
            jsonrpc_error_code: Some(-32002),
            ..ResponseMatcher::default()
        };
        let seen = ObservedResponse {
            http_status: Some(200),
            jsonrpc_error_code: Some(-32002),
            ..ObservedResponse::default()
        };
        assert!(rule.matches(&seen));
        assert!(matches_any(&[ResponseMatcher::default(), rule], &seen));
    }

    #[test]
    fn nested_paths_and_indices() {
        let value = json!({"a": {"b": [{"c": 7}]}});
        assert_eq!(lookup_path(&value, "$.a.b.0.c"), Some(&json!(7)));
        assert_eq!(lookup_path(&value, "$.a.x"), None);
    }
}
