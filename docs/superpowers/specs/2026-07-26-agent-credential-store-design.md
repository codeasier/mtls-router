# Agent Configuration Management — Sub-project 0: Global API Key Store

> 这是一份**重设计**设计的 **第一块（sub-project 0）**。
> 全文背景与三块切分见文档末尾「背景与切分」。

| 字段 | 值 |
|---|---|
| 日期 | 2026-07-26 |
| 子项目 | 0 / 3 — Global API Key Store |
| 范围 | 仅做全局 API key 的持久化、加载、清除；新增 Api Keys 区作为侧边栏独立入口 |
| 不做范围 | Agents 总览重构、Agent 独立面板、渲染与写入 UI 改动（属后两块） |
| 依赖 | 无；后两块必依赖本块 |
| 关联代码 | `desktop/src-tauri/src/{commands,manager,paths,types,lib}.rs`、`desktop/src/{ipc,model}.ts`、`desktop/src/{App,AgentPage}.tsx`、`desktop/src/locales/{en,zh-CN}.ts` |

---

## 背景与切分（保持简短；引出本子项目）

当前 Agent 配置模块被建模为一个七阶段向导（select → credential → discover → configure → preview → write → result）。该建模方式造成两个体验病根：

1. **「未检测到」是死文案**：manager 自 commit cc72164 (#74) 起就把 `detected` 硬编码为 `true`；`pathWritable` 向上回溯到第一个存在的父目录判断可写，`writeAtomic` 含 `os.MkdirAll(dir, 0o700)`。后端链路已支持"agent 未安装也能先写配置"。但 UI 的 `detectionState`/`detectionGuidance` 把 `!agent.detected` 当活跃分支处理，"未检测到"提示永远显示为前置门。
2. **重复输入 API key**：`ModelFlow { api_key: Zeroizing<String> }`（`commands.rs:37-46`）只能在 invoke 调用上下文中暂存。每次开向导 / 改完一个字段再保存都要重粘。安全收益近乎 0（key 已注定明文落盘进 agent 凭据文件 + 事务备份），代价却是实打实的。

`Commands.rs` 里 key 要走 Rust → JSON → stdin 协议 → Go manager 全路再被 `request.APIKey = ""` 清零（`app.go` 中）。链路越长，错位越深。

整个体验重构的核心是**让模块成为持续的管理台**，而非一次性向导。但本设计文档只覆盖 **sub-project 0**：先把"全局 API key 持久化 + Api keys 区"做掉——后两块（Agents 总览、Agent 独立面板）依赖它。

---

## §0.1 目标 / 非目标

**目标**

1. 把 API key 的唯一持有者从"每次 invoke 上下文"迁出到桌面端持久文件 `data_dir/credentials.json`（Unix `0o600` / Windows DACL 仅当前用户）。
2. 在侧边栏新增独立「Api keys」区（与 Agents、Router、Logs、Settings 并列），让用户能粘贴 / 替换 / 删除；显示 fingerprint 后四位与保存时间。
3. `commands.rs` 中 `ModelFlow` 改造为不再包含明文 key，所有 agent.* 调用前由 `CredentialStore::use()` 同步注入。
4. **协议零变更**：manager protocol v4 保持 15 个方法；CLI / setup 脚本路径无变化。

**非目标**

- Agents 总览页重构、独立 agent 配置面板设计、configure/preview UI 重新组织——属于 sub-project 1 / 2。
- OS keychain（macOS Keychain / Windows Credential Manager / Linux Secret Service）集成——按用户决策暂时不引入；架构预留未来可升级。
- 协议升级（bump 到 v5 与五方同步发布）——避免无意义复杂度。
- manager 侧任何改动。

---

## §0.2 架构概览

```
desktop Rust (Tauri process)
├── src/paths.rs               # credentials_path = data_dir/credentials.json
├── src/credential.rs          # NEW: CredentialStore 唯一文件拥有者
│   ├── read() -> CredentialSummary / CredentialError::NotFound
│   ├── write(api_key) -> CredentialSummary  # 原子化重命名
│   ├── delete() -> ()
│   ├── use() -> Zeroizing<String>  # 内部 clone，调用方用完 Zeroize
│   └── summary() -> CredentialSummary
├── src/types.rs               # +CredentialSummary (NO key field)
├── src/commands.rs            # 4 new commands: get_credential / save_credential /
│                              # delete_credential / (内部 use_credential)
│                              # ModelFlow 改造：api_key 字段拿掉
├── src/manager.rs             # TransportSession::send_request 增加可选 api_key 注入
└── src/lib.rs                 # AppState 注入 Arc<CredentialStore>

desktop Frontend (React)
├── src/ipc.ts                 # +getCredential / +saveCredential / +deleteCredential
├── src/model.ts               # +"api-keys" route
├── src/App.tsx                # +侧边栏 Api keys 项
├── src/ApiKeysPage.tsx        # NEW (only page touching the store from UI)
└── src/locales/{en,zh-CN}.ts  # +"apikey.*" namespace

disk
└── data_dir/credentials.json  # { "version": 1, "saved_at": "<RFC3339>", "key": "<opaque>" }
```

**模块边界硬约束**：

- `credential.rs` 只依赖 `paths.rs` 的 `credentials_path`、std、zeroize、serde。**不**依赖 `ManagerClient`、`AppState`、`tauri::State`。
- `commands.rs` 是 glue；不直接 `fs::read`/`fs::write`，所有 IO 必经 `CredentialStore`。
- `use_credential` **不**注册为 Tauri command；仅供 `commands.rs` 内部调用。前端**永远拿不到明文 key**。

---

## §0.3 数据流

### 有 key 路径（最常见）

```
用户首次在 Api keys 区粘贴并保存 key
  → CredentialStore::write(api_key)
  → data_dir/credentials.json 写入 (restrictPrivate → 0o600/DACL)
  → 返回 CredentialSummary { present: true, fingerprint, saved_at }

用户进 Agents 区/任意 sub-project 后调用
  api.discoverModels(["claude-code","opencode","codex"])
  → invoke("agent_models", { agents, owner: "desktop" })    // 无 api_key 字段
  → agent_models_command { flows, credential_store, .. }
  → CredentialStore::use()  → Zeroizing<String> { .. } key
  → 注入 key 至 JSON body 为 "api_key": "..."
  → 调用方 Zeroize::zeroize(&mut key) 后立即归还
  → transport.send_request() 至 manager
  → manager.DecodeParams() → request.APIKey = "" 后再处理
```

### 无 key 路径（首次启动 / 用户删除）

```
Api keys 区 status: { present: false }
  → Agents 总览仍渲染三 agent 卡片
  → 进入 agent 独立面板：模型选择区显示 "key 未配置" 引导 + "去 Api keys 区" 按钮
  → 用户保存 key 后返回：自动重新调用 agent.models
```

### 关键不变量（写进 §0.6 测试）

1. **key 永不传给前端任何状态**：Tauri IPC `DesktopApi`、React hook、locale 字符串、日志 stderr、 Redux/Devtools、React Devtools 都接触不到明文。
2. **key 留在 Rust 进程内存时长 ≤ 一次 IPC 调用窗口**：`use()` clone → 写 JSON body → Zeroize；调用栈返回时编译器尽力清零。
3. **Zeroizing<String> 不实现 Serialize / Deserialize**：编译期静态保证（不写 derive）。
4. **credentials.json 文件权限**：
   - Unix `0o600`，用现有文件权限工具（按 `internal/manager/agent/lock.go` 的 `restrictPrivate` 心智模型在 Rust 侧等价的 `set_permissions(0o600)` + `fsync`）。
   - Windows DACL 仅当前用户；用现有 std `OpenOptions` 无补 FILE_ATTRIBUTE_NORMAL + `icacls`？若目前 desktop 侧无 Windows DACL 代码，则暂以 `0o600` ACL + 文档风险说明替代，记入后续工作。
5. **JSON schema 版本化**：`{ "version": 1, "saved_at": "2026-07-26T...", "key": "<opaque>" }`；读时校验 `version == 1`，其它 version / 缺字段 / 错字段 → 当 InvalidFormat 处理。
6. **BOM/UTF 容忍**：写在 Rust 侧按标准库读，写时不要 BOM；损坏 / 解码失败 → 当 InvalidFormat。

---

## §0.4 错误处理

`enum CredentialError`（`thiserror`）含 4 个变体，每个变体在前端映射为稳定的错误码：

| Rust 变体 | IPC 错误码 | 前端行为 |
|---|---|---|
| `NotFound` | `CREDENTIAL_NOT_FOUND` | `present: false`，正常状态 |
| `InvalidFormat` | `CREDENTIAL_INVALID` | 启动时自动 unlink；UI 显示「凭据文件已损坏，请重新保存」 |
| `IoError(io::Error)` | `CREDENTIAL_IO_ERROR` | UI 显示「读写凭据失败：xxx」+ 「查看日志」按钮 |
| `LockTimeout` | `CREDENTIAL_LOCK_TIMEOUT` | UI 显示「另一个操作正在进行，请稍后重试」 |

**panic 安全**：所有 `#[tauri::command]` 路径**不**经过 `unwrap()`；`use()` 抛错时调用方把 key 当空、走无 key 路径（不会 panic 终止进程）。

**不重试**、**不带指数退避**、**不引入断路器**——本地端 IO 失败不是常态。

---

## §0.5 组件边界细述

### Rust 侧 (`desktop/src-tauri/src/`)

**新增 `credential.rs`**（~150 行）：

```
pub struct CredentialStore {
    path: PathBuf,
    inner: tokio::sync::Mutex<()>,  // 进程内并发；同进程足够（manager 子进程是另一进程，靠文件锁）
}

pub struct CredentialSummary {
    pub present: bool,
    pub fingerprint: String,        // base32 后 4 字符
    pub saved_at: Option<String>,   // RFC3339
}

#[derive(thiserror::Error, Debug)]
pub enum CredentialError {
    #[error("credential not found")]
    NotFound,
    #[error("credential file is malformed: {0}")]
    InvalidFormat(String),
    #[error("credential io error: {0}")]
    IoError(#[from] io::Error),
    #[error("credential lock timeout")]
    LockTimeout,
}

// 仅可导出 summary 到 IPC；key 走 Zeroizing<String> 内部分发
impl CredentialStore {
    pub async fn new(path: PathBuf) -> Result<Self, CredentialError>
    pub async fn summary(&self) -> Result<CredentialSummary, CredentialError>
    pub async fn write(&self, key: Zeroizing<String>) -> Result<CredentialSummary, CredentialError>
    pub async fn delete(&self) -> Result<(), CredentialError>
    pub async fn use_(&self) -> Result<Zeroizing<String>, CredentialError>
}
```

**为什么方法名 `use_`**：`use` 在 2024 edition 是关键字，要 `r#use` —— 选择 `use_` 后缀以减少噪音。

**修改 `paths.rs`**：
- `DesktopPaths` 新增 `credentials_path: PathBuf`。
- 路径与 `data_dir` 同根，永远同级。

**修改 `types.rs`**：
- 新增 `CredentialSummary`（仅 present + fingerprint + saved_at，无 key）。
- **不**新增 `Credential { key: String }` 类型——编译期避免误用 IPC。

**修改 `commands.rs`**：
- 4 个新 command：

```
#[tauri::command]
async fn get_credential(state: AppState) -> Result<CredentialSummary, CommandError>
#[tauri::command]
async fn save_credential(api_key: String, state: AppState) -> Result<CredentialSummary, CommandError>
#[tauri::command]
async fn delete_credential(state: AppState) -> Result<CredentialSummary, CommandError>
```

- `ModelFlow` 改造：
  - 拿掉 `api_key: Zeroizing<String>` 字段。
  - 拿掉 `confirmation_token: Zeroizing<String>` 字段（如果发现当前用法实际不需要——本设计阶段确认需求后决定）。
  - 仍保留 `flow_id` 作为一次性 token；调用 `agent_models_command` 等前从 `CredentialStore` `use()` 注入 key。
- `validate_api_key` 留下作为**前端输入校验**（长度 ≤ 16K、非空），不在 manager 边界做。

**修改 `manager.rs`**：
- `TransportSession::send_request` 增加可选 `api_key_injector: Option<Arc<CredentialStore>>` 字段。
- 发往 manager 的 JSON body 在 `use_credential()` 之前克隆、`use_credential()` 后注入 `"api_key"` 字段（仅当 store 有 key 时注入；缺失时不发送字段，保持与现有协议行为一致）。
- 调用方在 body flush 之后 Zeroize 内存暂存的 key。

**修改 `lib.rs`**：
- 启动时构造 `Arc<CredentialStore>`，注入 `AppState`。
- 注册 `get_credential / save_credential / delete_credential` 三个 invoke；**不**注册 `use_credential`。
- 增加启动时凭据文件清理路径：检测到坏 JSON 自动 `delete()` 并落日志。

### 前端 (`desktop/src/`)

**新增 `ApiKeysPage.tsx`**（~250 行）：

UI 结构（React 19 + 既定样式系统）：

```
<section class="apikey-page">
  <header>
    <h1>Api keys / API 密钥</h1>
    <p>你的接入凭证…</p>
  </header>
  <section class="apikey-status">
    <span class="status-indicator" data-state={present ? "saved" : "absent"} />
    <p>{present ? "已配置" : "尚未配置"}</p>
    {present && <dl><dt>指纹</dt><dd>…{fingerprint}</dd><dt>保存时间</dt><dd>{saved_at}</dd></dl>}
  </section>
  <form class="apikey-form">
    <label>
      API key
      <input type={show ? "text" : "password"} value={input} onChange={...} />
      <button type="button" onClick={toggleShow}>{show ? "隐藏" : "显示"}</button>
    </label>
    <button type="submit">{present ? "替换" : "保存"}</button>
    {present && <button type="button" class="text-button" onClick={delete}>删除</button>}
  </form>
  <section class="apikey-explainer">
    <h2>用在哪？</h2>
    <ul>
      <li>写入 Claude Code / opencode / Codex 的配置文件</li>
      <li>从 mtls-router 拉取可用模型目录</li>
    </ul>
    <h2>存哪里？</h2>
    <p>{paths.data_dir}/credentials.json，Unix 0600 / Windows 仅当前用户。</p>
  </section>
</section>
```

- 永远不把 `apiKey` 放进 URL、console.log、localStorage、Postman 历史。
- 提交后清空输入框；保留 "show/hide" 仅在本组件 useState。

**修改 `ipc.ts`**：

```
interface DesktopApi {
  ...
  getCredential(): Promise<CredentialSummary>
  saveCredential(apiKey: string): Promise<CredentialSummary>
  deleteCredential(): Promise<CredentialSummary>
  // 不暴露 useCredential
}
type CredentialSummary = { present: boolean; fingerprint: string; saved_at: string | null }
```

**修改 `model.ts`**：导航枚举加 `"api-keys"`。

**修改 `App.tsx`**：路由分支新增 `api-keys → ApiKeysPage`。

**修改 `locales/en.ts` & `locales/zh-CN.ts`**：加 `apikey.*` 命名空间。

**修改 `AgentPage.tsx`**：
- 仅做最小改动：拿掉 `apiKey` 状态；`discoverModels()` 不再传第二参；继续显示向导视图（sub-project 1 / 2 改其外层结构）。
- 注意：本子项目不动其它向导逻辑；后续子项目会重构主流程。

---

## §0.6 测试

### Rust 单测 (`credential.rs::tests`)

| 测试 | 场景 | 期望 |
|---|---|---|
| `summary_when_file_missing` | 文件不存在 | `NotFound` |
| `summary_when_file_empty` | `{}` | `InvalidFormat` |
| `summary_when_bad_json` | `{` not closed | `InvalidFormat` |
| `summary_when_bom` | `\xEF\xBB\xBF{...}` | ok 后正常读 |
| `summary_when_wrong_version` | `"version": 2` | `InvalidFormat` |
| `summary_when_missing_key` | `{ "version": 1, "saved_at": "..." }` | `InvalidFormat` |
| `write_then_summary` | 写入 → 摘要 | `present: true, fingerprint, saved_at` |
| `write_overwrites` | 写两次 → 第二次 | 仅最后一次 |
| `write_rejects_empty` | 空串 | `InvalidFormat` |
| `write_rejects_too_long` | > 16 KiB | `InvalidFormat` |
| `delete_then_summary` | 写 → 删 → 摘要 | `NotFound` |
| `use_returns_key_then_zeroize_drop` | use → drop | Zeroizing drop 后内存清零（静态检验: Zeroizing 类型本身） |
| `concurrent_save_and_use` | tokio join! save + use | 互斥，无撕裂读 |
| `corrupted_file_on_startup_is_cleaned` | lib.rs 启动时读坏 | 自动 delete + log |
| `file_mode_is_0600` | 写后 `metadata.permissions()` | `0o600` |

### Tauri command 测试 (`commands.rs::tests`)

| 测试 | 场景 |
|---|---|
| `get_credential_no_file` | 没有文件 → `present: false` |
| `save_then_get` | 保存 → get → `present: true` |
| `save_validates_empty` | 空串 → 错误 |
| `delete_then_use_returns_not_found` | 删 → use → 抛 NotFound |

### Frontend 单测 (`apiKeysState.test.ts`, `ApiKeysPage.test.tsx`)

| 测试 | 场景 |
|---|---|
| `save_invoke_signature` | 调用 `saveCredential('...')`，不向 console 输出、不进 history |
| `useCredential_unavailable_at_ipc` | ts 编译期: `useCredential` 字段不存在 |
| `ApiKeysPage_initial_absent` | 初始 present: false 显示「尚未配置」 |
| `ApiKeysPage_after_save` | 显示 fingerprint + saved_at |
| `ApiKeysPage_delete_returns_to_absent` | 删除后回到「尚未配置」 |

### 不被破坏（向后兼容）

| 测试 | 期望 |
|---|---|
| `manager.rs` 现有 TransportSession 测试 | store=None 时，body 不含 `api_key` 字段；store 有 key 时注入 |
| `tests/desktop_workflow_test.sh` 新增一例 | 通过 `save_credential` → `manager.discoverModels(agents)`（无 api_key 形参）成功 |

### 端到端

- `tests/desktop_workflow_test.sh` 加一例：
  1. 启动 desktop（非 GUI 模式 / mock）
  2. invoke `save_credential "fixture-key"`
  3. invoke `agent.models { agents: ["claude-code"] }`
  4. 校验 manager 收到的 JSON 含 `"api_key":"fixture-key"`
  5. invoke `delete_credential`
  6. invoke `agent.models`，校验 manager 收到的 JSON **不**含 `api_key` 字段

---

## §0.7 端到端不变量（重申一次）

1. 前端任何状态、任何日志、任何 IPC 序列化都不含明文 key。
2. Rust 进程内 key 仅在 `CredentialStore::use_()` 返回的 `Zeroizing<String>` 中暂存；调用方在 body flush 后 `zeroize()`。
3. manager 协议 v4 不变；CLI / setup 脚本路径无变化。
4. `data_dir/credentials.json` 权限：Unix `0o600`（硬保证）；Windows DACL 最佳努力。
5. 删除/损坏文件：UI 引导重新保存，不 panic。
6. 与已有 `desktop_workflow_test.sh`、`tests/setup_*_test.sh` 兼容。

---

## §0.8 风险与后续工作

| 风险 | 缓解 |
|---|---|
| Windows DACL 暂不能 100% 等价限制 | 文档风险；后续单独 issue 升级到 `winapi`/`windows-sys` |
| `commands.rs` 中 `ModelFlow` 移除 `api_key` 字段会改变 Rust 公共 API | 桌面 sidecar 全栈一体，安全 |
| 用 std `tokio::sync::Mutex` 而非 crate `fs2`，跨平台语义细微差异 | Windows 上同进程内 tokio Mutex 已足够，跨进程用文件锁补；测试覆盖并发场景 |
| 前端 React 19 的 StrictMode 双调用 useEffect 可能发出两次 `getCredential` | 这是幂等操作；不影响 summary |

未来升级到 OS keychain 时，只需替换 `credential.rs` 的 `read/write` 实现；IPC 接口稳定不动。

---

## §0.9 不在本子项目范围（后续 spec）

- **Sub-project 1**：Agents 区重构成「三 agent 并列总览 → 点进独立配置面板」；进入总览时按§0.3 自动预热。
- **Sub-project 2**：Agent 独立配置面板的 UI 重构（configure/preview 状态机收敛、existing prefill 持久化、增量保存）。

后续每个子项目都出独立 spec 与 plan。
