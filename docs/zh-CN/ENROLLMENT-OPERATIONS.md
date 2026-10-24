# 领证签发主：持久化与恢复

本文是新个人设备证书服务的部署模板，不代表生产 OAuth、CA、域名、CRL 分发或浏览器领证已经上线。原生授权已有隔离验证，实际 OAuth 应用及后续网关发布仍待验收。见[英文版](../ENROLLMENT-OPERATIONS.md)。

## 存储与单主运行

- 用独立 Unix 服务账户以前台方式运行 `codeasier-router-enroll`。[服务单元](../../deploy/enroll/codeasier-router-enroll.service)及[配置](../../deploy/enroll/enroll.env.example)仅为示例，应用/组织 ID 必须替换为已核验值。
- 状态目录 0700，DB/WAL/SHM、日志及密钥文件 0600。日志目录必须通过 `--journal-dir` / `MTLS_ENROLL_JOURNAL_DIR` 显式指定，不能在 state 内，也不能通过 symlink 别名混入。
- `/srv/codeasier-enroll-journal` 必须由服务账户拥有并保持在 state 目录之外。默认部署把它放在独立持久挂载上；2026-10-24 起允许“本地系统盘 + 独立异地快照”的降级部署：jump2 每 10 分钟把追加日志快照到 jump，恢复时使用该异地副本，且不得与 DB 备份同批回滚。此时磁盘丢失的 RPO 以最近一次成功快照为上限（当前 10 分钟），unit 只检查目录/权限，不再断言挂载点；该降级必须在验收台账中明确记录，不能把同盘兄弟目录本身说成独立恢复域。
- 日志只通过服务 API 追加，含随机 store ID、序号/哈希链及有类型参数的确定性 SQL 变更，包含已加密 GitCode 密文及公有证书结果。它属于敏感运维状态，排除在诊断之外，不能随普通 DB 备份回滚。
- 先 fsync 日志，再提交 SQLite 投影/检查点；两者之间崩溃可重放，保留原 serial 和所有撤销，包括尚未发布至网关 CRL 的决定。两者完成后才响应，响应中断不能当作未签发/未撤销。
- 文件锁串行化服务/admin 提交，签发租约也属于日志目录，因此换 state 路径不能再开一个签发主。迁主必须先停止并 fence 旧主，本地文件锁不是分布式 HA，备用节点不能凭复制的日志签发。

**仅在全新部署时**，以服务账户初始化：

```sh
codeasier-router-enroll journal-init --journal-dir /srv/codeasier-enroll-journal
```

已有日志不覆盖，普通启动不自动创建丢失日志。不要用初始化修复丢失的恢复依据。哈希链能发现损坏/错配，不能防御有权限的人同时回滚 DB 与日志；必须在运维层保管和验证可信最新日志。

## 离线根与受限在线 issuing CA

使用专用 clientAuth CA，与服务器 TLS、旧共享 client CA 和 updater 签名分离。离线根私钥不进入签发服务、nginx 或 CI。根 CA 的 index/serial/CRL number 随其加密私钥离线备份。先进入全新私有工作目录、设置 `umask 077`，创建空 `index.txt`、内容均为十六进制 `1000` 的 `serial`/`crlnumber` 及 `newcerts/`，复制 [offline-root.cnf](../../deploy/enroll/offline-root.cnf)。

在离线根主机上执行：

```sh
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -aes-256-cbc -out root-key.pem
openssl req -new -x509 -sha256 -days 3650 -config offline-root.cnf -extensions root_ca_ext -key root-key.pem -out root-cert.pem
```

在授权的签发主机生成在线 key 和公有 CSR（只把 CSR 送至离线根，key 留在私有位置）：

```sh
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out issuing-key.pem
openssl req -new -sha256 -key issuing-key.pem -subj /CN=CodeasierRouter-Personal-Issuer -out issuing.csr
```

回到离线根主机签发：

```sh
openssl ca -config offline-root.cnf -extensions issuing_ca_ext -days 1095 -in issuing.csr -out issuing-cert.pem -notext
openssl ca -config offline-root.cnf -gencrl -out root-crl.pem
```

只送回 issuing/root 公有证书及 root CRL。在线服务只接受匹配的未加密 PKCS#8 P-256 issuing key、`pathLen=0`、精确 `keyCertSign,cRLSign` 和 `clientAuth` EKU。叶身份由代码构造，不复制 CSR subject/SAN/CA 权限。根 CRL 保鲜及后续 issuing CRL 发布仍是独立运维职责。

