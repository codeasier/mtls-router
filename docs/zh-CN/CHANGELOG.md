# 更新日志

[English](../CHANGELOG.md)

## v0.4.1 - 2026-09-06

本次发布收紧桌面控制台：首屏主要操作在窄窗口下依然可达，删除 API key 需先确认，新增浅色/深色/暖沙外观主题，并且过期的健康检查结果不再掩盖或冒充真实的上游状态。

### 新增

- 在设置页新增暖沙（Warm sand）、浅色（Light）、深色（Dark）三种外观主题；主题选择仅保存在本机浏览器本地存储，不会发送给 manager。
- 删除已保存的 API key 前新增显式确认对话框，避免误点删除凭据；key 内容始终不会被读回或展示。
- Router 页新增「最近检查」时间展示，无需打开 Logs 页即可看到当前健康结果的新鲜度。

### 变更

- 重构 Router 首屏：固定主要操作按钮使其始终可达，布局在 320px 宽度下依然可用，并将数字导航标记替换为语义化图标。
- 区分健康状态：当前检查失败展示为降级 / 上游不可用，当前未知检查展示为等待结果，超过 30 秒的结果展示为已过期——过期或待定的健康结果不再被呈现为当前的上游故障。
- 更新桌面文档，覆盖外观主题、删除确认与更精细的健康文案。

### 修复

- 过期的健康结果不再能把 manager 上报的 degraded 状态改写成过期文案；过期遮罩经过门控，真实故障保持可见。
- 浅色主题的警告文案与蓝色强调色现满足 WCAG AA 对比度；进程读数使用进程范围的标签，不再把运行中的进程描述为已过期。
- 健康检查进行期间门控 Stop 操作，避免在未确认状态上执行动作。

### 安全与恢复

- 主题与语言偏好仅保存在浏览器本地存储；删除确认完全运行于现有桌面 webview 沙箱内，不新增任何 webview 能力。

---

## v0.4.0 - 2026-09-04

本次发布让桌面 router 故障在 manager 不可达时依然可诊断：在故障面上提供一键诊断快照复制与日志包导出，并修复一处侧栏动画问题。

### 新增

- 在桌面 Router 页新增不依赖 manager 的诊断快照：控制面读取失败现在展示为状态不可用，不再复用缓存的上游健康结论；最近一次 router 轮询快照会被持久化，即使界面已提示重启，复制依然可用。
- 新增 Rust 导出的支持包（support bundle），在 Router 故障面与 Logs 页均可导出：单个 zip 包含 `mtls-router-logs/` 下全部会话日志与快照，用户无需打开数据目录即可发送证据。
- 在故障排查指南中记录快照复制与日志包导出的用法，使远程支持指引与新的 Router 页一致。

### 修复

- 修复侧栏展开动画裁切标签的问题：参与宽度过渡的标签现使用 `white-space: nowrap` + `overflow: hidden`，文字随动画被侧栏裁切滑出，不再竖排数帧后跳变；800px 媒体查询下的短标签不受影响。
- 修复支持包导出失败时误删已存在的目标路径（现仅删除 tmp zip），以及首次轮询看门狗超时虚构 `start_failed` 状态（现为不可用，直到真实状态到来）。
- 修复恢复快照携带过期 manager 诊断，以及 `degraded` 状态坍缩为 `health_stale`/`unknown` 而与 Router 页不一致的问题。

### 安全与恢复

- 支持包仅含会话日志与诊断快照——不含凭据、Agent 配置或备份；快照持久化移出轮询写锁，磁盘同步不再阻塞读取。

---

## v0.3.9 - 2026-09-02

本次发布修复两处 fail-closed 缺陷：macOS 上依赖 `window.confirm` 的桌面按钮点击无反应，以及旧版 router 来源链允许集合在发现分类与生命周期迁移门控之间可能静默漂移。

### 修复

- 将 `window.confirm` 替换为遵循现有 danger-dialog 模式的应用内 `ConfirmDialog`（焦点圈定、Escape 与遮罩取消），使 macOS 上「安装并重启」「准备卸载并退出」按钮恢复响应；macOS WKWebView 缺少 confirm 对应代理而立即返回 false，Windows 与 Linux 不受影响。
- 将协议 1/3 旧版祖先集合统一收敛到 `protocol.IsLegacyLineageVersion` 单一定义，并由发现分类与生命周期迁移门控共同引用，避免两处漂移破坏 `legacy_managed` 路由或迁移门控。

### 安全与恢复

