# internal/manager/state

持久化的 manager 状态文件读写，以及桌面所有权锁。

## 文件

| 文件 | 职责 |
|------|------|
| `state.go` | `RouterState` 结构；`Read`/`Write`（router 状态）、`ReadJSON`/`WriteJSON`（通用） |
| `lock.go` | `Lock`、`AcquireLock(path)`、`(*Lock).Close()` —— 不等待的独占锁 |
| `lock_unix.go`、`lock_windows.go` | 平台文件锁原语 |
| `replace_unix.go`、`replace_windows.go` | `replaceFile` 原子替换与 `syncDir` |
| `permissions_unix.go`、`permissions_windows.go` | `restrictPath` —— Unix 权限位 / Windows DACL |

## 行为

- `WriteJSON` 使用同目录临时文件 + `sync` + 原子替换，避免写入过程中崩溃留下截断文件。
- `Read` 对「文件缺失」与「权限不足」保持 `errors.Is` 可判别；格式错误或 JSON 之后有多余内容则包装为 `ErrCorrupt`。
- `AcquireLock` 不阻塞等待：已被占用时直接返回 `ErrLocked`。

## 关键不变量

- `RouterState` **保留 setup 脚本使用的字段名**，桌面所有权字段（`owner`、`desktop_session_id`、`manager_*`）以增量方式追加，因此不需要迁移旧状态文件。
- 状态文件同时记录进程身份三元组（`pid`、`process_started_at`、`process_executable`）与协议身份（`deployment_id`、`management_protocol_version`），供 `../discovery` 做分类。
- 损坏的状态文件必须可区分于「不存在」—— 桌面端据此区分「首次启动」与「需要清理」。

## 依赖

- 标准库 + `golang.org/x/sys`

## 被依赖

- `../discovery`（读状态做分类）、`../lifecycle`（写状态与持锁）、`../trustedrouter`
