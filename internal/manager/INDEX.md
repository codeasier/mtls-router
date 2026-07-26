# internal/manager

mtls-router 的控制面：路由生命周期管理、Agent 配置、端口冲突解决。通过 stdin/stdout 上的换行分隔 JSON 通信（management protocol v4）。

## 子包

全部 14 个子包都有专属 `INDEX.md`（含文件映射、导出、不变量与依赖）。在某个子包内工作时请先读它的 INDEX.md；下表只做导航与一句话定位。

| 包 | 职责 | 关键导出 |
|---------|------|-------------|
| [`app`](app/INDEX.md) | 协议会话装配；将全部 15 个方法映射到服务；强制 API key 清零 | `App`、`New(Config, simplify)`、`Serve(ctx, input, output)` |
| [`protocol`](protocol/INDEX.md) | JSON 请求/响应类型、方法常量、错误码、按方法超时 | `Request`、`Response`、`Error`、`Method*`、`Code*`、`Deadlines()` |
| [`lifecycle`](lifecycle/INDEX.md) | router 进程 spawn/stop/reclaim；桌面前台 + CLI 分离模式；父进程监控；异常退出检测 | `Manager`、`Start(ctx, owner)`、`Stop(ctx)`、`Reclaim()`、`MonitorParent(ctx)` |
| [`discovery`](discovery/INDEX.md) | 通过关联 HTTP `/version` + `/health` 与持久状态文件及 OS 进程身份来分类 router 状态 | `Discoverer`、`Discover(ctx)`、`DiscoverStatus(ctx)`、`DiscoverStartupStatus(ctx, owner)`、`Classification` 常量 |
| [`agent`](agent/INDEX.md) | Agent 检测、配置渲染（Claude JSON / opencode JSON / Codex TOML + JSON auth）、带备份/回滚的事务性写入、模型发现 | `Service`、`Detect()`、`Render()`、`Write()`、`PreviewRequest()`、`DiscoverModels()` |
| [`agent/modelconfig`](agent/modelconfig/INDEX.md) | 无 key 的规范化 model config schema v1：decode、merge、canonical 序列化、token 签名 | `Config`、`Version`、`Decode()`、`DecodeStructural()`、`Canonical()`、`DeepMerge()`、`MaxConfigSize` |
| [`trustedrouter`](trustedrouter/INDEX.md) | 经 router `/v1/models` 的鉴权模型目录发现；写入前重新校验绑定 | `Coordinator`、`Fetch(ctx, owner, apiKey)`、`Revalidate(ctx, owner, apiKey, binding)` |
| [`occupant`](occupant/INDEX.md) | 端口占用者结构化诊断（Linux `/proc` + systemd cgroup、macOS `SYS_PROC_INFO`、Windows TCP owner table + SCM/SID/access preflight）与带一次性确认 token 的精确强制终止 | `Service`、`Inspect(ctx)`、`ForceTerminate(ctx, token)` |
| [`state`](state/INDEX.md) | router 进程身份的原子化 JSON 状态文件读写；文件锁 | `RouterState`、`Read(path)`、`Write(path, value)`、`AcquireLock(path)` |
| [`process`](process/INDEX.md) | PID + 启动时间 + 可执行文件三元身份校验；安全信号 | `Identity`、`Inspect(pid)`、`Validate(expected, binaryPath)`、`Signal(expected, binaryPath, sig)` |
| [`preset`](preset/INDEX.md) | 加载构建期注入的不可变 Agent model preset（base64，经 `-ldflags -X`） | `Load()` → `*modelconfig.Config` |
| [`metadata`](metadata/INDEX.md) | manager 握手信息与生产身份校验 | `Info()`、`ValidateProduction(artifacts...)` |
| [`paths`](paths/INDEX.md) | 跨平台按用户路径解析（CLI 状态目录 + 桌面数据目录） | `Paths`、`Resolve()` |
| [`modelcatalog`](modelcatalog/INDEX.md) | 模型目录 HTTP 客户端与 simplify 策略（链接期 `Simplify` 变量过滤含 `/` 的模型 ID） | `ParseSimplify()`、`Client` |

## 协议方法（15 个）

```
manager.info              diagnostics.collect
router.status             router.start            router.stop
router.health             router.version          router.logs
router.inspect_occupant   router.force_terminate_occupant
agent.detect              agent.models            agent.render
agent.preview             agent.write
```

## 架构模式

- **基本按请求无状态**：每个 JSON 请求独立处理；长生命周期状态位于 `lifecycle.Manager`、`agent.Service` 与 `state` 文件。一个例外是 `occupant.Service`，它在 `Inspect` 与 `ForceTerminate` 之间持有内存中一次性确认 token（互斥锁保护，30 秒后过期）。
- **桌面启动失败诊断**：预启动失败按稳定阶段和可选数值 OS 错误码生成安全诊断；启动后失败会终止并等待自有子进程。lifecycle 保留有界的原始输出，而 app 协议仅暴露脱敏的、会话作用域诊断。
- **结构化占用恢复**：inspection 用稳定 `force_terminate` / `manual_stop_required` / `unavailable` action 与 reason 表达可执行和阻断结果；只有 `force_terminate` 签发 token。Windows 已知不同 SID 直接拒绝；SID 或完整进程身份不可读时，只有无副作用终止权限预检成功才允许 PID-only。可靠识别的 Windows Service 或 systemd unit 只返回人工 supervisor 引导，manager 不执行停止命令也不提权；复制的 Windows 命令仅按管理员 PowerShell 安全引用，不适用于 `cmd.exe`；macOS 不猜测 launchd label。
- **信号与释放边界**：Unix/macOS 完整身份路径在信号前重验 PID + 启动时间 + 可执行文件；Windows PID-only 在信号前重验精确 listener owner、保护状态和终止权限。完整身份成功证明已验证进程身份不存在且端口首次释放；PID-only 成功只证明终止请求成功且原 listener PID 从端口消失，不独立证明进程完全结束。桌面端另行在约 10 秒内定期采样；`released` 只表示采样未检测到重新占用。
- **API key 清零**：`app` 中成功参数 decode 后，在显式退出路径上将 `request.APIKey = ""`。注意：若 `DecodeParams` 本身失败（如未知字段），已填充的字段可能未被清零。
- **事务恢复**：`agent` 写入使用带回滚能力的状态目录；`NewService()` 在启动时执行恢复。
- **discovery 分类**：按端口可达性、状态文件有效性、进程身份、健康结果分支判断决定 —— 非固定线性优先级。端口不可达时，先检查 stale 状态再判定 degraded。

## 路径约定

| 路径 | 内容 |
|------|---------|
| `~/.mtls-router/setup-state.json` | CLI router 状态 |
| `~/.mtls-router/mtls-router.log` | CLI router 日志 |
| `~/Library/Application Support/com.codeasier.mtls-router/`（macOS） | 桌面数据目录 |
| `%APPDATA%/com.codeasier.mtls-router/`（Windows） | 桌面数据目录 |
| `~/.local/share/com.codeasier.mtls-router/`（Linux） | 桌面数据目录 |

## 入口

`cmd/mtls-router-manager/main.go` —— 唯一命令为 `serve`；flag：`--router-sidecar`、`--listen`、`--desktop-session`、`--parent-pid`、`--parent-start`、`--parent-executable`。
