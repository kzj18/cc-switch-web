# v4.1.0 验收记录

日期：2026-10-31，Linux x86_64。全部变更测试使用临时配置和合成凭据；未切换实际 PM2
服务，未用真实客户端配置或运行数据库做写入验收。

## 通过的检查

- Rust Web feature 全库：403 passed。
- 原生 v2 服务层和 HTTP/CSRF 集成：10 passed。
- 原有 provider 服务回归：18 passed；Web 认证/CSRF 回归：18 passed。
- JSONC 路径、并发、符号链接及版本形状专项：5 passed，已包含于全库结果。
- 前端全量：106 files / 793 tests passed；最终相关表单回归：36 passed。
- TypeScript typecheck、Cargo fmt、Git whitespace 检查通过。
- `pnpm build:web` 和 release Web server example 构建通过。
- Toolbox Matrix：44 passed。
- Toolbox ai-config-sync provider/v2 相关回归：23 passed / 1 skipped。跳过项为需显式提供
  测试 server 的既有可选验收；另完成下面的真实 server 联调。
- Toolbox updater 的 source-version/build-worktree 回归：14 passed。

## 真实进程和网页

使用实际 release server 在临时 HOME / CC_SWITCH_ACCOUNT_HOME 启动：

1. HTTP 读取 `configVersions: [1, 2]`，导入原生 provider 和 MCP。
2. 通过实际 Matrix 类执行 OpenCode MCP 停用/启用，回读 `disabled` 及原生扩展。
3. 通过实际 Definitions 类更新 MCP URL，保留 OAuth、扩展字段及其他原生配置。
4. 用 ai-config-sync 自己的 Python 环境执行当前 provider 发布，返回
   `status=synced`、`updated=[opencode]`，本机文件字节未因发布变化。
5. 用 playwright-cli 打开实际 Web 页面，编辑 native provider JSON 并保存；回读确认
   仅目标 provider 的 baseURL 改变，MCP/plugins/model/small_model 保持原值。

二进制 SHA-256：

```text
e18e0e738751a9a872fff32e4f7c2e391c52fe06761b0cea5533163b9830cfd4
```

可复跑的联调入口在 Toolbox `scripts/cc-switch-matrix/tests/maintained_server_smoke.py`。
本机结果及两张网页验收截图位于该工具 `output/maintained-*`，不提交测试数据和运行产物。

## 边界与已有诊断

- 原生 v2 本地代理接管、未经验证的 OMO/OMO Slim 插件被明确拒绝；直连 provider 和原生
  MCP 是本次验收范围。不是官方 4.x 桌面功能的全量移植。
- Rust 1.99 对旧 `AtomicUsize::fetch_update` 输出弃用提示，位于未改动的 WebDAV 代码；
  Python 测试仍有已有 Starlette/httpx 或 AnyIO 弃用提示。
- 真实网页启动仍会对 Web 模式不支持的 `check_env_conflicts` 输出 501 及诊断；原生
  provider 编辑和保存验收通过，没有将这些已有诊断计为“无 console 错误”。
- 没有证明当前用户运行中的 CLI 已重载、实际 MCP 已握手，也没有将隔离验收当成现网部署。
