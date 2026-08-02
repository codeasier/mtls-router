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

如果任一 sidecar 缺失、不可执行、被修改或架构错误，启动会安全失败并提示需要重新安装。应用绝不会单独下载或替换某个 sidecar；请从可信来源重新安装完整桌面包，或在已安装应用仍能执行更新时使用有效的桌面整包更新。

## 在线更新

Stable 桌面 release 在 Windows x86_64/arm64、macOS Intel/Apple Silicon 和 Linux x86_64/arm64 上支持整包在线更新。更新会同时替换桌面应用及其匹配的 `mtls-router-manager` 和 `mtls-router` sidecar，不是只更新 sidecar。该能力仅属于桌面应用，不会新增或改变 CLI 二进制或安装脚本的更新行为。

应用每次启动会静默检查一次 `https://downloads.codeasier.top/mtls-router/latest/latest.json`；检查失败不会阻断启动。设置页面会显示当前和最新桌面版本、release 提供的更新说明，以及手动**检查更新**操作。应用只接受更高版本的 stable SemVer release；不会提供 prerelease 或含 build metadata 的版本，validation/非 stable 构建也不会发布到桌面更新 channel。

应用绝不会在未经确认时下载或安装更新。你确认界面显示的 stable 版本后，应用才会下载平台包、报告进度、校验强制的 Tauri updater 签名、只停止经过验证且由桌面端所有的 router、安装完整包并重启应用。兼容的外部 router 不会被停止。如果停止桌面端所属 router 后安装失败，应用会尝试重新启动该 router。

在每个平台上，只有当前包运行在对应平台 Tauri updater 支持的安装位置和文件系统布局中时，才能执行在线安装。如果 updater 报告当前位置不受支持或无法替换已安装包，请退出应用，再从可信 release 渠道手工安装完整新包；不要单独替换打包的 manager 或 router。

## Router 所有权和状态

Router 页面会区分本地进程和上游健康。运行中的进程可能处于健康、降级，或健康结果已超过 30 秒的 stale 状态。

- **桌面托管 router：**应用监督一个前台子进程。只有 PID、启动标识、可执行文件标识和所有权都通过检查时，停止和退出操作才可终止它。
- **外部 router：**只有 CLI 安装脚本管理，且记录的进程标识、`deployment_id` 和 `management_protocol_version` 都与桌面构建一致的 router 才能复用。不能只根据手工启动进程的 `/version` 响应结构信任它。
- **未知端口占用者：**应用绝不会自动终止它，也不会切换到其他端口。Protocol v4 检查会返回结构化恢复 action 和稳定 reason：符合条件的目标为 `force_terminate`，service、权限不足或其他用户为 `manual_stop_required`，生命周期已知的受保护进程或身份不可用为 `unavailable`。`insufficient_privilege` 也涵盖 Windows PPL 等操作系统保护拒绝终止访问的情况；访问被拒绝并不能可靠识别 PPL。`protected_process` 仍只用于应用生命周期已知的受保护目标。只有 `force_terminate` 包含短时、一次性确认 token。默认要求完整身份：只有 manager 验证当前用户所有权、进程名称、PID、启动标识、完整可执行文件路径和 listener 身份后，Router 页面才会提供**强制终止占用进程**。确认对话框会显示已验证详情，并警告终止会立即执行且可能导致未保存数据丢失。
- **Windows 仅 PID 例外：**如果进程 SID 可读且与桌面用户不同，Windows 会以 `different_user` 拒绝，绝不会把该已知身份降级为仅 PID。SID 或完整进程身份无法读取时可以考虑仅 PID，但前提是 TCP4 owner table 为精确的 `127.0.0.1:19099` listener 找到唯一 PID、已排除受保护生命周期目标，并且无副作用的 `PROCESS_TERMINATE` 权限预检成功。面板只显示 PID，并警告身份、所有者、启动时间和可执行文件仍未验证。终止前一刻，manager 会重复精确 owner 和保护检查并重新取得终止权限；listener 消失、变化、重复、wildcard、格式错误、不明确或此时无权访问都会被拒绝且不发送信号。无法读取的生命周期状态和 PID 重用仍是残余风险。应用绝不请求提权。
- **受 supervisor 管理或被阻断的占用者：**可靠识别的 Windows Service 或 Linux systemd 用户/系统 service 绝不会被强制终止。页面会显示有界且经过验证的 service/unit 标识，以及 `services.msc` / `sc.exe stop`、`systemctl --user stop` 或 `sudo systemctl stop` 固定示例。复制的 Windows `sc.exe` 命令经过安全引用，但仅适用于管理员 **PowerShell** 会话，不是 `cmd.exe` 命令。所有示例都只供人工参考：应用既不执行命令，也不请求 administrator/root 权限。权限不足和其他用户进程同样必须在正确身份或权限上下文中人工处理；不要提升桌面应用权限。macOS manager 不猜测 launchd label，因此经过验证的普通同用户进程仍可能允许强制终止；随后重新占用时只提供通用活动监视器/`launchctl` 引导。
- **终止与观察：**在完整身份模式下，manager 成功证明已确认的进程身份变为不存在且端口曾被观察到释放。在 Windows 仅 PID 模式下，成功只证明终止请求成功，并且原 listener PID 已从该精确端口消失；它不独立证明进程已完全结束。两种模式都不启动 router，也不承诺持续释放。桌面端随后在约 10 秒内定期采样状态。**端口保持释放**表示采样检查发现端口已释放且未检测到重新占用；采样发现新占用者时会显示**服务或守护程序重新占用端口**并触发新检查。两次检查之间的重新占用可能无法检测。主动启动 router 会取消观察，避免把桌面自有 router 误报为重新占用。
- **人工恢复与错误：**如果恢复不可用、你不接受引导或终止失败，请使用操作系统工具识别并停止或重新配置 listener，然后重试。确认阶段权限丢失、终止请求失败和未能及时证明端口释放会分别报告 `OCCUPANT_PERMISSION_DENIED`、`OCCUPANT_TERMINATION_FAILED` 和 `PORT_RELEASE_TIMEOUT`；这些错误都不表示向替代进程发送过信号。
- **陈旧状态：**PID 或可执行文件不匹配会报告 stale，且不会发送信号。人工清理前先核实进程和状态。
- **降级或陈旧健康：**router 进程可能仍接受本地连接，但当前无法证明上游可达。请重试健康检查并查看日志；不要把 stale 健康结果当作健康。

