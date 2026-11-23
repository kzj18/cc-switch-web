# CC Switch Web v4.2.0

同一个 MCP ID 可以为 Claude Code、Codex、Gemini CLI 和 OpenCode 保存各自的配置差异。例如，四个客户端可共用服务地址，并使用各自的 OAuth 客户端参数。每个客户端仍独立控制启用状态。

新接口通过 `mcpAppOverrideVersion: 1` 声明能力。写入验证共用定义、已有补丁和启用状态，沿用 Web 的 Basic Auth 与 CSRF。重新启用或同步时，CC Switch 会将已保存的差异应用到各客户端的原生配置。

本版提供由 GitHub Actions 构建的 Linux x86_64 Web 服务及 SHA-256 校验文件，构建基线为 Ubuntu 20.04。
