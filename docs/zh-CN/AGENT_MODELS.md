# Agent 模型配置

[English](../AGENT_MODELS.md)

本文是 management protocol v4 的用户与自动化契约。如果本文与客户端本地校验不一致，以 Go manager 为准。

## 服务契约

经过认证的上游 `GET /v1/models` 响应是候选模型的唯一来源。Manager 会先校验完整且有界的 OpenAI 兼容响应及每个 `data[].id`，再去重、执行唯一 ID 数量限制并按 UTF-8 byte 排序，最后应用不可变构建过滤。默认构建策略 `SIMPLIFY=True` 会排除包含 ASCII `/` 的有效 ID；使用 `SIMPLIFY=False` 构建的 manager 会保留全部有效 ID。全角斜杠 `／`、反斜杠 `\` 和其他非 ASCII 相似字符都不是 ASCII `/`，不受影响。最终过滤后的目录是 protocol result/token、existing 与 preset 可用性、导入 model config、render/preview 校验和写入时刷新的权威依据。该过滤不会改变 proxy 支持的路由：

| 客户端 | 路由 | 契约 |
|---|---|---|
| 模型目录 | `GET /v1/models` | 一个经过完整校验且有界的标准 JSON 响应 |
| Claude Code | `POST /v1/messages`，包括 `?beta=true` | Anthropic 请求字段和开放列表 `anthropic-*` header 原样透传；SSE 不缓冲 |
| Claude Code | `POST /v1/messages/count_tokens` | 精确 token 计数 |
| opencode | `POST /v1/chat/completions` | OpenAI Chat Completions 与流式响应 |
| 兼容客户端 | `POST /v1/completions` | OpenAI Completions 与可选流式响应 |
| Codex | `POST /v1/responses` | OpenAI Responses 与 SSE 流式响应 |

Claude deferred tool 字段以及未来开放列表 Anthropic 请求字段会原样透传。配置过程绝不调用 inference endpoint，也绝不会根据 ID、名称、目录位置或其他能力信号推断 1M context 支持。Router 会保留 authorization，但不选择模型，也不存储 key。Manager 目录中保留的每个 ID 都按支持上述全部路由处理；构建过滤是 manager 目录策略，不是 proxy 能力限制或运行时模型偏好。

## 交互流程

Shell、PowerShell 和桌面客户端都保持以下 protocol 顺序：

1. 检测并选择 Claude Code、opencode 和/或 Codex。
2. 隐藏读取 API key，并建立可信的 protocol-v4 loopback router。
3. 调用 `agent.models`，然后才显示共同的已排序目录。
4. 每个已选 Agent 优先使用有效 existing section，其次使用可见且已认证的 preset section，否则使用 empty section；用户必须检查或补全可编辑的 Agent 原生选择。绝不选择第一个模型，不使用模型名称或能力 heuristic，不修复不可用 preset，也不替换为其他模型。
5. Print 时渲染脱敏片段；write 时预览精确文件、备份、迁移、所有权和漂移影响。
6. 写入前用临时 key 重新获取目录，再执行一次原子多文件事务。

Shell 和 PowerShell 安装命令始终省略恢复 mode，因此只支持 merge。桌面端一次只打开一个持续单 Agent 面板：有效 Agent 使用 `merge`，符合条件的无效 Agent 使用 `rebuild`。每次桌面 preview 和事务都精确只包含该 Agent；重建还必须遵守下文契约。Management protocol 仍为 v4，并新增 `agent.cleanup.preview` 和 `agent.cleanup.write` 用于下文独立清理契约；这两个 method 不会暴露为 setup 命令。作为全协议加固，严格请求参数解码现在除了拒绝未知字段，还会在每一层 object 中递归拒绝重复 JSON key。

### 桌面持续面板

每次进入面板都会执行 `agent.detect`、读取桌面凭据摘要，并且只有存在 key 且目标可编辑或符合恢复条件时才调用已认证的 `agent.models`。桌面端以 `existing > preset > empty` 初始化，在同一页面保留编辑器和 preview rail，每次写入都必须先为当前草稿生成 preview，事务结束后仍留在面板中。写入成功会消费当前 flow、立即报告成功，然后重新执行 detection、凭据摘要和 discovery，用新 flow 加载实际落盘配置。该重载失败属于独立面板状态，不会把已完成写入改成失败。

初次或完整 reload discovery 缺少已保存 key、认证失败或目录失败时，面板只展示无 key 的 `agent.detect` metadata 和恢复操作。由于字段级 prefill、import、export、render 和 preview 都需要已认证 `agent.models` 签发的 catalog token，因此不能安全提供这些能力。后台候选刷新失败时则保留已有草稿和 active flow，提示无法验证外部状态，并继续允许仍然安全的操作。API key 始终留在 Rust 和含密钥的 manager 请求中，不会返回 webview，也不会进入 model config。

可编辑面板监听原生窗口 focus signal，每 15 秒最多启动一次候选 discovery；手动刷新绕过间隔，但不绕过 single-flight。Clean 面板采用候选状态。Dirty 面板保留 form baseline 和草稿：外部状态未变时只替换 active discovery，发生变化时必须显式选择保留草稿或加载磁盘状态。Detection 变为不兼容或不可写时会阻止编辑、import、preview 和 write；只有仍存在有效 active flow 时才能 export。刷新期间不存在轮询、自动合并、旧目录 fallback 或 Agent 文件重写。

前端最多拥有一个 active flow 和一个未决候选请求。兼容候选成功后先成为 active，再销毁旧 flow；clean mode transition 会先销毁不兼容的旧 flow，再启动全新 discovery。销毁请求失败时会去重保留并重试。过期、迟到、面板已卸载或目标不匹配的候选 flow 都会销毁；离开或卸载面板时也会销毁其 active flow。写入成功会消费 active flow，不再重复销毁。`PREVIEW_STALE`、`MODEL_FLOW_EXPIRED` 和 `MODEL_CATALOG_STALE` 会清除 preview approval、保留内存草稿，并在面板内重新 discovery；没有有效 flow 时 export 保持禁用。

只有在完整响应和全部 ID 通过校验后才会过滤。因此，包含 ASCII `/` 的 malformed ID 不会被隐藏，请求仍以 `MODEL_RESPONSE_INVALID` 失败。如果校验成功但全部 ID 都被过滤，发现或刷新以 `MODEL_CATALOG_EMPTY` 失败。

目录 token 会绑定 manager 的不可变构建策略；策略变化会使已有 token 失效，render、preview 或 write 前必须重新发现模型。

`agent print-config` 同样需要 key，因为它必须根据当前目录验证选择。它不会修改 Agent 文件、事务 journal、备份或 last-applied sidecar。模型发现可能启动 router，首次使用可能创建私有 token signing key。

模型目录只是配置时快照。Shell 和 PowerShell 客户端需要重新进入配置并提供 key 才能刷新。桌面端可以通过上述显式刷新或节流后的原生 focus discovery 替换该快照；这不会轮询或重写 Agent 文件。

### 构建 preset

Release 可以向 manager 注入一份不可变、无 key 的规范 preset。Manager 启动时会严格解码并执行结构校验；调用 `agent.models` 时，会把 preset 裁剪到请求的 Agent，并根据当前认证目录逐个独立校验 Agent section。只有全部引用的 base model ID 均可用时，才会完整返回该 section。任一 ID 缺失都会省略整个 section，并以非致命 `MODEL_NOT_AVAILABLE` metadata 报告已排序的缺失 base ID；其他 Agent 的有效 section 仍可使用。Manager 绝不会部分修复、deep-merge 或替换 preset section。

客户端对每个已选 Agent 独立采用 `existing > preset > empty` 初始化。Preset 值是可见、可编辑的默认值，不代表预览批准、写入确认或能力证明。交互编辑覆盖这些默认值。`--model-config=<path>` 和桌面导入会完整替换表单，并覆盖 existing/preset 初始化，而不是与其合并。被构建过滤排除的 existing 与 preset 选择会根据过滤后目录报告为不可用。导入内容如果选择该目录之外的 ID，在 render/preview 校验时仍以 `MODEL_CONFIG_INVALID` 失败；过滤不会静默删除或替换该选择。

## 规范模型配置

所有客户端使用同一份无 key JSON 文档。`version` 为 `1`；请求中的 `agents` 数组必须与实际存在的顶层 section 精确一致。除受限 `extra`、`options` 和 variant option object 外，未知字段都会被拒绝。输入必须是严格 JSON：重复 key、无效 UTF-8、非有限数字、不安全整数范围和受保护的凭据/连接路径都会被拒绝。规范字节使用 RFC 8785 JCS，不执行 Unicode normalization。

包含三个 Agent 的最小结构：

```json
{
  "version": 1,
  "claude": {
    "primary": {"model": "model-a"},
    "haiku": {"inherit_primary": true},
    "sonnet": {"inherit_primary": true},
    "opus": {"inherit_primary": true}
  },
  "opencode": {
    "default_model": "model-a",
    "models": {"model-a": {}}
  },
  "codex": {"model": "model-a"}
}
```

可在 `agent print-config` 或 `agent write-config` 中使用 `--model-config=<path>`，以该文档替代模型设置问答。文件必须是不超过 2 MiB 的普通非链接 JSON 文件，不得包含 API key、URL、provider identity、header、已获取目录或任意 Agent 配置。

### Claude Code

`primary` 必填。`haiku`、`sonnet` 和 `opus` 各自继承 primary selection，或明确选择另一个目录模型。可选 `fable` 存在时使用相同的继承或显式选择 union；省略表示 Fable 已禁用且不受管理，decoder 和客户端不会隐式合成该字段。每个显式选择都可包含可选显示 `name` 和可选 `context`。省略 `context` 表示 Claude 的 standard/default 行为；唯一允许的值是精确字符串 `"1m"`。规范 `model` 始终是已认证 base ID，不得以 `[1m]` 结尾。继承角色只能包含 `{"inherit_primary":true}`，并同时继承 model、name 和 context。

例如，启用的 Fable 可以是：

```json
{"inherit_primary": true}
```

也可以是显式选择：

```json
{"model": "model-a", "name": "可选显示名称", "context": "1m"}
```

Fable 没有 description key。其显式模型必须存在于认证目录；显式 Fable `context: "1m"` 与数值 `context_window` 冲突，规则与其他 Claude 显式选择相同。

`extra` 是字符串 map，仅允许以下 description key：

- `ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION`
- `ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION`
- `ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION`
- `ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION`

Claude section 还可包含彼此独立可选的 `context_window` 和 `max_output_tokens` 字段，两者都必须是正的安全整数。两者同时存在时，`max_output_tokens` 必须小于 `context_window`。`context_window` 与 primary 或任一 role 的显式 selection 上的 `context: "1m"` 冲突；数值全局预算与 selection-level `[1m]` 机制只能二选一。Manager 分别将配置值以精确十进制字符串渲染到 `CLAUDE_CODE_MAX_CONTEXT_TOKENS` 和 `CLAUDE_CODE_MAX_OUTPUT_TOKENS`。

Manager 只拥有文档声明的 `env` key，将它们合并到现有 `env`，并保留无关顶层和环境值。所有权包括两个数值预算环境 key：省略之前由 manager 管理的字段时，会移除其旧值。只有当已有文件中的这些值是规范的正十进制字符串，且生成的 Claude section 满足相同预算与 context 规则时，才会将其投影回规范配置。

Standard context 下，manager 原样渲染 base ID。使用 `context: "1m"` 时，仅在 Claude 文件渲染边界为 `ANTHROPIC_MODEL`、`ANTHROPIC_CUSTOM_MODEL_OPTION` 以及有效 Haiku、Sonnet、Opus 和已启用 Fable 模型值追加一个精确的末尾 `[1m]`。显示名称保持不变，manager 不写入 `CLAUDE_CODE_DISABLE_1M_CONTEXT`。已有值带一个精确末尾 `[1m]` 时，会投影回 base ID 加规范 context；错误、重复或位于中间的 marker 不会被修复。目录与写入时可用性校验始终使用 base ID。配置阶段不会推断模型是否支持 1M context；运行时拒绝不会触发模型 fallback 或配置重写。

启用 Fable 时，manager 始终渲染 `ANTHROPIC_DEFAULT_FABLE_MODEL`；只有继承或显式选择的有效结果包含名称时，才渲染 `ANTHROPIC_DEFAULT_FABLE_MODEL_NAME`。Fable model key 同时是投影启用信号：只有 name key 不会启用或投影 Fable。已有的已启用 Fable 只有在 model、name 和 context 都与 primary 精确相同时才投影为继承，否则保持显式选择。已启用 Fable 数据 malformed 或与数值 context 冲突时，整个 existing Claude section 都不可用。`configured=true` 仍不要求 Fable，因此 legacy Claude 文件不会变为不完整。

Claude 是可用性与初始化的原子单位。如果 preset 或已投影 existing 配置中启用的 Fable 模型不可用或无效，整个 Claude section 都不可用；绝不会单独删除、修复、替换或合并 Fable。OpenCode 与 Codex section 仍可独立使用。同样，`existing > preset > empty` 只会选择一个完整 Claude section：如果 existing Claude section 省略 Fable，绝不会把 preset Fable deep-merge 进去。

Fable 所有权是有条件且基于精确路径的。启用时，manager 拥有已渲染的 Fable model 路径，以及实际渲染时的 name 路径。省略时不认领这两个路径，并保留从未由 manager 所有的手工 Fable 值。如果先前 sidecar 能证明某个 Fable 路径由 manager 所有，禁用 Fable 会在同一个可恢复事务中删除该 stale 路径，同时保留无关环境值。在已有未托管值上启用 Fable 会产生 collision/drift，必须经过绑定预览的批准，绝不会静默覆盖。Sidecar 存储完整规范 Claude section，并且只在 Fable 路径实际受管理时记录当前路径。

启用的 Fable alias 要求 Claude Code 2.1.170 或更高版本。更早版本可能忽略 `ANTHROPIC_DEFAULT_FABLE_MODEL`；应禁用 Fable 或升级 Claude Code，而不能期待 manager fallback。该 alias 要求与数值 context 兼容性相互独立：从 Claude Code 2.1.193 开始，数值 `context_window` override 可直接作用于未知 custom model 名称。更早版本仍受支持，但可能忽略该数值 override；Fable 禁用时系统不设置 Claude Code 硬性最低版本。这些数值只控制 Claude Code 本地 token budgeting 与 compaction 行为，不会扩大、证明或以其他方式改变上游模型的实际 context 或 output capability。

### opencode

`models` 包含一个或多个明确选择的目录 ID，`default_model` 必须是其中之一。每个模型的 typed option 包括显示名称、reasoning、attachment、tool call、temperature、context/input/output limit、输入/输出 modality、interleaved reasoning 和受限 provider `options` object。模型还可包含 typed top-level `variants`。Variant 名称可扩展，但必须非空、不含控制字符且不超过 128 个 UTF-8 byte；每个名称映射到一个递归有界的 provider option object，并接受相同的受保护凭据与连接路径检查。系统继续接受 legacy `extra.variants` 输入，但同时在两个位置定义 `variants` 属于 field conflict，会被拒绝。除此之外，`extra` 只接受 pinned opencode schema 允许的字段，且不能与 typed 或 manager-owned 路径冲突。

只有显示 `name` 默认等于模型 ID。其他未设置可选字段会省略，而不是猜测。Manager 拥有 `provider.mtls-router` 和已取得所有权的根 `model`，同时保留其他 provider、`small_model` 和无关根设置。Typed variants 精确渲染到 `provider.mtls-router.models.<id>.variants`，并在 discovery 时从该 managed top-level model field 投影；任意 provider extra 不会被提取到规范配置。JSONC normalization 可能移除注释和格式，预览会明确显示。

### Codex

`model` 选择一个目录 ID。可选 typed field 为 `reasoning_effort`、`reasoning_summary`、`verbosity`、`context_window` 和 `auto_compact_token_limit`。v1 唯一允许的 `extra` key 是 `model_auto_compact_token_limit_scope`，值为 `total` 或 `body_after_prefix`。未设置可选 key 会省略；如果之前由 manager 所有，则会移除。

专用 provider 为 `model_providers.mtls-router`，使用 `wire_api = "responses"` 和可信 loopback `/v1` URL。Codex CLI 与 IDE 共享认证。切换到 `cli_auth_credentials_store = "file"`，并采用官方 `auth_mode = "apikey"` 加 `OPENAI_API_KEY` 文件认证前，预览会单独要求批准。不会删除 OS keyring 凭据。

## 检测、失败与刷新

`agent.detect` 不需要 key。`configured=true` 仅表示本地托管结构完整且内部一致，不表示模型当前可见或已获授权。只有成功的 `agent.models` 发现与写入时刷新才能证明当前授权；系统不持久化 verified timestamp。

发现和写入始终 fail closed。`MODEL_AUTH_FAILED`、`MODEL_DISCOVERY_FAILED`、`MODEL_RESPONSE_INVALID`、`MODEL_CATALOG_EMPTY`、`MODEL_CATALOG_STALE`、`MODEL_CONFIG_INVALID`、`MODEL_NOT_AVAILABLE`、`MANAGED_CONFIG_DRIFT`、`MODEL_STATE_INVALID`、`AGENT_OPERATION_BUSY` 和 `CODEX_AUTH_UNSUPPORTED` 都不会触发静态模型、旧目录 cache、隐式复用现有模型、替换模型或部分文件变更。处理方式见[故障排查](TROUBLESHOOTING.md#模型配置错误)。

如果所选 ID 在写入时刷新后从过滤目录消失，写入以 `MODEL_NOT_AVAILABLE` 失败；如果刷新后没有任何保留 ID，则更早的目录获取直接以 `MODEL_CATALOG_EMPTY` 失败。

Preset 模型不可用是唯一不导致 discovery 整体失败的情况：它会在 `preset.unavailable_agents` 中报告，省略不可用的完整 preset section，并继续提供 existing 配置和其他有效 preset section。它仍然绝不会触发替换或部分使用 preset。

## 托管配置清理

当 `agent.detect` 报告可信 last-applied sidecar 条目，且 `cleanup.managed=true`、`cleanup.available=true` 时，桌面端可以精确清理一个 Claude Code、OpenCode 或 Codex 配置。清理不需要 key，也不依赖 router trust、模型发现、catalog token、规范 model config 或桌面 model flow。总览不会为普通 `not_managed` Agent 显示该操作。Sidecar/signing 状态无效时报告 `model_state_invalid`；恢复未解决或写入已禁用时报告 `writes_disabled`。这两种情况都不会破坏基础 Agent 文件检测。

清理绝不会只根据 provider 名称推断所有权。它会校验 sidecar、加载已保存的 Agent model section，只读取其中记录的绝对文件路径，并根据已保存配置和当前结构收窄旧版本中过宽的所有权记录：

- **Claude Code：**只删除已记录的 `env.*` 路径；根 `env` object 为空时移除它。
- **opencode：**在验证 object 结构后删除 `provider.mtls-router`。只有 sidecar 拥有根 `model` 且当前字符串以精确 ASCII `mtls-router/` 开头时才删除该字段。正常写入后来保留的用户默认 model 不会被新认领。
- **Codex：**从 `config.toml` 删除 `model_providers.mtls-router`、必需的 `model_provider`、`model`、`cli_auth_credentials_store`，以及仅由已保存 Codex model config 表明曾生成的可选根字段；从 `auth.json` 删除 `auth_mode` 和 `OPENAI_API_KEY`。其他 auth metadata 会保留；OS keyring 凭据不会删除，也不会重建先前写入时被替换的 competing auth 字段。Config/auth 文件对始终属于同一个事务。

Preview 只返回路径名称和文件/state 影响，绝不返回当前值或内容。它不会创建任何持久文件系统状态：不创建文件、目录、备份、事务 journal 或协调 lock。对于已托管状态，它只打开并校验已经存在的私有事务 lock；lock 缺失或不安全时会 fail closed，绝不补建。Cleanup 专用 HMAC token 会绑定 Agent、每个源/目标路径及 keyed revision、`replace`/`delete` 操作、必需备份来源、已排序删除路径、整文件漂移标志，以及 sidecar revision/operation；它有意不包含 router、目录、API key 或规范 model claims。

整文件漂移要求 `approve_managed_overwrite=true`；批准不会扩大托管路径集合。Preview 后任一 Agent 文件或 sidecar revision 变化都会返回 `PREVIEW_STALE`。Write 会先为每个现有 Agent 文件和 sidecar 创建并验证私有 sibling 备份，再记录支持删除的 journal v3，按顺序处理 Agent 文件，最后更新或删除 sidecar。语义为空的 JSON/TOML 根会执行 `delete`，否则执行 `replace`。Journal v3 用不存在的 post revision 表示删除；启动恢复仍把 legacy v1/v2 journal 解码为仅 replace。Rollback 先恢复 manager state，再恢复 Agent 文件，避免所有权与文件分裂。

清理会删除所选 Agent 文件内的认证，但保留桌面全局凭据以及所有历史和新生成备份。备份可能含当前或旧 key。稳定清理错误包括 `INVALID_PARAMS`、`AGENT_NOT_MANAGED`、`MODEL_STATE_INVALID`、`CONFIG_INVALID`、`CONFIG_NOT_WRITABLE`、`PREVIEW_STALE` 和 `MANAGED_CONFIG_DRIFT`，以及已有的 `AGENT_OPERATION_BUSY`、`BACKUP_FAILED`、`WRITE_FAILED`、`ROLLBACK_FAILED`、`OPERATION_TIMEOUT` 事务/protocol 错误。`INVALID_PARAMS` 涵盖不支持的 Agent、malformed 或重复/未知请求字段，以及缺失或 malformed revision token 或必需 approval 字段。所有错误都不会返回配置值、key 材料、文件内容、URL 或备份内容。

## 破坏性重建恢复

正常 `merge` 会解析现有配置，只修改 manager-owned 路径，并保留受支持的无关数据。`rebuild` 不解析或合并 malformed 内容：经过单独且绑定预览的批准后，它会用全新渲染的纯托管文件替换完整的已批准 Agent 文件集。**重建会丢弃全部无关设置、注释、原始格式以及有效伴随文件中的元数据。继续前必须检查并保护每个备份。**

只有检测发现至少一个文件语法无效，且完整托管集合中的每个文件除此之外都安全时，才能重建：现有目标必须可读、是普通非链接文件且可写，其直接父目录必须存在、可用且可写。文件不可读、超限、非普通文件、是链接或不可写，父目录不可用，结构语法有效但不受支持，存在待恢复事务，或全局禁用写入，都会阻止重建。检测结果仅供提示；preview 和 write 会在事务 lock 下重复校验资格以及精确路径、格式、存在状态和 revision。

语法有效绝不是重建理由。语法有效但结构不受支持时，必须明确修复或迁移；parser 兼容性问题必须在 parser 中修复。例如，已接受的 BOM 前缀 JSON 和带引号 TOML key 应继续走保留 merge，不能作为恢复输入丢弃。

完整纯托管输出如下：

- **Claude Code：**一个根 object 只包含 `env` 的 `settings.json`。无条件 key 为 `ANTHROPIC_BASE_URL`、`ANTHROPIC_AUTH_TOKEN`、`ANTHROPIC_MODEL`、`ANTHROPIC_CUSTOM_MODEL_OPTION`、Haiku/Sonnet/Opus 的 `ANTHROPIC_DEFAULT_*_MODEL` key、`ENABLE_TOOL_SEARCH` 和 `DISABLE_AUTOUPDATER`。只有实际选择时才会添加显示名称 key（`ANTHROPIC_CUSTOM_MODEL_OPTION_NAME` 和 `ANTHROPIC_DEFAULT_*_MODEL_NAME` key）、Fable model/name key、`CLAUDE_CODE_MAX_CONTEXT_TOKENS`、`CLAUDE_CODE_MAX_OUTPUT_TOKENS` 和允许的 description extra。
- **opencode：**一个根 object 只包含 `model` 和 `provider.mtls-router` 的 strict JSON。Provider 精确只包含 `npm`、`name`、`options` 和 `models`；`npm` 为 `"@ai-sdk/openai-compatible"`，`name` 为 `"mtls-router"`，`options` 只包含 `baseURL` 和 `apiKey`，`models` 包含精确选择的 definition。已批准 `.jsonc` 路径会原地替换为 strict JSON；重建绝不会执行正常 sibling `opencode.json` 迁移。
- **Codex：**完整集合始终同时包含 `config.toml` 和 `auth.json`，即使只有一个文件 malformed。`config.toml` 只包含 `model_provider = "mtls-router"`、所选 `model`、`cli_auth_credentials_store = "file"`、已选择的可选模型设置，以及精确包含 `name`、`wire_api = "responses"`、`requires_openai_auth = true` 和 `base_url` 的 `model_providers.mtls-router`。`auth.json` 精确只包含 `auth_mode: "apikey"` 与 `OPENAI_API_KEY`。缺失伴随文件会创建；每个现有伴随文件都会替换，因此有效伴随 metadata 会丢失。

Preview 不创建文件、目录、journal 或备份。它显示脱敏托管片段、每个受影响的精确路径、创建或替换操作，以及计划的 sibling 备份 pattern。Write 要求已签名 preview revision 和精确的 `approve_rebuild` 集合；省略 mode 默认使用 merge，缺失、额外或重复的重建批准都会被拒绝。不存在自动恢复、解析绕过、`force`、全局覆盖，也不会在 merge 失败后 fallback 到 rebuild。

替换前，write 会为批准托管集合中的每个现有文件创建私有 sibling 备份，执行 sync、重新打开并验证逐字节相等。实际备份路径只在成功写入结果中显示；preview 只显示 pattern，绝不显示备份内容。失败操作即使留下诊断产物，也可能只报告错误。备份可能包含当前或旧 API key 及其他凭据，必须保持私有，不能未经脱敏直接附上。存在事务 journal 或未解决恢复时绝不能手工还原；应停止操作并联系维护者，因为修改目标会导致恢复无法证明其身份。恢复问题解决后，应停止拥有文件的 Agent，保留当前文件，验证每个原路径及父目录仍是预期当前用户所有且不是链接，并通过同目录私有临时文件加原子替换恢复，不能经链接直接复制。对于 Codex，应恢复事务前存在的每个文件；只有 preview/result 能证明伴随文件由该事务新建时才能删除它，否则应停止操作并联系维护者。

替换前发生文件变化、语法已修复、新增 blocker、目录变化或 revision stale，都会拒绝写入，并要求重新检测和预览。备份失败不会修改任何目标。之后的写入失败会一起回滚所有已修改 Agent 文件和 manager sidecar，并保留诊断备份。如果 rollback 或启动恢复无法证明已还原，系统会禁用写入，且在恢复问题解决前不能重建；调查期间不要删除 journal 或备份。

## 所有权、迁移与备份

Manager 只在私有 `agent-transactions/last-applied-model-config.json` sidecar 中记录规范已选模型 section、owned path、target path 和 keyed revision MAC。它不存储 key、目录、渲染文件、原始响应或无关 Agent 设置。OS-backed 当前用户 lock 会串行化桌面端与 CLI 操作。

注入的 preset 属于构建 metadata；仅执行 discovery 绝不会把它写入 Agent 文件、last-applied sidecar、journal、revision claim、备份、日志或诊断。只有用户通过正常 preview/write 流程批准的精确规范配置，才能进入 Agent 文件和事务状态。

已知 manager-owned 路径可以更新或删除。正常 merge 会保留无关设置；显式批准的 rebuild 遵守上文破坏性契约。未知 extension 冲突会被拒绝；托管 namespace 漂移必须通过绑定预览的 overwrite 批准。创建任何写入产物前，write 会重新检查 Agent 文件、sidecar revision、router identity 和当前模型可用性。

精确历史 v1 signature 可以迁移：Claude 从整个 `env` 替换改为 managed-key merge；opencode 从固定模型改为选定目录子集；Codex 从 `custom` 改为 `mtls-router`，并单独批准 auth 迁移。部分匹配或已修改的历史 signature 不会被认领。迁移绝不删除或重写已有备份。

已有文件与 sidecar 在同一个 journaled transaction 中备份和修改。适用时备份保留在源文件旁，使用私有权限，并可能包含当前或旧 API key，必须按敏感数据处理。Rollback 会一起恢复文件与 sidecar；恢复未解决时不要删除事务状态或备份。

## Protocol v4 自动化

自动化必须使用 receipt-verified `mtls-router-manager serve`。先调用 `manager.info` 并要求 management protocol `4`，然后调用：

1. `agent.models`，提供 `owner`、`agents` 和临时 `api_key`。
2. `agent.render` 获取 key 脱敏托管片段；或使用 `agents`、`catalog_token`、`model_config` 调用 `agent.preview`；请求 rebuild 时还必须提供每个 Agent 的 `modes` map。
3. `agent.write`，提供上述字段、相同 `modes`、预览返回的 `revision_token`、两个显式 approval boolean、与 rebuild-mode Agent 精确一致的 `approve_rebuild` 数组和临时 `api_key`。

清理自动化采用独立的两次调用：先使用精确一个 `agent` 调用 `agent.cleanup.preview`，再使用该 Agent、cleanup `revision_token` 和显式 `approve_managed_overwrite` 调用 `agent.cleanup.write`。这些请求会拒绝 API key、Agent 数组、catalog/model config、flow ID 和未知字段。Cleanup write 在 uncertain delivery 后不可 replay；应重新发现清理状态并生成新预览，而不是重发结果不确定的 write。

即使没有 preset 或没有有效的已请求 section，`agent.models` 也始终包含稳定 preset object：

```json
{
  "preset": {
    "model_config": {},
    "unavailable_agents": {}
  }
}
```

`model_config` 是仅包含完整有效已请求 section 的 versioned 规范文档。不可用 entry 为 `{"code":"MODEL_NOT_AVAILABLE","models":["missing-base-id"]}`。两个字段始终为 object，绝不会是 `null` 或省略；该 metadata 不含 key，也不会让本来成功的 discovery 变为失败。

Key 只能出现在两次 secret-bearing stdin/IPC 请求体中。不得放入参数、环境变量、model config、日志、shell history 或临时请求文件。Protocol v1-v3 请求以及混合 v3/v4 router、manager、setup receipt 或 desktop artifact 都会被拒绝；必须整体更新同一 release。

精确请求/结果 schema 和稳定错误由仓库内 canonical JSON Schema 与 manager protocol type 定义。测试使用的 Agent revision 与 source digest 记录在 [`internal/manager/agent/testdata/compatibility.json`](../../internal/manager/agent/testdata/compatibility.json)。
