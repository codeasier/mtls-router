# internal/manager/paths

manager 自有的按用户文件路径解析，不依赖桌面运行时。

## 文件

| 文件 | 职责 |
|------|------|
| `paths.go` | `Paths` 结构与 `Resolve()` —— 按当前用户与操作系统解析全部 manager 路径 |

## 导出

`Paths` 同时给出 CLI 兼容路径与桌面专用路径：

| 字段 | 默认值 |
|------|--------|
| `CLIStateDir` | `MTLS_ROUTER_STATE_DIR`，否则 `~/.mtls-router` |
| `CLIStateFile` | `<CLIStateDir>/setup-state.json` |
| `CLILogFile` | `MTLS_ROUTER_LOG_PATH`，否则 `<CLIStateDir>/mtls-router.log`；作为会话日志路径的 base |
| `DesktopDataDir` | `MTLS_ROUTER_DESKTOP_DATA_DIR`，否则按平台：Windows `%APPDATA%/com.codeasier.mtls-router`、macOS `~/Library/Application Support/com.codeasier.mtls-router`、其他 `${XDG_DATA_HOME:-~/.local/share}/com.codeasier.mtls-router` |
| `DesktopStateFile` | `<DesktopDataDir>/desktop-state.json` |
| `DesktopLogFile` | `<DesktopDataDir>/mtls-router.log`；作为会话日志路径的 base |
| `DesktopLockFile` | `<DesktopDataDir>/desktop-owner.lock` |

## 关键不变量

- CLI 路径必须与 `setup.sh` / `setup.ps1` 使用的位置一致，否则 CLI 安装的 router 无法被 manager 发现。
- 桌面路径与 CLI 路径分离，使桌面会话与 CLI 会话可以各自持有状态而不互相覆盖。
- 本包只做纯路径计算：不创建目录、不读写文件；`Resolve()` 唯一的失败来源是无法解析用户 home。

## 依赖

- 仅标准库（`os`、`path/filepath`、`runtime`）。
