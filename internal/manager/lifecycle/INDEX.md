# internal/manager/lifecycle

启动与停止 router，同时维持完整进程身份与桌面所有权不变量。

## 文件

| 文件 | 职责 |
|------|------|
| `lifecycle.go` | `Manager`、`New(Config, Dependencies)`；`Start(ctx, owner)`/`Stop`/`Reclaim`/`MonitorParent`/`RecentOutput`/`LogPath`/`UnexpectedExit`；`Error`（含 `Code`、`Stage`、`OSErrorCode`、`RecentOutput`）、`StartupStage`、`UnexpectedExit` 结构 |
| `launch.go` | `foregroundProcess` 接口与 `launchForeground` 分派 |
| `launch_unix.go` | Unix 前台子进程启动 |
| `launch_windows.go` | Windows 启动：creation flags + kill-on-close job object，保证 manager 退出时子进程不残留 |
| `output.go` | `boundedOutput` —— 有界地保留子进程最近输出 |
| `signal_unix.go`、`signal_windows.go` | `gracefulSignal()` 平台实现 |

## 两种启动模式

- **桌面前台模式**：router 作为 manager 的前台子进程运行，输出被 `boundedOutput` 捕获；Windows 上通过 job object 绑定生命周期。
- **CLI 分离模式**：router 以脱离进程方式启动，日志写入文件，manager 退出后仍存活。

## 行为

- `Config` 覆盖路径（router 二进制、桌面/CLI 状态文件、锁文件、日志）、身份（manager 与父进程的 `process.Identity`、版本、deployment ID、协议版本）与各类超时。
- `Dependencies` 把发现、进程校验、发信号、启动、状态读写、加锁、时钟全部做成可注入函数，因此生命周期逻辑可在无真实进程的情况下测试。
- `MonitorParent` 监控父进程消失；`Reclaim` 用于重新接管此前记录在状态文件中的 router。
- 每次启动预留按本地日期和启动时间命名的独立日志文件；进程创建成功后才更新对应 owner（CLI/desktop）的 latest 指针，`LogPath` 与持久状态指向当前或最近一次会话，而不是跨启动聚合文件。

## 关键不变量

- 启动失败按封闭的 `StartupStage`（日志目录/打开、状态协调、进程启动/检查/退出、ready 校验、身份校验、状态持久化）与可选数值 `OSErrorCode` 生成安全诊断；启动后失败必须终止并 `Wait` 自有子进程，不留孤儿。
- `RecentOutput` 是有界的**原始**输出，只保留在 lifecycle 内部；对外经 `../app` 暴露的诊断是脱敏且会话作用域的。
- 旧 desktop 状态仅在其记录 PID 已由 OS 明确证明不存在时删除后重试；PID 缺失、存活、不可访问或无法证明时继续 fail closed，且绝不因此发送信号。
- 停止 router 前必须经 `../process` 重验完整身份，绝不凭状态文件里的 PID 直接发信号。
- 桌面所有权通过 `../state` 的锁文件表达，同一时刻只有一个桌面会话可持有。

## 依赖

- `../discovery`、`../process`、`../state`、`../protocol`
- `golang.org/x/sys`（Windows job object）
