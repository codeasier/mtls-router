# 端口冲突诊断与恢复设计

状态：已批准的实现前设计，不代表当前已发布行为。

## 背景

桌面应用固定监听 `127.0.0.1:19099`。当前端口恢复功能在确认占用者身份后直接终止单个进程，但“能够识别进程”不等于“当前用户有权终止进程”，而且由服务或其他 supervisor 管理的进程可能在终止后自动重启。

Windows 上问题最明显：

- 同一账号以管理员权限启动的进程具有相同 SID，但普通桌面 manager 通常不能取得 `PROCESS_TERMINATE`。
- 其他用户、LocalSystem、NetworkService、PPL、System 或安全软件进程通常无法由当前用户终止。
- Windows Service 可能由 SCM 自动恢复；共享 `svchost.exe` 也不应按单个 listener PID 强杀。
- 当前 Windows PID-only 路径即使已经读到其他用户 SID，仍可能提供强杀入口，最终才因权限不足失败。

macOS 和 Linux 默认要求完整身份与当前用户所有权，因此跨用户进程通常不会显示强杀入口，但 launchd、systemd、容器或其他 watchdog 仍可能重新拉起同用户进程。仓库提供的 systemd unit 使用 `Restart=on-failure` 和 `RestartSec=5`，因此 SIGKILL 后会在当前短释放窗口之外重新占用端口。

## 目标

- 在点击强杀前区分可终止进程、权限阻断、其他用户、受保护进程和已识别的服务。
- 尽力显示 Windows Service 或 Linux systemd unit 的可靠标识，并提供人工停止引导。
- Windows 已知其他用户 SID 时明确拒绝，不再降级为 PID-only。
- 保留 Windows 身份确实不可读取时的受限 PID-only 恢复能力。
- 区分“原进程已终止”和“端口持续保持释放”，识别延迟自动重启。
- 保持现有安全边界：不请求 UAC/root、不执行系统命令、不切换端口、不自动终止或启动未知进程。

## 非目标

- 不让桌面应用直接管理 SCM、systemd、launchd、容器或第三方 supervisor。
- 不提升整个桌面应用或 manager 的权限。
- 不提供通用 shell、PowerShell、文件系统或 HTTP webview 能力。
- 不保证识别所有 supervisor，尤其不猜测无法可靠映射的 launchd label 或容器 workload。
- 不为 management protocol v3 保留双响应兼容层。

## 核心设计

### 诊断与终止授权分离

`router.inspect_occupant` 从“只有可强杀时返回 inspection”调整为“尽量返回结构化占用诊断”。预期阻断不再作为协议错误，而是通过 `recovery` 返回。只有 `recovery.action` 为 `force_terminate` 时才签发短时、一次性确认 token。

建议的响应形态：

```json
{
  "pid": 4242,
  "listen_addr": "127.0.0.1:19099",
  "verification_mode": "verified_identity",
  "process_name": "example.exe",
  "executable": "C:\\example.exe",
  "recovery": {
    "action": "manual_stop_required",
    "reason": "service_managed"
  },
  "supervisor": {
    "kind": "windows_service",
    "scope": "system",
    "identifiers": ["ExampleService"]
  }
}
```

`recovery.action` 是稳定枚举：

- `force_terminate`：允许显式确认后强杀，响应必须包含 token 和过期时间。
- `manual_stop_required`：应由用户在正确权限上下文或 supervisor 中停止，不得包含 token。
- `unavailable`：无法安全建立目标或目标受保护，不得包含 token。

`recovery.reason` 是稳定枚举：

- `service_managed`
- `insufficient_privilege`
- `different_user`
- `protected_process`
- `identity_unavailable`

action 与 reason 的映射固定如下：

- `force_terminate` 不携带 reason。
- `manual_stop_required` 只搭配 `service_managed`、`insufficient_privilege` 或 `different_user`。
- `unavailable` 只搭配 `protected_process` 或 `identity_unavailable`。

`supervisor` 为可选结构化信息：

- `kind`：`windows_service`、`systemd_user` 或 `systemd_system`。
- `scope`：`user`、`system` 或 `unknown`。
- `identifiers`：排序、去重、数量和长度受限的服务或 unit 标识。

manager 不生成任意命令字符串。前端只根据受限枚举和已验证标识显示固定模板，且绝不执行这些命令。

单个 supervisor identifier 的 UTF-8 长度不得超过 256 bytes，最多返回 16 个，编码后的 supervisor 结构不得超过 4 KiB。超限或非法标识不进入部分结果；该次 supervisor 分类整体视为不可用。

### 协议版本

