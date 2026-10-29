# watt-connect

Install **one** MCP server in your personal agent (Claude Code, Cursor, Codex,
OpenClaw, ...) and use official merchant MCP servers through it — first batch:
Luckin Coffee (瑞幸), McDonald's China (麦当劳) and Didi (滴滴). New merchants
show up from the catalog; you never reinstall or edit your agent config.

在 agent 里只装一个 MCP，就能发现和调用国内商家的官方 MCP。商家凭证只存在本机，
云端只放目录，watt-connect 永远不替用户付款。设计文档见
[docs/WATT_CONNECT_PLAN.md](docs/WATT_CONNECT_PLAN.md)。

## Install

Add to your agent's MCP configuration:

```json
{ "mcpServers": { "watt-connect": { "command": "npx", "args": ["-y", "watt-connect"] } } }
```

For Claude Code:

```bash
claude mcp add --scope user watt-connect -- npx -y watt-connect
```

## Use

Ask your agent, e.g. "帮我在瑞幸点一杯生椰拿铁". It will:

1. `search_services` → find `cn.lkcoffee/order`
2. `connect_service` → tell **you** to enter your token (never in chat):

   ```bash
   npx watt-connect connect cn.lkcoffee/order
   ```

   or open the one-time `http://127.0.0.1:<port>/connect/<nonce>` page it shows.
   The token is verified with the merchant, then stored in your system keychain.
3. `describe_service` → the merchant's live tools and schemas
4. `call_service` → `queryShopList` → `previewOrder` → (your confirmation) →
   `createOrder` → a payment link that **you** open and pay.

### Meta tools

| Tool | What it does |
|---|---|
| `search_services(query, category?)` | Search the catalog; shows whether each service is connected |
| `describe_service(service)` | Live `tools/list` when connected, catalog summary otherwise |
| `connect_service(service, mode?)` | Instructions for the user to enter a token (never accepts one) |
| `call_service(service, tool, arguments)` | Proxy a merchant tool call (preview-before-order enforced) |
| `install_service(service, agent)` | Direct mode: write the merchant into Claude Code / Cursor / Codex config |
| `list_connections()` | Connected services, mode, credential status |

### CLI

```text
watt-connect                      serve MCP over stdio (default)
watt-connect search [query]       search the catalog
watt-connect connect <service>    enter + verify + store a token (proxy mode)  [--web] [--sandbox] [--token-stdin]
watt-connect install <service> --agent <claude-code|cursor|codex>   direct mode
watt-connect uninstall <service> --agent <agent>
watt-connect disconnect <service> forget token and connection
watt-connect status               connections, credential status, catalog version
```

### Proxy vs direct

Per service. **Proxy** (default): agent → watt-connect (local) → merchant; the
token lives in the keychain; catalog updates need no agent changes. **Direct**:
the merchant is written into the agent's own config and watt-connect keeps no
copy of the token. McDonald's defaults to direct mode until commercial terms
are cleared; the local Wattetheria node is direct-only.

## Configuration

| Variable | Meaning |
|---|---|
| `WATT_CONNECT_HOME` | Put config, cache, vault fallback and logs under one directory |
| `WATT_CONNECT_VAULT` | `auto` (default), `keyring`, or `file` (encrypted file fallback) |
| `WATT_CONNECT_CATALOG_URL` | Override the signed catalog URL |
| `WATT_CONNECT_DEV_CATALOG` | Use an **unsigned** local catalog file (development only) |
| `WATT_CONNECT_NO_WEB` | Disable the local token page offered by `connect_service` |
| `WATT_CONNECT_LOG` | Log filter, e.g. `debug` (logs never contain arguments, results or tokens) |
| `WATT_CONNECT_BINARY` | npm launcher: use this binary instead of the platform package |

## Development

The workspace is self-contained: a plain `git clone` builds with no other
repositories checked out.

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

```bash
cargo run -p watt-connect-catalog -- validate
```

Tests run entirely against local mock merchants (`crates/connect-testkit`),
including an end-to-end test that drives the real binary over stdio.

Docs: [architecture](docs/ARCHITECTURE.md) · [catalog schema](docs/CATALOG_SCHEMA.md) ·
[security](docs/SECURITY.md) · [plan](docs/WATT_CONNECT_PLAN.md)

## License

Apache-2.0, see [LICENSE](LICENSE).
