# CC Switch Web v4.2.0

One MCP server ID can store separate configuration patches for Claude Code, Codex, Gemini CLI, and OpenCode. Clients can share a server URL while using their own OAuth client settings and enablement state.

Instances advertise `mcpAppOverrideVersion: 1`. Writes use the existing Basic Auth and CSRF checks and validate the expected shared definition, patch, and enablement state. CC Switch applies saved patches when writing or re-enabling the client configuration.

This release provides a Linux x86_64 Web server binary and its SHA-256 checksum, built by GitHub Actions on an Ubuntu 20.04 userspace baseline.