恢复步骤见[故障排查](TROUBLESHOOTING.md)。

## 托盘、关闭和退出

关闭主窗口只会把它隐藏到系统托盘，不会改变 router。在 macOS 上应用还会取消激活，让先前前台应用重新获得焦点；通过托盘图标或再次启动可重新激活并聚焦窗口。可以通过托盘图标打开窗口、启动或停止符合条件的 router、打开日志或退出。

退出与关闭窗口不同。退出会停止经过验证且由桌面应用所有的 router，然后退出应用。它绝不会停止兼容的外部 router 或无法验证的进程。未知占用者不能使用常规停止操作，托盘也不提供强制终止操作；只能在 Router 页面显式确认后终止占用进程。登录时启动会在下次用户登录后启动桌面应用，并执行同样的发现和所有权规则；它绝不会终止占用者，也不会盲目启动第二个 router。关闭标题栏仍然只隐藏窗口，并保留内存中的 Agent 草稿。在应用内离开 Agent 面板或通过任何可拦截路径真正退出时，未保存草稿都需要先确认放弃；preview、write 和写后 reload 执行期间不能离开。

## 配置 Agent

Agent 总览始终以独立卡片展示 Claude Code、OpenCode 和 Codex。每张卡片只显示配置文件是否存在、是否可写、是否有效、是否已配置以及恢复资格。检测只返回元数据，绝不返回已保存的 API key 明文，也不在 `PATH` 中查找或依赖 Agent CLI 命令。检测遵循 `CLAUDE_CONFIG_DIR`、`OPENCODE_CONFIG` 和 `CODEX_HOME`。

只要目标路径可写，每个受支持 Agent 都始终可以进入配置管理。每张卡片只会为对应的一个 Agent 打开持续面板；写入成功后编辑器仍然可用，可以反复维护配置。桌面端不会安装、启动或调用 Agent CLI，也不会在同一面板中选择或批量处理多个 Agent。

当 `agent.detect` 确认该 Agent 存在有效的 last-applied sidecar 条目且 Agent 写入可用时，卡片还会提供**清理 CodeasierRouter 配置**。清理按单个 Agent 执行，不是批量操作；它不使用 router、模型目录、model flow 或全局 API key。所有权缺失、无效或因未解决事务恢复而受阻时，自动清理会安全失败，不会根据配置外观猜测所有权；应保留文件和备份，并且只在解决事务状态后手工处理托管字段。