- 确认弹窗留在桌面 webview 沙箱内，不新增任何 webview 能力；旧版来源链接受集合现在只有单一事实来源，迁移门控不会与发现分类静默分叉。

---

## v0.3.8 - 2026-09-01

本次发布将按密钥的用量窗口从 API 密钥页移入主导航中独立的「用量」页，并为按模型明细表增加模型多选筛选与按请求/Token/费用列排序，使模型增多时用量依旧可查。

### 新增

- 主导航新增独立「用量」页，加载已保存密钥的用量窗口，API 密钥页回归纯凭据管理。
- 按模型用量表新增模型多选筛选（含全部）与按请求、Token、费用列排序。

### 变更

- 用量窗口从 API 密钥页移入新的「用量」页，API 密钥页提供前往该页的入口。

### 安全与恢复

- 用量窗口仍仅经可信本地路由查询当前已保存密钥的 `GET /v1/usage`；webview 继续只收到有界的用量汇总，绝不接触密钥明文。

---

## v0.3.7 - 2026-09-01

本次发布在桌面 API 密钥页展示已保存密钥的请求、token 与费用用量而绝不暴露密钥本身，并将桌面更新源迁移到国内可直连的端点且发布后校验线上更新源。

### 新增

- 桌面 API 密钥页增加用量窗口：经可信本地路由查询当前已保存密钥的 `GET /v1/usage`，展示请求数、token、费用、可选配额与按模型明细，且不会把密钥明文回读到 webview。

### 变更

- 将内置桌面更新端点迁移到 `release.codeasier.top`，各平台 URL 指向 tag 目录；旧客户端二进制仍内嵌 GitHub 端点，需手动安装本次版本一次。

### 修复

- 将 `apikey.usage` 拆成启动、`/version` 与用量聚合三段独立预算（20s + 15s + 25s），慢聚合在 router 启动和身份校验之后仍有 25 秒。
- 缺少 `by_model` 的 `/v1/usage` 响应改为拒绝，不再伪装成空模型列表。

### 安全与恢复

- 已保存的密钥始终留在 router 管线内，webview 只能看到用量汇总；发布工作流在更新 `latest` 后必须校验公开更新源上报新版本且各平台 URL 均指向该 tag 目录，否则发布失败。

---

## v0.3.6 - 2026-08-31

本次发布以经校验的安装来源链安全迁移旧版桌面 router，用有界结构化的 manager 诊断替代自由格式的启动失败摘要，并在 router 状态被证实安全前拒绝安装更新。

### 新增

- 仅在验证安装来源链与完整进程身份后，才显式迁移旧版桌面 router；对兼容世代保留非破坏性回收，普通启动保持 fail-closed。
- 持久化稳定的安装来源链，协调不可重放的 router 替换，并在安装包失败时恢复已停止的 router。

### 变更

- 以有界结构化的启动诊断替代自由格式的 manager 启动失败摘要，并在桌面 UI 中呈现可操作的旧版 router 状态。

### 安全与恢复

- 任何旧版迁移前必须先验证安装来源链；在 router 状态被证实安全前阻止 updater 安装，并补齐普通启动的 fail-closed 缺口。

### 测试

- 新增发布衍生的协议 1/3 golden fixtures，扩展 Go、Rust、前端与工作流对迁移与拒绝路径的覆盖。

---

## v0.3.5 - 2026-08-09

本次发布改用 GitHub Releases 恢复桌面在线更新，提供可操作的 router 启动诊断，并按启动会话归组 router 输出。

### 新增

- 按启动会话归组 router 输出，并确保最新会话的范围明确，且在 router 和桌面端重启后仍可持久保留。

### 变更

- 将 stable 桌面 updater feed 和已签名 updater 产物迁移到 GitHub Releases，同时保留 `downloads.codeasier.top` 作为 CLI 安装来源和二级发布镜像。

### 修复

- 修复 `v0.3.4` 内置的 updater channel，其原始 endpoint 当前不可访问。由于 `v0.3.4` 无法通过自身 updater 获得此修复，用户必须手动安装一次 `v0.3.5`；后续 stable 版本即可正常在线更新。
- 保留当前且经过脱敏的 router 启动诊断，提供去重、可操作的失败指引，并避免显示先前尝试留下的陈旧提醒。

### 安全与恢复

- 要求 stable GitHub Release 单调推进，并调整 recovery 发布顺序，确保可下载的 GitHub 产物先就绪，再推进二级镜像。

### 测试

- 扩展 Go、前端、Rust 和工作流覆盖，验证 updater 发布、启动诊断、提醒时效性和会话级日志持久化。

