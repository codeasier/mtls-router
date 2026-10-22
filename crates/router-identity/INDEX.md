# router-identity

桌面与 Linux CLI 共用个人身份核心，不依赖 Tauri、WebKit、密钥环或业务 key。

| 文件 | 职责 |
|---|---|
| `src/crypto.rs` | P-256 PKCS#8/CSR、SHA-256/S256、ASN.1 ECDSA 证明、固定 client-root 下链/SPKI/URI/期限验证 |
| `src/wire.rs` | 公开 native/续期/停用 DTO；不含私钥返回合同 |
| `src/platform.rs` | 私有权限、Unix flock/Windows LockFileEx、fsync/原子替换；供 CLI 状态文件复用 |
| `src/store.rs` | 单一私有状态完整分代、generation CAS、续期待办、停用/取消待办和证书反馈 |
| `src/client.rs` | 固定公信授权/只读恢复、严格 relay mTLS 续期、无重定向/隐式重试、摘要与有界后台退避 |
| `src/tests.rs` / `src/test_support.rs` | 绑定/错误 CA/乱序/崩溃/跨进程、账号切换、秘密出口与合成 CA；真实 HTTPS 由 Go/nginx harness 驱动 |

`identity/` 独立于 `credentials.json`/Agent 文件；目录/文件为 Unix 0700/0600 或 Windows 用户 DACL。原子状态同时保存匹配 key/cert，错误/未来 schema 不自动清空。退出先本地停用，远端未发布只报 pending；迟到登录、续期和 TLS 反馈不能污染新 generation。

续期待办保留原 key 与新 key/CSR/operation，完整 24h 恢复只读同一已提交结果，过期原证的例外不授权模型或新发证。公有 origin 不可由运行参数/IPC 覆盖；测试专用 resolver/origin 只编译于 `cfg(test)`。诊断只输出 `ConnectionSummary`，整目录/临时材料排除于支持包。

验证 `cargo test -p router-identity --locked`；Go 真实 interop：`MTLS_TEST_RUST_NATIVE=1 go test ./internal/routerenroll -run TestRealNginxRenewalAndObservedCRLLoad -count=1 -v`。原生 Windows 与真实组织/CA 验收不能被合成测试替代。
