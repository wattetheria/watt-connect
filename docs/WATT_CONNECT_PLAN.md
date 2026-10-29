# 商家 MCP 统一入口：实现方案（草案）

> 状态：草案，2026-10-29。项目名 `watt-connect`，仓库目录 `/Users/sac/Desktop/Watt/watt-connect`。
> 这是一个独立的新项目，不属于 wattetheria 节点；仓库初始化后本文移到 `watt-connect/docs/`。

## 1. 目标

用户在自己的 personal agent（Claude Code、Cursor、Codex、OpenClaw 等）里**只装一个 MCP**，就能发现和调用国内商家的官方 MCP（首批：滴滴、瑞幸、麦当劳）。之后有新商家上线，不需要重装、不需要改 agent 配置。

约束：

- **商家凭证只存在用户本机**，绝不上传到我们的云端。
- **我们的云端只放元数据**（目录），不经手任何调用流量。
- **永远不替用户付款**，只把商家返回的支付链接原样交给用户。

非目标：支付代扣、跨商家账号打通、替用户注册商家账号。

## 2. 模式 C：代理为主，直连可选

| | 代理模式（默认） | 直连模式（可选） |
|---|---|---|
| agent 连谁 | 只连 `watt-connect` | 直接连商家 MCP |
| 新商家上线 | 目录更新后自动可用，agent 不用改配置 | 要往 agent 配置里加一条，可能还要重启 |
| token 存哪 | `watt-connect` 的本地存储（系统 keychain） | agent 自己的配置/存储 |
| 调用路径 | agent → watt-connect（本机）→ 商家 | agent → 商家 |
| 适用 | 绝大多数用户 | 想让 agent 直连的用户；目录标记为不允许代理的商家；本地 stdio 型服务（如 wattetheria） |

模式按**商家**选，不按用户全局选：每个已连接的商家记录自己是 `proxy` 还是 `direct`。

## 3. 首批商家实测结论

2026-10-29 用无 token / 假 token 的 `initialize` 请求实测（没有下单，没有用真实 token）：

| | 滴滴 | 瑞幸 | 麦当劳 |
|---|---|---|---|
| Endpoint | `https://mcp.didichuxing.com/mcp-servers?key=…`<br>沙箱 `/mcp-servers-sandbox?key=…` | `https://gwmcp.lkcoffee.com/order/user/mcp` | `https://mcp.mcd.cn` |
| 凭证位置 | URL query `key` | `Authorization: Bearer` | `Authorization: Bearer` |
| 缺少凭证 | HTTP 200，`text/plain`：`Missing KEY parameter` | HTTP 401，JSON `{"message":"oauth token is not found"}` | HTTP **403**，JSON `{"code":"0001","msg":"系统错误"}` |
| 凭证无效 | HTTP **200**，JSON-RPC error `-32002 auth failed` | HTTP 401，JSON `{"message":"访问令牌无效或已过期"}` | HTTP 401，`text/plain`：`当前鉴权码不存在或已失效!` |
| OAuth 发现 | 无 | 401 带 `WWW-Authenticate: Bearer resource_metadata=…`（RFC 9728），但该 metadata 地址从我们这边访问返回 403 | 无 |
| Tools 数（据文档） | 13 | 8 | 33 |
| 预览/算价工具 | `taxi_estimate` | `previewOrder` | `calculate-price` |
| 下单工具 | `taxi_create_order` | `createOrder` | `create-order`、`mall-create-order`、`party-order-create` |
| 支付 | 正式环境真实扣费 | 返回支付链接/二维码，本人支付 | 返回支付链接 |

结论：**"凭证失效"的判断方式三家完全不一样**，不能只看 HTTP 401。这些规则必须写进目录（见 §5 `auth_failure`），由 watt-connect 统一转成一个错误类型。

## 4. 组件

