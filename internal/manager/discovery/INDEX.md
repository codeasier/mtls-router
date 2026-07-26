# internal/manager/discovery

把「监听 router 端点的那个进程是谁」分类清楚：关联 HTTP 元数据、持久状态文件与操作系统进程身份。

## 文件

| 文件 | 职责 |
|------|------|
| `discovery.go` | `Discoverer`、`New(Config)`、`Discover`/`DiscoverStatus`/`DiscoverStartupStatus`；`Classification` 常量；`Result`、`Version`、`Health` |

## 分类结果

| `Classification` | 含义 |
|---|---|
| `desktop_owned` | 由当前桌面会话拉起并持有的 router |
| `external_compatible` | 非本会话拉起，但版本 / deployment ID / 协议版本一致，可复用 |
| `degraded` | router 可达但 `/health` 报告 upstream 不可达 |
| `stale` | 状态文件指向的进程已不存在或身份不匹配 |
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

## 依赖

- `../state` —— 读取状态文件
- `../process` —— OS 进程身份校验
- `../protocol` —— `RouterOwner` 与错误码
- `internal/version` —— 默认 deployment ID 与协议版本
