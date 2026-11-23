#![cfg(feature = "web-server")]
#![allow(clippy::await_holding_lock)]

use axum::{
    body::{to_bytes, Body},
    http::{Method, Request, StatusCode},
};
use base64::Engine;
use cc_switch_lib::{
    web_api, AppState, AppType, McpService, MultiAppConfig, OpenCodeConfigFormat, Provider,
    ProviderMeta, ProviderService,
};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use tower::ServiceExt;

#[path = "support.rs"]
mod support;

const NATIVE: &str = r#"{
  // host settings and comments survive provider/MCP writes
  "model": "native/m",
  "small_model": "native/small",
  "providers": {
    "native": {"package":"aisdk:@ai-sdk/openai-compatible","settings":{"apiKey":"synthetic-old"},"models":{"m":{}},"extension":{"keep":true}},
    "builtin": {}
  },
  "provider": {"native":{"npm":"@ai-sdk/anthropic"},"legacy":{"npm":"@ai-sdk/openai"}},
  "mcp": {
    "timeout": {"catalog":1000,"execution":2000},
    "servers": {
      "remote": {"type":"remote","url":"https://mcp.example.test/mcp","oauth":false,"disabled":false,"headers":{"Authorization":"Bearer synthetic"},"timeout":{"execution":4000},"extension":"keep"},
      "local": {"type":"local","command":["node","server.js"],"cwd":"/synthetic/data","disabled":true}
    }
  },
  "plugins": [{"package":"native-plugin","options":{"keep":true}},"-disabled-plugin"],
  "unknown": {"keep":true},
}
"#;

fn setup(source: &str) -> (PathBuf, AppState) {
    support::reset_test_fs();
    let path = support::ensure_test_home().join(".config/opencode/opencode.jsonc");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, source).unwrap();
    (
        path,
        AppState::new_for_tests(MultiAppConfig::default()).unwrap(),
    )
}

fn read(path: &PathBuf) -> Value {
    json5::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn imports_native_precedence_and_preserves_empty_override_format() {
    let _guard = support::test_mutex().lock().unwrap();
    let (path, state) = setup(NATIVE);
    ProviderService::import_default_config(&state, AppType::Opencode).unwrap();
    let config = state.load_config().unwrap();
    assert_eq!(
        config.get_manager(&AppType::Opencode).unwrap().current,
        "native"
    );
    let providers = &config.get_manager(&AppType::Opencode).unwrap().providers;
    assert_eq!(
        providers["native"].settings_config["settings"]["apiKey"],
        "synthetic-old"
    );
    assert_eq!(
        providers["native"]
            .meta
            .as_ref()
            .unwrap()
            .opencode_config_format,
        Some(OpenCodeConfigFormat::V2)
    );
    assert_eq!(
        providers["builtin"]
            .meta
            .as_ref()
            .unwrap()
            .opencode_config_format,
        Some(OpenCodeConfigFormat::V2)
    );
    ProviderService::switch(&state, AppType::Opencode, "builtin").unwrap();
    assert!(read(&path)["providers"]["builtin"].is_object());
    assert!(read(&path)["provider"].get("builtin").is_none());
}

#[test]
fn provider_switch_and_update_preserve_jsonc_mcp_plugins_and_other_clients() {
    let _guard = support::test_mutex().lock().unwrap();
    let (path, state) = setup(&NATIVE.replace('\n', "\r\n"));
    let before = read(&path);
    let other = support::ensure_test_home().join(".codex/config.toml");
    std::fs::create_dir_all(other.parent().unwrap()).unwrap();
    std::fs::write(&other, "# unrelated\nmodel = 'keep'\n").unwrap();
    ProviderService::import_default_config(&state, AppType::Opencode).unwrap();
    ProviderService::switch(&state, AppType::Opencode, "native").unwrap();
    let mut provider = state
        .load_config()
        .unwrap()
        .get_manager(&AppType::Opencode)
        .unwrap()
        .providers["native"]
        .clone();
    provider.settings_config["settings"]["apiKey"] = json!("synthetic-new");
    ProviderService::update(&state, AppType::Opencode, provider).unwrap();
    let mut expected = before;
    expected["providers"]["native"]["settings"]["apiKey"] = json!("synthetic-new");
    assert_eq!(read(&path), expected);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("// host settings"));
    assert!(text.contains("\r\n"));
    assert_eq!(
        std::fs::read_to_string(other).unwrap(),
        "# unrelated\nmodel = 'keep'\n"
    );
}

