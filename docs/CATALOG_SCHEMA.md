# 目录 Schema

每个商家一个 MCP Registry `server.json` 文档（`catalog/services/<name 中 / 换成 .>.json`），
watt-connect 需要的扩展字段全部放在 `_meta["watt/connect"]`。机器可读的约束见
[`catalog/schema/watt-connect-meta.schema.json`](../catalog/schema/watt-connect-meta.schema.json)，
语义校验见 `cargo run -p watt-connect-catalog -- validate`（CI 两者都跑）。

## 顶层（server.json）

| 字段 | 说明 |
|---|---|
| `name` | `反向域名/名称`，如 `cn.lkcoffee/order`；即 `ServiceId` |
| `title` / `description` / `version` / `websiteUrl` | 展示用；`version` 用 `YYYY.MM.DD` |
| `remotes[]` | `{ "type": "streamable-http", "url": … }`，代理模式必需（https；仅 loopback 允许 http） |
| `packages[]` | 本地 stdio 包（如 `npx wattetheria mcp-proxy`），只用于直连安装 |

## `_meta["watt/connect"]`

| 字段 | 必填 | 说明 |
|---|---|---|
| `status` | 是 | `active` / `deprecated` / `revoked`。`revoked` 后不再代理、不出现在搜索结果 |
| `category` | | 搜索分类，如 `["food","coffee"]` |
| `credential` | 有远端时应填 | 见下 |
| `auth_failure[]` | 有凭证时必填 | 任一匹配即 `CREDENTIAL_INVALID` |
| `rate_limited[]` | | 任一匹配即 `RATE_LIMITED`（HTTP 429 总是算限流）。**计划外新增**，用于滴滴 `-32001` |
| `protocol_versions[]` | 代理时建议 | 已知可用版本，从新到旧 |
| `modes` | 是 | `{ "proxy": bool, "direct": bool, "default"?: "proxy"\|"direct" }`。`default` **计划外新增**，麦当劳为 `direct` |
| `policy.preview_before` | | `{ 下单工具: 预览工具 }` |
| `policy.payment_link_fields[]` | | 结果里表示支付链接的字段名 |
| `tools[]` | | `{ name, effect: read\|write\|order }` 摘要；真实 schema 以商家 `tools/list` 为准 |
| `sandbox.url` | | 沙箱 endpoint（滴滴） |
| `notes[]` | | 给用户看的注意事项。**计划外新增** |

### credential

```json
{
  "kind": "static_token",
  "placement": { "in": "header", "name": "Authorization", "format": "Bearer {token}" },
  "portal_url": "https://open.lkcoffee.com/",
  "ttl_hint_days": 30,
  "redact_patterns": []
}
```

- `placement.in`：`header` 或 `query`；`format` 必须含 `{token}`，省略即 `{token}`。
- `query` 放置时 `redact_patterns` 必填（URL 会出现在日志里）。

### 响应匹配器（`auth_failure` / `rate_limited` 的元素）

同一个匹配器里出现的条件**全部**满足才算匹配；列表中**任一**匹配器匹配即命中。空匹配器 `{}` 永远不匹配（校验也会报错）。

| 条件 | 含义 |
|---|---|
| `http_status` | HTTP 状态码 |
| `body_contains` | 原始 body 子串（区分大小写） |
| `jsonrpc_error_code` | JSON-RPC `error.code` |
| `json_path` + `equals` | body 解析为 JSON 后 `$.a.b.0.c` 处的值；字符串与数字按文本比较（`"0001"` ≠ `1`）。没有 `equals` 时只要求路径存在 |

三家示例：

```json
// 滴滴：失败都是 HTTP 200
"auth_failure": [
  { "http_status": 200, "body_contains": "Missing KEY parameter" },
  { "jsonrpc_error_code": -32002 }
],
"rate_limited": [{ "jsonrpc_error_code": -32001 }]

// 瑞幸
"auth_failure": [{ "http_status": 401 }]

// 麦当劳：缺 token 是 403 + code 0001，无效 token 是 401
"auth_failure": [
  { "http_status": 401 },
  { "http_status": 403, "json_path": "$.code", "equals": "0001" }
]
```

## 校验规则（`validate`）

- `name` 唯一；文件名必须与 `name` 对应；
- 至少允许一种模式；`modes.default` 必须是允许的模式；
- 代理模式需要 `streamable-http` remote；URL 必须 https（loopback 除外）；
- 有 `credential` 时 `auth_failure` 非空；`redact_patterns` 是合法正则；`format` 含 `{token}`；
- 每个 `effect: order` 的工具都必须出现在 `preview_before` 中，且对应的预览工具是 `read`；
- 目录源文件必须与编译进二进制的快照（`crates/connect-catalog/src/bundled.rs`）一致。

## 快照与签名

`watt-connect-catalog build` 把所有条目组装成：

```json
{ "schema_version": 1, "version": "2026.10.29.1767000000", "generated_at": "…", "services": [ … ] }
```

`catalog.sig`：

```json
{ "algorithm": "ed25519", "public_key": "<hex>", "signature": "<hex>" }
```

签名对象是快照 JSON 的 JCS（RFC 8785）规范化字节，算法 `ed25519`、公钥与签名均为 hex（与 watt-registry 的格式一致，由 `connect-catalog/src/verify.rs` 自行实现，直接基于 `ed25519-dalek`）。
客户端只接受 `catalog/catalog.pub` 中列出的公钥，且只接受版本号不小于当前版本的快照。

## 新增一个商家

1. 写 `catalog/services/<ns>.<name>.json`；
2. 在 `crates/connect-catalog/src/bundled.rs` 的 `SOURCES` 加一行；
3. 在 `fixtures/upstream/<merchant>/` 放实测（已脱敏）的鉴权失败响应，`crates/connect-upstream/tests/fixtures.rs` 加映射；
4. 需要的话在 `crates/connect-testkit/src/profiles/` 加 mock；
5. `cargo run -p watt-connect-catalog -- validate && cargo test --workspace`。