inspection 的成功响应语义和结构发生变化，management protocol 从 v3 升至 v4。router、manager 和 desktop 必须同版本发布；握手不匹配时继续直接拒绝，不尝试兼容解析。

## 平台行为

### Windows

检查顺序固定如下：

1. 使用 TCP4 owner table 确认精确 `127.0.0.1:19099` 的唯一 PID。继续拒绝 wildcard、重复、格式异常或不明确记录。
2. 查询 SCM，将 PID 映射到服务。找到一个或多个服务时返回 `service_managed`，不签发 token。多个服务共享 PID 时显示所有受限标识并明确禁止杀宿主。
3. 查询进程 SID。SID 明确不同则返回 `different_user`；SID 相同则继续完整身份检查；SID 无法读取时才进入 PID-only 候选。
4. 对候选执行无副作用权限预检。`OpenProcess(PROCESS_TERMINATE)` 被拒绝时返回 `insufficient_privilege`；成功时立即关闭预检句柄。
5. 完整身份候选继续要求 PID、启动时间、完整可执行路径、SID 和 listener 身份。PID-only 候选继续只显示 PID，不伪造名称、路径、所有者或启动时间。
6. 实际终止前重新检查 unknown occupant 分类、端口 owner、保护状态和完整身份（如可用），再重新取得终止句柄。预检不替代最终检查。

保护 desktop、manager 和可读取状态中的托管 router 必须先于服务分类和强杀授权。SCM 查询整体失败时不得猜测服务归属；其他证据满足时可以继续普通身份路径，但 UI 应仅陈述已证明的信息。

### Linux

继续通过 `/proc/net/tcp*`、socket inode、PID、启动时间、`exe` 和有效 UID 建立完整身份。额外读取最多 64 KiB 的 `/proc/<pid>/cgroup`：

- 只接受语法严格合法且明确以 `.service` 结尾的 systemd unit。
- `user.slice` 下归类为 `systemd_user`，`system.slice` 下归类为 `systemd_system`。
- `.scope`、容器 cgroup、未知层级、路径穿越、超长或 malformed 输入不归类为服务。
- 已可靠识别 unit 时返回 `service_managed`，不强杀。
- root 或其他 UID 返回 `different_user`；`hidepid`、namespace 或 procfs 权限阻断返回 `identity_unavailable`。

不得仅凭父进程为 PID 1 推断 systemd 服务。

### macOS

继续通过 `SYS_PROC_INFO` 建立 socket、PID、启动时间、完整路径和 UID 身份。其他 UID、身份不可读或受保护目标返回对应阻断原因。

第一阶段不新增 PID 到 launchd label 的推测映射。同用户普通进程仍可强杀；若进程随后被 launchd 或其他 supervisor 拉起，由释放后的观察流程报告重新占用，并显示通用 launchd 人工检查引导。

## 用户交互

Router 页面按 `recovery.action` 展示：

- `force_terminate`：显示已验证身份；Windows PID-only 使用现有强化警告和二次确认。
- `manual_stop_required`：不显示强杀按钮，显示阻断原因、service/unit 标识和人工停止方法。
- `unavailable`：显示具体原因和重新检查入口。

人工引导只展示，不执行：

- Windows Service：显示服务名称、`services.msc` 引导和管理员终端中的 `sc.exe stop "<service>"` 示例。
- systemd user：显示 `systemctl --user stop <unit>`。
- systemd system：显示 `sudo systemctl stop <unit>`。
- macOS 重占：提示先通过活动监视器或 `launchctl` 确认 label，再停止对应管理项。
- 非服务权限不足：提示从启动该进程的同一权限上下文正常退出，或使用系统任务管理工具；不建议以管理员/root 运行桌面应用。

命令使用代码块和复制按钮。前端必须按平台模板安全引用标识，React 仅按文本渲染；不得向 Tauri 增加 shell 执行能力。

## 两阶段恢复判定

manager 的同步阶段继续在短窗口内验证原目标退出及端口首次释放。成功响应固定为：

```json
{
  "termination": "process_terminated",
  "port_state": "released"
}
```

该结果只证明原进程已终止和端口曾释放，不承诺持续释放，也不返回或触发 router 启动。

桌面端随后进入约 10 秒的临时 `observing_release` 状态，复用 scheduler 轮询：

- 观察期始终为 absent：显示“端口保持释放”。
- 同一目标仍存在：显示“终止未生效”。
- 新 PID 重新占用：自动重新 inspection，并显示“原进程已终止，但服务或守护程序重新占用端口”。
- 用户主动启动 Router：取消观察并进入正常生命周期，不把自有 Router 误报为重占。
- 页面卸载、应用退出或 manager 重启：取消观察，不持久化观察状态。

