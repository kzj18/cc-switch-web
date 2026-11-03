# Maintained CC Switch Web v4.1.1

Fixes enabled Codex MCP servers disappearing after a current-provider synchronization.
The provider `sync-current` route now projects the unified MCP store instead of an empty
legacy per-client store, and saves the resulting native configuration back into the
current provider snapshot.

Claude, Codex, and Gemini full MCP projections use the unified store and each client's
enable flags. An explicitly empty unified store stays authoritative. Legacy-only
configuration remains readable before migration.

Regression coverage includes the complete MCP enable, provider switch, and repeated
`sync-current` request sequence, native HTTP headers and stdio arguments, disabled
entries, stale legacy records, and provider snapshot consistency. Native OpenCode v2
support and its versioned routes remain available.

This maintenance release is built from source for Web deployment. It retains the
existing Basic Auth, CSRF, API and SQLite v7 contracts. See [MAINTAINING.md](MAINTAINING.md)
for build and validation instructions.