#[test]
fn invalid_native_update_is_rejected_before_persistence() {
    let _guard = support::test_mutex().lock().unwrap();
    let (path, state) = setup(NATIVE);
    let mut provider = Provider::with_id(
        "invalid".into(),
        "Invalid".into(),
        json!({"package":false}),
        None,
    );
    provider.meta = Some(ProviderMeta {
        opencode_config_format: Some(OpenCodeConfigFormat::V2),
        ..Default::default()
    });
    assert!(ProviderService::add(&state, AppType::Opencode, provider).is_err());
    assert!(!state
        .load_config()
        .unwrap()
        .get_manager(&AppType::Opencode)
        .unwrap()
        .providers
        .contains_key("invalid"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), NATIVE);
}

#[test]
fn invalid_native_declaration_does_not_shadow_valid_legacy_or_get_overwritten() {
    let _guard = support::test_mutex().lock().unwrap();
    let source = r#"{"providers":{"shared":{"package":false}},"provider":{"shared":{"npm":"@ai-sdk/openai"}},"plugins":[]}"#;
    let (path, state) = setup(source);
    ProviderService::import_default_config(&state, AppType::Opencode).unwrap();
    let provider = state
        .load_config()
        .unwrap()
        .get_manager(&AppType::Opencode)
        .unwrap()
        .providers["shared"]
        .clone();
    assert_eq!(
        provider.meta.as_ref().unwrap().opencode_config_format,
        Some(OpenCodeConfigFormat::V1)
    );
    ProviderService::switch(&state, AppType::Opencode, "shared").unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn mcp_toggle_preserves_native_fields_and_imported_disabled_state() {
    let _guard = support::test_mutex().lock().unwrap();
    let (path, state) = setup(NATIVE);
    assert_eq!(McpService::import_from_opencode(&state).unwrap(), 2);
    let servers = McpService::get_all_servers(&state).unwrap();
    assert!(!servers["local"].apps.opencode);
    assert!(servers["remote"].apps.opencode);
    let before = read(&path);
    McpService::toggle_app(&state, "remote", AppType::Opencode, false).unwrap();
    let mut disabled = before.clone();
    disabled["mcp"]["servers"]["remote"]["disabled"] = json!(true);
    assert_eq!(read(&path), disabled);
    McpService::toggle_app(&state, "remote", AppType::Opencode, true).unwrap();
    assert_eq!(read(&path), before);
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("// host settings"));
}

#[test]
fn malformed_native_mcp_container_is_not_replaced() {
    let _guard = support::test_mutex().lock().unwrap();
    let source = r#"{"providers":{},"mcp":{"servers":false},"plugins":[]}"#;
    let (path, state) = setup(source);
    assert!(McpService::import_from_opencode(&state).is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn mcp_metadata_edit_does_not_rewrite_native_projection() {
    let _guard = support::test_mutex().lock().unwrap();
    let (path, state) = setup(NATIVE);
    McpService::import_from_opencode(&state).unwrap();
    let mut server = McpService::get_all_servers(&state).unwrap()["remote"].clone();
    server.name = "Only the display label changed".into();
    McpService::upsert_server(&state, server).unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), NATIVE);
}