清理会先显示无 key 审阅，其中包含精确托管路径名称、文件 `replace` 或 `delete` 影响、计划备份及其敏感性，以及 last-applied sidecar 变化。它只删除由 sidecar 和当前结构共同证明的所有权：Claude 的托管 `env.*` key；OpenCode 的 `provider.mtls-router`，以及在该根字段受管且当前值仍使用精确 `mtls-router/` 前缀时的根 `model`；Codex 的 `model_providers.mtls-router`、必需托管 model/auth-store 根字段、仅由已保存 Codex model config 表明曾生成的可选根字段，以及 `auth.json` 中的 `auth_mode` 和 `OPENAI_API_KEY`。其他 provider、设置和认证 metadata 会保留。Codex OS keyring 凭据不会删除，先前配置写入时被替换的 auth 字段也不会重建。

即使只改变了无关字段，整文件 revision 漂移也需要单独确认。已记录但已经不存在的 Agent 目标会视为已清理并要求该确认；不会为它创建备份或执行修改，其他目标和 sidecar 仍会继续纳入事务。其他目标读取错误仍会 fail closed。已签名 cleanup preview 会在写入前和 journal 前重新校验；已缺失目标还会在每次修改前，以及 sidecar 变更后、commit 前立即再次校验。目标重新出现时返回 `PREVIEW_STALE`，回滚先前变更并保留 sidecar 所有权。修改前，每个受影响的现有 Agent 目标和 manager sidecar 都会创建并验证私有备份，然后在同一个 journal 事务中变更，sidecar 始终最后处理。所有 Agent 文件备份都标记为敏感，因为保留的用户字段可能含凭据；manager sidecar 备份不敏感。现存文件的语义根已经为空，或删除托管字段后变空时，都会先备份再删除；否则会用保留后的内容替换。移除最后一个 Agent 条目时会删除最终 sidecar。操作失败或中断时，rollback 和启动恢复会一起恢复 Agent 文件与 sidecar。

清理会删除所选 Agent 文件中的托管认证字段，但绝不会删除桌面 `credentials.json` 中的全局 key，也不会删除任何历史 `*.bak-*` / `*.rollback-*` 文件。新生成的清理备份在成功后同样保留，并可能含 API key 或其他凭据。应把它们作为敏感恢复产物保护和审查；确认不再需要后再单独删除。

正常**合并**会保留受支持的无关配置。**备份并重建**是独立的破坏性恢复操作，只会为符合条件的语法无效文件显示。它会用纯托管输出替换完整的已批准 Agent 文件集；无关设置、注释、格式以及有效伴随文件中的元数据都会丢失。备份可能包含 API key。继续前必须确认这些损失并保护每个备份。

Agent 配置采用以下单 Agent 流程：

