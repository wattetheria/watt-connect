//! Call policy for proxied merchant tools. Pure logic, no IO, so every rule
//! is unit-testable:
//!
//! * an `order` tool needs a successful preview call in the same session,
//! * payment links in order results are passed through verbatim with an
//!   explicit "the user must pay personally" notice.

pub mod payment;
pub mod preview_gate;
pub mod session;

pub use payment::{PaymentLink, extract_payment_links, payment_notice};
pub use preview_gate::{GateDecision, ToolPolicy, check};
pub use session::{PREVIEW_TTL_SECS, PolicySession};