---

## v0.3.3 - 2026-08-01

本次发布新增按单 Agent 安全清理 CodeasierRouter 托管设置，引入分层桌面开发工作流，统一 CodeasierRouter 品牌，并在应用关闭到托盘时保留 macOS 全屏 Space。

### 新增

- 新增独立的 cleanup 预览与写入流程，用于移除单个 Agent 中由 CodeasierRouter 托管的 provider、model 和文件凭据设置，同时保留无关用户配置、桌面端全局 API key 与历史备份。
- 新增仅限浏览器的 mock 开发模式、已有 Tauri sidecar 复用和一次性真实 Agent 配置路径，并让生产构建拒绝打包 mock 代码。

### 变更

- 在 provider 展示、model-config 导出与 schema 元数据、桌面诊断和桌面发布产物中统一使用 CodeasierRouter 名称，同时保留兼容性敏感的内部标识符。

### 修复

- macOS 关闭到托盘时先隐藏应用再隐藏窗口，使全屏窗口保留其 Space，且不改变 Windows 或 Linux 的关闭顺序。

### 安全与恢复

- Cleanup 要求可信 sidecar 所有权，使用签名 revision 绑定无 key 预览，在托管状态漂移后要求确认，通过私有事务备份和支持删除的 journal 执行恢复，并在投递结果不确定后要求重新预览。

### 测试

- 扩展 Go、前端、Rust 和工作流覆盖，验证 cleanup 所有权、revision、文件竞态、备份与回滚恢复、不确定投递、响应式交互、mock 隔离、命名一致性和原生托盘顺序。

---

## v0.3.4 - 2026-08-02

本次发布新增签名整包桌面更新，确保验证包可启动，并优化桌面布局与微交互。

### 新增

- 为全部六个桌面目标新增 stable channel 更新：启动时静默检查，设置中可手动检查；用户确认后才安装并重启经过 updater 签名的完整桌面包，其中包含 manager/router sidecar，且不改变 CLI updater 路径。

### 变更

- 优化支持视口尺寸下的桌面间距、响应式布局、设置页呈现和交互反馈。

### 修复

- 通过提供稳定的应用元数据和安全启动路径，确保仅用于验证的桌面包可以启动，且不影响正式发布流程。

### 安全与恢复

- 要求 Tauri updater 签名和来自 `downloads.codeasier.top` 的 stable SemVer feed；updater 签名与 Windows/macOS 平台签名及 notarization 相互独立。

### 测试

- 扩展前端、Rust 和工作流覆盖，验证在线更新状态、签名、产物、验证包启动、响应式交互和布局回归。

---

## v0.3.2 - 2026-07-29

本次发布恢复桌面应用关闭到托盘时符合预期的 macOS 焦点行为，并提升 Agent 配置发现过程与表单的可读性。

### 修复

- macOS 主窗口关闭到托盘时取消激活应用，让先前应用重新获得焦点；从托盘恢复或再次启动时，先重新激活应用再恢复窗口。
- 未显示编辑器时居中呈现 Agent 发现与执行状态，已有编辑器时将活动操作保留在粘性状态栏中，同时移除装饰性请求标记并增大配置字体，改善高分辨率显示器上的可读性。

### 测试

- 扩展原生托盘关闭与激活覆盖，以及 Agent 状态位置、响应式布局、标记移除和配置字体的前端回归覆盖。

---

## v0.3.1 - 2026-07-28

本次发布提升桌面端布局密度与状态辨识度，移除不可靠的 Agent CLI 安装探测，并让认证模型发现过程清晰呈现处理中状态。

### 变更

- 移除 Agent CLI 安装探测及桌面状态；protocol v4 保留兼容字段，并固定返回 `detected=true` 和 `command=""`。
- 收紧受支持 viewport 尺寸下的桌面端间距，简化功能区标题，并用交通灯指示器替换 router 状态码表盘。

### 修复

- Agent 面板初始化认证模型发现时显示处理中状态，避免面板看似处于空闲状态。

### 测试

- 更新 manager 与桌面端覆盖，验证固定的 Agent 兼容字段、模型发现进度、响应式布局密度、router 状态指示器和 occupant focus 恢复。

---

## v0.3.0 - 2026-07-28

本次发布新增桌面端 Agent 凭据持久化、Agent 状态总览和持续的单 Agent 配置面板，同时将管理契约推进到 protocol v4，并强化 router 启动及权限感知的端口冲突恢复。

### 新增

