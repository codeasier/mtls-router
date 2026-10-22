# internal/routerenroll

个人设备证书控制面，与冻结 Go router/manager 分离；不承接模型流量或业务 API key。

## 文件映射与关键导出

| 文件 | 职责 |
|---|---|
| `contract.go` | 固定双 origin、`NativeStart` / `Identity`、`ParseCSR` 的 P-256 签名与 DER/SPKI 摘要、S256 证明、公开浏览器链接及 `WriteError` 封闭错误 |
| `contract_test.go` | 消费共用 JSON 合同，校验 CSR/证明、浏览器和请求字段边界及错误脱敏 |
| `certificate_fixture_test.go` | 内存生成专用 CA/受限签发者和正反证书、签名 CRL，校验公有证书场景 |
| `router_fixture_test.go` | 新认证代际、CLI 状态、精确管理路由及 HTTP/SSE 的 golden 合同 |
| `http.go` / `http_test.go` | `ValidateListen`、`NewHTTPServer`、`NewHandler`、`Serve`；显式 loopback、有界 HTTP、最小 ready、封闭访问日志与退出 |
| `store.go` / `store_test.go` | `OpenStore`；证书专用 SQLite schema、单连接 immediate 事务、WAL/FULL、AES-256-GCM 与 epoch/user/token-version 绑定、重启/未知 schema/错误密钥拒绝 |
| `store_unix.go` / `store_other.go` | Unix 服务账户所有权/0700/0600 与单主 OS 锁；非 Unix 签发主不支持且闭合拒绝 |
| `policy.go` / `gitcode.go` / `gitcode_test.go` | `LoadConfig` / `ActivatePolicy` / `NewGitCode`；组织稳定 ID/epoch、固定 OAuth/user/membership、最小 scope、加密 token 版本与可信用户绑定 |
| `issuer.go` / `issuer_test.go` | `LoadIssuer` / `NewIssuer` / `Issue`；P-256、匹配 PKCS#8、专用 clientAuth/pathLen=0 issuing CA、代码构造 URI 身份、随机 serial、期限上界 |
| `ledger.go` / `ledger_test.go` | 私有 `issueAuthorized`；事务签发台账/设备绑定/幂等恢复/唯一 serial、无秘密 `CertificateSummary`、撤销意图与封闭审计 |
| `admin.go` | `OpenAdmin`；仅服务账户私有目录权限保护的 `List` / `Revoke`，不读取秘密或取得签发能力，不创建公网管理 API |
| `journal.go` / `journal_test.go` | `InitJournal` / `RecoveryCheckpoint`、捕获 SQL 的 `writeTx`、日志先提交/SQLite 重放；旧快照、崩溃、错配/截断/缺失拒绝及并发单主回归 |
| `backup.go` | `Admin.Backup`；提交锁内 `VACUUM INTO`、新私有文件 fsync、无秘密检查点/文件摘要 |
| `ca_templates_test.go` | 真实 OpenSSL 在临时目录执行离线根/issuing CA/根 CRL 模板，再由 Go `LoadIssuer` 与签发器核验 |
| `native.go` / `native_test.go` | `NewService` / `Handler`；10 分钟 CSR/SPKI/设备/epoch/证明绑定原生事务，5 秒轮询、容量限制和证明保护的取消 |
| `browser.go` / `browser_test.go` | 固定 OAuth、Secure/HttpOnly/Lax cookie、一次性 state/CSRF、可信账号及组织确认、批准/拒绝与网络返回 CAS |
| `native_issue.go` / `native_issue_test.go` | 已批准事务的最终成员复核、用户/token/事务版本检查、同 serial 的 24 小时恢复、签发后取消转撤销待发布 |
| `limits.go` / `limits_test.go` | 32 个控制面工作槽、创建 burst 10 + 每秒 1 个补充、饱和时 ready 豁免、重复 JSON 字段拒绝 |
| `renewal.go` / `renewal_test.go` | 原 key 域分离持有证明、可信 mTLS 续期、单后继与原证过期后的完整 24h 只读结果恢复、交接退休 |
| `gateway.go` / `gateway_test.go` | 独立保护的 Unix socket；两监听先绑定再服务、有界退出、拒绝组可写父目录及占用路径 |
| `membership.go` / `membership_test.go` | 按用户 single-flight、加密 refresh token 轮换、300s 目标后台复核、可信退组撤销与网络返回 CAS |
| `crl.go` / `crl_test.go` | `BuildCRL` / `ValidateCRL`、issuer/root 签名与单调版本、小时刷新/新撤销即时生成、根 CRL 检查点 |
| `root_crl.go` | 根撤销只增检查点、已吊销 issuing CA 的提交门与只读恢复拒绝；有效根吊销作为拒绝材料继续发布 |
| `publication.go` / `publication_test.go` | 双 SSH 网关公有分发、逐节点 ack/覆盖验证、`Admin.PublicationStatus` 与 5 分钟降级证据 |
| `gateway_load.go` | `ApplyGatewayBundle`、高水位先落盘、原子 CRL/marker、nginx test/reload 后新连接观察代际才 ack |
| `gateway_reload.go` / `gateway_reload_test.go` | 按 master PID 快照旧 worker，以 `ps`/`lsof` 确认旧 TCP 监听全部关闭后才完成 reload；不等待在途 SSE 所在进程退出，检查失败不 ack |
| `nginx_integration_test.go` | 真实 nginx/OpenSSL 的强制 mTLS、Unix 续期、API key 透传、加载确认和撤销后拒绝；缺 nginx 显式 skip |
| `gateway_load_bench_test.go` | 显式 samples 开关的隔离 TLS/SSE/reload 压测、分位延时/队列/QPS/主从 worker 内存；不是生产入口容量证据 |
| `device_revoke.go` / `device_revoke_test.go` | 域分离设备退出、仅停用本绑定设备、幂等 pending/实际 revoked、迟到续期不能恢复 |

