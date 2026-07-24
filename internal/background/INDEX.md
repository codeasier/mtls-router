# internal/background

分离子进程管理与 `-backend` 模式下的有界日志文件处理。

## 文件

| 文件 | 职责 |
|------|------|
| `args.go` | `ChildArgs(args, logPath)` —— 剥离 `-backend`，确保存在 `-log`；`DefaultLogPath(exePath)` |
| `background.go` | `OpenLogFile`、`OpenBoundedLogWriter(path, maxBytes)` —— append-only 日志，超 4MB 自动截断 |
| `background_unix.go` | `Start(exePath, args, logPath)` —— unix fork/exec 分离子进程（setsid） |
| `background_windows.go` | `Start(...)` —— windows CREATE_NEW_PROCESS_GROUP 分离子进程 |
| `env.go` | `ChildEnv(env)` —— 移除 `MTLS_BACKEND`；`DesktopChildEnv(env)` —— 移除所有 `MTLS_*` 变量 |

## 关键不变量

- `ChildArgs` 必须移除 `-backend`/`--backend`，以防无限重复 fork。
- `DesktopChildEnv` 会剥离全部 `MTLS_*` 环境变量 —— 桌面启动只使用显式 flag。
- `boundedLogWriter` 在文件大小超过 `DefaultMaxLogBytes`（4MB）时截断文件（不 rename）。
- 日志文件权限为 `0600`。

## 消费者

- `main.go:startBackend` —— CLI 后台模式
- `internal/manager/lifecycle` —— 桌面 router 子进程启动
