# 安全：凭证、脱敏、支付边界

## 凭证不经过 agent / 模型

- 没有任何 meta tool 接收 token。`connect_service` 只返回操作说明；
  任何 meta tool 的参数里出现 `token` / `key` / `secret` / `password` / `api_key` 都会被拒绝并提示用户改走终端。
- 录入途径只有两条，都在用户本机：
  - `watt-connect connect <service>`：隐藏输入的终端提示，或 `--token-stdin`；
  - 本机网页 `http://127.0.0.1:<随机端口>/connect/<24 字节随机 nonce>`：只绑定 127.0.0.1；
    `Host` 必须等于 `127.0.0.1:<端口>`（防 DNS rebinding）；nonce 不对返回 404；
    保存成功或 10 分钟后关闭；响应带 `Cache-Control: no-store`、`Referrer-Policy: no-referrer` 和严格 CSP。
- 录入后先用该 token 做 `initialize` + `tools/list` 验证，成功才保存（`--no-verify` 可跳过）。

## 存储

| 内容 | 位置 |
|---|---|
| token | 系统 keychain，条目 `watt-connect/<service>`（账户名 `token`） |
| token（无 keychain 时） | `<data_dir>/vault.enc`：XChaCha20-Poly1305 加密的 JSON；密钥 `<data_dir>/vault.key`；两者 `0600` |
| 连接配置 | `config.toml`：服务 → 模式、时间、凭证状态。**不含 token** |
| 目录缓存 | `catalog.json` + `catalog.sig` + `etag`；每次加载重新验签 |

加密文件兜底防的是"vault 文件被单独拷走 / 同步"，防不了以同一用户身份运行的恶意代码——这一点系统 keychain 也只能部分做到。
`WATT_CONNECT_VAULT=keyring|file` 可强制后端。

内存里的 token 用 `zeroize` 包装（`Secret`），`Debug` 输出为 `Secret(<redacted>)`；
发往商家的 header 标记为 sensitive；`reqwest` 错误在上抛前去掉 URL（滴滴的 key 在 query 里）。

## 脱敏

所有日志行在写出前都经过进程级 redactor（`connect-vault::redact`），按顺序：

1. 已加载过的 token 原文（≥6 字符）→ `***`；
2. 目录 `redact_patterns`（如滴滴 `key=[^&\s"]+` → `key=***`）；
3. 内置：`Bearer xxx`、`token=` / `key=` / `access_token=` / `api_key=` 查询参数。

返回给 agent 的错误消息同样经过 redactor。日志只记录 service、tool、耗时、结果类型，**不记录参数和返回内容**。
`serve` 模式下 stdout 只用于 MCP 协议，日志写 stderr 与日志文件。

## 支付边界

- watt-connect 永远不替用户付款，不打开、不跟随、不处理支付链接。
- `effect: order` 的工具必须在同一会话 30 分钟内先成功调用目录指定的预览工具，否则返回 `PREVIEW_REQUIRED`，
  并提示 agent 先预览、给用户看价格、取得明确确认。每次下单会消耗这次预览。
- 下单结果原样返回，并追加"需要用户本人完成支付"和提取出的支付链接（按 `payment_link_fields`，找不到时退而查找键名含 `pay` 的 http(s) URL）。
- 网络错误不会自动重试下单（可能已被商家处理）；只有商家明确限流的请求才退避重试。

## 直连模式的已知代价

- token 写进 agent 自己的配置后由 agent 管理，watt-connect 删除自己的副本。
- 滴滴 key 在 URL query 中，直连时以明文 URL 出现在 agent 配置里；安装时会警告。
- 改配置文件前先备份为 `<file>.watt-connect-<时间>.bak`（`0600`）；新建的配置文件为 `0600`；只改自己的条目。
- Claude Code 优先使用 `claude mcp add --scope user`；CLI 失败时的错误信息不回显参数（可能含 token）。

## 目录完整性

- 远端目录必须由 `catalog/catalog.pub` 中的 Ed25519 公钥签名（JCS 规范化后签名），否则不使用。
- 签名私钥只存在 CI secret `CATALOG_SIGNING_KEY`，只被 `watt-connect-catalog sign` 读取。
- `WATT_CONNECT_DEV_CATALOG` 可加载未签名目录，仅供开发；启用时会打 warning 日志。
- 拉目录的请求不带任何 token 或用户信息，只有 `If-None-Match`。
