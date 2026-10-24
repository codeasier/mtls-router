# codeasier-router

新 Linux amd64/arm64 Rust 客户端，复用[共享核心](../../crates/INDEX.md)。构建/运行无 GUI、Tauri、WebKit、密钥环或 Go 子进程。

| 文件 | 职责 |
|---|---|
| `src/main.rs` | login/no-browser、identity status/renew、logout、前台 serve、version；固定 loopback、不抢端口 |
| `build.rs` | 与桌面共享公有信任输入、固定 origin 与版本/deployment/target；不读用户/CA 私钥 |
| `tests/lifecycle.rs` | 真子进程无证管理、OS 身份记录、SIGTERM、冲突与闭合参数 |

默认 XDG state `codeasier-router/`；私有 `identity/` 与公有 `router-state.json` 分离。进程记录只在监听成功后原子发布，退出仅清理仍属于自己的记录；文件 lease 不删除。服务与交互必须同用户/目录。

产物仅为精确命名的两个 Linux `.tar.gz` 与 SHA-256/Sigstore 材料，不加入桌面 bundle/feed；历史 Go/manager/setup 继续冻结。使用与验签见[中文说明](../../docs/zh-CN/LINUX-CLI.md)，用户 systemd 模板见[服务](../../deploy/cli/codeasier-router.service)。配置了原生双架构 CI 不等于验收已经执行。
