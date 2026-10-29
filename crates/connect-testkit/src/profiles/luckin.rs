//! Luckin: `Authorization: Bearer`; HTTP 401 for missing and invalid tokens.

use crate::mock_merchant::{
    AuthFailureStyle, CredentialCheck, MockConfig, MockTool, RateLimitStyle,
};
use serde_json::json;

#[must_use]
pub fn config(valid_token: &str) -> MockConfig {
    MockConfig {
        name: "luckin-mock".into(),
        credential: CredentialCheck::Header {
            name: "authorization".into(),
            prefix: "Bearer ".into(),
        },
        valid_token: Some(valid_token.into()),
        auth_style: AuthFailureStyle::Luckin,
        accepted_versions: Vec::new(),
        rate_limit_style: RateLimitStyle::Http429,
        tools: vec![
            MockTool::new(
                "queryShopList",
                json!({"shops": [{"shopId": "s1", "name": "望京店"}]}),
            ),
            MockTool::new(
                "searchProductForMcp",
                json!({"products": [{"productId": "p1", "name": "生椰拿铁"}]}),
            ),
            MockTool::new(
                "previewOrder",
                json!({"totalPrice": 19.9, "previewId": "pv-1"}),
            ),
            MockTool::new(
                "createOrder",
                json!({"orderId": "lk-1", "payUrl": "https://pay.lkcoffee.example/lk-1"}),
            ),
            MockTool::new("cancelOrder", json!({"cancelled": true})),
        ],
        sse: false,
    }
}
