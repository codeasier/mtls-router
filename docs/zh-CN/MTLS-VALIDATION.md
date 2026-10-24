# 个人 mTLS 验证与容量证据

[English](../MTLS-VALIDATION.md)

## 本地正确性矩阵

在仓库根目录、具备 nginx/`ps`/`lsof`/Go/Rust 及锁定依赖时，以隔离 nginx 进程的所有者运行：

```sh
MTLS_TEST_RUST_NATIVE=1 go test ./internal/routerenroll -run TestRealNginxRenewalAndObservedCRLLoad -count=1 -v
go test -race ./internal/routerenroll ./cmd/codeasier-router-enroll -count=1
cargo test --workspace --locked
make test-workflows
```

显式 Rust bridge 使用真实 `NativeClient`，经合成 HTTPS 信任、仓库 nginx 模板、Go enroll/假 GitCode、CSR 绑定签发、mTLS 业务转发与签名退出。Fixture CA 私钥只在内存，子进程仅取得公有信任文件与 loopback 位置；测试 origin 适配不进入生产构建。

矩阵还覆盖非成员、错误证明、同结果恢复、同证双入口、错 CA/EKU/到期、业务状态独立、流式上传/SSE/取消、模型零控制面查询、禁止 session/连接复用、reload/读头窗口、部分/全部叶撤销及根 authority 吊销。用户/操作 CAS 竞争与临近过期的完整 24h 恢复另有聚焦测试。

reload 回归会暂停旧 worker，验证即使新 worker/标记可用，旧 TCP 监听未关闭时也不能确认发布；恢复 worker 后允许监听退休，不等待已建立 SSE 结束。

## 隔离压测参考

```sh
MTLS_TEST_GATEWAY_SAMPLES=200 go test ./internal/routerenroll -run TestRealNginxRenewalAndObservedCRLLoad -count=1 -v
```

允许 10..1000 samples，每请求完整 mTLS 握手及合成首条 SSE。输出 baseline、常驻 SSE、reload 三阶段的握手/首事件 p95/p99、驱动队列、QPS/失败数、nginx master 加直接 worker 的 RSS 与 CPU 快照，Go driver heap 单列。这些只是本地合成测量，不是 LLM 首 token 或实际入口预算；`ps` CPU 快照也不是持续 CPU 利用率。

## 实际入口验收记录

在等价于实际入口 CPU/内存/WG pool 的隔离环境，记录 worker、连接/读头预算、CA/CRL 大小、包 revision。以代表性上传和长 SSE 收集持续 CPU/RSS、新请求 QPS、排队、握手与真实首 token p95/p99，复测资格复核/续期峰值和 CRL reload，确认模型控制面查询仍为零。只以该机器/负载形成预算，不把笔记本样本按比例换算；超预算仍保持严格认证/不重放，不能关闭 CRL 或 hostname 检查。

原生 Linux 双架构无 GUI 登录/服务替换、Windows DACL、五平台升级、实际 GitHub provenance、真实组织/DNS/WG 须分别留证。输入/证据缺失继续在任务表保持开放，工作流配置与 fixture 不等于验收。
