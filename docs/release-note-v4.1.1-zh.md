# 自维护 CC Switch Web v4.1.1

修复同步当前 provider 后 Codex 已启用 MCP 消失的问题。`sync-current` 改用统一 MCP
存储生成客户端配置，再把实际配置写回当前 provider 快照，避免旧的分客户端集合为空时
删除 `mcp_servers`。

Claude、Codex、Gemini 的全量 MCP 投影均按统一记录和各自开关生成配置。统一集合明确
为空时仍以它为准；尚未迁移的旧格式配置继续支持读取。

回归测试覆盖启用 MCP、切换 provider、重复执行 `sync-current` 的完整请求链，以及
原生 HTTP headers、stdio 参数、关闭项、残留旧记录和 provider 快照一致性。
原有 OpenCode v2 支持和专用 API 路由继续保留。

该维护版本从源码构建 Web 服务，沿用原有 Basic Auth、CSRF、API 与 SQLite v7 契约。
构建及验证方式见 [MAINTAINING.md](MAINTAINING.md)。