#[test]
fn minimal_native_config_rejects_unverified_omo_without_injecting_legacy_plugins() {
    let _guard = support::test_mutex().lock().unwrap();
    let source = r#"{"$schema":"https://opencode.ai/config.json","model":"native/m"}"#;
    let (path, state) = setup(source);
    let provider = Provider::with_id("omo".into(), "OMO".into(), json!({}), None);
    ProviderService::add(&state, AppType::Omo, provider).unwrap();
    assert!(ProviderService::switch(&state, AppType::Omo, "omo").is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    assert!(!support::ensure_test_home()
        .join(".config/opencode/oh-my-opencode.json")
        .exists());
}

#[tokio::test]
async fn web_capabilities_and_default_model_write_are_scoped() {
    let _guard = support::test_mutex().lock().unwrap();
    let (path, state) = setup(NATIVE);
    std::env::set_var("WEB_CSRF_TOKEN", "synthetic-csrf");
    let app = web_api::create_router(Arc::new(state), "synthetic-password".into());
    let auth = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode("admin:synthetic-password")
    );
    let request = Request::builder()
        .uri("/api/capabilities")
        .header("Authorization", &auth)
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(
        value["appFeatures"]["opencode"]["configVersions"],
        json!([1, 2])
    );
    assert_eq!(value["mcpAppOverrideVersion"], 1);
    let before = read(&path);
    for (model, success) in [("native/other", true), ("invalid", false)] {
        let request = Request::builder()
            .method(Method::PUT)
            .uri("/api/providers/opencode/default-model")
            .header("Authorization", &auth)
            .header("X-CSRF-Token", "synthetic-csrf")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({"model":model}).to_string()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status().is_success(), success);
    }
    let mut expected = before;
    expected["model"] = json!("native/other");
    assert_eq!(read(&path), expected);
}

#[tokio::test]
async fn same_mcp_id_uses_distinct_cli_patches_and_clears_disabled_opencode() {
    let _guard = support::test_mutex().lock().unwrap();
    let (path, state) = setup(NATIVE);
    let home = support::ensure_test_home();
    let base = json!({"type":"http","url":"https://example.test/mcp"});
    let record = serde_json::from_value(json!({
        "id":"remote", "name":"same MCP", "server":base,
        "apps":{"codex":true,"opencode":true}
    }))
    .unwrap();
    McpService::upsert_server(&state, record).unwrap();
    let state = Arc::new(state);
    std::env::set_var("WEB_CSRF_TOKEN", "synthetic-csrf");
    let app = web_api::create_router(state.clone(), "synthetic-password".into());
    let auth = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode("admin:synthetic-password")
    );
    let codex = json!({"oauth":{"client_id":"https://example.test/codex.json"}});
    let opencode = json!({"oauth":{"client_id":"https://example.test/opencode.json"}});
    let write = |cli: &str, expected_enabled, previous: Value, patch: Value, csrf| {
        let mut request = Request::builder()
            .method(Method::PUT)
            .uri(format!("/api/v1/mcp/servers/remote/app-overrides/{cli}"))
            .header("Authorization", &auth)
            .header("Content-Type", "application/json");
        if csrf {
            request = request.header("X-CSRF-Token", "synthetic-csrf");
        }
        request
            .body(Body::from(
                json!({
                    "expected_server":base, "expected_enabled":expected_enabled,
                    "expected_override":previous, "patch":patch
                })
                .to_string(),
            ))
            .unwrap()
    };
    for (cli, patch) in [("codex", codex.clone()), ("opencode", opencode.clone())] {
        for csrf in [false, true] {
            let result = app
                .clone()
                .oneshot(write(cli, true, Value::Null, patch.clone(), csrf))
                .await
                .unwrap();
            assert_eq!(
                result.status(),
                if csrf {
                    StatusCode::OK
                } else {
                    StatusCode::FORBIDDEN
                }
            );
        }
    }
    let stale = app
        .clone()
        .oneshot(write("opencode", true, Value::Null, Value::Null, true))
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let list = Request::builder()
        .uri("/api/v1/mcp/app-overrides")
        .header("Authorization", &auth)
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(list).await.unwrap();
    let actual: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(
        actual,
        json!({"remote":{"codex":codex,"opencode":opencode}})
    );
    assert_eq!(
        state.load_config().unwrap().mcp.servers.unwrap()["remote"].server,
        base
    );
    // An ordinary config replacement and a full sync retain native overrides.
    state.replace_config(&state.load_config().unwrap()).unwrap();
    McpService::sync_all_enabled(&state).unwrap();
    let codex_config: toml::Value = std::fs::read_to_string(home.join(".codex/config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        codex_config["mcp_servers"]["remote"]["oauth"]["client_id"].as_str(),
        Some("https://example.test/codex.json")
    );
    let open = read(&path);
    assert_eq!(
        open["mcp"]["servers"]["remote"]["oauth"]["client_id"],
        "https://example.test/opencode.json"
    );
    assert_eq!(open["mcp"]["servers"]["remote"]["extension"], "keep");
    McpService::toggle_app(&state, "remote", AppType::Opencode, false).unwrap();
    let reset = app
        .clone()
        .oneshot(write("opencode", false, opencode, Value::Null, true))
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::OK);
    let disabled = read(&path);
    assert_eq!(disabled["mcp"]["servers"]["remote"]["disabled"], true);
    assert!(disabled["mcp"]["servers"]["remote"].get("oauth").is_none());
    McpService::toggle_app(&state, "remote", AppType::Opencode, true).unwrap();
    assert!(read(&path)["mcp"]["servers"]["remote"]
        .get("oauth")
        .is_none());
    McpService::delete_server(&state, "remote").unwrap();
    assert!(McpService::app_overrides(&state).unwrap().is_empty());
}

