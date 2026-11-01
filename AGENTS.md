# Maintained CC Switch Web

This is an existing Web/Tauri application based on Laliet v0.21.0. Read
`README.md` and `docs/MAINTAINING.md` before changing runtime behavior.

- Preserve the existing Web server example, Basic Auth/CSRF, REST payloads and
  SQLite v7 data contract. Toolbox consumers include ai-config-sync and
  cc-switch-matrix; check them when changing provider or MCP operations.
- Keep native OpenCode v2 providers, mcp.servers and ordered plugins separate
  from legacy fields. Preserve provider source-format metadata through SQLite.
- Provider publication must not replay MCP. MCP stays machine-local and outside
  configuration/Git publication.
- Preserve JSONC comments, unknown fields, line endings and symbolic links;
  reject invalid files and concurrent edits without overwriting them.
- Keep native v2 writes behind explicit runtime capability and versioned HTTP
  routes. Old or unknown targets must remain protected.
- Do not install unverified v1 OMO plugins or apply v1 proxy takeover to native
  v2 configuration. Keep support limitations explicit.
- Tests and browser checks must use isolated temporary homes and credentials;
  never point them at the real client directories, database or running service.
- Use pnpm and the Cargo manifest under src-tauri. Follow the local validation
  commands and release/tag policy in docs/MAINTAINING.md.
- Preserve upstream copyright and MIT notices for transplanted code. Record the
  exact source tag/commit and scope of each upstream port.