```
┌──────────── 用户本机 ─────────────────────────────────────┐
│  Personal agent                                           │
│     │ stdio MCP                                           │
│     ▼                                                     │
│  watt-connect (Rust 单二进制，npm 分发)                         │
│   ├─ connect-server     对 agent 暴露 meta tools               │
│   ├─ connect-catalog    拉取/校验/缓存目录                      │
│   ├─ connect-vault      token 存取（keychain）                  │
│   ├─ connect-upstream   Streamable HTTP 客户端连接池            │
│   ├─ connect-policy     先预览后下单、支付链接透传               │
│   ├─ connect-installer  直连模式：写 agent 配置                  │
│   └─ CLI            connect / disconnect / status / install │
│     │ HTTPS（带用户 token，直连商家）                        │
└─────┼─────────────────────────────────────────────────────┘
      ▼
  滴滴 / 瑞幸 / 麦当劳 官方 MCP

  watt-connect ──HTTPS（只拉目录，不带任何 token）──▶ 目录服务
```

### 4.1 本地 `watt-connect`

- Rust 单二进制，按 wattetheria 的方式通过 npm 分发，用户配置写 `npx watt-connect`。技术栈见 §4.3，代码目录见 §4.4。
- 对 agent 是 stdio MCP server，对商家是 Streamable HTTP MCP client。
- wattetheria 现有的 MCP 是自己实现的协议层，这里不复用：watt-connect 主要是**客户端**角色，需要完整的版本协商、SSE、session 处理，直接用官方 SDK。

### 4.2 云端目录服务

- **P1 阶段**：不建服务，直接发布一个**签名的静态 JSON**（CDN 或对象存储），带版本号。三家商家手工维护即可。
- **P3 阶段**：迁到 `watt-servicenet`，在 `service-protocol` 里新增服务类型 `mcp_server`，复用它已有的 provider 注册、审核、治理和 Iroh 同步，并提供 `GET /v1/mcp-catalog?since=<version>`。只用 ServiceNet 的**注册与目录**能力，**不用 Relay 网关**，因为 Relay 会让调用经过网关，违反"流量和 token 不出本机"。

### 4.3 技术栈

**选型原则**：和 wattetheria / watt-servicenet / watt-registry 保持一致（Rust 2024、tokio、reqwest + rustls、axum、thiserror），能复用的 Watt 库直接复用，不引入第二种语言。

| 用途 | 选型 | 说明 |
|---|---|---|
| 语言 | Rust，edition 2024 | 与现有仓库一致；`rust-toolchain.toml` 固定版本 |
| MCP 协议 | `rmcp` 3.8.x（官方 Rust SDK，2026-10-26 仍在更新） | stdio server（对 agent）+ Streamable HTTP client（对商家）。**动工前确认**：client 端能否自定义 header / query、指定 `protocolVersion`、拿到原始 HTTP 状态码和响应体（`auth_failure` 分类要用） |
| 异步运行时 | `tokio` 1.44+ | |
| HTTP 客户端 | `reqwest` 0.12，`rustls-tls` | 不依赖系统 OpenSSL，便于跨平台出二进制 |
| 本机网页（录 token） | `axum` 0.8 | 只绑定 `127.0.0.1`，随机端口 + 一次性 nonce，录完即关 |
| 凭证存储 | `keyring` 4.x | macOS Keychain / Windows Credential Manager / Linux Secret Service；无 keychain 时退回加密文件 |
| 加密文件兜底 | `chacha20poly1305` | 与 watt-servicenet 一致 |
| 目录签名 | `watt-credential`（Ed25519）+ `serde_jcs` | JCS 规范化后签名，与 watt-registry 的做法一致 |
| 序列化 | `serde`、`serde_json` | |
| CLI | `clap` 4.5（derive） | |
| 本地路径 | `directories` | 配置、缓存目录按平台规范放 |
| 本地配置 | `toml` | 只存"已连接哪些商家、用什么模式"，**不存 token** |
| 错误 | `thiserror`（库）、`anyhow`（仅 apps） | |
| 日志 | `tracing` + `tracing-subscriber` | **只写 stderr 或日志文件**，stdout 留给 MCP 协议；统一走 `connect-vault::redact` 脱敏 |
| 测试 | `tokio::test`、`connect-testkit`（axum 写的 mock 商家）、`tempfile` | 不引入额外 mock 框架 |
| 分发 | npm 主包 + 6 个平台二进制包（`optionalDependencies`） | 照搬 wattetheria 的 `npm/native/*` 和 `npm-native-cli-release` 流程 |
| CI | GitHub Actions | `fmt` / `clippy -D warnings` / `test`；发版时 6 平台交叉编译 |

