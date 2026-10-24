# router-core

共享 HTTP/1 流式代理、rustls/ring 和独立 supervisor，不含 Tauri/GUI。

`src/router_core/` 保留原模块布局；桌面 `router_core/mod.rs` 为重导出。

| 文件 | 职责 |
|---|---|
| `tls.rs` / `connect.rs` | 私有身份快照、按 generation 缓存不可变 TLS、新请求完整握手、失败安全分类 |
| `startup.rs` / `supervisor.rs` | 配置/绑定/独立 runtime、并发/首包超时、panic/退出；个人模式无证仍启动管理 |
| `proxy.rs` | 请求/响应流式、取消、无发送后重放、保留业务状态/key、清除内部身份头 |
| `handlers.rs` | 精确 `/version`/`/health`；代码拥有的个人认证标记，health 恒 HTTP 200 |
| `probe.rs` | 独立探测预算，区分可信网关证书拒绝标记与业务 key 错误 |
| `stack.rs` / `config.rs` | 锁定栈、TLS floor、配置优先级与安全 URL/监听校验 |
| `parity.rs` / `personal_tests.rs` | 冻结 Go 合同与个人模式的有意差异、热切换/停用/在途 SSE |

个人请求在 blocking pool 读取本地快照，不访问 GitCode/控制面数据库。新请求捕获一代配置，已发流保持原连接；过期/停用不能借缓存复用。精确管理端点绕过业务饱和/超时，保持独立 health 处理。

静态合成材料模式仅用于历史对照/底层装配；生产桌面/CLI 使用 `TlsMaterials::from_identity`。`test-support` 仅服务跨 crate 合成 TLS 测试。验证 `cargo test -p router-core --locked`，实际网关撤销/连接语义另由 Go 真实 nginx 测试覆盖。
