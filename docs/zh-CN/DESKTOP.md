# 桌面应用

[English](../DESKTOP.md)

Tauri 桌面应用是固定服务 `mtls-router` 的当前用户控制面板。它打包了架构匹配的 `mtls-router-manager` 和 `mtls-router` sidecar；不会把任一 sidecar 安装到 `PATH`，也不提供证书、上游或 sidecar 替换控件。

> 当前仓库中的 CI 和 release workflow 会构建六个原生桌面包：Windows x86_64/arm64 NSIS 安装器、macOS Intel/Apple Silicon DMG，以及 Linux x86_64/arm64 AppImage。每个 package job 都在匹配的目标 runner 上执行检查。Release 签名取决于平台凭据，macOS notarization/stapling 还需要完整的 Apple notarization 凭据；每个目标的状态文件会记录结果。包检查不会安装或启动应用，因此仍需单独提供目标 runner 上成功启动的证据。详见[构建与发布](BUILD.md)。

## 安装

从可信的内部分发渠道获取与操作系统及 CPU 架构匹配的包。不要把 sidecar 从桌面包中移出、替换或单独运行。

- Windows：运行与 x86_64 或 arm64 匹配的当前用户安装器。应用设计不要求管理员提权。
- macOS：打开与 Intel 或 Apple Silicon 匹配的 DMG，然后把 `CodeasierRouter.app` 拖到 Applications 快捷方式上。
- Linux：给与 x86_64 或 arm64 匹配的 AppImage 添加执行权限，再以当前用户启动。

如果操作系统提示包未签名、未 notarize、已损坏或发布者未知，请停止安装并向分发方核实 release 状态。不要只根据包文件名绕过平台安全检查。

