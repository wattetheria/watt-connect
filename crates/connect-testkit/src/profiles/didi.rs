//! Didi: key in the `key` query parameter; failures come back as HTTP 200;
//! rate limiting is JSON-RPC `-32001`.

use crate::mock_merchant::{
    AuthFailureStyle, CredentialCheck, MockConfig, MockTool, RateLimitStyle,
};
use serde_json::json;

#[must_use]
pub fn config(valid_key: &str) -> MockConfig {
    MockConfig {
        name: "didi-mock".into(),
        credential: CredentialCheck::Query { name: "key".into() },
        valid_token: Some(valid_key.into()),
        auth_style: AuthFailureStyle::Didi,
        accepted_versions: Vec::new(),
        rate_limit_style: RateLimitStyle::JsonRpc(-32001),
        tools: vec![
            MockTool::new(
                "taxi_estimate",
                json!({"estimateId": "est-1", "price": 23.5}),
            ),
            MockTool::new(
                "taxi_create_order",
                json!({"orderId": "didi-1", "status": "dispatching"}),
            ),
        ],
        sse: true,
    }
}
