# 个人 mTLS 与 jump 平滑迁移 runbook

[English](../MTLS-MIGRATION.md) · [领证/CRL 运维](ENROLLMENT-OPERATIONS.md) · [Linux CLI](LINUX-CLI.md)

本文是分阶段运维计划。仓库模板与隔离测试不代表生产 DNS、CA、OAuth、WireGuard 已变更。关联运维变更须记录实际节点，并遵守当前 `monitoring/docs/operations/jump2-center-migration.md` 的顺序：**业务入口先迁、站点/WG 后迁、jump 最后降级**。

## 必备运维记录

新流量进入前分别记录 jump、jump2 的已核验值：

| 范围 | 所需证据 |
|---|---|
| 服务器身份 | `DNS:relay.codeasier.top` SAN、客户端固定服务器 CA；公信领证域名单独验证 |
| 客户端身份 | 专用离线根/issuing 指纹、有效 root/issuing CRL 的 number/digest/期限 |
| 控制面 | 已 fence 的单签发主、独立持久日志、专用 OAuth 回调/scope 与稳定组织 ID |
| 网关 | 精确 vhost/模板版本、实际加载标记/ack、接入同一签发主的通道 |
| 后端 | 各节点前向及**回程**、pool 成员、是否仍经 jump 桥接 |
| 灰度 | 实际可用包/tag、原 updater feed/签名、DNS TTL/缓存观察及回滚负责人 |

新 relay 不信任旧共享 client CA；旧 IP:9099 等历史 vhost 在迁移期间独立保留，不能把校验证书改成 optional 来完成首次领证。首次领证走公信 `enroll.codeasier.top`，模型走严格 `relay.codeasier.top` 加原业务 key，无在线鉴权子请求。

## A：先准备，再逐节点强制解析

1. DNS 切换前完成两节点服务器 SAN/信任及新 client CA/CRL。SNI 和 hostname 校验不可跳过，受控检查可只提供公有服务器 CA：

   ```sh
   openssl s_client -connect NODE_IP:443 -servername relay.codeasier.top -verify_hostname relay.codeasier.top -verify_return_error -CAfile server-ca.pem
   ```

   这只验证服务器身份，不等于已通过业务认证。
2. 专用验证设备通过临时 hosts/专用 DNS，把**同一 relay 域名**分别固定到每个节点，不能关闭 hostname/证书校验。同一已安装个人身份在两节点复用，设备证书不绑定入口 IP。
3. 验证 native 领证/续期、原 key/用量/图片/SSE、上传流式、取消和原样业务 401/403/5xx；使用原生应用持 Key 的既有路径，不把真实 key 放 shell 参数/env/日志。负例使用受控测试身份，不导出用户私钥。
4. 模型路径 GitCode/领证查询必须为零。缺证、旧共享、错 CA/EKU、过期/撤销均不能抵达后端；没有等价证据前保留禁用 resumption/keepalive/HTTP2 的基线。
5. 对双设备中的一台撤销，取得**两个**实际网关 ack 且覆盖全部撤销；新握手拒绝该设备，另一台仍正常。部分发布或过期 CRL 节点不得作为切换目标。

若 jump2 经 jump 桥接抵达后端，只标记“入口已验证，仍依赖 jump”，不能宣称站点/WG 已迁完。

## B：原生包灰度与 DNS

保持 `release.codeasier.top` 的服务器证书、六平台 feed 和原 updater key 可用。验证旧桌面可下载/验签完整新版，再一键领证；后续更新保留 API key、Agent URL/文件及私有身份。新 CLI 独立验签，不覆盖历史 Go 资产。

仅在两节点等价且回程均正常后，以已观察的低 TTL 将 relay 从已验证旧节点移到 jump2；不增加未经测试的第二 A 记录。旧节点继续服务 DNS 缓存和已获准 SSE，以真实流量/连接排空记录判断退役，不能只等 TTL；已发送请求不因切换重放。

签发主独立于 relay DNS。非签发节点仅经受限 pinned-key 隧道转发有界控制路径，模型仍走 backend pool。迁签发主须单独 fence/恢复备份、最新日志和匹配密钥，不能启用第二份活动副本。

## C：站点/WG 与退役

业务入口已证实脱离 jump 后，按运维 runbook 的前向/回程顺序迁站点/WG，核验每站点和必要客户端类别；API 入口成功不能代替站点证据。全部验证和缓存/长流排空后才降级 jump。

只有新版包可用、用户覆盖、授权/续期及回滚证据完整后，才退役旧共享入口。同步移除旧共享 CA 信任，保留历史 tag，并继续提供更新域名。

## 不降级的回滚

切回 DNS 前先证明旧节点具有**最新**有效 CRL、新个人 CA/服务器 SAN，以及 WG 变更后的正确后端回程。只回滚到该严格节点，不恢复共享凭据、key-only、旧撤销集合或陈旧 DB。签发连续性不可证时停止签发，保留仍有效的数据面；在途流正常完成，紧急硬断连属于另一项显式运维动作。

关联运维变更须附真实成员/非成员、双设备、原生升级、双节点 CRL/DNS、后端路径及 WG 回滚证据；缺失的生产或原生平台验证继续未完成，不能用隔离测试替代。
