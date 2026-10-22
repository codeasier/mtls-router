# 共享 Rust workspace

根 `Cargo.toml` / `Cargo.lock` 为唯一 workspace/锁文件，产物进入根 `target/`。
默认成员只有无 GUI 共享 crates；桌面需显式 `-p mtls-router-desktop`。

| 包 | 职责 |
|---|---|
| `router-contract` | [协议/状态/脱敏与进程身份](router-contract/INDEX.md)；桌面原模块重导出 |
| `router-core` | [HTTP/1/rustls/动态身份/管理端点/supervisor](router-core/INDEX.md)，无 Tauri/WebKit |
| `router-identity` | [私有存储/native 客户端/恢复/续期](router-identity/INDEX.md)，固定双 origin |

`router-core/test-support` 只向跨 crate 测试公开合成 CA/HTTP 服务，正常运行图不含该测试设施。
桌面继续进程内管理，protocol v4 的错误码/错误映射不变。构建身份由应用装配点显式传入，不从共享 crate 的构建环境冒认桌面版本。

验证：`cargo test --locked`（默认共享成员）、`cargo test --workspace --locked`（含桌面）及 `cargo fmt --all -- --check`。
