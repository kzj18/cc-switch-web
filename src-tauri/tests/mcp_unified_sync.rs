use std::fs;

use cc_switch_lib::{
    sync_enabled_to_claude, sync_enabled_to_codex, sync_enabled_to_gemini, McpServer,
    MultiAppConfig,
};
use serde_json::{json, Value};

#[path = "support.rs"]
mod support;
use support::{ensure_test_home, reset_test_fs, test_mutex};

fn server(id: &str, apps: Value, spec: Value) -> McpServer {
    serde_json::from_value(json!({"id": id, "name": id, "server": spec, "apps": apps})).unwrap()
}

fn sync_clients(config: &MultiAppConfig) {
    sync_enabled_to_claude(config).expect("Claude full projection");
    sync_enabled_to_codex(config).expect("Codex full projection");
    sync_enabled_to_gemini(config).expect("Gemini full projection");
}

#[test]
fn unified_projection_obeys_each_app_and_ignores_stale_legacy_definitions() {
    let _guard = test_mutex().lock().unwrap();
    reset_test_fs();
    let home = ensure_test_home();
    let mut config = MultiAppConfig::default();
    let servers = config.mcp.servers.as_mut().unwrap();
    let common = json!({"type": "stdio", "command": "fixture-command", "args": ["path with spaces"], "env": {"DATA_PATH": "local-data.jsonl"}});
    servers.insert(
        "common".into(),
        server(
            "common",
            json!({"claude": true, "codex": true, "gemini": true}),
            common,
        ),
    );
    servers.insert(
        "claude-only".into(),
        server(
            "claude-only",
            json!({"claude": true}),
            json!({"command": "claude-fixture"}),
        ),
    );
    servers.insert(
        "remote".into(),
        server(
            "remote",
            json!({"codex": true}),
            json!({"type": "http", "url": "https://mcp.example.test/mcp", "http_headers": {"Authorization": "Bearer fixture-token"}, "startup_timeout_sec": 30}),
        ),
    );
    servers.insert(
        "disabled".into(),
        server(
            "disabled",
            json!({}),
            json!({"command": "must-not-be-written"}),
        ),
    );
    for legacy in [
        &mut config.mcp.claude,
        &mut config.mcp.codex,
        &mut config.mcp.gemini,
    ] {
        // Stale entries must neither override a unified definition nor revive a
        // disabled server or add a legacy-only server to the current projection.
        for id in ["common", "disabled", "legacy-only"] {
            legacy.servers.insert(
                id.into(),
                json!({"enabled": true, "server": {"command": "stale-command"}}),
            );
        }
    }

    sync_clients(&config);

    let claude: Value =
        serde_json::from_str(&fs::read_to_string(home.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(claude["mcpServers"].as_object().unwrap().len(), 2);
    assert_eq!(claude["mcpServers"]["common"]["command"], "fixture-command");
    assert!(claude["mcpServers"]["claude-only"].is_object());
    assert!(claude["mcpServers"].get("disabled").is_none());
    assert!(claude["mcpServers"].get("legacy-only").is_none());

    let codex: toml::Value = fs::read_to_string(home.join(".codex/config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let entries = codex["mcp_servers"].as_table().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries["common"]["command"].as_str(),
        Some("fixture-command")
    );
    assert_eq!(
        entries["common"]["args"][0].as_str(),
        Some("path with spaces")
    );
    assert_eq!(
        entries["common"]["env"]["DATA_PATH"].as_str(),
        Some("local-data.jsonl")
    );
    assert_eq!(
        entries["remote"]["http_headers"]["Authorization"].as_str(),
        Some("Bearer fixture-token")
    );
    assert_eq!(
        entries["remote"]["startup_timeout_sec"].as_integer(),
        Some(30)
    );

    let gemini: Value =
        serde_json::from_str(&fs::read_to_string(home.join(".gemini/settings.json")).unwrap())
            .unwrap();
    assert_eq!(gemini["mcpServers"].as_object().unwrap().len(), 1);
    assert_eq!(gemini["mcpServers"]["common"]["command"], "fixture-command");
    assert!(gemini["mcpServers"]["common"].get("type").is_none());
}

#[test]
fn empty_unified_store_removes_mcp_without_falling_back_to_legacy() {
    let _guard = test_mutex().lock().unwrap();
    reset_test_fs();
    let home = ensure_test_home();
    let mut config = MultiAppConfig::default();
    config.mcp.servers.as_mut().unwrap().insert(
        "remove-me".into(),
        server(
            "remove-me",
            json!({"claude": true, "codex": true, "gemini": true}),
            json!({"command": "fixture-command"}),
        ),
    );
    sync_clients(&config);
    config.mcp.servers.as_mut().unwrap().clear();
    for legacy in [
        &mut config.mcp.claude,
        &mut config.mcp.codex,
        &mut config.mcp.gemini,
    ] {
        legacy.servers.insert(
            "remove-me".into(),
            json!({"enabled": true, "server": {"command": "stale-command"}}),
        );
    }
    sync_clients(&config);
    let codex: toml::Value = fs::read_to_string(home.join(".codex/config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert!(codex.get("mcp_servers").is_none());
    for path in [
        home.join(".claude.json"),
        home.join(".gemini/settings.json"),
    ] {
        let native: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert!(native["mcpServers"].as_object().unwrap().is_empty());
    }
}