## 不变量与依赖

- 自动领证固定 `enroll.codeasier.top`，模型固定 `relay.codeasier.top`；无可变回调/组织/入口输入。
- CSR subject/SAN/扩展仅是未经信任的请求数据，不能作为签发身份；仅接受签名有效的 ECDSA P-256 PKCS#10。
- CSR/SPKI 摘要按 DER 求 SHA-256 后小写 hex；交接 challenge 是 verifier UTF-8 的 SHA-256、无填充 base64url。
- verifier 为 32 随机字节的无填充 base64url；浏览器只见公开事务 ID。
- `cmd/codeasier-router-enroll` 以前台方式运行，flag 优先于同名 `MTLS_ENROLL_*` env；ready 校验独立日志/投影及当前有效 CA，不依赖 GitCode 在线状态。
- 所有控制面端点精确匹配；访问日志仅 method/path/status/bytes/latency，path/method 均为封闭分类，不含 query、请求头或正文。
- [共用 fixture](../../testdata/router-mtls-v1/protocol.json) 供 Go/Rust 使用，全部为合成数据，不代表生产验收。
- [证书 fixture](../../testdata/router-mtls-v1/certificates.json) 固定签发/恢复/交接场景；[路由 fixture](../../testdata/router-mtls-v1/router.json) 明确与旧共享身份代际的差异，后续核心/发现实现须按其验证。
- SQLite 驱动固定 `modernc.org/sqlite v1.61.0`（纯 Go）；其他依赖为标准库与 `golang.org/x/sys/unix`。
- 台账分离 policy/users/encrypted tokens/devices/enrollment/issuance/certificates/renewal/revocations/CRL/实际网关 ack；不包含 Router access/refresh 会话。
- `GITCODE_ORG_PATH` 缺省 `msinsight-org`、显式空值拒绝；首次核验绑定稳定 ID，配置变更推进 epoch 并记录旧证书撤销意图，不冒充网关已生效。
- GitCode user ID 为不透明稳定字符串，组织成员结果必须匹配该 user ID 与稳定组织；403/404/缺字段/路径漂移均不等同于确认退组。最小请求 scope 固定 `read_user read_groups`，真实应用验收仍待上线输入。
- schema 5 的 SQLite 是独立日志投影；已锚定 schema 2–4 通过日志化迁移增加浏览器、复核时间和 root CRL ack 字段，早期未锚定 schema 1 不自动接管。普通启动不新建日志；日志须独立持久/快照域，不能随 DB 回滚，哈希链不防御服务账户同时篡改两者。独立 `head.json` 高水位可发现完整尾记录丢失。
- 单主锁同时绑定 state 与 journal，持有到关闭，不删除锁文件；不同 state 目录不能绕过同一 journal 的签发租约。跨主机仍须先 fence 旧主，本地锁不是分布式 HA。
- 在线签发者仅持 issuing key；公有链固定 root→clientAuth issuing→leaf，叶仅 digitalSignature/clientAuth。身份 URI 为 `urn:codeasier:router:identity:v1:` 加无填充 base64url JSON，CSR 的 subject/SAN/CA 扩展不会被复制。
- 最终领证只在正确 native verifier、显式设备批准及可信成员复核后进行；先日志和 SQLite 持久化，再通过 native poll 返回公有结果，不向浏览器返回 private key 或原生证明。正常轮询不改变浏览器授权版本，取消/拒绝/策略变化会使迟到返回失效。
- 一次性 OAuth state 与浏览器 cookie 绑定，浏览器写操作校验固定 Origin、CSRF、单值字段与固定回调；标签/账号以 HTML 模板转义。签发后取消持久化单 serial 撤销并返回 `revocation_pending`，不宣称未签发或已全网生效。
- 管理命令均需 `--state-dir <dir> --journal-dir <independent-dir>`：`admin list`、`admin revoke --serial <serial>`（或单一 `--device` / `--user`）、`admin backup --output <new-private-file>`。撤销返回 `publication=pending`，不伪造网关生效。
- Admin 可与单一签发主并存，以 SQLite 短事务串行写撤销意图；只有签发主持有 OS 写主锁、签发 key 和 token key。CLI 无秘密 list 有界为 1–100 项。
- 新续期只在独立 Unix gateway Handler 上可达；公开 Handler 仅有已提交结果恢复和设备退出，不信任公网身份头。原证到期后的恢复例外不授权模型请求或新签发，24h 期限不延长。
- CRL 发布与资格维护不在模型路径；根 CRL 无效/过期时阻止新签发，模型仍按网关现用有效链/CRL 判断。双网关公有 SSH 分发固定 host key/forced command，根/设备私钥不进入传输。ack 先要求旧 worker 关闭 TCP 监听，再观察与 `ssl_crl` 同代际的 loopback marker；新旧 worker 会短暂共存，单次新 marker 不足以确认。loader 依赖 `ps`/`lsof`，需 root 或拥有 nginx 进程；`--nginx-pid-file` / `MTLS_GATEWAY_NGINX_PID_FILE` 默认 `/run/nginx.pid`，必须匹配实际配置。部分加载不称全局生效。
- 有效根 CRL 吊销 issuing CA 时先持久化不可移除的根撤销集合，提交门停止签发/恢复；继续发布最后有效 issuing CRL 与新根拒绝材料。网关同样防更高版本移除旧根撤销，不能因“CA 已吊销”而保留较宽松旧材料。
- [服务/CA 模板与恢复手册](../../docs/zh-CN/ENROLLMENT-OPERATIONS.md) 描述独立挂载、显式 journal-init、离线根、备份/恢复和跨主机 fencing；这些不是生产已部署证据。
