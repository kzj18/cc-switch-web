#![cfg(feature = "web-server")]
#![allow(clippy::await_holding_lock)]

use std::{fs, sync::Arc};

use axum::{
    body::{to_bytes, Body},
    http::{Method, Request, StatusCode},
};
use base64::Engine;
use cc_switch_lib::{web_api, AppState, AppType, MultiAppConfig, Provider};
use serde_json::{json, Value};
use tower::ServiceExt;

#[path = "support.rs"]
mod support;
use support::{ensure_test_home, reset_test_fs, test_mutex};

async fn request(app: &axum::Router, method: Method, path: &str, body: Value) -> Value {
    let auth = base64::engine::general_purpose::STANDARD.encode("admin:fixture-password");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("Authorization", format!("Basic {auth}"))
                .header("x-csrf-token", "fixture-csrf")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .expect("route response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(status, StatusCode::OK, "{path}: {bytes:?}");
    serde_json::from_slice(&bytes).expect("JSON response")
}

#[tokio::test]
async fn provider_switch_then_sync_current_keeps_unified_codex_mcp_and_snapshot() {
    let _guard = test_mutex().lock().unwrap();
    reset_test_fs();
    let home = ensure_test_home();
    std::env::set_var("WEB_CSRF_TOKEN", "fixture-csrf");

    let mut config = MultiAppConfig::default();
    let provider = Provider::with_id(
        "fixture".into(),
        "Fixture".into(),
        json!({
            "auth": {"OPENAI_API_KEY": "fixture-key"},
            "config": "# retained provider comment\nmodel = \"fixture-model\"\nmodel_context_window = 1000000\n"
        }),
        None,
    );
    config
        .get_manager_mut(&AppType::Codex)
        .unwrap()
        .providers
        .insert(provider.id.clone(), provider);
    let state = Arc::new(AppState::new_for_tests(config).unwrap());
    let app = web_api::create_router(state.clone(), "fixture-password".into());

    for (id, enabled, spec) in [
        (
            "remote",
            true,
            json!({"type": "http", "url": "https://mcp.example.test/mcp", "http_headers": {"Authorization": "Bearer fixture-token"}, "startup_timeout_sec": 30}),
        ),
        (
            "local",
            true,
            json!({"type": "stdio", "command": "fixture-command", "args": ["a path with spaces", "--flag"], "env": {"MEMORY_FILE_PATH": "fixture-memory.jsonl"}}),
        ),
        (
            "disabled",
            false,
            json!({"type": "stdio", "command": "must-not-be-written"}),
        ),
    ] {
        request(
            &app,
            Method::PUT,
            &format!("/api/mcp/servers/{id}"),
            json!({"id": id, "name": id, "server": spec, "apps": {"claude": false, "codex": enabled, "gemini": false, "opencode": false}}),
        )
        .await;
    }
    let unified_before = request(&app, Method::GET, "/api/mcp/servers", Value::Null).await;
    assert!(state.load_config().unwrap().mcp.codex.servers.is_empty());

    request(
        &app,
        Method::POST,
        "/api/providers/codex/fixture/switch",
        json!({}),
    )
    .await;
    let path = home.join(".codex/config.toml");
    let after_switch = fs::read_to_string(&path).unwrap();
    let switched: toml::Value = after_switch.parse().unwrap();
    assert_eq!(switched["mcp_servers"].as_table().unwrap().len(), 2);

    // This is the historical extra request after a provider switch. Repeated
    // synchronization must keep both native config and persisted provider state.
    for _ in 0..2 {
        request(&app, Method::POST, "/api/providers/sync-current", json!({})).await;
        let text = fs::read_to_string(&path).unwrap();
        let native: toml::Value = text.parse().unwrap();
        assert_eq!(native["mcp_servers"], switched["mcp_servers"]);
        assert_eq!(native["model"].as_str(), Some("fixture-model"));
        assert_eq!(native["model_context_window"].as_integer(), Some(1000000));
        assert!(text.contains("# retained provider comment"));
        let stored = state.load_config().unwrap();
        let manager = stored.get_manager(&AppType::Codex).unwrap();
        assert_eq!(manager.current, "fixture");
        assert_eq!(manager.providers["fixture"].settings_config["config"], text);
        assert_eq!(
            request(&app, Method::GET, "/api/mcp/servers", Value::Null).await,
            unified_before
        );
    }
}