- 新增由私有桌面凭据存储支持的 API 密钥页面，用于管理一个全局 Agent API key。Webview 只能读取摘要；Rust 仅在认证发现时按需加载明文，并在写入前立即重新加载。
- 新增 Agent 总览，以独立卡片展示 Claude Code、OpenCode 和 Codex；CLI 安装状态与配置文件存在性、可写性和有效性分别呈现，同时允许在目标可写时预先生成配置。
- 新增持续的单 Agent 面板：写入成功后保持打开，保护未保存草稿，支持手动或经原生 focus 节流触发的外部状态刷新，并要求显式解决冲突，不轮询或在后台重写文件。

### 变更

- 将匹配的 router、manager、setup、release metadata 和桌面端管理契约推进到 protocol v4；仍拒绝混合 generation。
- 围绕结构化恢复 action/reason 重构端口冲突处理，提供有界 Windows Service/Linux systemd supervisor 标识、明确的人工操作引导，以及终止后的重新占用采样观察。

### 修复

- 保留经过脱敏的 router 启动前诊断，在启动被拒绝后协调桌面状态，并允许存在无关陈旧 CLI state 时正常启动。
- Windows 已知其他用户 SID 时不再降级为仅 PID 恢复；身份不可读时要求终止权限预检，并保持 supervisor 恢复不提权且安全失败。
- 保持空 preview collection 的正确类型，并序列化 router 生命周期操作，避免重叠的启动、停止和退出请求破坏桌面状态。

### 测试

- 新增桌面端回归覆盖，验证凭据生命周期与清零边界、Agent 总览无障碍、持续面板草稿、刷新冲突、flow 清理、写后 reload 和退出保护。
- 扩展原生及跨平台覆盖，验证启动诊断、陈旧发现状态、supervisor 分类、权限预检、终止证据、重新占用采样、稳定错误和 protocol-v4 产物一致性。

---

## v0.2.1 - 2026-07-24

本次发布优化桌面端导航与 Agent 展示，改进可读性和滚动行为，并在桌面端所属 router 启动失败时保留有界且经过脱敏的诊断信息。

### 新增

- 新增可折叠桌面端侧栏并持久化用户偏好，同时确保紧凑图标栏仍可访问全部功能区。

### 变更

- 压缩 Agent 选择卡片，新增 Claude Code、OpenCode 和 Codex 官方 Logo，并统一面向用户的 `OpenCode` 名称，不改变内部标识符或配置路径。

### 修复

- 增大 Agent 配置标签、提示、模型标识符和控件的尺寸，提升可读性。
- 在桌面和移动布局中保持桌面端 tab header 固定，并将滚动限制在内容区域。
- 保留 router 启动失败时有界且经过脱敏的输出，包括 Windows 立即退出和继承 handle 的场景；通过 router 状态和运行日志提供诊断信息，同时避免混入外部 CLI router 的输出。

### 测试

- 新增桌面端回归覆盖，验证侧栏折叠及偏好持久化、配置文字、控件尺寸、滚动归属和失败日志导航。
- 扩展 manager lifecycle 和 app 测试，覆盖启动输出排空、清理、脱敏、日志合并和 Windows 立即退出行为。

---

## v0.2.0 - 2026-07-23

本次发布在 manager、安装脚本和桌面应用中引入经过认证、由模型目录驱动的 Agent 配置，并提供不可变构建 preset、显式恢复流程，以及针对模型可用性、授权和敏感配置状态的 fail-closed 处理。

### 新增

- 新增经过认证的 `GET /v1/models` 发现，以及 Claude Code、opencode 和 Codex 共用且兼容全部 endpoint 的模型目录。
- 新增 protocol-v2 `agent.models`/`agent.render`、无 key 规范 model config、Agent 原生选项、脱敏 render/preview、写入时目录刷新、托管所有权状态，以及漂移/Codex-auth 批准。
- 为每个显式 Claude 选择新增可选显示名称和规范 `context: "1m"`。规范与目录身份始终使用 base model ID；仅在 Claude 渲染边界追加 `[1m]`，不推断能力，也不管理 `CLAUDE_CODE_DISABLE_1M_CONTEXT`。
- 为 manager 二进制新增不可变、无 key 的构建 preset。Protocol v2 现在会在对每个已请求 Agent section 独立执行认证校验后，返回稳定的 `preset.model_config` 和 `preset.unavailable_agents` object。
- 新增仅供 manager 使用的 `SIMPLIFY` 构建策略。未设置/空值和 ASCII 大小写 boolean 会在编译前规范化；默认 `True` 排除包含 ASCII `/` 的有效 ID，`False` 保留全部有效 ID。
- 新增需单独批准的桌面端备份并重建恢复，仅适用于严格符合条件的语法无效 Claude Code、opencode 和 Codex 配置。语法有效、目标不安全或事务恢复未解决时仍不符合条件。