**什么情况下改用 TypeScript**：官方 TS SDK（`@modelcontextprotocol/sdk`，当前 1.32.x）是协议参考实现，新特性通常先出现在它那里。只有当 P0 发现 `rmcp` 缺了关键能力（例如走瑞幸标准 OAuth 所需的客户端流程，或者拿不到原始响应做 `auth_failure` 分类）且短期补不上时，才考虑把 `connect-upstream` 换成 TS。三家目前都是静态 token，Rust 足够。

**运行时数据位置**（通过 `directories` 取平台目录）：

| 内容 | 位置（macOS 示例） | 说明 |
|---|---|---|
| 连接配置 | `~/Library/Application Support/watt-connect/config.toml` | 商家 → 模式（proxy/direct）、连接时间 |
| 目录缓存 | `~/Library/Caches/watt-connect/catalog.json` + `catalog.sig` + `etag` | 离线可用 |
| token | 系统 keychain，条目名 `watt-connect/<service-name>` | 兜底文件：`…/watt-connect/vault.enc`，权限 `0600` |
| 日志 | `~/Library/Logs/watt-connect/` | 已脱敏，不含参数和返回内容 |

### 4.4 代码目录结构

新仓库 `watt-connect`：

```
watt-connect/
├── Cargo.toml                    workspace，统一依赖版本
├── rust-toolchain.toml
├── AGENTS.md / CLAUDE.md         协作规则（参照 wattetheria）
├── README.md
├── LICENSE*                      待定，见 §14
│
├── apps/
│   ├── watt-connect/                 用户安装的二进制
│   │   └── src/
│   │       ├── main.rs           入口：无子命令时 = `serve`（stdio MCP）
│   │       ├── cli.rs            clap 定义
│   │       ├── config.rs         config.toml 读写
│   │       ├── connect_web.rs    本机录 token 的临时网页
│   │       └── commands/
│   │           ├── serve.rs      组装各 crate，启动 stdio MCP server
│   │           ├── connect.rs    `watt-connect connect <service>`
│   │           ├── disconnect.rs
│   │           ├── status.rs     已连接商家 + 凭证状态
│   │           └── install.rs    `watt-connect install <service> --agent <name>`（直连模式）
│   │
│   └── watt-connect-catalog/         运维工具，不随 npm 发布
│       └── src/main.rs           `validate` / `sign` / `diff` 目录；私钥从环境变量或 KMS 读
│
├── crates/
│   ├── connect-types/                共享类型，无 IO
│   │   └── src/lib.rs            ServiceId、ToolEffect、ConnectionMode、ConnectErrorCode
│   │
│   ├── connect-catalog/              目录：解析、校验、拉取、缓存、搜索
│   │   └── src/
│   │       ├── schema.rs         server.json + `_meta["watt/connect"]` 结构
│   │       ├── verify.rs         签名校验（watt-credential）
│   │       ├── fetch.rs          带 ETag 的增量拉取
│   │       ├── cache.rs          本地缓存读写
│   │       └── search.rs         内存检索（名称/分类/描述）
│   │
│   ├── connect-vault/                凭证与脱敏
│   │   └── src/
│   │       ├── lib.rs            `Vault` trait
│   │       ├── keyring.rs        系统 keychain 实现
│   │       ├── file.rs           加密文件兜底
│   │       └── redact.rs         按 redact_patterns 脱敏 URL / 文本
│   │
│   ├── connect-upstream/             连商家
│   │   └── src/
│   │       ├── pool.rs           每个商家一个 rmcp client，懒连接、复用 session
│   │       ├── credential.rs     按 placement 注入 header 或 query
│   │       ├── negotiate.rs      协议版本协商与降级重试
│   │       ├── classify.rs       auth_failure / 限流 / 其他错误 → ConnectErrorCode
│   │       └── backoff.rs        限流指数退避
│   │
│   ├── connect-policy/               调用约束，纯逻辑、无 IO
│   │   └── src/
│   │       ├── session.rs        每个 agent session 的预览记录
│   │       ├── preview_gate.rs   order 类工具必须先有预览
│   │       └── payment.rs        从结果中提取支付链接并加提示
│   │
│   ├── connect-installer/            直连模式写 agent 配置
│   │   └── src/
│   │       ├── lib.rs            `AgentInstaller` trait：detect/install/uninstall/list
│   │       ├── backup.rs         改配置前备份
│   │       ├── claude_code.rs    优先调用 `claude mcp add`
│   │       ├── cursor.rs
│   │       └── codex.rs
│   │
│   ├── connect-server/               对 agent 暴露的 MCP server
│   │   └── src/
│   │       ├── server.rs         rmcp server、工具注册
│   │       └── tools/
│   │           ├── search.rs     search_services
│   │           ├── describe.rs   describe_service
│   │           ├── connect.rs    connect_service
│   │           ├── call.rs       call_service
│   │           ├── install.rs    install_service
│   │           └── list.rs       list_connections
│   │
│   └── connect-testkit/              仅 dev-dependency
│       └── src/
│           ├── mock_merchant.rs  axum 写的可配置 mock MCP 商家
│           └── profiles/         复现三家的鉴权失败、限流、版本拒绝
│               ├── didi.rs
│               ├── luckin.rs
│               └── mcd.rs
│
├── catalog/                      目录源文件（人工维护，CI 校验后签名发布）
│   ├── services/
│   │   ├── cn.didichuxing.ride.json
│   │   ├── cn.lkcoffee.order.json
│   │   ├── cn.mcd.order.json
│   │   └── io.wattetheria.node.json
│   ├── schema/watt-connect-meta.schema.json
│   └── catalog.pub               签名公钥（私钥绝不进仓库）
│
├── fixtures/upstream/            P0 实测录下的真实响应（已脱敏），给 classify 的单测用
│   ├── didi/
│   ├── luckin/
│   └── mcd/
│
├── tests/                        端到端：stdio 驱动 watt-connect ↔ connect-testkit mock 商家
│
├── npm/
│   ├── package.json              主包，`bin: watt-connect`，optionalDependencies 指向平台包
│   ├── bin/watt-connect.js           找到对应平台二进制并 exec
│   ├── lib/native.js
│   └── native/                   darwin-arm64 / darwin-x64 / linux-arm64 / linux-x64 / win32-arm64 / win32-x64
│
├── scripts/                      打包、暂存平台二进制（参照 wattetheria/scripts）
│
├── docs/
│   ├── ARCHITECTURE.md
│   ├── CATALOG_SCHEMA.md         `_meta["watt/connect"]` 字段说明
│   └── SECURITY.md               凭证、脱敏、支付边界
│
└── .github/workflows/
    ├── ci.yml                    fmt / clippy / test / 目录 schema 校验
    ├── release.yml               6 平台构建 + npm 发布
    └── catalog-publish.yml       目录变更 → 签名 → 上传 CDN
```

