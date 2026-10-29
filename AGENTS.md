# Agent guide for watt-connect

## Layout

- `apps/watt-connect` — the shipped binary (stdio MCP server + CLI).
- `apps/watt-connect-catalog` — catalog validate/build/sign tool. Not shipped. The only place a signing key is used.
- `crates/connect-*` — libraries. Dependency direction is strictly downward:
  `connect-server` → {`connect-catalog`, `connect-vault`, `connect-upstream`, `connect-policy`, `connect-installer`} → `connect-types`.
  `connect-upstream` must not know about meta tools; `connect-policy` must not do IO.
- `catalog/services/*.json` — catalog source (MCP registry `server.json` + `_meta["watt/connect"]`).
  Adding a service also requires listing it in `crates/connect-catalog/src/bundled.rs`.
- `fixtures/upstream` — recorded merchant responses; never put real tokens there.
- `npm/` + `scripts/` — npm launcher and six platform packages.

## Rules

- Tokens never go to the agent, config files, logs, or our servers. Do not add
  a meta tool argument that accepts a credential.
- stdout is the MCP channel in `serve`; log with `tracing` only (stderr / log file).
- Never log tool arguments or results; log service, tool, elapsed time, outcome.
- Never open, follow or act on payment links.
- Merchant-specific behaviour belongs in the catalog (`auth_failure`,
  `rate_limited`, `policy`), not in code.

## Checks before committing

```bash
cargo fmt --all -- --check
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

```bash
cargo test --workspace
```

```bash
cargo run -p watt-connect-catalog -- validate
```

```bash
(cd npm && npm run check)
```

The workspace is self-contained; do not add path or git dependencies on other repositories.