### 变更

- Shell、PowerShell 和桌面端改为 key-before-discovery，对每个 Agent 使用可编辑的 `existing > preset > empty` 初始化，省略未设置可选字段，并取消静态/cache 模型 fallback。显式 `--model-config` 和桌面导入仍是完整替换。
- 精确调整自动选择行为：只有精确 ID 通过当前认证目录校验的可见构建 preset 才能初始化表单。仍禁止选择第一个模型、使用模型名称或能力 heuristic、部分修复 preset、替换模型和运行时 fallback。
- Claude 改为 managed `env` merge；opencode 改为精确选择的 provider 目录；Codex 从历史 `custom` provider 迁移到专用 `mtls-router`，并单独批准 file-backed API-key auth。
- 检测现在只描述本地结构完整性；当前授权只能由模型发现和写入时刷新证明。
- 将经过校验、去重、数量限制、排序和构建过滤的目录作为 protocol token/result、existing 与 preset 可用性、导入、预览和写入时刷新的权威依据。过滤发生在完整校验之后，因此隐藏位置的 malformed ID 仍返回 `MODEL_RESPONSE_INVALID`，全部被过滤时返回 `MODEL_CATALOG_EMPTY`，刷新时模型消失仍会安全失败。
- Rebuild 不再对 malformed 输入执行保留 merge，而是渲染纯托管文件：Claude 只保留托管 `env`，opencode 在已批准路径变为 strict JSON，Codex 同时替换 `config.toml` 和 `auth.json`。安装脚本 Agent 命令仍只支持 merge，不提供强制覆盖 fallback。

### 安全与发布

- 新增私有签名 catalog/revision state、共享操作 lock、事务 sidecar/备份、兼容性 revision pin，以及拒绝混合 protocol generation 的 release preflight。
- 明确 Agent 文件与批准的备份可能含 key，而 model config、token、sidecar、日志、诊断和 protocol result 不含 key。
- 新增可选 `AGENT_MODEL_PRESET_BASE64` release input，并提供 preflight 校验和相同的 standalone/desktop manager 注入。无效非空输入会让 manager 启动失败且不泄漏内容；空输入有效，router binary（包括 desktop router sidecar）绝不会收到 preset 数据。
- Release 构建会在编译前规范化 `SIMPLIFY`，并只向 standalone 和 desktop manager 注入同一个值。它不是 router/运行时偏好，也不会改变 proxy 路由支持。
- Rebuild 会在替换前逐字节备份完整已批准集合中的每个现有文件。成功结果只报告敏感备份路径而不显示内容；备份失败不修改目标，之后失败会执行事务 rollback，无法证明恢复完成时会禁用后续写入。

---

## v0.1.8 - 2026-07-16

本次发布改进 Windows 桌面端进程约束，确保 CodeasierRouter 启动的 router 不会在所属桌面会话结束后继续运行，也不会留下可见的控制台窗口。

### 修复

- 将 Windows 桌面端启动的 router 纳入关闭即终止的 Job Object，并在所属 manager 会话结束时停止该 router。
- 将生产环境 Windows 桌面构建标记为 GUI 应用，使启动 CodeasierRouter 时不再打开控制台窗口。

### 测试

- 新增 Windows 生命周期测试，覆盖挂起状态启动进程、Job Object 配置，以及进程约束关闭时终止 router。
- 新增 Windows GUI subsystem 的 release 包校验和 manager 会话清理测试。

---

## v0.1.7 - 2026-07-15

本次发布取代未发布的 `v0.1.6` tag。由于 fallback Intel macOS bundle sealing 失败，`v0.1.6` Release workflow 未创建 GitHub Release；该 tag 保持不变以便审计。

### 修复

- 修复 fallback macOS 打包：在 bundling 前对内嵌 router 和 manager sidecar 执行 ad-hoc 签名，再签名生成的 desktop executable，最后 seal 应用 bundle。
- fallback 签名保持显式且非递归，使包校验能够继续比较已打包 sidecar 与其已签名源文件的哈希。

### 测试

- 扩展 release workflow 断言，强制 fallback macOS 按依赖顺序签名，并继续拒绝递归 bundle 签名。

---

## v0.1.6 - 2026-07-15

