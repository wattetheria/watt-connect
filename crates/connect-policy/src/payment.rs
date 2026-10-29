//! Payment link pass-through. watt-connect never opens, follows or pays a
//! payment link; it only finds them in the merchant's result and tells the
//! agent, explicitly, that the user has to pay personally.

use serde::Serialize;
use serde_json::Value;
use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaymentLink {
    pub field: String,
    pub value: String,
}

const MAX_DEPTH: usize = 12;

/// Collect payment links from a tool result (as JSON). `fields` are the
/// catalog's `payment_link_fields`. Text content blocks that contain JSON are
/// searched too, since many merchants return JSON as text. When no configured
/// field is found, keys containing `pay` whose value is an http(s) URL are
/// reported as a fallback.
#[must_use]
pub fn extract_payment_links(result: &Value, fields: &[String]) -> Vec<PaymentLink> {
    let mut found = Vec::new();
    collect(
        result,
        &|key, value| fields.iter().any(|f| f == key) && !value.is_empty(),
        &mut found,
        0,
    );
    if found.is_empty() {
        collect(
            result,
            &|key, value| {
                key.to_ascii_lowercase().contains("pay")
                    && (value.starts_with("https://") || value.starts_with("http://"))
            },
            &mut found,
            0,
        );
    }
    found.dedup();
    found
}

fn collect(
    value: &Value,
    wanted: &dyn Fn(&str, &str) -> bool,
    out: &mut Vec<PaymentLink>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    match value {
        Value::Object(map) => {
            for (key, item) in map {
                if let Value::String(text) = item {
                    if wanted(key, text) {
                        out.push(PaymentLink {
                            field: key.clone(),
                            value: text.clone(),
                        });
                        continue;
                    }
                    if let Some(embedded) = embedded_json(text) {
                        collect(&embedded, wanted, out, depth + 1);
                    }
                } else {
                    collect(item, wanted, out, depth + 1);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect(item, wanted, out, depth + 1);
            }
        }
        Value::String(text) => {
            if let Some(embedded) = embedded_json(text) {
                collect(&embedded, wanted, out, depth + 1);
            }
        }
        _ => {}
    }
}

fn embedded_json(text: &str) -> Option<Value> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        serde_json::from_str(trimmed).ok()
    } else {
        None
    }
}

/// The notice appended to every order result.
#[must_use]
pub fn payment_notice(links: &[PaymentLink]) -> String {
    let mut text = String::from(
        "【需要用户本人完成支付 / The user must complete payment personally】\n\
         watt-connect 不会打开、跟随或代付支付链接。请把下面的链接原样交给用户，由用户本人支付。\n\
         watt-connect never opens, follows or pays payment links. Hand them to the user unchanged.",
    );
    if links.is_empty() {
        text.push_str(
            "\n未在结果中识别到支付链接；如需支付，请按商家返回内容引导用户在商家 App 内完成。\n\
             No payment link was recognised; if payment is required, guide the user to finish it in the merchant's app.",
        );
    } else {
        for link in links {
            let _ = write!(text, "\n- {}: {}", link.field, link.value);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_configured_fields_in_structured_and_text_content() {
        let result = json!({
            "content": [{"type": "text", "text": "{\"order\":{\"orderId\":\"1\",\"payUrl\":\"https://pay.example/1\"}}"}],
            "structuredContent": {"qrCode": "https://qr.example/1"}
        });
        let links = extract_payment_links(&result, &["payUrl".into(), "qrCode".into()]);
        assert_eq!(
            links,
            vec![
                PaymentLink {
                    field: "payUrl".into(),
                    value: "https://pay.example/1".into()
                },
                PaymentLink {
                    field: "qrCode".into(),
                    value: "https://qr.example/1".into()
                },
            ]
        );
    }

    #[test]
    fn falls_back_to_pay_like_url_keys() {
        let result = json!({"data": {"paymentLink": "https://pay.example/2", "payType": "wechat"}});
        let links = extract_payment_links(&result, &[]);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].value, "https://pay.example/2");
    }

    #[test]
    fn notice_mentions_user_payment() {
        let notice = payment_notice(&[]);
        assert!(notice.contains("用户本人完成支付"));
        let notice = payment_notice(&[PaymentLink {
            field: "payUrl".into(),
            value: "https://p".into(),
        }]);
        assert!(notice.contains("- payUrl: https://p"));
    }
}
