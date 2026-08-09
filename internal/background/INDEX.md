# internal/background

分离子进程管理与 `-backend` 模式下的有界日志文件处理。

## 文件

| 文件 | 职责 |
|------|------|
| `args.go` | `ChildArgs(args, logPath)` —— 剥离 `-backend`，确保存在 `-log`；`DefaultLogPath(exePath)` |
| `background.go` | `PrepareSessionLogPath(basePath, startedAt)` 按日期/启动时间预留会话文件，`LatestSessionLogPath(basePath)` 恢复最近会话；`OpenLogFile`、`OpenBoundedLogWriter(path, maxBytes)` 负责 append-only 写入与单文件 4MB 上限 |
| `background_unix.go` | `Start(exePath, args, logPath)` —— unix fork/exec 分离子进程（setsid） |
| `background_windows.go` | `Start(...)` —— windows CREATE_NEW_PROCESS_GROUP 分离子进程 |
| `env.go` | `ChildEnv(env)` —— 移除 `MTLS_BACKEND`；`DesktopChildEnv(env)` —— 移除所有 `MTLS_*` 变量 |

## 关键不变量

- `ChildArgs` 必须移除 `-backend`/`--backend`，以防无限重复 fork。
- `DesktopChildEnv` 会剥离全部 `MTLS_*` 环境变量 —— 桌面启动只使用显式 flag。
- 默认后台及 manager 启动日志写入 `<base-name>-logs/YYYY-MM-DD/HH-MM-SS[-N].log`；同秒启动用递增后缀隔离，状态文件记录精确会话路径。
- `boundedLogWriter` 在单个会话文件大小超过 `DefaultMaxLogBytes`（4MB）时截断文件（不 rename）。
- 日志文件权限为 `0600`。

## 消费者

- `main.go:startBackend` —— CLI 后台模式
- `internal/manager/lifecycle` —— 桌面 router 子进程启动