本次发布简化了 CodeasierRouter 桌面界面，并强化 fallback macOS 应用打包流程，确保未签名构建在组装 DMG 前仍具备有效的 bundle 结构。

### 变更

- 简化桌面 router 页面、导航、设置入口和状态展示，降低视觉密度并突出主要 router 控制项。

### 修复

- 在创建 DMG 前对 fallback macOS 应用 bundle 执行 ad-hoc 代码签名，确保缺少 release 签名凭据时，修改后的 bundle 仍得到正确 sealing。

### 测试

- 更新桌面 UI 测试，以覆盖精简后的 router 使用体验。
- 扩展包校验和 release workflow 回归测试，覆盖 fallback macOS bundle sealing。

---

## v0.1.5 - 2026-07-15

本次发布更新了 CodeasierRouter 桌面界面，并改进 macOS 安装和托盘集成。即使 manager 进程的 PATH 中看不到受支持的 CLI 可执行文件，现在仍可配置 Agent；release 发布流程也新增受控恢复路径和更严格的产物校验。

### 新增

- 新增受控 release 恢复 workflow，可复用与失败 tag 构建匹配且已经验证的产物，无需移动或重写 release tag。
- 新增原生 macOS 托盘 template 资源，具备 Retina 尺寸和透明安全边界。

### 变更

- 重新设计桌面界面，在导航、router 控制、Agent 配置、日志和设置中采用统一的暖米色与橙色视觉体系。
- Claude Code、opencode 和 Codex 配置目标不再依赖 CLI 检测结果，同时继续将可执行文件路径保留为可选诊断信息。
- 将确定性的 release 组装逻辑提取到共享打包脚本，供正常发布和恢复发布共同使用。

### 修复

- 为 macOS DMG 添加 `Applications` 快捷方式，并新增包检查以拒绝缺失或指向错误位置的快捷方式。
- 将密集的 macOS 托盘字母图标替换为稳定的原生 template 图标，使其适配浅色和深色菜单栏渲染。
- 强化失败 release 的恢复流程，包括恢复 draft、显式指定仓库、使用正确的 GitHub 上传端点、精确校验产物清单、重新验证 tag SHA，以及防止 latest 版本降级。

### 测试

- 扩展 release workflow 和打包回归覆盖，验证恢复安全性、确定性组装和精确产物清单。
- 新增 CLI 缺失场景下的 Agent 检测/配置测试，以及 macOS 包和托盘检查。

---

## v0.1.4 - 2026-07-13

本次发布引入 CodeasierRouter，即基于 Tauri 2 的桌面控制面板和共享管理服务，同时强化 CLI router 的 TLS、流式传输、后台进程、安装和进程身份行为。Release tooling 现在会为全部六个受支持的操作系统/架构目标构建并检查原生桌面包及匹配的 router/manager 产物。

### 新增

- 新增 Tauri 2 桌面控制面板，提供当前用户 router 生命周期管理、独立进程/上游健康状态、托盘操作、默认登录时启动、有界日志、诊断、设置和中英文界面。
- 新增经过验证的架构专用 `mtls-router-manager` 和凭据注入 `mtls-router` 桌面 sidecar，提供构建期/运行时哈希与架构校验，以及 manager version/target/deployment/protocol handshake。
- 新增安全的外部 CLI router 复用、`127.0.0.1:19099` 未知占用冲突处理、stale 进程身份保护和 degraded/stale 健康显示。
- 新增 Claude Code、opencode 和 Codex 检测、结构化预览、敏感备份、原子事务写入、陈旧预览拒绝，以及共享 Go manager 提供的回滚/恢复。
- 新增双语桌面操作和故障排查指南，覆盖安装、首次启动、Agent 安全边界、卸载、凭据轮换和包验证。

### 变更

- 将 router 生命周期和 Agent 文件管理提取到 `mtls-router-manager serve`，由桌面应用和安装脚本 wrapper 共享串行 line-delimited JSON stdin/stdout protocol。
- CLI release 安装改为把 router 和 manager 作为一组匹配二进制共同 staging、验证、安装并记录 receipt。
- 移除 `MTLS_ROUTER_OPENAI_API_KEY`。交互式 setup 隐藏读取 key；自动化必须先预览，再只通过 manager stdin 发送携带临时 key 的 `agent.write`。

### 修复