独立的 32 字节 AES token key 由文件提供，不放 argv/env；新实例可在 `umask 077` 下执行 `openssl rand -out token-key 32`。专用 OAuth secret 用获批的密钥管理器写入另一个私有文件，不打印、不进入模型流量或支持包。

## 在线备份与恢复

先建服务账户拥有的私有备份目录，再执行：

```sh
codeasier-router-enroll admin backup --state-dir /var/lib/codeasier-router-enroll --journal-dir /srv/codeasier-enroll-journal --output /private-backups/enroll-unique.sqlite3
```

该操作持有提交锁完成追平和 `VACUUM INTO`，fsync 一个新的 0600 备份，并输出公有 store ID/序号/摘要/文件校验和 manifest，拒绝覆盖已有备份。manifest 只用于文件完整性，不能代替最新日志。token key、issuing key/cert、根公有链和离线根状态须分别加密备份。恢复同一身份时不要生成替代密钥。

1. 停止领证服务和 admin 写入，fence 旧签发主，独立保管当前可信最新日志；保留网关现用 CRL，不随数据库回滚其版本。
2. 校验备份完整性、所有权/权限、token key、签发链及日志来源；把一致 SQLite 备份恢复到空私有 state 目录，不带旧 WAL/SHM，也不从该 DB 备份恢复日志。
3. 使用当前独立日志目录和原密钥文件，只启动一个服务。服务核验 store ID/schema/检查点/哈希链后，先重放缺失提交再就绪。完整日志与原密钥也可以重建完全丢失的 DB。
4. 对比 admin 公有证书/撤销摘要、最新检查点和各网关实际 CRL 版本/ack，再恢复控制面可用性。尚无发布确认的本地决定仍为 `publication=pending`。
5. 日志缺失/损坏/不完整/错配、DB 超前、路径不安全或密钥错误均停止恢复。保留证据并取得可信最新日志；确实无法取得时停止原 CA 签发，走显式 CA 事故恢复流程，不能重置日志或降低校验。

当前 schema 为 5；已由日志锚定的 schema 2–4 使用同一提交协议迁移。早期没有独立依据的开发 DB 会被拒绝，不能自动接管；须明确迁移/恢复决定，而不是直接认可旧快照。

## 续期与完整 24 小时结果恢复

新续期使用精确 `POST https://relay.codeasier.top/enroll/renew`，只由独立 gateway Unix socket 接收；公开 Handler 不能把调用者提供的证书头当成签发授权。nginx worker 仅获 socket 访问组及 0750 runtime 目录的访问权限，state/key 目录仍为 0700；socket 为 0660，已有或陈旧 socket 不自动删除。

续期轮换本地 key，绑定 operation UUID/新 CSR 并强制复核成员；7 天窗口、单后继及 user/token/device 版本在提交时再次检查。后台复核目标 300 秒，GitCode 故障不等于退组。token 刷新保持加密，包括刷新成功后成员查询暂时失败时的 refresh token 轮换。

`POST https://enroll.codeasier.top/enroll/renew/recover` 只读取已提交的公有结果。原 key 的新鲜签名绑定用途/origin/path、issuer/serial、operation 和 CSR DER SHA-256；客户端须私有保留两代待办 key，直至激活或恢复期限结束。完整 24 小时从提交算起，不因原证提前到期而缩短；过期原证仍不能用于模型 TLS 或新签发。撤销、设备停用、策略变化或后继撤销仍拒绝恢复；未知操作/恢复超时不补发新证。

设备退出通过独立域分离签名调用 `/enroll/device/revoke`，只影响该绑定设备。配置网关完整加载撤销集合前始终返回 `revocation_pending`；本地停用与远端确认是不同状态。

## 签名 CRL、双网关与实际加载确认

有效离线根 CRL 吊销 issuing CA 属于**拒绝材料**，不能当成损坏更新而忽略。其只增撤销集合先入独立日志/检查点，再阻止新签发；发布器可用最后仍有效的 issuing CRL 搭配新根 CRL 分发，不再使用已吊销 key，网关加载后拒绝该 issuer 全部叶证。更高 number 不能移除记住的根撤销；更换 authority 仍走显式事故流程。

网关只发封闭证书拒绝 JSON/marker，并隐藏后端伪造的同名 marker，业务状态原样保留。原生诊断区分“拒绝”与“已确认撤销”，按 generation/serial 保护反馈，迟到失败不能污染新账号。

