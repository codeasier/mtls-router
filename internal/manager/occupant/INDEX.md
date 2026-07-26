# internal/manager/occupant

对占用 router loopback TCP 端口的进程做结构化诊断，并在严格前提下执行精确的强制终止。

## 文件

| 文件 | 职责 |
|------|------|
| `service.go` | `Service`、`New(Config, Dependencies)`、`Inspect(ctx)`、`ForceTerminate(ctx, token)`；一次性确认 token 的铸造与校验 |
| `types.go` | `Identity`、`Target`、`Inspection`、`Recovery`、`Result`；`RecoveryAction`、`RecoveryReason`、`VerificationMode`、`SupervisorKind`、`SupervisorScope` |
| `inspect_linux.go` | 经 `/proc` 找到 listener，并解析 systemd cgroup 判定 supervisor |
| `inspect_darwin.go` | 经 `SYS_PROC_INFO` 解码 TCP 记录定位 listener |
| `inspect_windows.go` | TCP owner table + SCM 服务归属 + session ID |
| `windows_target.go` | Windows target 构造、SID/权限预检、supervisor 标识规范化 |
| `pid_only_unsupported.go` | 非 Windows 平台显式关闭 PID-only 路径 |
| `inspect_unsupported.go` | 其余平台的兜底：一律 `ErrIdentityUnavailable` |

## 恢复动作

`Inspection` 用稳定的 action + reason 表达结论，前端据此渲染，不自行推断：

| `RecoveryAction` | 含义 |
|---|---|
| `force_terminate` | 可执行强制终止，**且只有这一种会签发 token** |
| `manual_stop_required` | 识别为 Windows Service 或 systemd unit —— 只返回人工引导 |
| `unavailable` | 身份不可读或权限不足，无可执行动作 |

## 关键不变量

- **一次性确认 token**：`Inspect` 为可终止目标铸造随机 token，互斥锁保护，**30 秒后过期**，`ForceTerminate` 消费后立即作废。这是本包唯一的跨请求内存状态。
- **两种校验模式**：
  - 完整身份路径（Unix/macOS）在发信号前重验 PID + 启动时间 + 可执行文件；成功证明「已验证的那个进程不存在了」且端口首次释放。
  - Windows PID-only 路径在发信号前重验精确 listener owner、保护状态与终止权限；成功**只证明**终止请求成功且原 listener PID 从端口消失，**不独立证明进程完全结束**。
- Windows 上 SID 不同直接拒绝。SID 或完整身份不可读时，只有无副作用的终止权限预检成功才允许走 PID-only。
- 可靠识别为 Windows Service 或 systemd unit 的占用者只返回人工 supervisor 引导 —— manager **不执行停止命令、不提权**。复制给用户的 Windows 命令只按管理员 PowerShell 安全引用，不适用于 `cmd.exe`。macOS 不猜测 launchd label。
- 桌面端另行在约 10 秒内定期采样；`released` 只表示采样期间未检测到重新占用。

## 依赖

- `../process` —— 完整身份校验（本包不复用其发信号 API 走 PID-only）
- `../protocol` —— 错误码
- `golang.org/x/sys`

## 测试

- 各平台 `inspect_*_test.go` —— 原生 listener 定位与 supervisor 分类
- `integration_test.go`、`signal_windows_integration_test.go` —— 端到端终止路径
- CI 与 release 均单独执行 `go test ./internal/manager/occupant -count=1`（禁用缓存，需要真实 OS 行为）