- 拒绝非 HTTPS upstream URL，并将配置的最低 TLS 版本一致应用于启动探测、`/health` 和代理上游流量。
- 保持访问日志链路中的即时响应流式传输，并让代理请求处理继续使用 reverse proxy 的直接流式链路，不引入透传 request body wrapper。
- 防止 detached 子进程继承 backend 模式并递归创建进程。
- 强化 router 停止和安装事务对缺失、陈旧或不匹配的进程身份与 release 产物状态的 reconciliation。
- 新的健康快照到达后清除可恢复的桌面 router 错误提示，同时保持 sidecar 完整性失败的 fail-closed 行为。

### 安全

- 桌面/manager 状态、日志、诊断、protocol 响应、进程参数和环境变量不会有意保留 Agent API key。Agent 自有文件和显式批准的恢复备份仍是持久化边界，必须按敏感数据保护。
- 明确记录分发二进制中共享内嵌客户端私钥可被提取，必须通过完整替代 release 和服务端吊销进行轮换。
- 卸载会保留 Agent 文件、敏感备份、日志和状态。Windows 安装器集成必须移除当前用户 autostart；macOS/Linux 用户必须执行**准备卸载**、等待应用退出，再删除应用。

### 测试

- 扩展 Go、shell、React、Rust 和 workflow 覆盖，验证 TLS policy、流式传输、后台子进程状态、进程身份、manager protocol、Agent 配置事务、桌面编排、包检查和签名状态报告。

### Release 状态

- CI 和 release workflow 现在会为全部六个目标构建原生桌面包：Windows x86_64/arm64 NSIS、macOS Intel/Apple Silicon DMG，以及 Linux x86_64/arm64 AppImage。每个匹配的目标 runner 都会执行强制包检查。Release job 仅在签名凭据完整时签名 Windows/macOS 包，仅在额外 Apple 凭据完整时 notarize/staple macOS 应用，并为每个目标生成一个明确的状态文件。包检查不会安装或启动应用，因此目标 runner 上成功安装/启动的独立证据仍是 release gate。

---

## v0.1.3 - 2026-07-10

本次发布重点提升 release 安装可靠性和代理行为正确性。打包后的安装脚本默认指向可访问的下载服务器；代理移除了不再使用的流式检测预读路径；客户端请求体读取失败会正确返回 bad request；`/health` 现在会按运行时配置探测实际上游目标。

### 变更

- 简化代理请求处理，移除未使用的流式检测预读路径，同时保留反向代理直接流式转发行为。
- 调整 release 打包安装脚本默认值，使安装器默认从配置的发布服务器地址下载二进制文件。

### 修复

- 修复打包安装脚本使用下载服务器直连 IP，避免部分环境下域名不可达导致安装失败。
- 修复客户端请求体读取失败的错误分类，使其返回 `400 Bad Request`，不再误归类为上游代理失败。
- 修复 `/health` 未传入运行时探测配置的问题，使健康检查指向当前配置的上游目标。

### 测试

- 新增客户端请求体读取错误分类和 health probe 配置传递的回归覆盖。
- 保持安装脚本测试覆盖与 release 下载默认值、router health 行为同步。

---

## v0.1.2 - 2026-06-21

### 新增

- 新增拆分后的安装入口命令，分别用于 router 安装和 agent 配置。
- 新增已有 opencode JSONC 配置的交互式迁移流程。
- 新增安装流程中的 router 生命周期管理命令，支持启动、停止、重启和状态类操作。
- 新增全构建产物统一注入的构建元信息：
  - version
  - commit
  - build date
- 新增内部构建信息端点：`/internal/version`。
- 新增 router listener 管理端点：
  - `/version`
  - `/health`

### 变更

- 调整 Codex CLI 安装配置，改为使用最小化 custom provider 配置。
- 更新 opencode 安装配置，使用 `/v1` base URL，并适配 JSONC 配置格式。
- 调整 Claude 和 opencode 目标的模型 ID，去掉 `cx/` 前缀。
- 更新 README，使其与当前安装脚本默认行为一致，并补充管理端点说明。

### 修复

- 修复安装流程在仅写入配置时意外启动 router 的问题。
- 修复默认日志位置，避免将 setup 日志写入安装目录。
- 修复 Windows 下 router 启动行为。
- 修复 Windows PowerShell 脚本编码和 JSON 解析行为。
- 修复 Windows 下 Codex CLI 配置匹配和认证文件生成问题，包括无 BOM 的 `auth.json` 输出。
- 加强安装脚本中 router 生命周期命令的稳定性。

### 测试

- 更新非交互式安装参数流程的 shell 测试。
- 扩展 PowerShell JSON 处理和生命周期相关行为的安装脚本测试覆盖。

---

