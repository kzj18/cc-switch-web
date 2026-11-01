use crate::config::atomic_write;
use crate::error::AppError;
use crate::jsonc_document::JsoncDocument;
use crate::provider::OpenCodeConfigFormat;
use crate::settings::get_opencode_override_dir;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap as IndexMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const STANDARD_OMO_PLUGIN_PREFIXES: [&str; 2] = ["oh-my-openagent", "oh-my-opencode"];
const SLIM_OMO_PLUGIN_PREFIXES: [&str; 1] = ["oh-my-opencode-slim"];
fn opencode_config_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn read_config_contents(path: &Path) -> Result<Option<Vec<u8>>, AppError> {
    match std::fs::read(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(AppError::io(path, err)),
    }
}

fn matches_plugin_prefix(plugin_name: &str, prefix: &str) -> bool {
    plugin_name == prefix
        || plugin_name
            .strip_prefix(prefix)
            .map(|suffix| suffix.starts_with('@'))
            .unwrap_or(false)
}

fn matches_any_plugin_prefix(plugin_name: &str, prefixes: &[&str]) -> bool {
    prefixes
        .iter()
        .any(|prefix| matches_plugin_prefix(plugin_name, prefix))
}

fn canonicalize_plugin_name(plugin_name: &str) -> String {
    if let Some(suffix) = plugin_name.strip_prefix("oh-my-opencode") {
        if suffix.is_empty() || suffix.starts_with('@') {
            return format!("oh-my-openagent{suffix}");
        }
    }
    plugin_name.to_string()
}

pub fn get_opencode_dir() -> PathBuf {
    crate::config::get_client_config_dir_path(get_opencode_override_dir(), ".config/opencode")
        .unwrap_or_else(|_| PathBuf::from(".config").join("opencode"))
}

pub fn get_opencode_dir_info() -> Result<crate::config::ConfigDirInfo, AppError> {
    crate::config::get_client_config_dir_info(get_opencode_override_dir(), ".config/opencode")
}

pub fn get_opencode_config_path() -> PathBuf {
    let jsonc = get_opencode_dir().join("opencode.jsonc");
    match std::fs::symlink_metadata(&jsonc) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            get_opencode_dir().join("opencode.json")
        }
        _ => jsonc,
    }
}

fn resolve_opencode_config_path() -> Result<PathBuf, AppError> {
    resolve_config_path(&get_opencode_dir())
}

fn resolve_config_path(dir: &Path) -> Result<PathBuf, AppError> {
    for name in ["opencode.jsonc", "opencode.json"] {
        let path = dir.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(_) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(AppError::io(&path, error)),
        }
    }
    Ok(dir.join("opencode.json"))
}

struct OpenCodeDocument {
    path: PathBuf,
    selected_path: PathBuf,
    previous_contents: Option<Vec<u8>>,
    document: JsoncDocument,
}

impl OpenCodeDocument {
    fn load(path: &Path) -> Result<Self, AppError> {
        if path.is_symlink() && !path.exists() {
            return Err(AppError::Config(
                "OpenCode config symlink target is unavailable".into(),
            ));
        }
        let selected_path = path.to_path_buf();
        let path = if path.exists() {
            std::fs::canonicalize(path).map_err(|error| AppError::io(path, error))?
        } else {
            path.to_path_buf()
        };
        let previous_contents = read_config_contents(&path)?;
        let source = match &previous_contents {
            Some(contents) => std::str::from_utf8(contents).map_err(|e| {
                AppError::Config(format!(
                    "Invalid UTF-8 in OpenCode config {}: {e}",
                    path.display()
                ))
            })?,
            None => "{\n  \"$schema\": \"https://opencode.ai/config.json\"\n}\n",
        };
        let document = JsoncDocument::parse(source).map_err(|e| {
            AppError::Config(format!("Invalid OpenCode config {}: {e}", path.display()))
        })?;
        for key in ["provider", "providers", "mcp", "plugin", "plugins"] {
            if document.root_key_count(key)? > 1 {
                return Err(AppError::Config(format!(
                    "Duplicate OpenCode config key: {key}"
                )));
            }
        }
        Ok(Self {
            path,
            selected_path,
            previous_contents,
            document,
        })
    }

