# Upstream response fixtures

Responses recorded during the 2026-10-29 probe (`initialize` with no token and
with a fake token; no orders, no real tokens), plus a few synthetic cases
marked as such. They drive the `auth_failure` / `rate_limited` classification
tests in `crates/connect-upstream/tests/fixtures.rs` against the rules in
`catalog/services/*.json`.

Format: `http_status`, `content_type`, `body`, optional `jsonrpc_error_code` /
`jsonrpc_error_message`, and the `expected` `ConnectErrorCode`.

Fixtures must never contain a real token. Replace P0 recordings here once real
read-only calls have been captured (redact first).