**crate 依赖方向**（只能向下依赖，不能反向）：

```
apps/watt-connect
   └── connect-server
         ├── connect-catalog ─┐
         ├── connect-vault ───┤
         ├── connect-upstream ┼── connect-types
         ├── connect-policy ──┤
         └── connect-installer┘
```

- `connect-upstream` 只认识"service + 凭证 + 请求"，不知道 meta tools 的存在。
- `connect-policy` 不做 IO，方便单测。
- `connect-installer` 只被 `connect-server`（`install_service`）和 CLI（`install` 子命令）调用。
- `apps/watt-connect-catalog` 只依赖 `connect-catalog` 和 `connect-types`，签名私钥只出现在这个工具里。

**npm 包名**：`npx watt-connect` 要求主包是无 scope 的 `watt-connect`；如果用 `@wattetheria/connect`，用户就得写 `npx @wattetheria/connect`。平台包沿用 `@wattetheria/connect-<platform>` 的命名。npm 包名待定。

## 5. 目录 Schema

在 MCP 官方 Registry 的 `server.json` 格式上扩展，扩展字段放在 `_meta["watt/connect"]` 下，避免自造一套。示例（瑞幸）：

```json
{
  "name": "cn.lkcoffee/order",
  "title": "瑞幸咖啡",
  "description": "查门店、搜商品、预览价格、下单",
  "version": "2026.10.29",
  "remotes": [
    { "type": "streamable-http", "url": "https://gwmcp.lkcoffee.com/order/user/mcp" }
  ],
  "_meta": {
    "watt/connect": {
      "status": "active",
      "category": ["food", "coffee"],
      "credential": {
        "kind": "static_token",
        "placement": { "in": "header", "name": "Authorization", "format": "Bearer {token}" },
        "portal_url": "https://open.lkcoffee.com/",
        "ttl_hint_days": 30,
        "redact_patterns": []
      },
      "auth_failure": [
        { "http_status": 401 }
      ],
      "protocol_versions": ["2025-06-18", "2025-03-26"],
      "modes": { "proxy": true, "direct": true },
      "policy": {
        "preview_before": { "createOrder": "previewOrder" },
        "payment_link_fields": ["payUrl", "qrCode"]
      },
      "tools": [
        { "name": "queryShopList", "effect": "read" },
        { "name": "searchProductForMcp", "effect": "read" },
        { "name": "previewOrder", "effect": "read" },
        { "name": "createOrder", "effect": "order" },
        { "name": "cancelOrder", "effect": "write" }
      ]
    }
  }
}
```