    // The caller holds opencode_config_lock from path selection through commit.
    fn save(self) -> Result<(), AppError> {
        let source = self.document.validated_source()?;
        let selected_target = if self.selected_path.exists() {
            std::fs::canonicalize(&self.selected_path)
                .map_err(|error| AppError::io(&self.selected_path, error))?
        } else {
            self.selected_path.clone()
        };
        if selected_target != self.path
            || read_config_contents(&self.path)? != self.previous_contents
        {
            return Err(AppError::Conflict(format!(
                "OpenCode config changed on disk. Please reload and try again: {}",
                self.path.display()
            )));
        }
        if self.previous_contents.as_deref() != Some(source.as_bytes()) {
            atomic_write(&self.path, source.as_bytes())?;
        }
        Ok(())
    }
}

pub(crate) fn read_opencode_config_from_path(path: &Path) -> Result<Value, AppError> {
    Ok(OpenCodeDocument::load(path)?.document.value().clone())
}

pub fn read_opencode_config() -> Result<Value, AppError> {
    read_opencode_config_from_path(&resolve_opencode_config_path()?)
}

fn edit_config(
    resolve_path: impl FnOnce() -> Result<PathBuf, AppError>,
    edit: impl FnOnce(&mut Value),
) -> Result<bool, AppError> {
    try_edit_config(resolve_path, |config| {
        edit(config);
        Ok(())
    })
}

/// Like `edit_config`, but the edit may reject the change; nothing is written then.
fn try_edit_config(
    resolve_path: impl FnOnce() -> Result<PathBuf, AppError>,
    edit: impl FnOnce(&mut Value) -> Result<(), AppError>,
) -> Result<bool, AppError> {
    let _guard = opencode_config_lock().lock()?;
    let path = resolve_path()?;
    let mut document = OpenCodeDocument::load(&path)?;
    let mut desired = document.document.value().clone();
    edit(&mut desired)?;
    if !document.document.apply(&desired)? {
        return Ok(false);
    }
    document.save()?;
    Ok(true)
}

pub fn get_providers() -> Result<Map<String, Value>, AppError> {
    Ok(get_providers_with_format()?
        .into_iter()
        .map(|(id, (value, _))| (id, value))
        .collect())
}

/// Preserve the declaration's format alongside its JSON, including package-less
/// built-in overrides. Do not recursively mix legacy and native entries.
pub fn get_providers_with_format(
) -> Result<IndexMap<String, (Value, OpenCodeConfigFormat)>, AppError> {
    let mut config = read_opencode_config()?;
    let mut take = |key| match config.get_mut(key).map(Value::take) {
        Some(Value::Object(providers)) => providers,
        _ => Map::new(),
    };
    let (legacy, native) = (take("provider"), take("providers"));
    let mut providers: IndexMap<_, _> = legacy
        .into_iter()
        .map(|(id, value)| (id, (value, OpenCodeConfigFormat::V1)))
        .collect();
    for (id, value) in native {
        match native_provider_problem(&value) {
            None => {
                providers.insert(id, (value, OpenCodeConfigFormat::V2));
            }
            Some(path) => {
                let field = if path.is_empty() { "<root>" } else { &path };
                log::warn!(
                    "Invalid native OpenCode provider '{id}' at {field}, leaving its source untouched"
                );
            }
        }
    }
    Ok(providers)
}

/// Validate known native fields before giving V2 precedence over V1. Keep the
/// original JSON (including unknown extensions) rather than projecting it into
/// a partial type. Constraints follow OpenCode v2.0.12, commit 2670273ff17d:
/// packages/schema/src/config/provider.ts, model.ts and provider.ts.
pub fn is_native_provider(value: &Value) -> bool {
    native_provider_problem(value).is_none()
}

