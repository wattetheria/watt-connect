# 架构

设计依据见 [WATT_CONNECT_PLAN.md](WATT_CONNECT_PLAN.md)。本文描述代码实际怎么组织、请求怎么流动。

## 进程与数据流

```
agent ──stdio MCP──▶ watt-connect serve ──Streamable HTTP（带用户 token）──▶ 商家 MCP
                          │
                          ├─ 每 6 小时 ──HTTPS（不带任何 token）──▶ 签名目录（GitHub Pages）
                          └─ 本机：config.toml / 系统 keychain / 目录缓存 / 日志
```

CLI 子命令（`connect`、`install`、`status`…）与 `serve` 共用同一个 `ConnectContext`，
所以终端里 `watt-connect connect` 之后，正在运行的 `serve` 立刻能用：`config.toml`
每次访问都重新读取，upstream 连接池按"URL + token 指纹"自动换会话。

## Crate

| crate | 职责 | 关键类型 |
|---|---|---|
| `connect-types` | 无 IO 的共享类型 | `ServiceId`、`ToolEffect`、`ConnectionMode`、`ConnectErrorCode`、`ResponseMatcher`、`CredentialPlacement` |
| `connect-catalog` | 目录 schema、校验、签名校验、ETag 拉取、缓存、搜索、内置快照 | `Catalog`、`ServerEntry`、`CatalogStore` |
| `connect-vault` | keychain / 加密文件、脱敏 | `Vault`、`Secret`、`redact` |
| `connect-upstream` | 连商家：凭证注入、协议版本协商、失败分类、限流退避、会话池 | `UpstreamPool`、`UpstreamTarget`、`ObservingClient` |
| `connect-policy` | 纯逻辑：先预览后下单、支付链接提取 | `PolicySession`、`check`、`extract_payment_links` |
| `connect-installer` | 直连模式写 agent 配置 | `AgentInstaller`（Claude Code / Cursor / Codex） |
| `connect-server` | 6 个 meta tools 与 rmcp `ServerHandler` | `ConnectServer`、`ConnectContext`、`ConnectionStore` |
| `connect-testkit` | axum mock 商家，复现三家行为 | `MockMerchant`、`profiles::{didi,luckin,mcd}` |

## 目录加载顺序

1. `WATT_CONNECT_DEV_CATALOG`（未签名，仅开发）；否则
2. 本地缓存（每次加载都重新验签）与**编译进二进制的快照**中版本较新的那个；
3. `serve` 启动后立即、之后每 6 小时带 `If-None-Match` 拉远端，验签通过且版本更新才替换并写缓存。

`catalog/catalog.pub` 里没有公钥时，远端目录一律拒绝，只用内置快照——这让 P1 在目录服务就绪前也能工作。

## 调用商家（connect-upstream）

- 协议层用官方 `rmcp` 3.8 的 Streamable HTTP 客户端（会话、SSE、JSON-RPC 关联都由它处理）。
- HTTP 层用自己的 `ObservingClient`（实现 rmcp 的 `StreamableHttpClient`）：
  凭证 header 在这里注入；每次失败都记录**原始状态码、Content-Type、截断后的 body、JSON-RPC 错误码、Retry-After**。
  这是 §4.3 "动工前确认"那一项的答案：rmcp 默认后端会丢掉滴滴 `200 text/plain` 这类响应的 body，所以换后端，而不是换语言。
- `classify`：先匹配目录 `auth_failure` → `CREDENTIAL_INVALID`；再 HTTP 429 或 `rate_limited` → `RATE_LIMITED`；
  再看是否版本被拒；没有任何 HTTP 响应 → `UPSTREAM_UNAVAILABLE`；其他 → `UPSTREAM_ERROR`。
- 协议版本：先发 SDK 支持的、仍带 `initialize` 握手的最新版本（`2025-11-25`），失败再按目录 `protocol_versions` 从新到旧重试；
  鉴权失败、限流、网络不可达不重试。
- 限流：只有 `RATE_LIMITED` 会重试（最多 2 次，指数退避，尊重 `Retry-After`，上限 10 秒）。
  网络错误**不**自动重试——下单请求可能已经被商家处理，重试会重复下单。

## call_service 的检查顺序

1. 目录里有这个服务、未 `revoked`、允许代理；
2. 已连接且是 proxy 模式；
3. 已知 `credential_invalid` → 直接返回固定提示和 `portal_url`，不再打商家；
4. keychain 里有 token；
5. preview gate：`policy.preview_before[tool]` 对应的预览工具在本会话 30 分钟内成功过；
6. 调用；鉴权失败则标记 `credential_invalid`；
7. 成功：预览工具记入会话；下单工具消耗预览，并在结果末尾追加"需要用户本人完成支付"和提取到的支付链接。

agent 会话 = 一个 `serve` 进程（stdio），因此预览记录是进程内状态。

## 与计划的差异

见 [WATT_CONNECT_PLAN.md 的实现记录](WATT_CONNECT_PLAN.md#实现记录2026-10-29)。
