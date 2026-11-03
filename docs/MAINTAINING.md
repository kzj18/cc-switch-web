# 自维护 Web 分支

维护仓库：https://github.com/kzj18/cc-switch-web

本分支以实际使用的 `Laliet/cc-switch-web` v0.21.0
（`324dfc384d195eebc3442966b73f97fa4919a458`）为基线，保留 Web server example、
Basic Auth / CSRF、现有 REST API 和 SQLite Schema v7。开发目录使用 `origin` 指向
本仓库，`upstream` 保留 Laliet 来源。

## 本轮移植范围

- 参考官方 `farion1231/cc-switch` v4.0.22 的 `jsonc_document.rs` 和
  `opencode_config.rs`，以及 `dowdsa/cc-switch-web` 的 v4.0.16 Web 快照
  `df7e6339050b4c6b456b73187c95f9f326d03aad`。
- 读取 v1 `provider` 与 v2 `providers`，有效原生声明优先；无效原生声明不遮蔽有效
  v1 声明，原文件保留。provider 元数据中的 `opencodeConfigFormat` 保存格式来源，
  包括空对象和只覆盖内置模型的配置。
- v2 provider 通过 JSON 编辑器维护。保存只修改目标 provider，保留模型选择、
  MCP、插件、未知字段、JSONC 注释和已有换行；检测到并发文件修改时拒绝覆盖。
- 原生 MCP 使用 `mcp.servers`；停用保留定义并写 `disabled: true`。更新定义时保留
  目标机器已有的 OAuth、timeout、cwd 和扩展字段。统一数据库仍是共享连接定义，
  目标机器的专用扩展不作为跨机配置模板自动复制。
- provider 操作在原生 v2 环境中不重放 MCP。MCP 的导入、开关和定义写入继续使用
  原有独立 API，不纳入 provider/Git 同步。
- 原生 `plugins` 的字符串、package/options 对象及顺序在配置写入中保留。
  未验证的 OMO / OMO Slim v1 预设拒绝应用到原生 v2，v1 路径保持可用。
- 原生 v2 本地路由接管暂未实现；该操作会明确拒绝，保留直连配置。v1 路由仍保留。

这是对现有 Web 架构的选择性移植，不代表完整合并官方 4.x 的桌面功能或数据库结构。
桌面自动升级签名与原上游发布链没有在本轮重新建立；本维护分支优先从源码构建 Web
服务，不能将历史 README 的上游预编译下载链接视为本分支的发布产物。

## 外挂接口

`GET /api/capabilities` 为 OpenCode 增加：

```json
{"appFeatures":{"opencode":{"configVersions":[1,2]}}}
```

调用者按每个实例分别验证数组中的整数 `2`。旧服务缺少该字段、返回错误或返回错误类型时，
必须继续禁止原生 v2 写入。普通 `mcp: true` 不代表具备 v2 配置写入能力。

原有 provider 和 MCP 接口保持路径与返回契约；新增的
`GET /api/providers/opencode/default-model` 读取顶层模型，
`PUT /api/providers/opencode/default-model` 接收 `{"model":"provider/model"}`，
只修改 `model`，保留 `small_model` 及其他配置。写请求沿用 Basic Auth / CSRF。

外挂确认能力后，原生写入使用专用路由：`POST /api/v2/opencode/providers`、
`PUT /api/v2/opencode/mcp/servers/<id>` 和
`POST /api/v2/opencode/mcp/servers/<id>/enabled`。它们复用既有 payload/回读模型。
写入前再次确认能力；即使目标在能力查询后退回旧版，旧服务也只会对这些新路径返回 404，
不会把原生数据交给旧版通用写入器。

`ai-config-sync` 只向当前 provider 发布该 provider 的片段和格式元数据；
`cc-switch-matrix` 保留自己的分组、审阅、幂等和回读逻辑，并按实例能力开放 OpenCode。

## MCP 全量同步的数据源

SQLite 读回的统一 `mcp.servers` 和每个服务器的 `apps` 开关是全量投影的数据源，
Claude、Codex、Gemini 使用相同的选择规则。统一集合存在但为空时仍以它为准，
不会回退到旧的分客户端记录；只有尚未迁移、缺少统一集合的配置才读取旧记录。

`POST /api/providers/sync-current` 在写入 Codex provider 后投影这些启用项，再读回
实际 `config.toml` 更新 provider 快照。不能从 SQLite 快照中为空的旧
`mcp.codex.servers` 推断用户关闭了全部 MCP；否则页面仍显示开启，但 Codex MCP 表会被删除。
该流程包含导入配置和切换配置目录后的显式同步，不需要通过成对切换 MCP 开关来补救。

## 版本与交付

本维护线使用 `4.1.0` 起的稳定版本号。原因是 fork 继承了历史 `v3.7.0` tag，而
Laliet Web 线采用 `v0.x`；Toolbox 会比较“按时间最新”与“语义版本最高”的稳定 tag，
两者不一致时拒绝无交互构建。维护版本必须同步更新 package.json、Cargo.toml、Cargo.lock
和 tauri.conf.json，并在验证通过的提交上建立稳定 tag。该版本号标识本维护线，
不表示官方桌面功能已完整移植。

交付顺序为：隔离测试、构建 Web、提交并建立稳定 tag、发布 fork 提交/tag、更新 Toolbox
子模块 URL 和 gitlink。远端发布与实际运行服务切换分别处理，不能将子模块更新当成已经部署。
保留原服务的认证、数据库、客户端配置与 updater 输出目录；不直接让测试服务使用真实 HOME。

## 验证

```bash
pnpm install --frozen-lockfile
pnpm typecheck
pnpm exec vitest run --maxWorkers=2 --minWorkers=1
pnpm build:web
cd src-tauri
cargo test --locked --no-default-features --features web-server --test opencode_v2 --test provider_service --test web_auth -- --test-threads=1
cargo test --locked --no-default-features --features web-server --test import_export_sync --test mcp_unified_sync --test web_provider_mcp_sync -- --test-threads=1
cargo test --locked --no-default-features --features web-server --lib -- --test-threads=1
cargo build --locked --no-default-features --features web-server --example server
```

测试必须使用临时用户目录及 `CC_SWITCH_ACCOUNT_HOME`，不能读取、重写真实客户端配置。
本分支的原生 v2 验收覆盖实际服务层及 Web 路由，包含格式来源的数据库往返、MCP 禁用/启用、
无效数据拒绝、JSONC 与其他客户端保护。运行中的 CLI 是否已重载、MCP 是否握手成功，仍须
在真实客户端做独立验收，不能从 API 回读成功推断。

## 来源与许可证

上述 JSONC 编辑器及原生 provider 校验/读写代码基于官方项目 MIT 许可代码调整。
保留原作者版权及仓库 LICENSE；新增 Web 维护代码同样按现有 MIT 许可提供。

参考：

- https://github.com/Laliet/cc-switch-web/tree/v0.21.0
- https://github.com/farion1231/cc-switch/tree/v4.0.22
- https://github.com/dowdsa/cc-switch-web/tree/df7e6339050b4c6b456b73187c95f9f326d03aad
