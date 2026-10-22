# router-contract

无 GUI 的共享合同；不持有个人密钥或业务 API key。

| 文件 | 职责 |
|---|---|
| `src/protocol.rs` | management v4 方法、deadline、稳定错误映射 |
| `src/error.rs` | `CommandError` 与安全错误结构，桌面凭据错误转换留在桌面适配 |
| `src/process.rs` | PID/OS start/executable 关联与完整身份下的信号 |
| `src/cli_state.rs` | schema 1 无秘密 `ProcessRecord`、预期 XDG 路径及代码拥有的认证标记 |
| `src/redaction.rs` | 字段白名单、PEM/API key/OAuth/原生证明清洗 |
| `src/manager_diagnostics.rs` | 有界闭集诊断环，不保留原始 stderr |
| `src/windows_security.rs` | Windows 当前用户 SID、保护 DACL 与私有权限检查 |

桌面的同名模块重导出该实现。身份状态位置不能来自未经验证的 HTTP；v4 自报也不是 OS 所有权证明。依赖见 Cargo manifest，验证 `cargo test -p router-contract --locked`；Windows 行为需原生 runner。