/// The dotted path of the first known field that does not match the native
/// schema; an empty path means the declaration is not an object. Like OpenCode,
/// which decodes with `onExcessProperty: "ignore"`, treat V1-only keys such as
/// `npm` and `options` as unknown extensions rather than as a format mismatch.
pub fn native_provider_problem(value: &Value) -> Option<String> {
    let Some(obj) = value.as_object() else {
        return Some(String::new());
    };
    if let Some(key) = first_invalid_field(
        obj,
        &[
            ("name", Value::is_string),
            ("package", Value::is_string),
            ("canonical", Value::is_string),
            ("env", native_string_array),
            ("settings", native_provider_settings),
            ("headers", native_headers),
            ("body", Value::is_object),
        ],
    ) {
        return Some(key.to_string());
    }
    match obj.get("models") {
        None => None,
        Some(Value::Object(models)) => models.iter().find_map(|(id, model)| {
            native_model_problem(model).map(|key| match key {
                "" => format!("models.{id}"),
                key => format!("models.{id}.{key}"),
            })
        }),
        Some(_) => Some("models".to_string()),
    }
}

type NativeFieldCheck<'a> = (&'a str, fn(&Value) -> bool);

/// Optional means absent, not null. Unknown keys remain untouched at every level.
fn first_invalid_field<'a>(
    obj: &Map<String, Value>,
    fields: &[NativeFieldCheck<'a>],
) -> Option<&'a str> {
    fields
        .iter()
        .find(|(key, check)| obj.get(*key).is_some_and(|value| !check(value)))
        .map(|(key, _)| *key)
}

fn native_fields_valid(value: &Value, fields: &[NativeFieldCheck<'_>]) -> bool {
    value
        .as_object()
        .is_some_and(|obj| first_invalid_field(obj, fields).is_none())
}

fn native_string_array(value: &Value) -> bool {
    value
        .as_array()
        .is_some_and(|values| values.iter().all(Value::is_string))
}

fn native_finite(value: &Value) -> bool {
    value.as_f64().is_some_and(f64::is_finite)
}

fn native_integer(value: &Value) -> bool {
    // Effect Schema.Int uses JavaScript's safe integer range, including 1.0.
    value
        .as_f64()
        .is_some_and(|n| n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0)
}

fn native_compaction(value: &Value) -> bool {
    value.as_object().is_some_and(|obj| {
        matches!(
            obj.get("type").and_then(Value::as_str),
            Some("summary" | "native")
        )
    })
}

fn native_provider_settings(value: &Value) -> bool {
    native_fields_valid(
        value,
        &[
            ("timeout", |v| v == &Value::Bool(false) || native_finite(v)),
            ("chunkTimeout", native_finite),
            ("compaction", native_compaction),
            ("transport", |v| {
                matches!(v.as_str(), Some("http" | "websocket"))
            }),
        ],
    )
}

fn native_headers(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|headers| headers.values().all(Value::is_string))
}

// Model/variant settings only constrain compaction; timeout and transport here
// are package-specific extensions, unlike provider settings.
fn native_model_settings(value: &Value) -> bool {
    native_fields_valid(value, &[("compaction", native_compaction)])
}

fn native_model_overlays(value: &Value) -> bool {
    native_fields_valid(
        value,
        &[
            ("settings", native_model_settings),
            ("headers", native_headers),
            ("body", Value::is_object),
        ],
    )
}

/// The first invalid field of a model; an empty name means it is not an object.
fn native_model_problem(value: &Value) -> Option<&'static str> {
    let Some(obj) = value.as_object() else {
        return Some("");
    };
    first_invalid_field(
        obj,
        &[
            ("modelID", Value::is_string),
            ("family", Value::is_string),
            ("name", Value::is_string),
            ("package", Value::is_string),
            ("disabled", Value::is_boolean),
            ("settings", native_model_settings),
            ("headers", native_headers),
            ("body", Value::is_object),
            ("compatibility", native_compatibility),
            ("capabilities", |v| {
                v.as_object().is_some_and(|obj| {
                    obj.get("tools").is_some_and(Value::is_boolean)
                        && obj.get("input").is_some_and(native_string_array)
                        && obj.get("output").is_some_and(native_string_array)
                })
            }),
            ("variants", |v| {
                v.as_array().is_some_and(|variants| {
                    variants.iter().all(|variant| {
                        variant.get("id").is_some_and(Value::is_string)
                            && native_model_overlays(variant)
                    })
                })
            }),
            ("cost", |v| match v.as_array() {
                Some(costs) => costs.iter().all(native_cost),
                None => native_cost(v),
            }),
            ("limit", |v| {
                native_fields_valid(
                    v,
                    &[
                        ("context", native_integer),
                        ("input", native_integer),
                        ("output", native_integer),
                    ],
                )
            }),
        ],
    )
}