1. 打开卡片。面板始终重新执行 `agent.detect`，只读取凭据摘要；存在 key 且目标可编辑时，再为该 Agent 调用 `agent.models`。有效且可写的 Agent 使用**合并**；无效 Agent 只有在检测标记为符合条件时才能使用**备份并重建**，其他无效配置必须手工修复。
2. Rust 会加载由 API 密钥页面管理的全局 API key，并通过可信本地 router 仅为该 Agent 发现 manager 经过认证和构建过滤的模型目录。总览既不加载模型，也不使用凭据。初次或完整 reload discovery 没有 key、认证失败或目录不可用时，面板会保持打开，只展示安全检测 metadata 和恢复操作，不展示字段级配置。该边界是因为 render 和 preview 都需要由已认证 `agent.models` discovery 签发的 catalog token。后台刷新失败时则保留已有草稿与 active flow，提示无法验证外部状态，并继续允许仍然安全的操作。Webview 绝不会收到 key。
3. 默认会排除包含 ASCII `/` 的有效模型 ID；使用 `SIMPLIFY=False` 构建的 release 会保留它们。该过滤目录是目标 Agent、导入配置、preset、预览和刷新的权威依据。这是不可变的 manager 构建策略，不是运行时偏好，也不是对 proxy 路由支持的限制。桌面端绝不会选择第一个模型、按模型名称或能力推断选择，也不会替换模型。可见的构建 preset 只有在 manager 根据该目录验证该 section 的全部精确 ID 后，才能提供该 section 作为可编辑初始值。
4. 目标 Agent 采用 `existing > preset > empty` 初始化；界面会标明其 section 来自 existing 配置还是推荐 preset，并为不可用的完整 preset section 列出缺失 base ID。配置 Agent 原生选择：Claude 主模型/角色继承以及可选显示名称和 Standard/1M context、OpenCode 模型子集/默认/选项，或一个 Codex 模型/选项。Claude 提供本地化的**启用 Fable**控件。空 Claude 表单默认禁用 Fable；启用时创建 inherit-primary 选择，并显示与现有角色相同的继承/显式模型、显示名称和 Standard/1M 控件；禁用时删除完整 Fable 选择及其 metadata。Existing Claude 作为完整 section 优先于 preset Claude，因此不会把 preset Fable 合并进省略 Fable 的 existing Claude section。Preset 值始终可编辑且不代表任何批准。未设置的可选字段保持省略。可以导入或导出无 key 的规范 model config；导入会完整替换当前表单，而不是与 existing 或 preset 值合并，并在导出、预览与写入中精确保留已启用或省略的 Fable。
5. 生成精确的结构化预览，审查脱敏片段以及每个创建、替换、保留、迁移、漂移批准、状态和备份操作。预览规范化后的 model config 会成为写入输入。正常 merge 在未设置显式 `OPENCODE_CONFIG` 时，会把标准 `~/.config/opencode/opencode.jsonc` 迁移到 sibling `opencode.json`；已有 sibling 会构成迁移冲突。显式 `.jsonc` override 会原地规范化。Rebuild 则在已批准 JSON 或 JSONC 路径原地替换为 strict JSON，不执行 sibling 迁移。Codex rebuild 始终替换 `config.toml` 和 `auth.json`，创建缺失的伴随文件并丢弃有效伴随 metadata；该文件对中每个现有文件都必须备份。
6. 批准并写入。托管漂移和 Codex 认证变更仍需显式批准。Rebuild 需要单独的破坏性确认，并且必须与单 Agent 预览精确一致。创建任何写入产物前，Rust 会重新加载已保存的全局 key，manager 会刷新目录。事务成功会立即显示，然后面板重新执行 detection、凭据摘要和模型发现，用新 flow 加载实际文件。重载失败不会改变写入成功结论；重试会完整执行加载流程。结果可以关闭，无需离开面板。

可编辑面板打开期间，原生窗口 focus 事件最多每 15 秒检查一次外部文件变化。手动刷新可以绕过该间隔，但所有刷新始终保持 single-flight。Clean 草稿会采用最新配置；未保存草稿绝不会被覆盖：外部状态未变时保留草稿，发生变化时必须选择保留草稿或加载磁盘状态，目标变为不兼容或不可写时则阻止编辑，直到在 active flow 仍有效时导出、放弃草稿或安全恢复。刷新不轮询，也绝不会写 Agent 文件。

导入与导出仍是单 Agent、无 key 的规范 JSON 操作，但都要求有效 active model flow。导入会替换当前草稿并使已有 preview 失效；导出不改变草稿、preview 或 approval 状态。存在冲突时必须先解决冲突才能导入。如果 stale/expired flow 无法重新发现，内存草稿仍然可见，但在恢复有效 flow 前不能执行受 catalog 校验的导出。

重建资格要求至少一个文件语法无效，并且完整托管文件集没有其他 blocker。结构有效但不受支持、文件不可读或超限、非普通文件、链接、路径不可写、父目录不可用、存在待恢复事务或禁用写入时，都不符合条件。语法有效和 parser 兼容性问题必须修复，不能重建。重建只应用于打开当前工作流的 Agent；不存在自动恢复、解析绕过、全局强制覆盖，也不会在 merge 失败后 fallback 到 rebuild。

