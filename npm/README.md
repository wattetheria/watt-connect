# watt-connect

Install one MCP server in your agent and use official merchant MCP servers
(Luckin Coffee, McDonald's China, Didi, ...) through it. New merchants appear
without changing your agent configuration.

```json
{ "mcpServers": { "watt-connect": { "command": "npx", "args": ["-y", "watt-connect"] } } }
```

Connect a merchant (your token stays in this computer's keychain):

```bash
npx watt-connect connect cn.lkcoffee/order
```

See https://github.com/wattetheria/watt-connect for documentation.