fn native_compatibility(value: &Value) -> bool {
    native_fields_valid(
        value,
        &[
            ("reasoningField", Value::is_string),
            ("requireReasoning", Value::is_boolean),
            ("maxTokensField", |v| {
                matches!(v.as_str(), Some("max_completion_tokens" | "max_tokens"))
            }),
            ("requireFinishReason", Value::is_boolean),
            ("requireAssistantAfterTool", Value::is_boolean),
            ("supportsPromptCacheKey", Value::is_boolean),
        ],
    )
}

fn native_cost(value: &Value) -> bool {
    value.get("input").is_some_and(native_finite)
        && value.get("output").is_some_and(native_finite)
        && native_fields_valid(
            value,
            &[
                ("tier", |v| {
                    v.as_object().is_some_and(|obj| {
                        obj.get("type").and_then(Value::as_str) == Some("context")
                            && obj.get("size").is_some_and(native_integer)
                    })
                }),
                ("cache", |v| {
                    native_fields_valid(v, &[("read", native_finite), ("write", native_finite)])
                }),
            ],
        )
}

// Keys only a V1 declaration has, and keys only a native one has. Mirrored by
// isNativeOpencodeConfig in src/components/providers/forms/helpers/opencodeFormUtils.ts.
const LEGACY_ONLY_KEYS: [&str; 3] = ["npm", "options", "api"];
const NATIVE_ONLY_KEYS: [&str; 5] = ["package", "settings", "headers", "body", "canonical"];

/// Infer the format of a declaration whose source is unknown. Unlike validation,
/// V1-only keys decide here: a native declaration would not normally carry them.
pub fn provider_format(
    value: &Value,
    source: Option<OpenCodeConfigFormat>,
) -> OpenCodeConfigFormat {
    source.unwrap_or_else(|| {
        let has_any = |keys: &[&str]| keys.iter().any(|key| value.get(*key).is_some());
        if !has_any(&LEGACY_ONLY_KEYS) && has_any(&NATIVE_ONLY_KEYS) {
            OpenCodeConfigFormat::V2
        } else {
            OpenCodeConfigFormat::V1
        }
    })
}

/// The declaration to write for a stored provider, and its format. Settings are
/// normally the declaration itself but may hold a full config (older copies or a
/// pasted file); pick from it as the reader does, preferring a valid native entry.
pub fn provider_fragment<'a>(
    id: &str,
    settings: &'a Value,
    source: Option<OpenCodeConfigFormat>,
) -> Result<(&'a Value, OpenCodeConfigFormat), AppError> {
    let Some(obj) = settings.as_object().filter(|obj| {
        ["$schema", "provider", "providers"]
            .iter()
            .any(|key| obj.contains_key(*key))
    }) else {
        return Ok((settings, provider_format(settings, source)));
    };
    log::warn!(
        "OpenCode provider '{id}' has full config structure in settings_config, attempting to extract fragment"
    );
    let native = obj.get("providers").and_then(|providers| providers.get(id));
    let legacy = obj.get("provider").and_then(|providers| providers.get(id));
    match (native, legacy) {
        (Some(native), legacy) if legacy.is_none() || is_native_provider(native) => {
            Ok((native, OpenCodeConfigFormat::V2))
        }
        (_, Some(legacy)) => Ok((legacy, OpenCodeConfigFormat::V1)),
        _ if obj.contains_key("provider") || obj.contains_key("providers") => {
            Err(AppError::localized(
                "provider.opencode.fragment_missing",
                format!("OpenCode 配置中没有供应商「{id}」"),
                format!("OpenCode config does not contain provider '{id}'"),
            ))
        }
        _ => Ok((settings, provider_format(settings, source))),
    }
}