滴滴的差异部分：

```json
"credential": {
  "kind": "static_token",
  "placement": { "in": "query", "name": "key" },
  "portal_url": "https://mcp.didichuxing.com/",
  "redact_patterns": ["key=[^&]+"]
},
"auth_failure": [
  { "http_status": 200, "body_contains": "Missing KEY parameter" },
  { "jsonrpc_error_code": -32002 }
],
"sandbox": { "url": "https://mcp.didichuxing.com/mcp-servers-sandbox" }
```

麦当劳的差异部分：

```json
"auth_failure": [
  { "http_status": 401 },
  { "http_status": 403, "json_path": "$.code", "equals": "0001" }
]
```

字段说明：

- `credential.placement`：凭证放哪（`header` / `query`）。以后出现 OAuth 商家时，`kind` 加 `oauth`。
- `auth_failure`：任意一条命中就判定为"凭证失效"，watt-connect 统一转成 `CREDENTIAL_INVALID`，并告诉用户去 `portal_url` 重新获取。
- `tools[].effect`：`read` / `write` / `order`。只是**摘要**，真实 schema 以连接后商家返回的 `tools/list` 为准，目录不复制完整 schema。
- `policy.preview_before`：下单前必须先成功调用过对应的预览工具（见 §7）。
- `payment_link_fields`：从返回结果里提取支付链接的字段名。具体字段名在 P0 用真实 token 确认。

**签名与更新**：

- 整个目录快照用 Ed25519 签名，公钥内置在二进制里。可以复用 `watt-credential` 的签名能力。
- watt-connect 启动时、之后每 6 小时，带 `ETag` 拉一次目录。拉取失败就用本地缓存，离线也能用已连接的商家。
- `status: "revoked"` 的条目：watt-connect 停止代理该商家，并提示用户。

## 6. Meta Tools（watt-connect 对 agent 暴露）

