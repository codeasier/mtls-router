# Linux 个人设备命令行客户端

[English](../LINUX-CLI.md) · [领证运维](ENROLLMENT-OPERATIONS.md)

`codeasier-router` 是新的 Linux amd64/arm64 无 GUI Rust 客户端，与桌面共享身份和流式传输核心，不需要 Tauri/WebKit、DISPLAY 或密钥环，也没有 API key/token/private key 参数入口。历史 Go `mtls-router`、manager、setup 继续冻结，旧归档不能替代此客户端。

## 下载与验签

选择已发布版本和架构。每架构只发布 `CodeasierRouter-cli-linux-<arch>.tar.gz` 及 `.tar.gz.sigstore.json` provenance bundle，均纳入 `SHA256SUMS`；producer `.sha256` 只作预检，不对外发布。归档内只有 `codeasier-router`。CLI 使用 GitHub OIDC/Sigstore，桌面继续使用独立的 Tauri updater 签名与五平台 feed。

在下载目录使用较新的 GitHub CLI：

```sh
version=vX.Y.Z
arch=amd64
gh release download "$version" --repo codeasier/mtls-router --pattern "CodeasierRouter-cli-linux-$arch.*" --pattern SHA256SUMS
awk -v name="CodeasierRouter-cli-linux-$arch.tar.gz" '$2 == name || $2 == name ".sigstore.json" { print }' SHA256SUMS >cli-checksums.txt
test "$(wc -l <cli-checksums.txt)" -eq 2
sha256sum -c cli-checksums.txt
sha=$(gh api "repos/codeasier/mtls-router/commits/$version" --jq .sha)
gh attestation verify "CodeasierRouter-cli-linux-$arch.tar.gz" --repo codeasier/mtls-router --signer-workflow codeasier/mtls-router/.github/workflows/release.yml --source-ref "refs/tags/$version" --source-digest "$sha" --bundle "CodeasierRouter-cli-linux-$arch.tar.gz.sigstore.json" --deny-self-hosted-runners
tar -xzf "CodeasierRouter-cli-linux-$arch.tar.gz"
```

未验签不执行，不能用镜像提供的 repo/workflow 身份替换固定身份。把已验证二进制安装为 `~/.local/bin/codeasier-router`。`--version` 只输出公有构建/协议元数据，不需要证书。

## 无需操作证书文件的登录

```sh
codeasier-router login --no-browser
codeasier-router identity status
codeasier-router serve
```

在手机/另一设备打开输出的**公信 HTTPS 链接**，登录 GitCode，核对账号、组织、设备和短码后批准。原 Linux 进程自动生成私钥/CSR 并安装绑定证书，浏览器不传递私钥。不带 `--no-browser` 且存在图形会话时可调用 `xdg-open`；无 GUI 仍输出链接。业务 API key 与 Agent 配置保持独立。

默认状态目录为 `${XDG_STATE_HOME:-$HOME/.local/state}/codeasier-router/`；`--data-dir` / `MTLS_ROUTER_DATA_DIR` 显式改为其他私有目录，`--listen` / `MTLS_ROUTER_LISTEN` 指定 `127.0.0.1:<port>`（默认 19099），flag 优先。登录、状态、退出与服务必须使用**同一用户、同一目录**。桌面只在预期 XDG 状态位置关联 CLI，并校验 OS 身份、deployment 和认证代际，不信任 HTTP 自报任意状态路径。

`--no-browser` 的 env 孪生为 `MTLS_ROUTER_NO_BROWSER=true`，显式 `--no-browser=false` 等 flag 优先，且只作用于 login。

私有 `identity/` 中的原子状态含 key/待办证明，不复制到日志或支持包；独立 `router-state.json` 只含进程/公有协议元数据，仅由本进程清理。

## 续期、恢复与退出

`identity renew` 检查提前 7 天窗口；前台服务自动退避续期、分代切换，在途 SSE 保留。已提交但丢失的续期响应可在完整 24 小时内经固定公信 origin、使用原设备 key 取回，即使原证已过期；恢复端点不能创建新签发操作。保留私有待办到完成，证书到期/授权不可恢复时重新浏览器登录。

`logout` 先停用本地身份，再尝试远端撤销。离线或尚未发布时保留 `revocation_pending` 私有待办，由 `serve` 或再次 logout 重试，不删除 API key、不改 Agent 文件。只有实际网关证据才报确认。

无可用证书仍提供 loopback `/version`、`/health`（health HTTP 200），业务返回封闭证书错误。端口冲突明确失败，不换端口、不终止占用者；`serve` 前台运行并有界处理 SIGTERM。

## 同用户 systemd 与替换

使用[用户 unit](../../deploy/cli/codeasier-router.service)，不换成另一账户的系统服务。unit 显式使用 `~/.local/state/codeasier-router`；自定义 XDG 时同时调整 unit 与交互命令。

```sh
codeasier-router --data-dir "$HOME/.local/state/codeasier-router" login --no-browser
systemctl --user daemon-reload
systemctl --user enable --now codeasier-router.service
```

执行前将 unit 放在 `~/.config/systemd/user/codeasier-router.service`，不传后台参数。更新时停止本用户拥有的服务、验签新归档、替换二进制，再重启同一 unit；保留 `identity/` 和全部现有 Agent/API-key 文件。旧 Go 服务须先明确归属后单独处理，新 CLI 不会终止未知占用者或借用内嵌共享凭据。

工作流配置了无 GUI 软件包的原生 Linux amd64/arm64 检查，但配置不等于实际 runner 已完成；真实跨设备批准、运行中重新登录、升级与服务替换仍需目标节点验收。