/// Reject a native declaration OpenCode would skip, naming the offending field.
pub fn validate_native_provider(id: &str, value: &Value) -> Result<(), AppError> {
    match native_provider_problem(value) {
        None => Ok(()),
        Some(path) if path.is_empty() => Err(AppError::localized(
            "provider.opencode.native_not_object",
            format!("OpenCode 原生供应商「{id}」必须是 JSON 对象"),
            format!("Native OpenCode provider '{id}' must be a JSON object"),
        )),
        Some(path) => Err(AppError::localized(
            "provider.opencode.native_invalid_field",
            format!("OpenCode 原生供应商「{id}」的字段 {path} 不符合 OpenCode V2 配置格式"),
            format!("Native OpenCode provider '{id}' has an invalid field: {path}"),
        )),
    }
}

pub fn set_provider_with_format(
    id: &str,
    config: Value,
    format: OpenCodeConfigFormat,
) -> Result<(), AppError> {
    try_edit_config(resolve_opencode_config_path, |full_config| {
        let key = match format {
            OpenCodeConfigFormat::V1 => {
                if full_config
                    .get("providers")
                    .and_then(|providers| providers.get(id))
                    .is_some_and(is_native_provider)
                {
                    return Err(AppError::localized(
                        "provider.opencode.native_shadows_legacy",
                        format!("OpenCode 配置中已有原生 V2 格式的供应商「{id}」，请重新导入供应商后再编辑"),
                        format!(
                            "OpenCode provider '{id}' has a native V2 declaration. Reload providers before editing it."
                        ),
                    ));
                }
                "provider"
            }
            OpenCodeConfigFormat::V2 => {
                validate_native_provider(id, &config)?;
                "providers"
            }
        };

        // 判空要连「存在但不是对象」一起算：否则写入会静默失效——界面显示添加成功而
        // 文件里没有。provider 段是 cc-switch 的投影区，归一化不会碰用户自有的
        // model / theme 等顶层配置；providers 是原生声明，格式有误时拒绝而不是清空。
        match full_config.get(key) {
            Some(Value::Object(_)) => {}
            Some(_) if format == OpenCodeConfigFormat::V2 => {
                return Err(AppError::localized(
                    "provider.opencode.providers_not_object",
                    "OpenCode 配置中的 providers 必须是 JSON 对象",
                    "OpenCode providers must be a JSON object",
                ));
            }
            Some(_) => {
                log::warn!("OpenCode 的供应商配置格式有误，将清空原有供应商配置，再保存当前供应商");
                full_config[key] = json!({});
            }
            None => full_config[key] = json!({}),
        }
        full_config[key][id] = config;
        Ok(())
    })
    .map(|_| ())
}

pub fn remove_provider(id: &str) -> Result<(), AppError> {
    edit_config(resolve_opencode_config_path, |config| {
        let removed_legacy = config
            .get_mut("provider")
            .and_then(Value::as_object_mut)
            .is_some_and(|providers| providers.remove(id).is_some());
        // A valid native declaration shadowed the legacy one; remove both so the
        // older one cannot resurface. An invalid one never took effect: leave it.
        if let Some(providers) = config.get_mut("providers").and_then(Value::as_object_mut) {
            if !removed_legacy || providers.get(id).is_some_and(is_native_provider) {
                providers.remove(id);
            }
        }
    })
    .map(|_| ())
}

/// Top-level `model` (`"<provider>/<model>"`): the model OpenCode starts with.
pub fn get_default_model() -> Result<Option<String>, AppError> {
    Ok(read_opencode_config()?
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string))
}