agent 永远只看到这几个工具，商家再多它们也不变：

| 工具 | 作用 |
|---|---|
| `search_services(query, category?)` | 在目录里搜商家，返回名称、简介、是否已连接 |
| `describe_service(service)` | 已连接：返回商家实时 `tools/list` 的完整 schema；未连接：返回目录里的摘要和连接方式 |
| `connect_service(service)` | 开始连接流程，返回给用户的操作说明（**不接收 token**，见 §8） |
| `call_service(service, tool, arguments)` | 代理模式下调用商家工具 |
| `install_service(service, agent)` | 直连模式：把商家写进指定 agent 的配置 |
| `list_connections()` | 已连接的商家、模式、凭证状态 |

可选增强（P2）：对支持 `notifications/tools/list_changed` 的客户端，把用户**常用**的商家工具以 `luckin__createOrder` 这类带命名空间的形式直接暴露，省掉一次 `describe_service`。默认关闭，避免工具数量膨胀。

## 7. 调用策略（`connect-policy`）

- **先预览后下单**：调用 `effect: "order"` 的工具前，同一个 session 里必须有一次对应预览工具的成功调用，否则 watt-connect 直接返回错误，提示 agent 先预览、再让用户确认。
- **支付透传**：从下单结果里提取支付链接，原样返回，并在结果里明确写"需要用户本人完成支付"。watt-connect 不打开、不跟随、不处理支付链接。
- **限流**：麦当劳 429、滴滴 `-32001` 统一转成 `RATE_LIMITED`，watt-connect 做指数退避，最多重试 2 次。
- **日志**：只记录 service、tool、耗时、结果类型，**不记录参数和返回内容**；所有 URL 按 `redact_patterns` 脱敏。

## 8. 凭证（`connect-vault`）

**存储**：

- 首选系统 keychain：macOS Keychain、Windows Credential Manager、Linux Secret Service（Rust `keyring` crate）。
- 没有 keychain 的环境（如无桌面 Linux）：本地加密文件，权限 `0600`。
- key 命名：`watt-connect/<service-name>`。token 不进配置文件、不进日志、不出本机。

**录入**（关键：**token 不经过 agent / 模型**）：

1. agent 调用 `connect_service("cn.lkcoffee/order")`。
2. watt-connect 返回："请在终端运行 `npx watt-connect connect cn.lkcoffee/order`"，或者给出一个本机链接 `http://127.0.0.1:<随机端口>/connect/<一次性 nonce>`。
3. 用户在终端提示符或本机网页里粘贴 token。watt-connect 先用这个 token 发一次 `initialize` + `tools/list` 验证，成功才写入 keychain。
4. 如果客户端支持 MCP 的 URL 模式 elicitation，可以直接推送第 2 步的本机链接。

**不允许**：让用户把 token 直接贴进 agent 对话框。那样 token 会进入模型上下文和对话记录。

**失效**：命中 `auth_failure` 时，watt-connect 把该商家标为 `credential_invalid`，`call_service` 返回固定提示和 `portal_url`。瑞幸大约每月会遇到一次。

**直连模式**：token 写进 agent 自己的配置，之后由 agent 管理，watt-connect 不再保存。滴滴的 key 在 URL 里，所以会以明文 URL 的形式出现在 agent 配置中，这是直连模式的已知代价，安装时要提示用户。

## 9. 直连模式安装器（`connect-installer`）

每个 agent 一个写入器，接口统一：`detect() / install(entry, token) / uninstall(name) / list()`。

- **优先调用 agent 自带的 CLI**，比如 Claude Code 用 `claude mcp add --transport http <name> <url> --header "Authorization: Bearer …"`，不直接改它的文件。
- 没有 CLI 的 agent 才改 JSON 配置文件：写之前备份，只改自己的条目，保留其他内容不动。
- 每个写入器的配置路径和格式在实现时逐个确认，不能凭记忆写死。

首批支持：Claude Code、Cursor、Codex。其余按需求加。