#[tokio::test]
async fn versioned_native_routes_keep_existing_auth_and_config_contracts() {
    let _guard = support::test_mutex().lock().unwrap();
    let (path, state) = setup(NATIVE);
    ProviderService::import_default_config(&state, AppType::Opencode).unwrap();
    McpService::import_from_opencode(&state).unwrap();
    let mut provider = state
        .load_config()
        .unwrap()
        .get_manager(&AppType::Opencode)
        .unwrap()
        .providers["native"]
        .clone();
    provider.settings_config["settings"]["apiKey"] = json!("synthetic-versioned");
    std::env::set_var("WEB_CSRF_TOKEN", "synthetic-csrf");
    let app = web_api::create_router(Arc::new(state), "synthetic-password".into());
    let auth = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode("admin:synthetic-password")
    );
    for (endpoint, body) in [
        (
            "/api/v2/opencode/providers",
            serde_json::to_value(provider).unwrap(),
        ),
        (
            "/api/v2/opencode/mcp/servers/remote/enabled",
            json!({"enabled":false}),
        ),
    ] {
        for csrf in [false, true] {
            let mut request = Request::builder()
                .method(Method::POST)
                .uri(endpoint)
                .header("Authorization", &auth)
                .header("Content-Type", "application/json");
            if csrf {
                request = request.header("X-CSRF-Token", "synthetic-csrf");
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::from(body.to_string())).unwrap())
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                if csrf {
                    StatusCode::OK
                } else {
                    StatusCode::FORBIDDEN
                }
            );
        }
    }
    let actual = read(&path);
    assert_eq!(
        actual["providers"]["native"]["settings"]["apiKey"],
        "synthetic-versioned"
    );
    assert_eq!(actual["mcp"]["servers"]["remote"]["disabled"], true);
    assert_eq!(actual["mcp"]["servers"]["remote"]["extension"], "keep");
    assert_eq!(actual["plugins"], read_from_source()["plugins"]);
}

fn read_from_source() -> Value {
    json5::from_str(NATIVE).unwrap()
}