Rebuild 输出有意只包含托管内容：Claude `settings.json` 只包含托管 `env`；opencode strict JSON 只包含根 `model` 和 `provider.mtls-router`；Codex `config.toml` 只包含托管 model/provider 设置，`auth.json` 只包含 `auth_mode` 和 `OPENAI_API_KEY`。精确输出和资格契约见 [Agent 模型配置](AGENT_MODELS.md#破坏性重建恢复)。

写入前，manager 会再次确认文件仍与已批准的 revision 一致。`PREVIEW_STALE` 表示目标已变化；此时不会开始写入，必须重新预览。替换现有文件前会创建备份。桌面端只提交一个 Agent，同时保留该 Agent 完整文件集的事务安全：失败时会回滚本事务已经修改的文件，但保留诊断备份。如果无法证明回滚成功，后续 Agent 写入会安全失败。

备份保留在原配置旁边，可能包含旧 API key。它们属于敏感恢复产物，应当像原 Agent 文件一样保护、保留或删除。预览只显示计划的 sibling 备份 pattern；创建并逐字节验证备份后，成功结果才显示实际路径。两者都绝不会显示备份内容，失败操作可能只显示错误。存在事务 journal 或未解决恢复时绝不能手工还原；应联系维护者，因为修改目标会导致恢复无法证明其身份。恢复问题解决后，应停止拥有文件的 Agent，保留当前文件，验证原路径及父目录仍是预期当前用户所有且不是链接，再通过同目录私有临时文件加原子替换恢复。对于 Codex，只恢复事务前存在的文件；只有已审查操作能证明伴随文件之前不存在时，才能删除该事务新建的文件，否则应联系维护者。

检测中的 `configured` 仅表示本地托管字段结构完整且内部一致，不证明所选模型当前已获授权。进入面板、手动刷新和节流后的原生 focus 刷新都会使用已保存的全局 key 获取新目录；系统不会轮询或在后台重写 Agent 文件。目录/认证/校验失败、模型消失、未批准漂移或所有权状态无效都会安全失败，不使用静态/cache fallback，也不会部分写入。服务契约、规范 schema、省略、迁移与所有权规则见 [Agent 模型配置](AGENT_MODELS.md)。

对于 Claude，规范配置会分别存储认证 base model ID 和可选精确字段 `context: "1m"`。启用 Fable 会渲染 `ANTHROPIC_DEFAULT_FABLE_MODEL` 及可选的 `ANTHROPIC_DEFAULT_FABLE_MODEL_NAME`；省略 Fable 时不会渲染或认领这两个 key。启用 Fable 要认领已有未托管值时，预览会显示 collision；禁用时只删除能证明之前由 manager 所有的 stale 路径，并保留从未取得所有权的手工 key。Manager 只在渲染 Claude 模型环境变量时追加 `[1m]`；它不会推断 1M 支持，也不会写入 `CLAUDE_CODE_DISABLE_1M_CONTEXT`。运行时拒绝不会触发 fallback 或重写。Fable alias 要求 Claude Code 2.1.170 或更高版本。与此独立，数值 custom-model context override 从 Claude Code 2.1.193 起可直接作用于未知模型名称；更早版本可能忽略它。Preset discovery 本身不会写 Agent 文件或 manager 事务状态，preset 数据在桌面 flow 中只作为不含 key 的模型状态保存。

## 图片对话

图片区为两个不可变预置提供多个本地对话：`cx/gpt-5.5-image` 和 `ag/gemini-3.1-flash-image`。可用性采用 fail-closed 规则，只接受认证后的 `GET /v1/models/image` 返回的精确 ID；该契约固定核验于 9Router `v0.5.45` commit `6fcd27337a7893642c7fe630840d0a641743f28f`。ID 缺失、重命名、别名或近似匹配都保持不可用；应用保留当前选择，绝不自动替换模型。该来源版本中的两个 upstream adapter 都带有 deprecated/risk notice，依赖前应重新核验目录和 release 契约。

没有参考图的提示词会向 `POST /v1/images/generations` 发送一次生图请求。上传一张经过验证的本地图片，或对已有结果选择**继续编辑**，会显式向同一 JSON 端点增加一个 data URI `image` 字段。应用绝不会隐式引用上一张结果，不支持远程图片 URL 或多参考图，且只接受 `data[0].b64_json` 输出。全应用最多执行一个图片操作；用户可以取消，系统不会自动重试，退出时中断的操作会在下次启动时标记为 interrupted，绝不会重放。

提示词 UTF-8 上限为 20 KiB。每张导入或生成图片的解码后上限为 20 MiB，单边不超过 16,384 像素，总像素不超过 64 MP；generation JSON 响应上限为 32 MiB。Rust 只接受 magic bytes、尺寸和静态格式校验都通过的 PNG、JPEG 与 WebP。

对话 metadata 和按内容寻址的图片资产保存在当前用户应用数据目录的 `image-conversations/` 下。它们受当前用户文件权限保护，但不额外加密。删除对话时先提交保留内容的快照，再删除不再被其他对话引用的资产；中断的清理由后续启动时的孤儿恢复继续处理。损坏或未知版本的快照会原样保留并 fail closed，直到用户显式确认**重建图片数据**；该操作会删除全部图片对话和资产。卸载不会自动删除这些对话或资产。

Webview 只接收 conversation/message/asset/model ID、提示词、状态和只读 `image-asset` custom URI；不会获得 API key、base64 图片正文、绝对路径、任意文件访问或网络能力。Rust 读取已保存 key 前，会从 manager 获取完整私有信任状态，并在同一个 loopback HTTP/1.1 连接上校验 `/version`、PID/启动/可执行文件身份和 `/health`。图片就绪状态会刻意要求 `/health` 返回 `status: "ok"`；上游降级时，即使 router 的健康端点仍返回 HTTP 200，图片凭据通道也保持关闭。认证目录与 generation 请求继续使用该不可重拨连接。

## API key 边界和限制

API 密钥页面会在桌面凭据存储中保存、替换或删除一个全局 API key；只接受不含空格或控制字符的可打印 ASCII key。Webview 只能读取摘要，绝不能回读明文。Agent 总览既不读取也不验证该 key。只有点击卡片操作后，Rust 才会按需为 `agent.models` 加载它，并在 `agent.write` 前再次加载当前保存值。桌面 `ModelFlow` 只包含 Agent、目录和预览模式状态，不包含 API key。Timeout、malformed response、manager restart 或 uncertain delivery 后，secret-bearing 调用绝不会自动 replay。除凭据存储以及 Agent 文件或已批准备份外，应用不会有意把 key 放入桌面或 manager 持久状态、进程参数、环境变量、日志、诊断、model config、catalog/revision token、预览响应或写入响应。

在清理该 Agent 前，目标 Agent 的配置文件仍需按该 Agent 的要求持久化 key。单 Agent 清理会删除 Agent 文件中的托管凭据，但有意保留桌面全局 key。用户批准的恢复与清理备份也可能持久化旧 key。Rust 每次按需使用 key 时都会把它保存在 zeroizing 内存中，但清除应用引用只是 best effort，不保证能从进程或操作系统内存中进行取证级擦除。

`MTLS_ROUTER_OPENAI_API_KEY` 已移除，不再提供 key。精确的非交互 manager 自动化见 [stdin manager 自动化](#stdin-manager-自动化)。

## stdin manager 自动化

自动化必须调用经安装 receipt 验证的 `mtls-router-manager serve`，要求 `manager.info` protocol `4`，使用临时 key 调用 `agent.models`，构造规范 model config，调用无 key 的 `agent.render` 或 `agent.preview`，最后使用 revision token、显式 approval 和临时 key 调用 `agent.write`。Key 不得出现在参数、导出的环境变量、model-config value、日志或临时请求文件中。Catalog token 可以有意跨 one-shot manager 进程验证。精确流程见 [protocol v4 自动化契约](AGENT_MODELS.md#protocol-v4-自动化)。

## 凭据模型

每个生产包绑定一个服务环境。router sidecar 包含共享客户端证书、共享私钥、上游 CA 和默认上游 URL。获得包的用户可以提取这些内嵌值；打包方式和桌面 UI 无法阻止提取。桌面包只能分发给可信内部用户。

吊销或轮换需要：使用新凭据材料构建替代 release，分发完整替代包，并在服务端拒绝旧凭据。应用不支持运行时证书导入、profile 切换或独立 sidecar 更新。Stable 桌面整包更新可以在用户确认后分发替代应用及其匹配 sidecar；CLI 安装仍使用现有 CLI 分发和 setup 流程。

本地 listener 是可信 localhost 上的 plain HTTP。不要把管理端点或 listener 暴露到公网。

## 卸载

- Windows：先退出应用，再从 Windows 设置中卸载。生产安装器必须在卸载时移除桌面应用的当前用户登录启动注册。
- macOS 和 Linux：打开设置，选择**准备卸载**并确认。等待应用移除当前用户登录启动项并退出，之后才能删除 macOS 应用或 Linux AppImage。

任何平台的卸载流程都不会恢复、删除或重写 Agent 配置、敏感备份、router 日志、应用状态或诊断状态。只有在确认恢复和诊断保留需求后，才应单独删除这些内容。