## 10. 协议版本

- watt-connect 作为客户端，`initialize` 时先发自己支持的最新版本，服务器返回哪个版本就用哪个。
- 如果服务器报错，按目录里的 `protocol_versions` 从新到旧依次重试。
- 据第三方资料，麦当劳只支持 `2025-06-18` 及更早版本，官方 README 没写，P0 阶段要实测确认。

## 11. 与 wattetheria 的关系

- 两个 MCP 互相独立，可以同时安装。
- wattetheria MCP 作为目录里的一个条目，只支持直连模式（它是本地 stdio：`npx wattetheria mcp-proxy`）。用户可以通过 `install_service` 把它装进 agent。
- 商家 token 和节点身份密钥互不共享。

## 12. 测试

- **单元测试**：每家商家的 `auth_failure` 分类，直接使用 §3 实测到的响应作为 fixture。
- **Mock 商家**：本地起三个 mock MCP server，分别复现三家的鉴权失败、限流、版本拒绝行为，CI 里跑端到端测试。
- **滴滴沙箱**：用测试 key 跑一遍真实端到端（不产生真实订单）。需要团队申请一个测试 key，放进 CI secret。
- 瑞幸、麦当劳没有沙箱，**CI 只用 mock**，真实账号只做手动的只读验证。

## 13. 里程碑

| 阶段 | 内容 | 完成标准 |
|---|---|---|
| **P0 实测**（约 1 周） | 团队成员用自己的账号拿三家 token；只读调用 `tools/list` 和查询类工具；记录真实 schema、支付链接字段、token 有效期、协议版本 | §3 表格里所有"据文档"都替换成实测结果 |
| **P1 MVP** | 静态签名目录（3 家）、代理模式、keychain、`watt-connect connect` CLI、6 个 meta tools、先预览后下单、日志脱敏 | 在 Claude Code 里只装 `watt-connect`，能完成瑞幸"查门店→预览→下单→拿到支付链接" |
| **P2 直连 + 体验** | `connect-installer`（Claude Code / Cursor / Codex）、本机网页录入 token、常用工具直接暴露 | 一条命令把麦当劳装进 Cursor 并能调用 |
| **P3 目录上云** | 迁到 `watt-servicenet` 的 `mcp_server` 服务类型；商家自助提交与审核；撤销列表 | 新商家从提交到用户可搜到，不需要发新版 watt-connect |

## 14. 待决问题

1. **商家条款**：麦当劳可以支持，用户用自己的 token 在本机调用即可。已读到的限制来自官方接入指南（GitHub `M-China/mcd-mcp-server`）：示例仅限非商业使用，商业转售、付费分发、流量变现、暗示官方背书需书面授权。申请 token 时同意的《MCP 服务规则》全文尚未阅读，P0 申请时要通读。我们自己对麦当劳调用收费、加广告、导流抽佣之前，先拿授权；在那之前对麦当劳默认用直连模式最稳妥。滴滴、瑞幸的条款还没查到。
2. **瑞幸 OAuth**：瑞幸的 401 响应声明了标准 MCP OAuth 的 resource metadata，但这个地址从海外访问返回 403，大概率是海外访问被拦。P0 在国内网络下复测；如果能访问，瑞幸改走标准 OAuth，省掉手动复制 token。
3. **滴滴 key 绑定的账号**：key 是绑定个人出行账号，还是开发者账号？这决定订单记在谁名下。P0 确认。
4. **许可证**：wattetheria 是 AGPL；watt-connect 作为装在用户本机的客户端，许可证要单独决定。
5. **Pro 权限**：滴滴下单需要 Pro 版，据报道需要激活码。普通用户只能用 Beta（只能查价和跳转 App），这会影响首批体验。

## 实现记录（2026-10-29）

本仓库按本文实现了 **P1 MVP 全部内容**，以及 **P2 的直连安装器和本机网页录入**。"常用工具直接暴露"（P2）与目录上云（P3）未做。

