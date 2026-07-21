# 更新日志

[English](../CHANGELOG.md)

## 未发布

### 新增

- 新增经过认证的 `GET /v1/models` 发现，以及 Claude Code、opencode 和 Codex 共用且兼容全部 endpoint 的模型目录。
- 新增 protocol-v2 `agent.models`/`agent.render`、无 key 规范 model config、Agent 原生选项、脱敏 render/preview、写入时目录刷新、托管所有权状态，以及漂移/Codex-auth 批准。
- 为每个显式 Claude 选择新增可选显示名称和规范 `context: "1m"`。规范与目录身份始终使用 base model ID；仅在 Claude 渲染边界追加 `[1m]`，不推断能力，也不管理 `CLAUDE_CODE_DISABLE_1M_CONTEXT`。
- 为 manager 二进制新增不可变、无 key 的构建 preset。Protocol v2 现在会在对每个已请求 Agent section 独立执行认证校验后，返回稳定的 `preset.model_config` 和 `preset.unavailable_agents` object。
- 新增仅供 manager 使用的 `SIMPLIFY` 构建策略。未设置/空值和 ASCII 大小写 boolean 会在编译前规范化；默认 `True` 排除包含 ASCII `/` 的有效 ID，`False` 保留全部有效 ID。

### 变更

- Shell、PowerShell 和桌面端改为 key-before-discovery，对每个 Agent 使用可编辑的 `existing > preset > empty` 初始化，省略未设置可选字段，并取消静态/cache 模型 fallback。显式 `--model-config` 和桌面导入仍是完整替换。
- 精确调整自动选择行为：只有精确 ID 通过当前认证目录校验的可见构建 preset 才能初始化表单。仍禁止选择第一个模型、使用模型名称或能力 heuristic、部分修复 preset、替换模型和运行时 fallback。
- Claude 改为 managed `env` merge；opencode 改为精确选择的 provider 目录；Codex 从历史 `custom` provider 迁移到专用 `mtls-router`，并单独批准 file-backed API-key auth。
- 检测现在只描述本地结构完整性；当前授权只能由模型发现和写入时刷新证明。
- 将经过校验、去重、数量限制、排序和构建过滤的目录作为 protocol token/result、existing 与 preset 可用性、导入、预览和写入时刷新的权威依据。过滤发生在完整校验之后，因此隐藏位置的 malformed ID 仍返回 `MODEL_RESPONSE_INVALID`，全部被过滤时返回 `MODEL_CATALOG_EMPTY`，刷新时模型消失仍会安全失败。

### 安全与发布

- 新增私有签名 catalog/revision state、共享操作 lock、事务 sidecar/备份、兼容性 revision pin，以及拒绝混合 protocol generation 的 release preflight。
- 明确 Agent 文件与批准的备份可能含 key，而 model config、token、sidecar、日志、诊断和 protocol result 不含 key。
- 新增可选 `AGENT_MODEL_PRESET_BASE64` release input，并提供 preflight 校验和相同的 standalone/desktop manager 注入。无效非空输入会让 manager 启动失败且不泄漏内容；空输入有效，router binary（包括 desktop router sidecar）绝不会收到 preset 数据。
- Release 构建会在编译前规范化 `SIMPLIFY`，并只向 standalone 和 desktop manager 注入同一个值。它不是 router/运行时偏好，也不会改变 proxy 路由支持。

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