1. 签发主安装当前离线 `root-crl.pem` 和私有 0600 的[双网关配置](../../deploy/enroll/gateways.json.example)，设置 `MTLS_ENROLL_ROOT_CRL_FILE`、`MTLS_ENROLL_GATEWAYS_FILE`。核验根 CRL 签名/期限、issuing CA 撤销及单调 number/digest；无效根材料阻止新签发，不降低 TLS。
2. 每个网关仅安装 pinned **公有** root/issuing 证书、服务二进制及 root 拥有的 `/var/lib/codeasier-router-gateway`（0700），不复制 issuing/OAuth/token key。安装并适配 [nginx 模板](../../deploy/nginx/codeasier-personal.conf.example)，替换服务器 TLS 和后端路由，旧共享 CA vhost 保持隔离。领证页必须发送 `Referrer-Policy: same-origin`；`no-referrer` 会让 Chromium 对同源表单提交 `Origin: null`，被严格 `authorize` 校验以 `ENROLL_INVALID_REQUEST` 拒绝。客户端构建的 `UPSTREAM_CA_PEM` 必须是真实 relay 公有根（当前 ISRG Root X1）；占位 CA 会无法验证服务器证书，并可能被误报为 `CLIENT_CERT_REJECTED`。CRL SHOULD 携带 AKI；为兼容既有离线根 CRL，加载器仅在 AKI 缺失时退回“签名 + RawIssuer 绑定”，非空 AKI 必须等于 issuer SKI。新 root CRL 由 `offline-root.cnf` 的 `crl_ext` 生成 AKI。
   loader 还依赖 `ps` 与 `lsof`，必须以 root 或 nginx master/worker 的所有者运行，不能把权限不足导致的描述符不可见当作已退出。`--nginx-pid-file` / `MTLS_GATEWAY_NGINX_PID_FILE` 默认 `/run/nginx.pid`，必须匹配实际 nginx 配置的 PID 文件。
3. 签发服务账户 `known_hosts` 固定两个 SSH host key。专用 `codeasier-crl` 账户仅允许固定 loader：使用 root 拥有的 forced command 及精确 sudo 规则 `/usr/local/libexec/codeasier-router-enroll gateway-load --id jump`（另一节点为 `jump2`）；不授予任意命令、参数通配或登录 shell。loader 从 stdin 读取有界公有 JSON，只输出 ack。
4. 首次配置可能已安装有效 CRL/标记文件，但 nginx 尚未运行时不能 ack。配置校验后启动 nginx，再重试同一版本。正常每小时刷新，有新撤销及时生成；`current-crl.pem` 同时包含 issuing/root CRL。
5. loader 先 fsync `high-water.json`，核验签名/issuer/链/期限/number/digest，再原子安装文件并 `nginx -t` / reload。配置测试失败保留旧文件，但高水位不倒退；发出 reload 后保留有效新材料以重试，不回滚撤销集合。
6. 先快照 master 的旧 worker，等待它们全部关闭 TCP 监听 socket。nginx 可能先启动新 worker，再通知旧 worker 退出，因此仅看到一次新标记不足以确认。旧 worker 可继续持有已建立的 SSE 连接，loader 不等待其进程退出；检查失败/超时保持未确认。然后必须以新 TCP 连接从**仅 loopback** `127.0.0.1:19102/crl-loaded` 观察到与 `ssl_crl` 同一配置代际编译的 issuer/number/digest/root-digest 标记才 ack。文件拷贝或 reload 信号成功都不足以确认；不得公网暴露/转发该端点。

签发主持久化各网关 ack 并核对当前所有撤销决定的覆盖，报告 `pending`/`loaded`、逐节点状态及错过 5 分钟目标的 `target_exceeded`。已加载撤销的小时刷新不会被误报成新撤销超时。

```sh
codeasier-router-enroll admin publication --state-dir /var/lib/codeasier-router-enroll --journal-dir /srv/codeasier-enroll-journal --issuer-cert-file /etc/codeasier-router-enroll/issuing-cert.pem --root-crl-file /etc/codeasier-router-enroll/root-crl.pem --gateways-file /etc/codeasier-router-enroll/gateways.json
```

非签发节点使用[转发 unit](../../deploy/enroll/codeasier-enroll-forward.service.example)，经 pinned host key 的 SSH 把 loopback 19101 与续期 Unix socket 接到同一签发主。主节点限制该账户的 TCP 目标和 gateway socket 访问组，禁止 shell 与私有 state/key 访问；不启动第二个签发器。模型流量仍走 nginx backend pool，不进入控制面隧道。

重启保留网关高水位和最新公有材料，不恢复旧网关快照，使用签发主最新发布恢复。隔离测试已覆盖部分/伪造 ack、加载中断和真实 nginx mTLS 续期/reload/撤销拒绝；真实双节点生产/DNS 验收仍是独立任务。
