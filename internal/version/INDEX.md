# internal/version

链接期构建元数据变量与访问器。

## 文件

| 文件 | 职责 |
|------|------|
| `version.go` | `Version`、`Commit`、`BuildDate`、`DeploymentID` 变量；`ManagementProtocolVersion` 常量；`Info()`、`InfoJSON()` |

## 链接期变量

通过 `-ldflags -X github.com/codeasier/mtls-router/internal/version.*` 设置：

| 变量 | 默认值 | 说明 |
|----------|---------|-------------|
| `Version` | `"dev"` | 语义版本 / git tag |
| `Commit` | `"unknown"` | 短 git SHA |
| `BuildDate` | `"unknown"` | UTC ISO-8601 构建时间戳 |
| `DeploymentID` | `"dev"` | 服务环境标识 |

## 常量

- `ManagementProtocolVersion = "3"` —— 代码所有，不可在链接期覆盖。

## 注入点

- `scripts/build.sh`（本地）
- `Dockerfile`（容器）
- `.github/workflows/release.yml`（发布）
- `desktop/scripts/build-sidecars.sh`（桌面 sidecar）
