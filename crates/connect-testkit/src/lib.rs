//! Test-only helpers: configurable mock merchant MCP servers.
//!
//! The mocks speak just enough Streamable HTTP MCP (JSON and SSE responses,
//! `Mcp-Session-Id`) for watt-connect's client, and reproduce how each real
//! merchant reports a missing/invalid credential, rate limiting and protocol
//! version rejection.

pub mod mock_merchant;
pub mod profiles;

pub use mock_merchant::{
    AuthFailureStyle, CredentialCheck, MockConfig, MockMerchant, MockTool, RateLimitStyle,
};
