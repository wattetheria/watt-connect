//! McDonald's: `Authorization: Bearer`; missing token is HTTP 403
//! `{"code":"0001"}`, invalid token is HTTP 401 `text/plain`; HTTP 429 for
//! rate limiting; only protocol versions up to `2025-06-18`.

use crate::mock_merchant::{
    AuthFailureStyle, CredentialCheck, MockConfig, MockTool, RateLimitStyle,
};
use serde_json::json;

#[must_use]
pub fn config(valid_token: &str) -> MockConfig {
    MockConfig {
        name: "mcd-mock".into(),
        credential: CredentialCheck::Header {
            name: "authorization".into(),
            prefix: "Bearer ".into(),
        },
        valid_token: Some(valid_token.into()),
        auth_style: AuthFailureStyle::Mcd,
        accepted_versions: vec!["2025-06-18".into(), "2025-03-26".into()],
        rate_limit_style: RateLimitStyle::Http429,
        tools: vec![
            MockTool::new("calculate-price", json!({"price": 39.0})),
            MockTool::new(
                "create-order",
                json!({"data": {"orderId": "mcd-1", "payLink": "https://pay.mcd.example/mcd-1"}}),
            ),
        ],
        sse: false,
    }
}