/// Only rewrites the top-level `model`; `small_model` and everything else stay as is.
pub fn set_default_model(model: &str) -> Result<(), AppError> {
    let model = model.trim();
    if !model
        .split_once('/')
        .is_some_and(|(provider, id)| !provider.is_empty() && !id.is_empty())
    {
        return Err(AppError::InvalidInput(format!(
            "OpenCode model must be \"<provider>/<model>\": {model}"
        )));
    }
    edit_config(resolve_opencode_config_path, |config| {
        config["model"] = json!(model);
    })
    .map(|_| ())
}

pub const STANDARD_OMO_PLUGIN: &str = "oh-my-openagent@latest";
#[allow(dead_code)]
pub const LEGACY_OMO_PLUGIN: &str = "oh-my-opencode@latest";
pub const SLIM_OMO_PLUGIN: &str = "oh-my-opencode-slim@latest";

pub fn write_opencode_config(config: &Value) -> Result<(), AppError> {
    try_edit_config(resolve_opencode_config_path, |value| {
        *value = config.clone();
        Ok(())
    })
    .map(|_| ())
}

pub(crate) fn native_config(config: &Value) -> bool {
    config.get("providers").is_some()
        || config.get("plugins").is_some()
        || config.get("mcp").and_then(|v| v.get("servers")).is_some()
        || config.pointer("/mcp/timeout").is_some_and(|value| {
            value.as_object().is_some_and(|timeout| {
                timeout.contains_key("catalog") || timeout.contains_key("execution")
            })
        })
        || (config.get("provider").is_none()
            && config.get("plugin").is_none()
            && config
                .get("mcp")
                .map_or(true, |v| v.as_object().is_some_and(|o| o.is_empty())))
}

fn native_markers(config: &Value) -> bool {
    config.get("providers").is_some()
        || config.get("plugins").is_some()
        || config.pointer("/mcp/servers").is_some()
}

pub fn uses_native_config() -> Result<bool, AppError> {
    Ok(native_config(&read_opencode_config()?))
}

pub fn get_mcp_servers() -> Result<Map<String, Value>, AppError> {
    let config = read_opencode_config()?;
    let mcp = if native_config(&config) {
        config.pointer("/mcp/servers")
    } else {
        config.get("mcp")
    };
    match mcp {
        None => Ok(Map::new()),
        Some(Value::Object(servers)) => Ok(servers.clone()),
        Some(_) => Err(AppError::Config(
            "OpenCode MCP servers must be an object".into(),
        )),
    }
}

fn mcp_servers_mut(config: &mut Value, native: bool) -> Result<&mut Map<String, Value>, AppError> {
    let root = config
        .as_object_mut()
        .ok_or_else(|| AppError::Config("OpenCode config must be an object".into()))?;
    let mcp = root
        .entry("mcp")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| AppError::Config("OpenCode mcp must be an object".into()))?;
    if !native {
        return Ok(mcp);
    }
    mcp.entry("servers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| AppError::Config("OpenCode mcp.servers must be an object".into()))
}

pub fn set_mcp_server(id: &str, mut spec: Value) -> Result<(), AppError> {
    if !spec.is_object() {
        return Err(AppError::McpValidation(
            "OpenCode MCP server must be an object".into(),
        ));
    }
    try_edit_config(resolve_opencode_config_path, |config| {
        let native = native_config(config);
        let servers = mcp_servers_mut(config, native)?;
        if native {
            let incoming = spec.as_object_mut().unwrap();
            incoming.remove("enabled");
            incoming.insert("disabled".into(), json!(false));
        }
        // Preserve host-specific OAuth, timeout, cwd and extension fields. Only
        // replace the transport fields owned by the unified definition.
        let mut merged = servers
            .get(id)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for key in [
            "type",
            "command",
            "args",
            "environment",
            "env",
            "url",
            "headers",
            "enabled",
        ] {
            merged.remove(key);
        }
        merged.extend(spec.as_object().unwrap().clone());
        servers.insert(id.into(), Value::Object(merged));
        Ok(())
    })
    .map(|_| ())
}

