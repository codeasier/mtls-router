# internal/manager/metadata

manager / router / 桌面端的构建身份握手元数据，以及发布前的生产身份校验。

## 文件

| 文件 | 职责 |
|------|------|
| `metadata.go` | `Info()` —— 返回代码自有的 manager 握手元数据（`protocol.ManagerInfoResult`）；`ValidateProduction(artifacts ...Identity)`；`Identity{DeploymentID, ManagementProtocolVersion}` |

## 行为

- `Info()` 供 `manager.info` 方法直接返回，字段来自 `internal/version`（`Version`/`Commit`/`BuildDate`/`DeploymentID`）与代码常量 `ManagementProtocolVersion`。
- `ValidateProduction` 拒绝开发态/默认身份（如 `dev`）以及任意一件产物与其他产物不一致的情形。发布工作流可在打包 router、manager、desktop 前调用。

## 关键不变量

- 管理协议版本是代码自有的常量，不经 `-ldflags` 注入 —— router 与 manager 的构建无法各自声明不同版本。
- 混合代产物必须在发布阶段就被拒绝，而不是留到桌面端启动握手才发现。

## 依赖

- `internal/version` —— 构建元数据与 `ManagementProtocolVersion`
- `internal/manager/protocol` —— `ManagerInfoResult` 类型
