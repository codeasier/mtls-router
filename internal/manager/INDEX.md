# internal/manager

mtls-router 的控制面：路由生命周期管理、Agent 配置、端口冲突解决。通过 stdin/stdout 上的换行分隔 JSON 通信（协议 v3）。

## 子包

| 包 | 职责 | 关键导出 |
|---------|------|-------------|
| `app` | 协议会话装配；将全部 15 个方法映射到服务；强制 API key 清零 | `App`、`New(Config, simplify)`、`Serve(ctx, input, output)` |
| `protocol` | JSON 请求/响应类型、方法常量、错误码、按方法超时 | `Request`、`Response`、`Error`、`Method*`、`Code*`、`Deadlines()` |
| `lifecycle` | router 进程 spawn/stop/reclaim；桌面前台 + CLI 分离模式；父进程监控；异常退出检测 | `Manager`、`Start(ctx, owner)`、`Stop(ctx)`、`Reclaim()`、`MonitorParent(ctx)` |
| `discovery` | 通过关联 HTTP `/version` + `/health` 与持久状态文件及 OS 进程身份来分类 router 状态 | `Discoverer`、`Discover(ctx)`、`DiscoverStatus(ctx)`、`Classification` 常量 |
| `agent` | Agent 检测、配置渲染（Claude JSON / opencode JSON / Codex TOML + JSON auth）、带备份/回滚的事务性写入、模型发现 | `Service`、`Detect()`、`Render()`、`Write()`、`PreviewRequest()`、`DiscoverModels()` |
| `agent/modelconfig` | 无 key 的规范化 model config schema v1：decode、merge、canonical 序列化、token 签名 | `Config`、`Version`、`Decode()`、`DecodeStructural()`、`Canonical()`、`DeepMerge()`、`MaxConfigSize` |
| `trustedrouter` | 经 router `/v1/models` 的鉴权模型目录发现；写入前重新校验绑定 | `Coordinator`、`Fetch(ctx, owner, apiKey)`、`Revalidate(ctx, owner, apiKey, binding)` |
| `occupant` | 端口占用者身份检查（Linux `/proc`、macOS `SYS_PROC_INFO`、Windows `GetExtendedTcpTable`）与带一次性确认 token 的受保护强制终止 | `Service`、`Inspect(ctx)`、`ForceTerminate(ctx, token)` |
| `state` | router 进程身份的原子化 JSON 状态文件读写；文件锁 | `RouterState`、`Read(path)`、`Write(path, value)`、`AcquireLock(path)` |
| `process` | PID + 启动时间 + 可执行文件三元身份校验；安全信号 | `Identity`、`Inspect(pid)`、`Validate(expected, binaryPath)`、`Signal(expected, binaryPath, sig)` |
| `preset` | 加载构建期注入的不可变 Agent model preset（base64，经 `-ldflags -X`） | `Load()` → `*modelconfig.Config` |
| `metadata` | manager 握手信息与生产身份校验 | `Info()`、`ValidateProduction(artifacts...)` |
| `paths` | 跨平台按用户路径解析（CLI 状态目录 + 桌面数据目录） | `Paths`、`Resolve()` |
| `modelcatalog` | 模型目录 HTTP 客户端与 simplify 策略（链接期 `Simplify` 变量过滤含 `/` 的模型 ID） | `ParseSimplify()`、`Client` |

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
- **桌面启动失败诊断**：启动后失败会终止并等待自有子进程；lifecycle 保留有界的原始输出，而 app 协议仅暴露脱敏的、会话作用域诊断。
- **信号前身份校验**：Unix/macOS 上经校验的身份路径在每次信号前校验 PID + 启动时间 + 可执行文件。Windows 上 PID-only 路径先经 `InspectPIDOwner` 重新确认监听 PID，再直接调用 `SignalPID`（不做启动时间/可执行文件检查）。
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