pub fn set_mcp_server_enabled(id: &str, enabled: bool) -> Result<(), AppError> {
    try_edit_config(resolve_opencode_config_path, |config| {
        let native = native_config(config);
        let servers = mcp_servers_mut(config, native)?;
        if let Some(spec) = servers.get_mut(id) {
            if native {
                let entry = spec.as_object_mut().ok_or_else(|| {
                    AppError::McpValidation("OpenCode MCP server must be an object".into())
                })?;
                entry.remove("enabled");
                entry.insert("disabled".into(), json!(!enabled));
            } else if !enabled {
                servers.remove(id);
            }
        }
        Ok(())
    })
    .map(|_| ())
}

pub fn remove_mcp_server(id: &str) -> Result<(), AppError> {
    try_edit_config(resolve_opencode_config_path, |config| {
        let native = native_config(config);
        if config.get("mcp").is_some() {
            mcp_servers_mut(config, native)?.remove(id);
        }
        Ok(())
    })
    .map(|_| ())
}

fn plugin_name(value: &Value) -> Option<&str> {
    value
        .as_str()
        .or_else(|| value.get("package").and_then(Value::as_str))
}

pub fn add_plugin(plugin: &str) -> Result<(), AppError> {
    try_edit_config(resolve_opencode_config_path, |config| {
        let key = if native_markers(config) {
            "plugins"
        } else {
            "plugin"
        };
        let normalized = canonicalize_plugin_name(plugin);
        let plugins = config
            .as_object_mut()
            .unwrap()
            .entry(key)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| AppError::Config(format!("OpenCode {key} must be an array")))?;
        if plugins
            .iter()
            .any(|v| plugin_name(v) == Some(normalized.as_str()))
        {
            return Ok(());
        }
        let omo = matches_any_plugin_prefix(&normalized, &STANDARD_OMO_PLUGIN_PREFIXES)
            || matches_any_plugin_prefix(&normalized, &SLIM_OMO_PLUGIN_PREFIXES);
        if omo {
            plugins.retain(|v| {
                plugin_name(v).map_or(true, |name| {
                    !matches_any_plugin_prefix(name, &STANDARD_OMO_PLUGIN_PREFIXES)
                        && !matches_any_plugin_prefix(name, &SLIM_OMO_PLUGIN_PREFIXES)
                })
            });
        }
        plugins.push(json!(normalized));
        Ok(())
    })
    .map(|_| ())
}

pub fn add_standard_omo_plugin() -> Result<(), AppError> {
    add_plugin(STANDARD_OMO_PLUGIN)
}
pub fn add_slim_omo_plugin() -> Result<(), AppError> {
    add_plugin(SLIM_OMO_PLUGIN)
}
#[allow(dead_code)]
pub fn remove_plugin_by_prefix(prefix: &str) -> Result<(), AppError> {
    remove_plugins_by_prefixes(&[prefix])
}

pub fn remove_plugins_by_prefixes(prefixes: &[&str]) -> Result<(), AppError> {
    try_edit_config(resolve_opencode_config_path, |config| {
        for key in ["plugins", "plugin"] {
            if let Some(plugins) = config.get_mut(key).and_then(Value::as_array_mut) {
                plugins.retain(|v| {
                    plugin_name(v).map_or(true, |name| !matches_any_plugin_prefix(name, prefixes))
                });
            }
        }
        Ok(())
    })
    .map(|_| ())
}