## v0.1.1 - 2026-06-19

### 新增

- 新增 macOS/Linux 和 Windows 一键安装脚本：
  - `setup.sh`
  - `setup.ps1`
- 安装脚本支持自动下载当前平台和 CPU 架构对应的最新 release 二进制文件。
- 新增交互式配置向导，支持配置以下本地 agent：
  - Claude Code
  - opencode
  - Codex CLI
- 修改 agent 配置前会自动备份已有配置文件。
- 新增后台运行模式：`-backend`。
- 新增日志文件参数：`-log`。
- 新增后台模式和日志路径环境变量：
  - `MTLS_BACKEND`
  - `MTLS_LOG`
- 新增跨平台后台进程启动支持：
  - Unix/macOS/Linux 使用独立进程会话
  - Windows 使用 detached process 创建方式
- 新增安装脚本测试套件。
- 新增 PowerShell 安装流程测试。
- 新增 `make test-shell`，用于运行 shell 安装脚本测试。
- 新增维护者构建和发布文档：`docs/BUILD.md`。

### 变更

- 更新 README，补充一键安装、手动下载、Windows 使用、后台模式和 agent 配置说明。
- 将详细构建和发布说明从 README 移动到 `docs/BUILD.md`。
- CI 新增 shell 安装脚本测试。
- 改进 meta flag 处理，使运行时参数可以正确透传。
- 补充 Windows release 使用方式和生产环境服务托管建议。

### 修复

- 修复 `-backend`、`-log` 等运行时参数的解析问题。
- 修复 Windows 安装向导行为，使其与 Unix 安装流程更一致。
- 修复 Windows 下 agent 配置行为。
- 收紧后台启动集成和日志文件处理。

### 测试

- 新增后台参数处理和日志行为的 Go 测试。
- 新增配置字段的 Go 测试。
- 新增 `-version`、`-help` 和运行时参数处理测试。
- 新增安装脚本测试，覆盖 clean setup、latest version detection、target selection、Claude Code、opencode、Codex CLI 和 PowerShell 流程。

---

## v0.1.0 - 2026-06-18

### 新增

- `mtls-router` 首个发布版本。
- 新增单二进制本地反向代理，用于将本地 plain HTTP 流量转发到上游 HTTPS mTLS 服务。
- 新增通过构建期 linker variables 嵌入证书和配置的能力：
  - `main.clientCertPEM`
  - `main.clientKeyPEM`
  - `main.upstreamCAPEM`
  - `main.upstreamURL`
  - `main.version`
- 新增本地 HTTP 监听，默认地址为 `127.0.0.1:19099`。
- 新增基于嵌入式客户端证书、私钥和上游 CA 的 mTLS transport。
- 启动时会先探测上游健康状态，成功后才开始接受本地流量。
- 新增请求转发到配置的上游 URL。
- 新增透明请求体流式转发。
- 新增 Server-Sent Events 响应处理，并保留适合流式传输的响应头。
- 新增对包含 `"stream": true` 的 JSON 请求的流式请求检测。
- 新增结构化访问日志。
- 新增 `SIGINT` 和 `SIGTERM` 优雅退出。
- 支持通过参数、环境变量、构建期值和默认值进行运行时配置。
- 新增本地开发构建脚本：`scripts/build.sh`。
- 本地开发构建支持自动生成占位证书文件。
- 新增 GitHub Actions CI workflow。
- 新增 GitHub Actions release workflow。
- 新增 Linux、macOS、Windows 的 amd64 和 arm64 release 交叉编译。
- 新增 Docker 支持：`Dockerfile`。
- 新增 systemd 服务单元：`systemd/mtls-router.service`。
- 新增 README、设计文档、实现计划文档和 MIT license。

### 配置

- 新增配置优先级：参数 > 环境变量 > 构建期值 > 默认值。
- 新增环境变量：
  - `MTLS_LISTEN_ADDR`
  - `MTLS_UPSTREAM_URL`
  - `MTLS_TLS_MIN`
  - `MTLS_TIMEOUT`
  - `MTLS_DEBUG`
- 新增运行时参数：
  - `-listen`
  - `-upstream`
  - `-tls-min`
  - `-timeout`
  - `-debug`
  - `-version`
  - `-help`
  - `-h`

### 测试

- 新增证书加载与校验单元测试。
- 新增配置加载与校验单元测试。
- 新增上游健康探测单元测试。
- 新增日志辅助函数单元测试。
- 新增反向代理 director、错误处理、响应修改、流式检测和 mTLS transport setup 相关单元测试。