Release asset 集中每个桌面包都有一个 `.sha256` 文件和一个 `signing-status-<os>-<arch>.txt` 文件。它们证明记录的 checksum 和签名/notarization 结果，不证明安装或启动成功。还必须取得[包验证](BUILD.md#包验证)要求的独立目标平台启动证据。

## 首次启动

应用首次启动时会：

1. 根据目标架构和构建期 SHA-256 校验打包的 manager/router sidecar，然后检查 manager 的目标平台、版本、deployment ID 和 management protocol。
2. 检查 `127.0.0.1:19099`。
3. 复用由 CLI 安装脚本启动且可信兼容的 router；端口空闲时则启动打包的 router。
4. 打开 Router 页面，分别检查进程可用性和上游 mTLS 健康状态。
5. 默认启用当前用户登录时启动。无需管理员权限即可立即在设置中关闭。

首次启动绝不会修改 Claude Code、opencode 或 Codex 文件。第二次启动应用只会激活现有窗口，不会再启动一组 manager/router。

如果任一 sidecar 缺失、不可执行、被修改或架构错误，启动会安全失败并提示需要重新安装。应用不会自行下载或替换 sidecar；请从可信来源重新安装完整桌面包。

## Router 所有权和状态

Router 页面会区分本地进程和上游健康。运行中的进程可能处于健康、降级，或健康结果已超过 30 秒的 stale 状态。

- **桌面托管 router：**应用监督一个前台子进程。只有 PID、启动标识、可执行文件标识和所有权都通过检查时，停止和退出操作才可终止它。
- **外部 router：**只有 CLI 安装脚本管理，且记录的进程标识、`deployment_id` 和 `management_protocol_version` 都与桌面构建一致的 router 才能复用。不能只根据手工启动进程的 `/version` 响应结构信任它。
- **未知端口占用者：**应用绝不会自动终止它，也不会切换到其他端口。如果 manager 能证明唯一且可识别的 listener 属于当前用户，Router 页面会提供显式的**强制终止占用进程**恢复操作。确认对话框会显示进程名称、PID 和完整可执行文件路径，并警告终止会立即执行且可能导致未保存数据丢失。该操作不会请求提权，不适用于其他用户、身份不明确、已变化、受保护或无法验证的进程，释放端口后也不会启动 router。如果无法检查或终止，请使用操作系统工具识别并停止或重新配置 listener，然后重试。
- **陈旧状态：**PID 或可执行文件不匹配会报告 stale，且不会发送信号。人工清理前先核实进程和状态。
- **降级或陈旧健康：**router 进程可能仍接受本地连接，但当前无法证明上游可达。请重试健康检查并查看日志；不要把 stale 健康结果当作健康。

恢复步骤见[故障排查](TROUBLESHOOTING.md)。

## 托盘、关闭和退出

关闭主窗口只会把它隐藏到系统托盘，不会改变 router。可以通过托盘图标打开窗口、启动或停止符合条件的 router、打开日志或退出。

退出与关闭窗口不同。退出会停止经过验证且由桌面应用所有的 router，然后退出应用。它绝不会停止兼容的外部 router 或无法验证的进程。未知占用者不能使用常规停止操作，托盘也不提供强制终止操作；只能在 Router 页面显式确认后终止占用进程。登录时启动会在下次用户登录后启动桌面应用，并执行同样的发现和所有权规则；它绝不会终止占用者，也不会盲目启动第二个 router。

## 配置 Agent

Agent 页面支持 Claude Code、opencode 和 Codex。检测只返回元数据：路径、格式、是否存在、可写性以及已配置或无效状态，不返回已存储的 API key 值。检测遵循 `CLAUDE_CONFIG_DIR`、`OPENCODE_CONFIG` 和 `CODEX_HOME`；Codex home 目录存在时也可识别 Codex 桌面安装。

Agent 配置必须显式执行 key-before-discovery 流程：

1. 刷新检测，只选择有效且可写的 Agent。
2. 输入 API key。React 会立即清空输入框；Rust 只在一次性且不可 replay 的临时流程中保留它。
3. 通过可信本地 router 发现 manager 经过认证和构建过滤的模型目录。默认会排除包含 ASCII `/` 的有效 ID；使用 `SIMPLIFY=False` 构建的 release 会保留它们。该过滤目录是所有已选 Agent、导入配置、preset、预览和刷新的权威依据。这是不可变的 manager 构建策略，不是运行时偏好，也不是对 proxy 路由支持的限制。桌面端绝不会选择第一个模型、按模型名称或能力推断选择，也不会替换模型。可见的构建 preset 只有在 manager 根据该目录验证某个 section 的全部精确 ID 后，才能提供该 section 作为可编辑初始值。
4. 每个 Agent 独立采用 `existing > preset > empty` 初始化；界面会标明 section 来自 existing 配置还是推荐 preset，并为不可用的完整 preset section 列出缺失 base ID。配置各 Agent 原生选择：Claude 主模型/角色继承以及可选显示名称和 Standard/1M context、opencode 模型子集/默认/选项，以及一个 Codex 模型/选项。Claude 提供本地化的**启用 Fable**控件。空 Claude 表单默认禁用 Fable；启用时创建 inherit-primary 选择，并显示与现有角色相同的继承/显式模型、显示名称和 Standard/1M 控件；禁用时删除完整 Fable 选择及其 metadata。Existing Claude 作为完整 section 优先于 preset Claude，因此不会把 preset Fable 合并进省略 Fable 的 existing Claude section。Preset 值始终可编辑且不代表任何批准。未设置的可选字段保持省略。可以导入或导出无 key 的规范 model config；导入会完整替换当前表单，而不是与 existing 或 preset 值合并，并在导出、预览与写入中精确保留已启用或省略的 Fable。
5. 生成结构化预览，审查脱敏片段以及每个创建、替换、保留、迁移、漂移批准、状态和备份操作。不设置显式 `OPENCODE_CONFIG` 时，标准 `~/.config/opencode/opencode.jsonc` 会迁移到同目录的 `opencode.json`；已有同名 sibling 会构成迁移冲突。显式 `OPENCODE_CONFIG` 指定 `.jsonc` 文件时，只会在该精确路径原地替换为 strict JSON；已有文件会备份，并且不会触碰 sibling `opencode.json`。两种 JSONC 操作都会丢失注释和格式。Codex 可能同时修改 `config.toml` 和 `auth.json`，切换 file-backed API-key auth 需要单独批准。
6. 批准并写入。Manager 会消耗内存中的 key，在创建任何写入产物前刷新目录，随后检查修改和备份路径。

写入前，manager 会再次确认文件仍与已批准的 revision 一致。`PREVIEW_STALE` 表示目标已变化；此时不会开始写入，必须重新预览。替换现有文件前会创建备份。一次多 Agent 写入是一个事务：失败时会回滚本事务已经修改的文件，但保留诊断备份。如果无法证明回滚成功，后续 Agent 写入会安全失败。

备份保留在原配置旁边，可能包含旧 API key。它们属于敏感恢复产物，应当像原 Agent 文件一样保护、保留或删除。预览和结果页面会标明备份路径，但绝不会显示备份内容。

检测中的 `configured` 仅表示本地托管字段结构完整且内部一致，不证明所选模型当前已获授权。需要手工刷新时重新进入配置并提供 key；系统不会后台同步目录或重写 Agent 文件。目录/认证/校验失败、模型消失、未批准漂移或所有权状态无效都会安全失败，不使用静态/cache fallback，也不会部分写入。服务契约、规范 schema、省略、迁移与所有权规则见 [Agent 模型配置](AGENT_MODELS.md)。

对于 Claude，规范配置会分别存储认证 base model ID 和可选精确字段 `context: "1m"`。启用 Fable 会渲染 `ANTHROPIC_DEFAULT_FABLE_MODEL` 及可选的 `ANTHROPIC_DEFAULT_FABLE_MODEL_NAME`；省略 Fable 时不会渲染或认领这两个 key。启用 Fable 要认领已有未托管值时，预览会显示 collision；禁用时只删除能证明之前由 manager 所有的 stale 路径，并保留从未取得所有权的手工 key。Manager 只在渲染 Claude 模型环境变量时追加 `[1m]`；它不会推断 1M 支持，也不会写入 `CLAUDE_CODE_DISABLE_1M_CONTEXT`。运行时拒绝不会触发 fallback 或重写。Fable alias 要求 Claude Code 2.1.170 或更高版本。与此独立，数值 custom-model context override 从 Claude Code 2.1.193 起可直接作用于未知模型名称；更早版本可能忽略它。Preset discovery 本身不会写 Agent 文件或 manager 事务状态，preset 数据也不会存入桌面端 secret-bearing flow。

## API key 边界和限制

桌面应用会把输入 key 提交给 `agent.models`，立即清空密码输入框，并只在 zeroizing Rust 临时流程状态中保留到一次 `agent.write`。Timeout、malformed response、manager restart 或 uncertain delivery 后，secret-bearing 调用绝不会自动 replay。应用不会有意把 key 放入桌面或 manager 持久状态、进程参数、环境变量、日志、诊断、model config、catalog/revision token、预览响应或写入响应。

所选 Agent 的配置文件仍需按该 Agent 的要求持久化 key。用户批准的恢复备份也可能持久化旧 key。清除 JavaScript 和 Rust 中的应用引用只是 best effort，不保证能从进程或操作系统内存中进行取证级擦除。

`MTLS_ROUTER_OPENAI_API_KEY` 已移除，不再提供 key。精确的非交互 manager 自动化见 [stdin manager 自动化](#stdin-manager-自动化)。

## stdin manager 自动化

自动化必须调用经安装 receipt 验证的 `mtls-router-manager serve`，要求 `manager.info` protocol `2`，使用临时 key 调用 `agent.models`，构造规范 model config，调用无 key 的 `agent.render` 或 `agent.preview`，最后使用 revision token、显式 approval 和临时 key 调用 `agent.write`。Key 不得出现在参数、导出的环境变量、model-config value、日志或临时请求文件中。Catalog token 可以有意跨 one-shot manager 进程验证。精确流程见 [protocol v2 自动化契约](AGENT_MODELS.md#protocol-v2-自动化)。

## 凭据模型

每个生产包绑定一个服务环境。router sidecar 包含共享客户端证书、共享私钥、上游 CA 和默认上游 URL。获得包的用户可以提取这些内嵌值；打包方式和桌面 UI 无法阻止提取。桌面包只能分发给可信内部用户。

吊销或轮换需要：使用新凭据材料构建替代 release，分发完整替代包，并在服务端拒绝旧凭据。应用不支持运行时证书导入、profile 切换、sidecar 更新或自动更新。

本地 listener 是可信 localhost 上的 plain HTTP。不要把管理端点或 listener 暴露到公网。

## 卸载

- Windows：先退出应用，再从 Windows 设置中卸载。生产安装器必须在卸载时移除桌面应用的当前用户登录启动注册。
- macOS 和 Linux：打开设置，选择**准备卸载**并确认。等待应用移除当前用户登录启动项并退出，之后才能删除 macOS 应用或 Linux AppImage。

任何平台的卸载流程都不会恢复、删除或重写 Agent 配置、敏感备份、router 日志、应用状态或诊断状态。只有在确认恢复和诊断保留需求后，才应单独删除这些内容。
