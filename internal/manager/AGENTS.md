# internal/manager

mtls-router 控制面子系统的边界与工作流。上层范围：仓库根 [AGENTS.md](../../AGENTS.md)。14 个子包的文件映射、关键导出与不变量见各子包 `INDEX.md`（导航表在 [INDEX.md#子包](INDEX.md#子包)）；本文件只写子系统级边界、协议契约与测试焦点。

## 边界定位

- 独立 Go 构建目标：入口 [`cmd/mtls-router-manager/main.go`](../../cmd/mtls-router-manager/main.go)，与数据面 router 同属根 module 但单独产出二进制。`./scripts/build.sh` 与 `desktop/scripts/build-sidecars.sh` 都会构建它，桌面再把它作为校验过的 sidecar 打包。
- 唯一命令 `serve`，通过 stdin/stdout 交换换行分隔 JSON。flag：`--router-sidecar`、`--listen`（记录 router 的本地监听地址，默认 `127.0.0.1:19099`，供 discovery/健康关联使用）、`--desktop-session` 与 `--parent-pid` / `--parent-start` / `--parent-executable`（桌面父身份，四个 flag 必须成套提供，缺一即拒绝启动）。
- 协议所有权：management protocol v4 的请求/响应类型、方法常量、稳定错误码与按方法超时全部定义在 `protocol/`；`app/` 把全部 17 个方法装配到各服务并执行会话级策略（如 API key 清零）。
- 仅本构建目标消费的链接期变量：`internal/manager/preset.Encoded`（base64 Agent model preset）与 `internal/manager/modelcatalog.Simplify`（模型目录过滤策略）；都不在 router 二进制中注入。

## 协议契约（必须一起变更）

- 协议版本 `4` 是跨组件契约：router、manager、setup receipt、release metadata 与桌面端必须同版本。`desktop/scripts/build-sidecars.sh` 硬编码校验 `MANAGEMENT_PROTOCOL_VERSION == 4`，桌面端在启动握手时再次校验并拒绝混合代。
- 修改 `protocol/` 的类型、错误码或超时后，同步检查桌面端镜像类型 `desktop/src-tauri/src/types.rs`、错误映射 `desktop/src-tauri/src/error.rs`，以及入口 main 的 flag 组合校验。
- 错误码是稳定 API，可用于程序分支；错误消息文本仅作诊断，不属于契约（根 AGENTS.md 同样约束）。

## 运行形态与状态

- 基本按请求无状态；长生命周期状态只存在于 `lifecycle.Manager`（router 子进程 spawn/监控/回收、父进程监控）、`agent.Service`（事务与恢复）与 `state/` 状态文件。唯一的内存中跨请求状态是 `occupant.Service` 在 `Inspect` 与 `ForceTerminate` 之间持有的一次性确认 token（30 秒过期）。
- Agent 写入与 cleanup 共用带回滚能力的事务状态目录：journal v3 显式记录 `replace/delete`，`NewService()` 启动时执行恢复。备份文件（`*.bak-*`、`*.rollback-*`）与源文件同目录且权限受限，内容为替换前原始字节，可能含旧 key，必须按凭据对待。
- occupancy/终止的精确行为边界（结构化 action/reason、Windows SCM/systemd 引导、信号前身份重验）见 [INDEX.md#架构模式](INDEX.md#架构模式)；不要放宽其中的 fail-closed 分支。

## API key 边界

- key 只允许出现在 `agent.models` 与 `agent.write` 的 stdin 请求体内；`app` 在成功 decode 后将 `request.APIKey` 尽力清零。不要把 key 引入环境变量、CLI 参数、model config、日志或 journal（根 AGENTS.md 同样约束）。
- `agent/modelconfig` 的无 key 设计是硬约束：schema 主动拒绝 key-like 字段名，状态文件与 token 只存 HMAC 摘要和无密钥声明。
- `agent.write` 会把 key 明文写入目标 Agent 凭据文件（Claude `settings.json`、opencode `opencode.json`、Codex `auth.json`；Codex `config.toml` 不含 key）。路径解析在 `agent/paths.go`，遵循 `CLAUDE_CONFIG_DIR` / `OPENCODE_CONFIG` / `CODEX_HOME` 覆盖语义，路径表见 [INDEX.md#路径约定](INDEX.md#路径约定)。

## 测试焦点

- `go test ./internal/manager/...` 覆盖全部子包；`occupant` 依赖真实 OS 行为（Linux `/proc` + systemd cgroup、macOS `SYS_PROC_INFO`、Windows TCP owner table + SCM/SID），改动它或其依赖后必须单独执行 `go test ./internal/manager/occupant -count=1` —— CI 的 rust job 会在 Linux/macOS/Windows 三平台各自运行该命令。
- CLI 侧对 manager 行为的端到端断言位于仓库根 `tests/setup_*_test.sh`（`make test-shell`）；桌面侧的协议镜像契约测试位于 `desktop/src-tauri`（`cargo test --locked`）。协议变更时三处证据都要对齐。
