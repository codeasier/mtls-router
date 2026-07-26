# internal/manager/process

信号发送前的进程身份获取与校验：PID + 启动时间 + 可执行文件三元组。

## 文件

| 文件 | 职责 |
|------|------|
| `process.go` | `Identity`、`Status`、`Inspect(pid)`、`Validate(expected, binaryPath)`、`SameIdentity`、`NormalizeExecutable`、`Signal`、`SignalIdentity`；`ErrNotFound`、`ErrIdentityMismatch` |
| `process_linux.go` | Linux 平台的 `inspect` / `sameStartIdentity` / `signalProcess`（procfs） |
| `process_darwin.go` | macOS 平台实现 |
| `process_windows.go` | Windows 平台实现 |

## 导出

- `Identity{PID, StartedAt, Executable}` —— 抵御 PID 复用所需的最小身份。
- `Status`：`genuine` / `absent` / `stale`。**身份不完整或不可读一律归为 `stale`**，不会被乐观判定为 genuine。
- `Signal(expected, binaryPath, sig)` —— 发信号前立即重新校验完整身份，并施加「必须是受管 router 二进制路径」的检查。
- `SignalIdentity(expected, sig)` —— 同样先校验完整身份，但不施加 binary-path 检查。

## 关键不变量

- 本包**不对外提供 PID-only 的发信号 API**。仅凭 PID 终止进程的路径只存在于 `../occupant`，且带额外的 listener owner 与权限预检。
- 校验与发信号之间不留窗口：`Signal` 在系统调用前立即重验，而非依赖调用方先前的校验结果。
- `NormalizeExecutable` 会移除 Linux procfs 对「运行中镜像已被替换」追加的后缀，避免升级后误判身份不匹配。

## 依赖

- 标准库 + `golang.org/x/sys`（Windows/Unix 系统调用）

## 测试

- `process_test.go`、`process_linux_test.go` —— 身份获取与分类
- `process_signal_unix_test.go`、`process_signal_windows_test.go` —— 发信号前的重校验行为
