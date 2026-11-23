use super::super::{from_json_string, lock_conn, to_json_string, Database};
use crate::{
    app_config::{
        AppType, McpAppOverrides, McpApps, McpConfig, McpRoot, McpServer, MultiAppConfig,
    },
    error::AppError,
};
use rusqlite::{params, Connection};
use std::collections::HashMap;

const APP_OVERRIDES: &str = "mcp_app_overrides_v1";

fn merge_legacy_mcp_servers(
    servers: &mut HashMap<String, McpServer>,
    legacy: &McpConfig,
    app: AppType,
) {
    for (id, entry) in &legacy.servers {
        let enabled = entry
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let server = entry
            .get("server")
            .cloned()
            .unwrap_or_else(|| entry.clone());

        if let Some(existing) = servers.get_mut(id) {
            existing.apps.set_enabled_for(&app, enabled);
            continue;
        }

        let name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(id)
            .to_string();
        let description = entry
            .get("description")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned);
        let homepage = entry
            .get("homepage")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned);
        let docs = entry
            .get("docs")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned);
        let tags = entry
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let mut apps = McpApps::default();
        apps.set_enabled_for(&app, enabled);

        servers.insert(
            id.clone(),
            McpServer {
                id: id.clone(),
                name,
                server,
                apps,
                description,
                homepage,
                docs,
                tags,
            },
        );
    }
}

fn normalized_mcp_servers(config: &MultiAppConfig) -> HashMap<String, McpServer> {
    let mut servers = config.mcp.servers.clone().unwrap_or_default();
    merge_legacy_mcp_servers(&mut servers, &config.mcp.claude, AppType::Claude);
    merge_legacy_mcp_servers(&mut servers, &config.mcp.codex, AppType::Codex);
    merge_legacy_mcp_servers(&mut servers, &config.mcp.gemini, AppType::Gemini);
    merge_legacy_mcp_servers(&mut servers, &config.mcp.opencode, AppType::Opencode);
    servers
}

impl Database {
    pub(crate) fn load_mcp_root(&self, conn: &Connection) -> Result<McpRoot, AppError> {
        let mut root = McpRoot::default();
        let mut servers = HashMap::new();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, server_config, description, homepage, docs, tags,
                        enabled_claude, enabled_codex, enabled_gemini, enabled_opencode
                 FROM mcp_servers",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            })
            .map_err(|e| AppError::Database(e.to_string()))?;
        for row in rows {
            let (
                id,
                name,
                server_raw,
                description,
                homepage,
                docs,
                tags_raw,
                claude,
                codex,
                gemini,
                opencode,
            ) = row.map_err(|e| AppError::Database(e.to_string()))?;
            servers.insert(
                id.clone(),
                McpServer {
                    id,
                    name,
                    server: from_json_string(&server_raw, "mcp server")?,
                    apps: McpApps {
                        claude: claude != 0,
                        codex: codex != 0,
                        gemini: gemini != 0,
                        opencode: opencode != 0,
                    },
                    description,
                    homepage,
                    docs,
                    tags: from_json_string(&tags_raw, "mcp tags")?,
                },
            );
        }
        root.servers = Some(servers);
        root.app_overrides = self
            .load_json_setting_with_conn(conn, APP_OVERRIDES)?
            .unwrap_or_default();
        let servers = root.servers.as_ref().expect("servers loaded");
        root.app_overrides.retain(|id, _| servers.contains_key(id));
        Ok(root)
    }

    pub(crate) fn update_mcp_app_overrides<R>(
        &self,
        update: impl FnOnce(&mut McpRoot) -> Result<R, AppError>,
    ) -> Result<R, AppError> {
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|e| AppError::Database(e.to_string()))?;
        let mut root = self.load_mcp_root(&tx)?;
        let result = update(&mut root)?;
        Self::set_json_setting_tx(&tx, APP_OVERRIDES, &root.app_overrides)?;
        tx.commit().map_err(|e| AppError::Database(e.to_string()))?;
        Ok(result)
    }

    pub(crate) fn save_mcp_tx(
        tx: &rusqlite::Transaction<'_>,
        config: &MultiAppConfig,
    ) -> Result<(), AppError> {
        let servers = normalized_mcp_servers(config);
        for (id, server) in &servers {
            tx.execute(
                "INSERT OR REPLACE INTO mcp_servers (
                    id, name, server_config, description, homepage, docs, tags,
                    enabled_claude, enabled_codex, enabled_gemini, enabled_opencode
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    id,
                    server.name,
                    to_json_string(&server.server)?,
                    server.description,
                    server.homepage,
                    server.docs,
                    to_json_string(&server.tags)?,
                    i64::from(server.apps.claude),
                    i64::from(server.apps.codex),
                    i64::from(server.apps.gemini),
                    i64::from(server.apps.opencode),
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        }
        // A deleted server's patch must not attach to a future definition
        // that reuses its ID. Ordinary upserts leave existing patches intact.
        let raw: Option<String> = {
            use rusqlite::OptionalExtension;
            tx.query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [APP_OVERRIDES],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| AppError::Database(e.to_string()))?
        };
        if let Some(raw) = raw {
            let mut overrides: McpAppOverrides = from_json_string(&raw, APP_OVERRIDES)?;
            let previous = overrides.len();
            overrides.retain(|id, _| servers.contains_key(id));
            if overrides.len() != previous {
                Self::set_json_setting_tx(tx, APP_OVERRIDES, &overrides)?;
            }
        }
        Ok(())
    }
}
