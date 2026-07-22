# 桌面应用故障排查

[English](../TROUBLESHOOTING.md)

删除状态前，先使用 Router、日志和设置页面。诊断摘要和界面日志会经过安全过滤，但原始本地文件仍应按内部诊断数据处理。未经 API key 检查，不要附上 Agent 配置或备份内容。

## 包被阻止或 release 状态不明确

Workflow 会构建六个原生桌面包，并在匹配的目标 runner 上检查每个包，但签名是有条件的，而且包检查不会安装或启动应用。

1. 确认包与操作系统和架构匹配：Windows x86_64/arm64 NSIS、macOS Intel/Apple Silicon DMG，或 Linux x86_64/arm64 AppImage。
2. 使用配套 `.sha256` 文件验证包。
3. 阅读匹配的 `signing-status-<os>-<arch>.txt`。签名凭据不可用时，Windows 和 macOS 包可能未签名；notarization 凭据不可用时，macOS 包可能已签名但未 notarize。Linux 状态会明确报告未配置包签名。
4. 向分发方索取匹配目标 runner 上成功安装/启动的独立证据。成功的包检查 job 或状态文件不属于启动证据。

除非已经审查包 checksum、记录状态、分发策略和所需目标平台启动证据，否则不要绕过操作系统警告。详见[桌面应用](DESKTOP.md#安装)和[构建与发布](BUILD.md#包验证)。

## Sidecar 校验失败

常见现象包括 `SIDECAR_MISSING`、`SIDECAR_INVALID`、架构错误或提示重新安装。

1. 退出应用。
2. 确认桌面包与操作系统和 CPU 架构匹配。
3. 从可信 release 来源重新安装完整包。
4. 不要单独下载 manager/router、把 CLI 二进制复制到应用内、把修改执行权限当作绕过方式，也不要关闭完整性检查。

桌面应用不会独立修复或下载 sidecar。如果重新安装已验证包后仍失败，请保留错误和包标识并交给维护者。

## 端口 19099 被占用

桌面应用固定使用 `127.0.0.1:19099`，绝不会选择其他端口或自动终止占用者。常规停止、退出、登录时启动和托盘操作都不会终止未知占用者。

1. 使用可信安装包中的 `./setup.sh router status` 或 `.\setup.ps1 router status` 检查是否有 CLI 管理的 router。
2. 如果桌面应用报告兼容外部 router，复用属于预期行为；桌面应用不会拥有或停止它。
3. 所有平台默认只有在完整身份和当前用户所有权通过验证后，Router 页面才会提供**强制终止占用进程**。请先核对进程名称和 PID，再在确认对话框中核对完整可执行文件路径。macOS 和 Linux 没有此规则的例外。
4. 仅在 Windows 上，如果 TCP4 owner table 为精确的 `127.0.0.1:19099` listener 找到唯一 PID，但无法获得完整身份，页面可能改为显示**未验证**的仅 PID 目标。此视图只显示 PID，不能证明身份、所有者、启动时间或可执行文件；即使可读取的 SID 属于其他用户，该进程也可能进入此流程。除非接受这种降级保证，否则请选择取消。
5. 强制终止会立即执行，不会先尝试优雅停止，并且可能导致未保存数据丢失。两种模式都要求使用短时、一次性 token 进行显式确认，且都不会请求 administrator 或 root 提权。
6. 对于完整身份模式，manager 会在发送信号前一刻重新验证完整 listener 和进程身份。对于 Windows 仅 PID 恢复，manager 会立即复查同一精确端口仍由同一唯一 PID 占用，并确认该 PID 不是桌面应用、manager，也不是可读取桌面或 CLI 生命周期状态中的托管 router。如果 listener 已消失、PID 已变化，或变为重复、wildcard、格式错误或其他不明确状态，manager 会拒绝且不发送信号。
7. Windows 仅 PID 恢复会跳过无法读取的生命周期状态，因此托管 router 可能未被识别为受保护进程。Windows 可能拒绝终止，最终 owner 检查与终止之间也仍可能发生 PID 重用。等待端口释放时，listener 消失表示成功；listener 变化或身份不明确会报告错误，且不会向替代进程发送信号。
8. 成功后端口会释放，但 router 保持停止。准备好后请手工选择启动 router；登录时启动不会在此恢复流程中自动启动它。
9. 如果无法检查、你不接受仅 PID 警告或终止失败，请使用操作系统工具识别 listener，并在独立确认所有权和身份后停止或重新配置它，然后在 Router 页面重试。

手工启动的 router 会被有意视为未知，除非完整 CLI setup 状态能够证明其进程身份以及 deployment/protocol 兼容性。

## Router 状态陈旧

Stale 表示记录的 PID、进程启动标识或可执行文件标识不再匹配。manager 会保留状态用于诊断，且不会发送信号。

1. 独立检查报告的 PID 和可执行文件。
2. 如果真实 router 仍在运行，请使用拥有它的工具停止；只有确认身份后才能人工停止。
3. 原因不清楚时保留状态和日志；不要通过修改 PID 让状态看似有效。
4. 确认进程消失后重启桌面应用。如果 stale 持续出现，请联系维护者。

## 运行中但上游不可用

该状态表示本地 router 进程可用，但最新上游 mTLS 检查失败。超过 30 秒的健康结果会显示为 stale，而不是健康。

1. 选择重试健康检查。
2. 检查本地网络、DNS、proxy/VPN 策略、时钟和上游可用性。
3. 打开日志并复制安全过滤后的诊断摘要。
4. 不要手工替换内嵌证书。凭据或 deployment 轮换需要完整替代 release。

Router 进程状态和上游健康彼此独立。仅因短暂上游检查降级，不需要停止健康的本地进程。

## Router 退出或 manager 不可用

Router 意外退出后不会进入无限重启循环。manager 退出时，桌面应用最多尝试一次有界恢复；恢复失败会禁用生命周期命令，直到应用重启。

打开日志、保留诊断摘要，然后只重启一次桌面应用。如果错误指向 sidecar 校验问题，请重新安装。不要通过在其他端口启动另一个 router 来绕过。

如果新构建或新安装的 manager 在接受 protocol request 前以 `invalid embedded Agent model preset` 退出，则其非空构建期 `AGENT_MODEL_PRESET_BASE64` 无效。Manager 会有意隐藏原始编码和解码 preset 内容，并在 Agent transaction recovery 前失败。用户应重新安装修正后的完整 release；维护者应修正或清空 repository variable，再重新构建 standalone 和 desktop manager 产物。不要 patch 打包 sidecar，也不要把 preset 注入 router。

如果它以 `invalid embedded simplify value` 退出，则 manager 被直接 link 了无效或空的 `modelcatalog.Simplify` 值。用户应重新安装修正后的完整 release。维护者应通过仓库构建脚本使用未设置/空的 `SIMPLIFY`，或 `true`/`false` 的任意 ASCII 大小写形式，再以相同规范值重新构建 standalone 和 desktop manager 产物。脚本会在编译前拒绝其他所有值；不要 patch sidecar，也不要把 `SIMPLIFY` 加入 router 运行时配置。

## Agent 配置不可用或不可写

- Claude Code、opencode 和 Codex 始终作为受支持的配置目标可用；桌面应用不会安装或启动它们的 CLI。
- `command` 为空只表示 manager 进程无法找到 Agent CLI，不会阻止创建或更新配置。
- 启动桌面应用前，确认 `CLAUDE_CONFIG_DIR`、`OPENCODE_CONFIG` 或 `CODEX_HOME` 指向预期的当前用户位置。
- 恢复当前用户对配置文件及其目录的写权限。不要以 administrator 或 root 运行桌面应用来绕过所有权问题。

## Agent 配置无效

`CONFIG_INVALID` 表示无法安全解析现有 JSON、JSONC、TOML 或 Codex auth JSON。不会修改任何文件。

1. 打开检测结果显示的路径。
2. 修复语法；如果对应 Agent 可能并发写入，请先停止它。
3. 对没有显式 `OPENCODE_CONFIG` 的标准 `~/.config/opencode/opencode.jsonc`，检查同目录已有的 sibling `opencode.json` 是否与 JSONC 到 JSON 迁移冲突。
4. 对显式 `.jsonc` `OPENCODE_CONFIG`，检查精确覆盖路径及其父目录可写；sibling `opencode.json` 与此无关，也不会作为回退路径。
5. 刷新检测并生成新预览。

manager 会保留受支持的无关设置，但不会猜测如何修复无效语法。

## 预览已陈旧

`PREVIEW_STALE` 表示所选目标在预览后发生变化。写入会在修改前被拒绝。

客户端允许时，请返回配置，使用保留的无 key 目录/config 生成新预览。如果 catalog token 同样 stale，则重新输入 key 并发现模型。对于显式 `OPENCODE_CONFIG`，确认精确覆盖路径在预览后没有发生变化。不要用旧 revision token 重试。

## 模型配置错误

所有模型错误都会安全失败：不会修改 Agent 或 last-applied sidecar 文件，也不会使用静态模型、缓存目录、现有模型或替代模型 fallback。

构建 preset 不可用时会采用不同报告方式：`agent.models` 仍成功，省略受影响 Agent 的完整 preset section，并在 `preset.unavailable_agents` 下以 `MODEL_NOT_AVAILABLE` 列出缺失 base ID。Existing section 和其他 Agent 的有效 preset section 仍可使用。请明确选择模型或要求分发方更新 release；manager 不会部分使用、修复或替换不可用 section。Preset notice 只是可编辑推荐，不证明模型支持 Claude 1M context。

Manager 的构建过滤目录是可用性边界。默认构建策略会有意排除包含 ASCII `/` 的有效 ID；引用这些 ID 的 existing 和 preset section 会报告为不可用，选择这些 ID 的导入配置会以 `MODEL_CONFIG_INVALID` 失败。请使用当前 manager 显示的模型，或要求分发方提供使用 `SIMPLIFY=False` 构建的 release；该策略不能作为运行时偏好更改。全角斜杠和反斜杠不会触发过滤，proxy 路由支持保持不变。

| Code | 处理方式 |
|---|---|
| `MODEL_AUTH_FAILED` | 重新输入 API key；目录 endpoint 返回了 401 或 403。 |
| `MODEL_DISCOVERY_FAILED` | 检查可信本地 router、网络和上游服务后重试。Redirect 和非认证 HTTP 失败使用此 code。 |
| `MODEL_RESPONSE_INVALID` | 报告上游服务契约失败；成功响应 malformed、超限，或不是标准 `data[].id` JSON。完整响应会在过滤前校验，因此即使 malformed ID 原本会被过滤，仍返回此错误。 |
| `MODEL_CATALOG_EMPTY` | 响应中没有被当前 manager 保留的有效 ID。确认账户/key 可见性；如果全部可见 ID 都含 ASCII `/`，请使用 `SIMPLIFY=False` 构建的 release，或要求服务提供适合的 ID，然后重新发现。写入时刷新出现同一错误表示写入尚未开始。 |
| `MODEL_CATALOG_STALE` | 重新发现模型。Router 地址、deployment、protocol、owner 或 token trust state 已变化。 |
| `MODEL_CONFIG_INVALID` | 按报告的 JSON Pointer 修正规范 model config。导入内容选择过滤目录之外的 ID 时仍无效。不要在 `extra`/`options` 中放入凭据、URL、provider/header 字段或任意 Agent 配置。 |
| `MODEL_NOT_AVAILABLE` | 写入时刷新发现所选模型已从过滤目录消失。重新发现并明确选择；manager 不会自动替代。如果刷新后没有保留任何 ID，则改为 `MODEL_CATALOG_EMPTY`。 |
| `MANAGED_CONFIG_DRIFT` | 生成新预览，只检查列出的托管 namespace，然后明确批准覆盖或取消。 |
| `MODEL_STATE_INVALID` | 保留 Agent 备份，并先解决 transaction journal。仅在审查后整体移走无效 `agent-transactions` 目录；不要只替换 signing key 或 sidecar。 |
| `AGENT_OPERATION_BUSY` | 等待另一个桌面/CLI Agent 操作结束后重试。不要删除 lock 或 transaction state。 |
| `CODEX_AUTH_UNSUPPORTED` | 重试前解决 forced ChatGPT login、managed policy 或不兼容 credential-store policy。Manager 不会降低 policy，也不会删除 OS keyring 凭据。 |

检测中的 `configured=true` 不是授权结果，只表示本地托管结构完整。使用模型发现检查当前 key visibility。目录只能手工刷新：重新进入 Agent 配置并提供 key。详见 [Agent 模型配置](AGENT_MODELS.md)。

对于 Claude，规范 `context` 只接受精确 `"1m"`；`model` 中必须填写已认证 base ID，绝不能使用以 `[1m]` 结尾的 ID。Manager 只在渲染 Claude settings 时追加该 suffix，不会推断能力。如果 Claude 或上游在运行时拒绝 1M，请选择 Standard 或其他经过显式验证的选择并写入新预览；系统不会自动 fallback 或重写配置。

Fable 是可选的。启用时，其显式模型必须仍在认证目录中；Fable 模型不可用会使整个 Claude section 不可用，而不是只删除 Fable。如果已有手工 `ANTHROPIC_DEFAULT_FABLE_MODEL` 或 `ANTHROPIC_DEFAULT_FABLE_MODEL_NAME` 引发 collision，请检查预览，并且只在 manager 应替换该精确值时批准认领所有权。禁用 Fable 会保留从未取得所有权的手工 key，只删除先前 sidecar 能证明由 manager 所有的 stale 路径。Fable alias 需要 Claude Code 2.1.170 或更高版本。该要求与数值 context override 兼容性相互独立：未知 custom model 名称的直接数值 override 需要 Claude Code 2.1.193 或更高版本，更早版本可能忽略它。

## 写入或回滚失败

多 Agent 写入是事务性的。后续操作失败时，已经替换的目标会恢复，并保留诊断备份。`ROLLBACK_FAILED` 表示无法证明恢复完成，因此会禁用后续 Agent 写入。

调查期间不要删除备份或 manager 事务状态。记录结果中的修改、备份和 rollback-backup 路径，退出拥有这些文件的 Agent，然后联系维护者。备份可能含旧 API key，不能未经脱敏直接附上。

## 登录启动或卸载准备失败

设置变更只作用于当前用户，不应要求提权。如果**准备卸载**无法确认登录启动已禁用，应用会保持打开，此时不能继续删除。

macOS/Linux 上请重试**准备卸载**，确认应用退出后再删除。Windows 上应使用生产安装器的卸载器，由它负责删除当前用户注册。卸载不会删除 Agent 文件、备份、日志或状态。
