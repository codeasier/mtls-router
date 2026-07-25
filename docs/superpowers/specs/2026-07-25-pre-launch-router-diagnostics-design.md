# Issue #115：预启动路由失败诊断设计

## 目标

桌面 manager 在 router 进程创建前失败时，生成非空、有界且不泄漏敏感信息的诊断。该诊断必须同时出现在 `router.status` 和 `router.logs` 中；Windows 进程创建失败必须标识 `stage=process_launch`，并在系统提供错误码时包含十进制数值错误码。现有协议错误码、进程启动后的输出收集和失败恢复行为保持不变。

## 范围

本修复覆盖桌面 router 启动过程中的三类预启动阶段：

- `log_directory`：创建或限制桌面日志目录失败。
- `log_open`：打开或检查桌面日志文件失败。
- `process_launch`：创建 router 进程或完成 Windows 启动准备失败。

不扩展 manager 协议 schema，不改变 CLI detached 启动，不暴露原始路径、证书信息、凭据或 OS 错误文本。

## 设计

### Lifecycle 元数据

`lifecycle.Error` 增加稳定的启动阶段和可选数值 OS 错误码。预启动调用点保留底层错误用于进程层内部判断，但只通过 `errors.As` 提取 `syscall.Errno` 的无符号十进制数值。提取失败时省略 OS 错误码，诊断仍包含阶段和现有协议错误码。

阶段使用封闭常量，避免调用点产生任意文本。进程成功创建后的失败继续使用现有 `Launched`、`RecentOutput` 和清理流程。

### App 协议边界

桌面 `router.start` 返回带稳定阶段的 lifecycle 错误时，app 将其锁存为会话级启动失败。诊断由固定字段组成：

```text
stage=<stage> code=<protocol_code> [os_error=<decimal>]
```

该字符串同时作为 `router.status.last_error` 和失败日志中的唯一合成行，因此 `router.logs.lines` 能返回完全相同的诊断。字符串只包含固定键、封闭阶段、稳定协议错误码和可选数字，不使用底层 `error.Error()`。现有 `router.start` 错误响应仍由 `mapLifecycleError` 生成，错误码和消息不变。

启动后的失败仍使用现有有界输出、协议边界脱敏和行截断逻辑。成功重启继续清除锁存失败。

### 桌面状态协调

Rust orchestration 在任何 `router.start` 拒绝后立即调用 `router.status`。若状态读取成功，则先更新 scheduler，再返回原始启动错误；若状态读取失败，则保留原始启动错误并维持现有状态错误刷新机制。`ROUTER_DEGRADED` 的可用状态兼容逻辑保持不变。

`RouterPage` 在启动 catch 分支中立即刷新 scheduler snapshot，不再只为 `ROUTER_DEGRADED` 执行同步刷新。刷新后的 `start_failed` 状态可立即展示诊断，同时页面继续显示现有本地化安全操作错误。

## 安全与边界

- 原始 filesystem/process 错误文本不跨越 manager 协议边界。
- 原始路径、命令参数、环境变量、证书和 API key 不参与诊断格式化。
- OS 错误码仅接受从 `syscall.Errno` 提取的数字；不解析错误字符串。
- 合成诊断受现有日志行数和单行长度上限约束。
- `router.status` 与 `router.logs` 使用同一个已锁存诊断源，避免内容漂移。

## 测试

- Lifecycle 单元测试覆盖三个稳定阶段、包装 errno 提取和无 errno 时的省略行为。
- Windows 定向测试覆盖进程创建类错误的数值系统错误码。
- App 单元测试验证预启动失败被安全锁存、`router.status` 与 `router.logs` 诊断一致、原始错误 canary 和路径不出现，并验证成功重启清除诊断。
- Rust orchestration 测试验证普通启动拒绝后立即调用 `router.status` 并更新 scheduler，同时返回原始错误。
- React 测试验证启动拒绝后立即刷新 snapshot 并展示 `start_failed` 诊断。
- 先运行相关包和组件的 focused checks，再运行 `go test ./...`、`go vet ./...`、Go 格式检查及桌面相关完整检查。

## 非目标

- 不新增协议错误码或协议字段。
- 不持久化会话级失败诊断。
- 不改变 router 子进程 stdout/stderr 的 drain、日志合并或脱敏规则。
- 不重构通用 lifecycle 错误体系或桌面轮询架构。
