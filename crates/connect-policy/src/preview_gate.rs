//! "Preview before order": an `order` tool may only be called after its
//! catalog-declared preview tool succeeded in the same session.

use crate::session::PolicySession;
use connect_types::ToolEffect;

/// What the catalog says about one tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolPolicy<'a> {
    /// `None` when the catalog does not list the tool.
    pub effect: Option<ToolEffect>,
    /// Preview tool required before this one (`policy.preview_before`).
    pub preview_tool: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    Allow,
    /// Call `preview_tool` successfully first, then ask the user to confirm.
    RequirePreview {
        preview_tool: String,
    },
}

#[must_use]
pub fn check(
    session: &PolicySession,
    service: &str,
    policy: ToolPolicy<'_>,
    now: u64,
) -> GateDecision {
    // A preview mapping always applies; an order tool without one is rejected
    // by catalog validation, so it cannot reach here from a valid catalog.
    match policy.preview_tool {
        Some(preview) if !session.has_fresh_preview(service, preview, now) => {
            GateDecision::RequirePreview {
                preview_tool: preview.to_owned(),
            }
        }
        _ => GateDecision::Allow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDER: ToolPolicy<'static> = ToolPolicy {
        effect: Some(ToolEffect::Order),
        preview_tool: Some("previewOrder"),
    };

    #[test]
    fn order_requires_preview_then_allows() {
        let mut session = PolicySession::new();
        assert_eq!(
            check(&session, "cn.lkcoffee/order", ORDER, 10),
            GateDecision::RequirePreview {
                preview_tool: "previewOrder".into()
            }
        );
        session.record_preview("cn.lkcoffee/order", "previewOrder", 10);
        assert_eq!(
            check(&session, "cn.lkcoffee/order", ORDER, 20),
            GateDecision::Allow
        );
    }

    #[test]
    fn reads_and_unknown_tools_pass() {
        let session = PolicySession::new();
        let read = ToolPolicy {
            effect: Some(ToolEffect::Read),
            preview_tool: None,
        };
        let unknown = ToolPolicy {
            effect: None,
            preview_tool: None,
        };
        assert_eq!(check(&session, "s", read, 0), GateDecision::Allow);
        assert_eq!(check(&session, "s", unknown, 0), GateDecision::Allow);
    }
}