观察不阻塞当前 manager 调用，因此无需把 10 秒等待塞进 `router.force_terminate_occupant` 的短 deadline。

## 错误处理

身份、所有权、服务和权限等预期诊断通过 inspection 返回。协议错误只用于查询整体失败、超时、无效响应或确认阶段竞态。

确认阶段保留或新增以下稳定错误码：

- `OCCUPANT_CHANGED`：端口 owner、身份或 token 已变化。
- `OCCUPANT_PERMISSION_DENIED`：预检后权限发生变化，或最终取得终止权限失败。
- `OCCUPANT_TERMINATION_FAILED`：终止请求已尝试但进程未按预期退出。
- `PORT_RELEASE_TIMEOUT`：无法在短窗口内证明端口释放。

前端为这些错误提供独立中英文文案。`OCCUPANT_TERMINATION_FAILED` 和 `PORT_RELEASE_TIMEOUT` 不再映射为“暂时无法检查占用进程”。

## 实现边界

- `internal/manager/occupant`：扩展 inspection、recovery、supervisor 类型；实现平台探测和权限预检。
- `internal/manager/protocol`：定义 v4 响应与稳定错误码。
- `internal/manager/app`：映射结构化诊断和终止错误，不承载平台判断。
- `desktop/src-tauri`：严格解析并透传 v4 类型，维护短期观察状态，不执行系统命令。
- `desktop/src/RouterPage.tsx`：渲染分类、人工引导和重新占用结果。
- 用户可见英文文档及 `docs/zh-CN/` 对应文档同步更新；根和 manager `INDEX.md` 使用中文更新。

所有 OS 返回的服务标识都必须限制单项长度、数量和总响应大小，并在显示前通过严格结构校验。标识只用于诊断展示，不得进入可执行命令通道。

## 测试策略

### Windows 单元测试

- 同 SID、普通权限、非 Service 时允许强杀。
- 同 SID 但 `PROCESS_TERMINATE` 预检拒绝时返回 `insufficient_privilege`。
- 明确其他 SID 时返回 `different_user`，不降级 PID-only。
- SID 查询失败但权限预检成功时允许 PID-only。
- SID 查询失败且预检拒绝时不签发 token。
- PID 映射单个 Service、多个共享 Service、无 Service 和 SCM 查询失败。
- 预检成功但确认阶段 access denied 时返回 `OCCUPANT_PERMISSION_DENIED`。
- Service、PID 或 listener 在确认前变化时不发送终止请求。
- System、PPL 和桌面生命周期保护目标保持禁止。

### Linux 单元测试

- system/user service cgroup 的严格识别。
- `.scope`、容器 cgroup、路径穿越、超长和 malformed 输入拒绝分类。
- root/其他 UID、`hidepid`、不可读 `exe`、`fd` 或 `cgroup` 的稳定结果。
- 普通同用户非服务进程保持原强杀能力。

### 跨平台服务测试

- 阻断 inspection 绝不包含 token；只有 `force_terminate` 包含未过期的一次性 token。
- 强杀前继续重验完整身份或 Windows PID owner。
- supervisor 信息不能绕过 protected 检查。
- 枚举未知值、字段不完整或超限响应必须 fail closed。

### 桌面测试

- 每种 action/reason 的按钮、文案和焦点行为。
- 服务标识安全显示及命令引用。
- 强杀后 10 秒保持释放、新 PID 重占和同目标未退出。
- 用户主动启动时取消观察。
- 页面卸载、manager 错误和轮询乱序不会显示陈旧成功。

### 目标平台人工验收

CI 中的同权限 helper 不能证明跨完整性和 Service 行为。发布前在对应原生平台人工验证：

- Windows 同账号高完整性进程、其他账号进程、单独 Service、共享宿主 Service 和自动恢复 Service。
- Linux system/user service、`Restart=on-failure` 延迟重启、root service 和受限 procfs。
- macOS 同用户进程、其他用户进程和 launchd `KeepAlive` 重启。

## 验收标准

- 注定因明确权限边界失败的目标不再显示强杀按钮。
- 已知其他 Windows SID 不再进入 PID-only。
- 已识别 Service/unit 显示可靠标识和人工停止引导，且桌面不执行命令。
- 普通同用户进程仍可通过一次性确认 token 安全终止。
- 延迟重新占用不会继续显示“端口保持释放”。
- Windows、macOS、Linux 的完整身份、protected target 和 PID 竞态保护不退化。
- protocol v4、双语用户文档、INDEX 导航和发布一致性检查同步完成。