已经确认的事项：

- **rmcp 能力（§4.3 "动工前确认"）**：rmcp 3.8 可以指定 `protocolVersion`、自定义 header 和 URL query；
  拿原始状态码和响应体靠自定义 HTTP 后端实现（`connect-upstream/src/http.rs` 实现 rmcp 的 `StreamableHttpClient`）。
  协议层仍完全由 rmcp 处理，所以**不需要改用 TypeScript**。
- 三家的鉴权失败、限流、版本拒绝都用 mock 复现并通过测试（`crates/connect-testkit`、`crates/connect-upstream/tests`）。
- P1 完成标准对应的测试：`apps/watt-connect/tests/e2e_stdio.rs` 以子进程方式启动真实二进制，经 stdio MCP 完成
  "查门店 → 预览 → 下单 → 拿到支付链接"（瑞幸 mock），并检查磁盘上任何文件都不含明文 token。

与上文的差异（均为实现时的取舍）：

| 位置 | 计划 | 实现 | 原因 |
|---|---|---|---|
| §4.3 HTTP 客户端 | `reqwest` 0.12 | 0.13（rustls） | rmcp 3.8 依赖 0.13，避免两个版本 |
| §4.4 `tests/` | 仓库根目录 | `apps/watt-connect/tests/` | 需要 `CARGO_BIN_EXE_watt-connect` 才能驱动真实二进制 |
| §4.4 | — | 新增 `apps/watt-connect/src/logging.rs`、`.github/actions/watt-deps` | 日志统一脱敏；CI 复用 sibling checkout |
| §5 schema | — | 新增 `rate_limited[]`、`modes.default`、`notes[]` | 滴滴 `-32001` 限流写进目录而不是代码；麦当劳默认直连（§14.1）；给用户的注意事项 |
| §5 目录分发 | CDN / 对象存储 | GitHub Pages：`https://wattetheria.github.io/watt-connect/catalog/v1/catalog.json` | 先有一个可用地址；可用 `WATT_CONNECT_CATALOG_URL` 或 `config.toml` 覆盖 |
| §5 | — | 二进制内置目录快照，作为没有缓存、拉取失败时的兜底 | 离线首启也可用；签名公钥配置前也可用 |
| §5 签名公钥 | 内置 | `catalog/catalog.pub` 目前**为空** | 私钥不能由实现者生成后交付。运行 `cargo run -p watt-connect-catalog -- keygen`，公钥写入 `catalog.pub`，私钥存为 `CATALOG_SIGNING_KEY` secret 后，远端目录才会被接受 |
| §7 预览 | 同一 session 成功预览过 | 另加：预览 30 分钟有效，每次下单消耗一次预览 | 避免一次预览后无限下单 |
| §7 重试 | 限流最多重试 2 次 | 同；另外网络错误**不**重试 | 下单可能已被处理，重试会重复下单 |
| §9 Codex | 优先 CLI | 直接编辑 `config.toml`（`toml_edit`，保留注释） | `codex mcp add` 不能设置静态 header |
| §14.4 许可证 | 待定 | Apache-2.0 | 沿用仓库已有的 LICENSE |

仍需 P0 实测确认（目录里先按文档/推测填写）：

- 瑞幸、麦当劳、滴滴的完整工具列表（目录只列了文档中出现的工具）；未列出的工具不受预览门控，按只读处理。
- 麦当劳 `mall-create-order`、`party-order-create` 的预览工具：目前保守地都映射到 `calculate-price`。
- 三家 `payment_link_fields` 的真实字段名；目前另有"键名含 pay 的 URL"兜底。
- 三家支持的协议版本；麦当劳按第三方资料只写到 `2025-06-18`。
- Cursor / Codex 配置格式按 2026-10-29 的公开文档实现（Cursor：`~/.cursor/mcp.json` 的 `url`+`headers`；
  Codex：`[mcp_servers.<name>]` 的 `url`+`http_headers`），需在真实客户端上验证一次。