#[allow(dead_code)]
pub fn has_plugin(plugin: &str) -> Result<bool, AppError> {
    has_any_plugin(&[plugin])
}
#[allow(dead_code)]
pub fn has_any_plugin(names: &[&str]) -> Result<bool, AppError> {
    let config = read_opencode_config()?;
    let key = if native_config(&config) {
        "plugins"
    } else {
        "plugin"
    };
    Ok(config
        .get(key)
        .and_then(Value::as_array)
        .is_some_and(|plugins| {
            plugins.iter().any(|v| {
                plugin_name(v)
                    .is_some_and(|p| names.iter().any(|name| p == canonicalize_plugin_name(name)))
            })
        }))
}
pub fn has_standard_omo_plugin() -> Result<bool, AppError> {
    has_any_plugin_prefix(&STANDARD_OMO_PLUGIN_PREFIXES)
}
pub fn has_slim_omo_plugin() -> Result<bool, AppError> {
    has_any_plugin_prefix(&SLIM_OMO_PLUGIN_PREFIXES)
}
fn has_any_plugin_prefix(prefixes: &[&str]) -> Result<bool, AppError> {
    let config = read_opencode_config()?;
    let key = if native_config(&config) {
        "plugins"
    } else {
        "plugin"
    };
    Ok(config
        .get(key)
        .and_then(Value::as_array)
        .is_some_and(|plugins| {
            plugins.iter().any(|v| {
                plugin_name(v).is_some_and(|name| matches_any_plugin_prefix(name, prefixes))
            })
        }))
}

#[cfg(test)]
mod document_tests {
    use super::*;

    #[test]
    fn global_native_mcp_timeout_is_not_imported_as_a_server() {
        assert!(native_config(&json!({"mcp":{"timeout":{"catalog":1000}}})));
        assert!(!native_config(
            &json!({"mcp":{"timeout":{"type":"local","command":["node"]}}})
        ));
    }

    #[test]
    fn concurrent_edit_is_rejected_without_overwriting_the_other_writer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("opencode.jsonc");
        std::fs::write(&path, "{/*keep*/\"providers\":{}}").unwrap();
        let mut document = OpenCodeDocument::load(&path).unwrap();
        document
            .document
            .apply(&json!({"providers":{"new":{}}}))
            .unwrap();
        let concurrent = "{/*concurrent*/\"providers\":{},\"unknown\":true}";
        std::fs::write(&path, concurrent).unwrap();
        assert!(document.save().is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), concurrent);
    }

    #[test]
    fn configuration_path_prefers_existing_jsonc_and_duplicate_controlled_keys_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let json = directory.path().join("opencode.json");
        let jsonc = directory.path().join("opencode.jsonc");
        std::fs::write(&json, "{}").unwrap();
        std::fs::write(&jsonc, "{\"providers\":{},\"providers\":{}}").unwrap();
        assert_eq!(resolve_config_path(directory.path()).unwrap(), jsonc);
        assert!(OpenCodeDocument::load(&jsonc).is_err());
        assert_eq!(std::fs::read_to_string(json).unwrap(), "{}");
    }

    #[test]
    #[cfg(unix)]
    fn preserves_symbolic_link_and_rejects_retargeting() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.jsonc");
        let other = directory.path().join("other.jsonc");
        let link = directory.path().join("opencode.jsonc");
        std::fs::write(&target, "{/*keep*/\"providers\":{}}").unwrap();
        std::fs::write(&other, "{}").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let mut document = OpenCodeDocument::load(&link).unwrap();
        document
            .document
            .apply(&json!({"providers":{"new":{}}}))
            .unwrap();
        document.save().unwrap();
        assert!(link.is_symlink());
        assert!(std::fs::read_to_string(&target)
            .unwrap()
            .contains("/*keep*/"));
        let document = OpenCodeDocument::load(&link).unwrap();
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&other, &link).unwrap();
        assert!(matches!(document.save(), Err(AppError::Conflict(_))));
        assert_eq!(std::fs::read_to_string(other).unwrap(), "{}");
    }

    #[test]
    #[cfg(unix)]
    fn broken_preferred_jsonc_link_never_falls_back_to_json() {
        let directory = tempfile::tempdir().unwrap();
        let json = directory.path().join("opencode.json");
        let jsonc = directory.path().join("opencode.jsonc");
        std::fs::write(&json, "{\"keep\":true}").unwrap();
        std::os::unix::fs::symlink(directory.path().join("missing"), &jsonc).unwrap();
        assert_eq!(resolve_config_path(directory.path()).unwrap(), jsonc);
        assert!(OpenCodeDocument::load(&jsonc).is_err());
        assert!(jsonc.is_symlink());
        assert_eq!(std::fs::read_to_string(json).unwrap(), "{\"keep\":true}");
    }
}
