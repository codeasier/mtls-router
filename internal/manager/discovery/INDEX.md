# internal/manager/discovery

把「监听 router 端点的那个进程是谁」分类清楚：关联 HTTP 元数据、持久状态文件与操作系统进程身份。

## 文件

| 文件 | 职责 |
|------|------|
| `discovery.go` | `Discoverer`、`New(Config)`、`Discover`/`DiscoverStatus`/`DiscoverStartupStatus`；`Classification` 常量；`Result`、`Version`、`Health` |
| `../testdata/desktop-state-v0.1.8.json`、`desktop-state-v0.2.0.json` | 从真实 release schema 派生的 protocol 1/3 脱敏 golden 状态；验证旧谱系分类只接受完整进程身份与受支持协议 |

## 分类结果

| `Classification` | 含义 |
|---|---|
| `desktop_owned` | 由当前桌面会话拉起并持有的 router |
| `external_compatible` | 非本会话拉起，但版本 / deployment ID / 协议版本一致，可复用 |
| `degraded` | router 可达但 `/health` 报告 upstream 不可达 |
| `stale` | 状态文件指向的进程已不存在或身份不匹配 |
| `legacy_managed` | 本安装数据目录中的桌面状态能与仍存活的 router 完整身份关联，但会话、协议/部署世代或安装身份不属于当前 epoch |
| `absent` | 端口无人监听 |
| `unknown_occupant` | 端口被占，但对方不是可识别的 router —— 移交 `../occupant` 处理 |

## 行为

- 三个入口是同一个 `discover(ctx, includeHealth, ignoreStaleCLI)` 的三种参数组合：
  - `Discover` —— `includeHealth=true`：额外发起 `/health` 请求，用于需要确认 upstream 可用性的场景。
  - `DiscoverStatus` —— `includeHealth=false`：只看 `/version` 与状态文件，用于状态展示。
  - `DiscoverStartupStatus(ctx, owner)` —— `includeHealth=false`，且 owner 为桌面时 `ignoreStaleCLI=true`，使桌面启动不被过期的 CLI 状态文件阻塞。
- `Config` 的 `ReadState` / `ValidateProcess` / `DialContext` / `HTTPClient` 均可注入，测试因此不需要真实进程或网络。
- 分类同时看三个来源：HTTP `/version` + `/health` 的响应、状态文件内容、`../process` 给出的 OS 身份。任一来源缺失都会降级分类，而不是乐观判定。

## 关键不变量

- `degraded` 与 `absent` 必须可区分 —— router 的 `/health` 恒返回 200，「连接被拒绝 = 未启动，200 = 已启动（可能降级）」是唯一无歧义的判据。
- 复用外部 router 前必须校验 deployment ID 与管理协议版本一致，拒绝混合代。
- 状态文件里的 PID 必须配合启动时间与可执行文件一起校验，仅 PID 相符不足以判定 `desktop_owned`。
- `/version` 元数据单独出现不得升级为 `legacy_managed`；必须与本桌面状态文件中的 genuine 进程身份关联。安装身份不匹配时不得归为本安装的 owned/legacy；installation-aware 状态还必须携带与当前包精确匹配的正数 package generation，generation 0 只接受无 installation ID 的 protocol 1/3 祖先状态（版本集合以 `../protocol` 的 `IsLegacyLineageVersion` 为准）。

## 依赖

- `../state` —— 读取状态文件
- `../process` —— OS 进程身份校验
- `../protocol` —— `RouterOwner`、错误码与 `IsLegacyLineageVersion` 遗留祖先集合
- `internal/version` —— 默认 deployment ID 与协议版本
