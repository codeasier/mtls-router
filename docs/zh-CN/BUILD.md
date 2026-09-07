# 构建与发布

[English](../BUILD.md)

本文档面向构建 Tauri 桌面应用并运行冻结 Go 兼容测试套件的维护者。桌面应用把 mTLS router 与管理控制面内嵌在自身进程内；当前仓库中的 CI 和 release workflow 会构建全部六个原生桌面包目标，并在匹配的 runner 上检查每个包。精确 stable `vX.Y.Z` tag release 只发布这些桌面包及其签名 updater 产物。Windows/macOS 签名和 macOS notarization/stapling 取决于完整平台凭据；Tauri updater 签名则是 stable release 的独立强制要求。包检查会执行只覆盖初始化的启动 smoke test，但不会安装、正常启动或更新应用；每个发布包都必须保留独立签名状态和目标 runner 上成功安装/启动/更新的证据。

> **CLI 停止维护。** `v0.4.1` 之后的 release 不包含 `mtls-router` 或 `mtls-router-manager` 二进制、安装脚本、CLI 归档或服务包装；`scripts/package-release.sh` 会拒绝任何此类文件。Go 源码、`scripts/build.sh` 与安装脚本作为冻结的参考实现保留在仓库中：它们的测试证明内嵌 Rust router 与 manager 保持了历史 HTTP、protocol v4 与状态文件契约，`internal/manager/testdata` 下的 release golden 则驱动历史 router 迁移测试。历史 CLI release 仍可从各自 tag 下载且不会被覆盖。

## 工具链和 lockfile

- Go：`go.mod` 要求 Go `1.26.2`。Go 用于冻结的兼容测试（`go test ./...`、禁用缓存的原生 occupant 测试）与历史 `scripts/build.sh`；桌面构建本身不调用 Go。
- Node.js：`desktop/package.json` 要求 Node.js `>=22.12.0` 并声明 `npm@11.6.2`；必须结合 `desktop/package-lock.json` 使用 `npm ci`。
- Rust：桌面 crate 声明 `rust-version = 1.77.2`；使用满足要求的 Rust toolchain，并通过 `--locked` 使用 `desktop/src-tauri/Cargo.lock` 构建。
- Tauri：JavaScript 和 Rust Tauri 依赖在 `desktop/package.json` 和 `desktop/src-tauri/Cargo.toml` 中精确锁定，并由各自 lockfile 解析。release 构建不得传入 `--ignore-version-mismatches`。

平台构建还需要 Tauri 2 的操作系统前置条件：受支持 WebView 和原生打包工具、Rust target 以及标准压缩/校验工具。占位 router 凭据由 Rust 构建脚本生成，因此 `openssl` 不再是桌面构建的前置条件。桌面包应在目标操作系统上生成并启动验证，不能假设交叉编译出的 bundle 一定可安装。

## Go 检查和构建（冻结的 CLI 参考实现）

在仓库根目录运行：

```bash
test -z "$(gofmt -l .)"
go test ./...
go vet ./...
make test-shell
```

构建本地开发用的两个 Go 程序：

```bash
go build -trimpath -o mtls-router .
go build -trimpath -o mtls-router-manager ./cmd/mtls-router-manager
```

仓库构建接受仅供 manager 使用的构建环境变量 `SIMPLIFY`。
`scripts/build.sh`（历史 CLI）和 `desktop/src-tauri/build.rs`（内嵌 manager）
会在调用任何编译器前将其规范化：未设置、空值或 `true` 的任意 ASCII 大小写形式
视为启用；`false` 的任意 ASCII 大小写形式视为关闭。包含空白、数字、非 ASCII
相似字符及其他任何值都会在编译前以 `invalid SIMPLIFY value` 失败。Go 构建中规范值
只会 link 到 `github.com/codeasier/mtls-router/internal/manager/modelcatalog.Simplify`；
桌面构建中则以 `MTLS_SIMPLIFY` 编译进内嵌 manager。router 绝不会收到它。默认启用会从
manager 目录排除包含 ASCII `/` 的有效模型 ID；关闭则保留全部有效 ID。这是不可变的
manager 构建策略，不是运行时设置、配置偏好或 router 选项，也不参与运行时配置优先级。

直接构建 manager 时，可以显式关闭过滤：

```bash
go build -trimpath \
  -ldflags "-X 'github.com/codeasier/mtls-router/internal/manager/modelcatalog.Simplify=False'" \
  -o mtls-router-manager ./cmd/mtls-router-manager
```

直接构建时，省略该 `-X` 赋值即可使用代码默认值 `True`。不得直接 link 空值：
`-X github.com/codeasier/mtls-router/internal/manager/modelcatalog.Simplify=`
会覆盖代码默认值，并使 manager 在 protocol serving 或 Agent transaction recovery
前以 `invalid embedded simplify value` 启动失败。

Manager 可从 `AGENT_MODEL_PRESET_BASE64` 接收一份可选构建期 Agent model preset。该值必须是无 key、包含至少一个 Agent section 的规范 version-1 model-config 文档经过 strict standard Base64 编码后的结果。`scripts/build.sh` 只把它注入历史 manager 二进制中的 `github.com/codeasier/mtls-router/internal/manager/preset.Encoded`；`desktop/src-tauri/build.rs` 则以 `MTLS_AGENT_MODEL_PRESET_BASE64` 编译进内嵌 manager。router 绝不会收到该值。未设置或空值表示无 preset。非空值 malformed 时，Go manager 会启动失败，桌面的内嵌 manager 会在提供任何 Agent 方法前报告 `MANAGER_INIT_FAILED`，且都不会打印编码或解码内容。

`mtls-router-manager` 只有一个命令 `serve`。它从 stdin 逐行读取 JSON 请求，串行处理，只把协议响应写到 stdout，并在 stdin EOF 时正常退出。诊断应写 stderr 或日志。绝不能把 API key 加入 manager 参数或环境变量。

Agent 配置使用 management protocol v4。Release 测试通过仓库 parser 与 snapshot 覆盖验证生成的 Claude JSON、opencode JSON 和 Codex TOML/auth 输出。测试使用的精确 current stable Agent/schema 输入，包括 source URL、revision、digest 和 retrieval date，固定在 [`internal/manager/agent/testdata/compatibility.json`](../../internal/manager/agent/testdata/compatibility.json)。更新 pin 时必须审查上游 schema，按需更新 renderer/schema 测试，并保持中英文 Agent 文档一致。

## 本地占位 router

本地 router 开发运行：

```bash
./scripts/build.sh
```

如果以下文件全部不存在，脚本会生成三个占位文件：

- `secrets/client.pem`
- `secrets/client.key`
- `secrets/upstream-ca.pem`

文件只存在一部分会被拒绝。占位二进制预期会快速启动失败，直到使用真实上游配置和证书材料构建。绝不能发布占位二进制。

## 生产元数据和凭据

Router 不在运行时读取证书。以下仅 router 使用的值通过 linker 注入 `main`：

- `main.clientCertPEM`
- `main.clientKeyPEM`
- `main.upstreamCAPEM`
- `main.upstreamURL`

以下共享元数据变量需要同时注入 router 和 manager：

- `github.com/codeasier/mtls-router/internal/version.Version`
- `github.com/codeasier/mtls-router/internal/version.Commit`
- `github.com/codeasier/mtls-router/internal/version.BuildDate`
- `github.com/codeasier/mtls-router/internal/version.DeploymentID`

`internal/version.ManagementProtocolVersion` 是代码内 protocol ID，当前为 `4`，不是可用 `-X` 注入的 linker 变量。`DeploymentID` 是固定服务环境的非敏感标识。生产构建必须让 router、manager 和 desktop 使用相同的非空、非 `dev`、非 `unknown` deployment ID 和 protocol ID。默认开发身份会有意禁用外部 router 复用。

Router 构建示例：

```bash
go build -trimpath \
  -ldflags "-s -w \
    -X 'main.clientCertPEM=$(cat secrets/client.pem)' \
    -X 'main.clientKeyPEM=$(cat secrets/client.key)' \
    -X 'main.upstreamCAPEM=$(cat secrets/upstream-ca.pem)' \
    -X 'main.upstreamURL=https://router.example.com' \
    -X 'github.com/codeasier/mtls-router/internal/version.Version=v0.2.0' \
    -X 'github.com/codeasier/mtls-router/internal/version.Commit=$(git rev-parse --short=12 HEAD)' \
    -X 'github.com/codeasier/mtls-router/internal/version.BuildDate=$(date -u +%Y-%m-%dT%H:%M:%SZ)' \
    -X 'github.com/codeasier/mtls-router/internal/version.DeploymentID=production-service'" \
  -o mtls-router .
```

内嵌客户端私钥是共享凭据，任何获得历史 router 二进制或桌面包的人都可以提取。只能发布给可信内部用户。轮换需要构建包含新凭据材料的替代 release，并在服务端吊销旧凭据；桌面应用没有运行时凭据导入或组件更新器。

## 桌面检查

在 `desktop/` 中安装 lockfile 固定的 Node 依赖并运行完整检查：

```bash
npm ci
npm run verify
```

不需要任何 sidecar 准备：当 `secrets/` 与凭据环境变量都不存在时，Rust 构建脚本会自动内嵌占位 router 凭据，因此全新 checkout 可以立即构建和测试。

各项精确命令为：

```bash
npm run static:check
npm run typecheck
npm test
npm run build
npm run rust:format
npm run rust:test
```

`npm run rust:test` 展开为 `cargo test --manifest-path src-tauri/Cargo.toml --locked`。未打包 Rust 构建使用 `cargo build --manifest-path src-tauri/Cargo.toml --locked`。

## 内嵌 router 与 manager

桌面可执行文件内含 mTLS router（`desktop/src-tauri/src/router_core`）与管理控制面（`desktop/src-tauri/src/manager_core`）。Tauri `bundle.externalBin` 为空，也不依赖 shell 插件：正常路径绝不拉起 Go 子进程，`desktop/scripts/verify-package.sh` 在包内发现 `mtls-router` 或 `mtls-router-manager` 时会直接失败。

`desktop/src-tauri/build.rs` 提供过去由 Go 链接器注入的全部输入：

| 输入 | 来源 | 编译为 |
|---|---|---|
| 客户端证书、私钥、上游 CA | 仓库根目录 `secrets/` 下的三个文件，**或者** `CLIENT_CERT_PEM`、`CLIENT_KEY_PEM`、`UPSTREAM_CA_PEM` 三个变量；不得同时提供；都不存在时用 `rcgen` 生成有效期一天的占位证书对 | `$OUT_DIR/router-credentials/` 下的文件，由 `runtime.rs` 通过 `include_str!` 内嵌 |
| 上游 URL | `UPSTREAM_URL`，默认 `https://upstream.placeholder.invalid` | `MTLS_UPSTREAM_URL` |
| 版本、deployment ID、协议 | `VERSION`（默认 crate 版本）、`DEPLOYMENT_ID`（默认 `dev`）、`MANAGEMENT_PROTOCOL_VERSION`（必须为 `4`） | `MTLS_MANAGER_VERSION`、`MTLS_DEPLOYMENT_ID`、`MTLS_MANAGEMENT_PROTOCOL_VERSION` |
| commit 与构建时间 | `COMMIT` / `BUILD_DATE`，否则取 `git rev-parse --short HEAD` 与当前 UTC 时间 | `MTLS_ROUTER_COMMIT`、`MTLS_ROUTER_BUILD_DATE` |
| Agent model preset、目录策略 | `AGENT_MODEL_PRESET_BASE64`、`SIMPLIFY` | `MTLS_AGENT_MODEL_PRESET_BASE64`、`MTLS_SIMPLIFY` |

`RELEASE_BUILD=1` 会在版本或 deployment ID 为默认值、凭据将是占位值、或上游 URL 不是 HTTPS 时让构建失败；release workflow 在每个桌面 job 上都设置它。不完整的凭据集合、文件与环境变量混用，以及无效 `SIMPLIFY` 值会让所有构建失败。

manager 以由 Rust target triple 派生的操作系统/架构标签报告其目标：

| Release 目标 | Rust/Tauri target triple | manager 目标标签 |
|---|---|---|
| Windows x86_64 | `x86_64-pc-windows-msvc` | `windows/amd64` |
| Windows arm64 | `aarch64-pc-windows-msvc` | `windows/arm64` |
| macOS Intel | `x86_64-apple-darwin` | `darwin/amd64` |
| macOS Apple Silicon | `aarch64-apple-darwin` | `darwin/arm64` |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `linux/amd64` |
| Linux arm64 | `aarch64-unknown-linux-gnu` | `linux/arm64` |

运行时，桌面在进程内构造内嵌 manager，校验 `manager.info` 报告的 target、版本、deployment ID 与协议和编译值一致，然后才提供桌面命令 API。router 运行在带独立 Tokio runtime 的 supervisor 线程上，具备请求与关闭超时、并发上限和 panic 隔离；router 故障只会降级状态，不会拖垮控制面。router 访问日志写入 `<data-dir>/mtls-router-logs/<日期>/<时间>.log`，只含 method、path、status、bytes 和 latency。

### 分层本地桌面开发

按改动类型选择最短反馈循环。这些命令都不会绕过 manager 握手、凭据隔离、preview/revision 校验或 Agent 事务写入保护。

| 改动类型 | 命令 | 说明 |
| --- | --- | --- |
| 仅 React/UI | `cd desktop && npm run dev:mock` | 只跑 Vite + HMR。通过现有 `App` 边界注入内存 `DesktopApi`。绝不读写真实凭据或 Agent 配置。可选场景：`?mockScenario=success\|protocol-error\|preview-stale\|write-fail`（或 `window.__MTLS_MOCK_SCENARIO__`）。生产构建无法启用 mock（仅 `DEV && VITE_MOCK=true`）。 |
| Rust/Tauri | `cd desktop && npm run dev:tauri:reuse` | 以内嵌 router 与 manager 启动 `tauri dev`，并固定 `VITE_MOCK=false` 与开发身份。无需任何前置准备。 |
| 真实 Agent 链路（隔离路径） | `cd desktop && npm run dev:agent` | 显式覆盖 `MTLS_ROUTER_DESKTOP_DATA_DIR`、`CLAUDE_CONFIG_DIR`、`OPENCODE_CONFIG`、`CODEX_HOME` 到可丢弃根目录（或 `MTLS_ROUTER_DEV_AGENT_ROOT`），再包装 reuse。**不**隔离固定 router 端口 `127.0.0.1:19099`；请避免与日常 router 实例并行。 |
| router 凭据、上游、preset 或策略 | 设置上表中的构建输入，然后 `npm run tauri -- dev` | `secrets/` 文件或上述环境变量变化时 `build.rs` 会重新运行并重新内嵌。 |
| 安装器 / 发布布局 | `make desktop-package-current` | 完整打包路径。 |

带显式开发身份的完整本地启动：

```bash
cd desktop
DEPLOYMENT_ID=dev VERSION=dev MANAGEMENT_PROTOCOL_VERSION=4 npm run tauri -- dev
```

本机 bundle 构建需要显式设置 release 元数据：

```bash
DEPLOYMENT_ID=production-service \
VERSION=v0.2.0 \
MANAGEMENT_PROTOCOL_VERSION=4 \
npm run tauri -- build --target aarch64-apple-darwin
```

根据表格使用 runner 对应 target triple。`npm run tauri -- build` 会运行前端生产构建和 Tauri bundling；当前 `bundle.targets` 为 `all`。初始预期产物是当前用户 Windows 安装器、macOS 应用/DMG 和 Linux AppImage，但仅仅因为 Tauri 生成了文件，并不表示它可以发布。

## 签名和 notarization

Release workflow 实现了有条件的平台签名和状态验证：

- `WINDOWS_CERTIFICATE` 或 `WINDOWS_CERTIFICATE_PASSWORD` 任一不可用时，Windows 包保持未签名。两者都存在时，workflow 会签名桌面可执行文件和 NSIS 安装器，使用 Authenticode 验证两者，并在安装器内含 CLI router 或 manager 可执行文件时失败。
- `APPLE_CERTIFICATE` 或 `APPLE_CERTIFICATE_PASSWORD` 任一不可用时，macOS 包保持未签名。两者都存在时，workflow 会签名应用可执行文件、应用 bundle 和 DMG，然后验证签名。
- 仅当 `APPLE_ID`、`APPLE_PASSWORD` 和 `APPLE_TEAM_ID` 也全部存在时，才会 notarize 并 staple 已签名的 macOS 应用。执行这些步骤时，workflow 会验证 Gatekeeper assessment 和 stapled ticket。
- Linux 未配置包签名。
- 每个桌面目标都会生成 `signing-status-<os>-<arch>.txt`，明确报告 unsigned、signed 或 signed-and-notarized 状态，以及未达到更强状态的原因。
- 绝不能根据 Tauri 构建成功、文件名、CI job 名称或证书变量存在推断状态。

本地包默认未签名，除非经过单独且验证过的签名流程。CI 有意使用 `--no-sign` 做包验证；release workflow 则根据凭据可用性选择签名或未签名分支并记录结果。组织策略要求签名/notarization，而对应状态文件无法证明时，必须阻止生产分发。

### Updater 签名与 channel

Tauri updater 签名与操作系统平台签名保护不同的信任边界，二者不能相互替代：

- Tauri updater 签名证明在线更新产物由已安装应用内嵌公钥所对应的私钥签发。Windows、macOS 和 Linux 都必须先通过该签名校验才能安装更新。
- Windows Authenticode 与 macOS code signing/notarization 为下载和安装的软件建立 publisher/platform 信任。即使 Tauri updater 签名有效，只要分发策略要求，它们仍然是必需的。Linux 当前没有配置平台包签名，但其在线更新产物仍强制要求 Tauri 签名。

只有精确 stable `vX.Y.Z` tag 才会生成在线更新。Validation dispatch、prerelease tag 和其他 ref 都保持 `createUpdaterArtifacts` 关闭，也不会推进 channel。Stable 构建内嵌 endpoint `https://release.codeasier.top/latest.json`，且每次 release 生成的 `latest.json` 中各平台 URL 均指向 `https://release.codeasier.top/mtls-router/<tag>/`。Release 汇总会发布含六个平台的 `latest.json`、每个平台 updater 产物及 `.sig`，同时将 tag 目录镜像至 `downloads.codeasier.top` 并原子推进其 `latest` symlink 作为二级分发点；updater feed 与产物下载均由 `release.codeasier.top` 提供。Windows 和 Linux 直接复用最终 NSIS/AppImage 包作为 updater 产物；macOS 还会发布签名的 `CodeasierRouter-darwin-<arch>.app.tar.gz`。每个 updater 产物、签名和 `latest.json` 都由 `SHA256SUMS` 覆盖。

在可信且不被录屏/记录的 operator 工作站上，从 `desktop/` 目录一次性生成 updater keypair。下面的命令只包含输出路径，密码由交互提示读取，不包含任何 key 或密码值：

```bash
npm exec tauri -- signer generate -w /secure/offline/CodeasierRouter-updater.key
```

不得传入 `--password`，不得把密码或私钥放入环境变量，不得启用 verbose shell trace、录制终端，或把生成的 key material 粘贴到命令、日志、issue 或仓库文件。应分别保护并备份生成的私钥文件及其密码，同时保留生成的 companion public key 用于配置。丢失私钥后将无法发布已安装应用信任的更新。轮换公钥同样必须有显式迁移或手工重装方案，因为现有应用只信任其构建时内嵌的公钥。

在仓库 GitHub **Settings > Secrets and variables > Actions** 界面中，通过受保护 operator 流程创建以下 repository Secrets，只把值粘贴到 secret value 输入框：

- `TAURI_SIGNING_PRIVATE_KEY`：生成的完整私钥内容。
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`：生成时输入的密码。
- `TAURI_UPDATER_PUBKEY`：生成的完整 companion public-key 内容。公钥本身不保密，但本 workflow 有意从受保护 Secrets 界面读取它，stable release preflight 也要求该值存在。

还需创建 repository variable `TAURI_UPDATER_PUBKEY_SHA256`，其值为 `node ./scripts/updater-public-key-fingerprint.mjs /secure/offline/CodeasierRouter-updater.key.pub` 输出的 canonical 公钥指纹。该值不保密，用于在构建前阻止意外替换公钥；修改它属于显式 key rotation，仍需执行上文的迁移方案。

这三个 Secret 不得使用命令行 secret setter 配置：命令参数、shell interpolation、重定向的临时文件、debug 输出和被收集的 CI 日志都不是获批 secret transport。任一 updater key 输入、固定公钥指纹或 HTTPS endpoint 缺失或不一致时，stable release 会在打包前失败。每个原生 package check 还会在上传 artifact 前，用内嵌公钥验证生成的 updater 签名；updater 配置只写入 runner 私有临时文件，绝不能打印 key 内容。

## 包验证

两个 workflow 都会在原生匹配 runner 上对六个包逐一调用 `desktop/scripts/verify-package.sh`。该脚本会拒绝 host/target 不匹配；解包 NSIS、DMG 或 AppImage；检查包/版本身份；在包内存在 `mtls-router` 或 `mtls-router-manager` 可执行文件时失败；检查桌面可执行文件的格式及架构；检查 macOS/Linux 可执行权限；从包内 desktop executable 构造 Tauri 应用以初始化已注册插件但不进入事件循环；并执行内嵌 manager 握手，要求编译时的版本、deployment ID、protocol 与目标一致。无图形环境的 Linux 检查会在 Xvfb 下执行初始化 smoke test。Release workflow 还会在发布前验证每个生成的 `.sha256`。

这些自动包检查不会安装包，也不覆盖正常 GUI 启动、setup hook、事件循环、首次启动行为或 updater 网络路径。发布前，必须保留 workflow 检查输出，并从每个匹配目标 runner 保留完整 release checklist 的独立证据：

1. 确认包和可执行文件架构与目标一致。
2. 检查包内容，确保只有一个桌面可执行文件、没有任何 CLI router/manager 二进制，且不存在原始 PEM/key 文件。
3. 确认 macOS/Linux 执行权限，并验证无需提权的当前用户安装/启动。
4. 确认包在 `SHA256SUMS` 中的条目及其 `.sha256` 文件与下载得到的字节一致。
5. 通过桌面运行 `manager.info`，并向运行中的内嵌 router 请求 `/version`；要求 desktop、内嵌 manager、内嵌 router 和 release artifact metadata 的版本、非默认 deployment ID 及 management protocol `4` 一致。在任何 key-bearing Agent 请求前拒绝全部 protocol 混合组合。
6. 使用平台原生工具验证 Windows 签名，或 macOS code signature、notarization 和 stapling；状态缺失时必须明确记录。
7. 安装并启动，验证首次启动、第二实例激活、内嵌 manager 失败行为、托盘/关闭/退出、默认 autostart、外部复用、从 `v0.4.1` 桌面或 CLI 安装迁移历史 router、未知端口冲突、Agent 预览/写入/回滚、日志以及卸载准备/清理。
8. 确认 Windows 卸载移除当前用户 autostart。确认 macOS/Linux **准备卸载**在删除前移除 autostart 并退出。
9. 确认卸载不删除或重写 Agent 文件、敏感备份、日志或状态。
10. 扫描源码、日志、诊断、router 之外的包内容和发布校验文件，排除意外 API key 或凭据文件。
11. 在真实 Windows x86_64/arm64、macOS Intel/Apple Silicon 和 Linux x86_64/arm64 目标上，把上一 stable 包安装到受支持且可写的位置，再通过受控真实 feed 更新到候选版本。确认启动与手动检查、显式确认、签名校验、下载/安装/重启、候选 desktop/manager/router 版本、router 所有权行为，以及故意放在不受支持或不可写安装位置时的恢复。Mock UI、包检查和全新安装候选包都不能代替该上一版本到下一版本测试。

任何目标缺少包检查、签名状态、成功安装/启动证据或上一版本到下一版本的真实平台更新证据时，都不能推进 stable 更新 channel。Workflow 配置、已上传 artifact、本地 Tauri 构建、mock updater 行为和包检查本身都不属于运行时证据。

## 原生端口恢复验收

CI 和 release target runner 会在 Windows、macOS、Linux 上原生执行 `go test ./internal/manager/occupant -count=1`。Windows helper 是测试进程在相同账号、相同完整性级别下创建的子进程；它只能证明同权限原生检查、无副作用的终止权限预检、精确终止 helper 和释放端口。它不能证明跨权限、其他用户、Service、PPL、root、受限 procfs、systemd 或 launchd 行为。CI 不得为了这些测试提升权限或创建平台服务。

发布前必须从受控、可销毁的主机保留下列人工证据。记录已抹除 confirmation token 的 inspection JSON、页面显示的操作，以及最终状态或观察结果。阻断 inspection 必须同时省略 `confirmation_token` 和 `expires_at`；可强制终止的 inspection 必须同时包含两者。

### Windows 人工矩阵

| 场景 | 受控设置 | 必需证据 |
| --- | --- | --- |
| 同账号高完整性 | 用桌面用户账号提升权限后启动进程并占用端口；桌面应用保持普通权限。 | `manual_stop_required` / `insufficient_privilege`，无 token 或强制操作；从匹配的高完整性上下文停止后端口释放。 |
| 其他用户 | 在交互会话中用第二个本地账号占用端口。 | `manual_stop_required` / `different_user`，无 token，不降级为 PID-only，且不泄露进程元数据。 |
| 单个 Service | 运行一个可销毁的 Windows Service，并由其 service process 持有 listener。 | `manual_stop_required` / `service_managed`、`windows_service` / `system`、精确 service name、无 token，并显示经过安全引用且仅供管理员 PowerShell 人工使用的 `sc.exe` 命令；不能用于 `cmd.exe`。 |
| 共享 service host | 让两个可销毁的 Service 共享持有 listener 的同一进程。 | 全部 service name 排序后显示；不提供 token 或按进程强制终止，指引停止 Service 而不是共享 host PID。 |
| SID 或进程身份不可读 | 使用获批的可销毁同 session fixture，在保留精确唯一 listener PID 和 terminate access 的同时隐藏 SID 或完整进程身份。 | 只有终止权限预检成功后才返回带 token 和 expiry 的 `windows_pid_only`；已知不同 SID 仍必须返回 `manual_stop_required` / `different_user`。成功只证明终止请求成功且原 listener PID 已从端口消失，不独立证明进程完全退出。 |
| SCM 自动恢复 | 为可销毁的 listener Service 配置延迟 SCM recovery，再从有权限的外部测试终端终止它。 | 终止前 Service 已被阻断；SCM 重启后冲突再次出现，并保持同一 Service 诊断。由于应用正确地没有签发 force token，不得把此外部触发的重启记作应用 release observation 证据。 |
| PPL 或 System | 使用获批的可销毁 protected-process fixture，或 System 持有的 listener，且不得削弱主机保护。 | 不提供 token 或强制操作。终止权限预检被拒绝时接受 `insufficient_privilege`；其他身份/保护边界阻断恢复时接受 `identity_unavailable` 或 `protected_process`。记录实际产生结果的权限边界。 |

### Linux 人工矩阵

| 场景 | 受控设置 | 必需证据 |
| --- | --- | --- |
| 用户 service | 从可销毁的 `systemd --user` `.service` 占用端口。 | `manual_stop_required` / `service_managed`、`systemd_user` / `user`、精确 unit name、无 token，并显示 user service 停止引导。 |
| 系统 service | 从可销毁的系统 `.service` 占用端口。 | `manual_stop_required` / `service_managed`、`systemd_system` / `system`、精确 unit name、无 token，并显示 system service 停止引导。 |
| `Restart=on-failure` | 为可销毁 unit 添加延迟 restart policy，并从有权限的外部终端终止它。 | kill 前 unit 已被阻断，restart 后冲突以同一 unit 诊断返回。由于已分类 unit 绝不可强制终止，此外部 kill 不是应用 release observation 证据。 |
| root owner | 从不属于托管 router 的普通 root 进程占用端口。 | `manual_stop_required` / `different_user`，无 token 或强制操作。 |
| 受限 procfs | 在可销毁环境中 mount 或配置 procfs，使桌面用户无法读取 listener 身份文件。 | `unavailable` / `identity_unavailable`，无 token，且不把部分身份当作可强制终止目标。 |
| 自定义 slice | 把系统 unit 放在合法的自定义 `.slice` 祖先下，例如 `codeasier.slice/router.slice`。 | 精确 `.service` 仍归类为 `systemd_system`；合法自定义 slice 祖先不会隐藏 supervisor。 |
| delegated service cgroup | 在保留合法 delegated descendants 的情况下，把子进程放到所属 `.service` cgroup 之下。 | 仍能按正确 user/system scope 识别所属 unit；delegated child 不会变成普通可强制终止进程。 |

### macOS 人工矩阵

| 场景 | 受控设置 | 必需证据 |
| --- | --- | --- |
| 当前用户 | 从无 supervisor 的普通当前用户进程占用端口。 | `force_terminate`，token 与 expiry 同时存在，已验证原进程身份不存在且端口首次释放；约 10 秒采样检查未检测到重新占用后为 `released`。 |
| 其他用户 | 从第二个本地账号占用端口。 | `manual_stop_required` / `different_user`，无 token 或强制操作，且不泄露进程元数据。 |
| launchd `KeepAlive` | 从配置了延迟 restart 的可销毁当前用户 launch agent 占用端口。 | 不猜测 launchd label，普通同用户进程可强制终止；确认进程身份不存在并首次释放后，采样检查发现 replacement PID 时在观察窗口内产生 `reoccupied`。 |

## Release workflow

当前 `.github/workflows/release.yml` 由六个原生 runner 构建并检查 Windows x86_64/arm64 NSIS 安装器、macOS Intel/Apple Silicon DMG，以及 Linux x86_64/arm64 AppImage；不再有 CLI 构建 job。手工 dispatch 只用于验证，可以选择一个桌面目标及可选 HTTPS upstream override；它不会生成 updater 产物。精确 stable 版本 tag 始终忽略验证 override，等待全部六个桌面 job，验证六个包 checksum 和签名 updater pair，汇总 `SHA256SUMS` 与 `latest.json`，发布并镜像桌面 asset 和六个签名状态文件，再原子推进 `latest` updater channel。`scripts/package-release.sh` 强制执行 allowlist：只允许发布 `CodeasierRouter-<os>-<arch>` 包、其 `.sha256` 与 updater `.sig` 文件、macOS `.app.tar.gz` updater 归档、`signing-status-*.txt`、`SHA256SUMS` 和 `latest.json`；任何 `mtls-router*` 二进制、安装脚本、CLI 归档或服务包装都会让打包失败。历史 release 绝不会被覆盖。

生产桌面构建需要 repository secrets `CLIENT_CERT_PEM`、`CLIENT_KEY_PEM`、`UPSTREAM_CA_PEM`，以及 variables `UPSTREAM_URL` 和非默认 `DEPLOYMENT_ID`；`build.rs` 在 `RELEASE_BUILD=1` 保护下将它们内嵌。Stable 桌面 updater 发布还要求 `TAURI_SIGNING_PRIVATE_KEY`、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 和 `TAURI_UPDATER_PUBKEY`，以及固定 repository variable `TAURI_UPDATER_PUBKEY_SHA256`。可选 repository variable `AGENT_MODEL_PRESET_BASE64` 会提供给每个桌面包的内嵌 manager；空值有效并表示无 preset。Release preflight 会在 matrix build 前通过冻结的 Go manager loader 校验已配置的值且不打印其内容，桌面自身的 loader 在启动时执行同样严格的校验。可选 repository variable `SIMPLIFY` 遵循上述规范化规则，未设置或为空时默认启用；它会在 matrix fan-out 前规范化，并以同一个规范值编译进每个桌面包。Router 绝不会收到这两个仅供 manager 使用的值。可选平台凭据会选择上文所述的签名/notarization release 分支；与这些可选凭据不同，精确 stable tag 强制要求全部 updater-key 输入存在。

每个桌面 matrix producer 都会生成 code-owned protocol metadata。`scripts/package-release.sh` 在组装 release 前要求每个桌面 producer 恰好一个 metadata 文件、全部文件声明 schema `1` 与 management protocol `4`，且每个 producer 名称都以 `desktop-` 开头。正常发布和恢复发布共用此 preflight，因此有效但 protocol 混合、或包含 CLI producer 的 artifact set 无法发布。

使用 `gh` 设置 release 输入：

```bash
gh secret set CLIENT_CERT_PEM --repo codeasier/mtls-router < secrets/client.pem
gh secret set CLIENT_KEY_PEM --repo codeasier/mtls-router < secrets/client.key
gh secret set UPSTREAM_CA_PEM --repo codeasier/mtls-router < secrets/upstream-ca.pem
gh variable set UPSTREAM_URL --repo codeasier/mtls-router --body "https://router.example.com"
gh variable set DEPLOYMENT_ID --repo codeasier/mtls-router --body "production-service"
gh variable set SIMPLIFY --repo codeasier/mtls-router --body "False"
```

省略或清空 `SIMPLIFY` repository variable 即使用默认 `True` 策略。它不是用户偏好，manager 构建完成后不能更改。

只有在从已审查的无 key 规范文档生成 strict standard Base64 后，才能通过受保护的 repository-variable 流程设置 `AGENT_MODEL_PRESET_BASE64`。Preset 中不得放入 API key、URL、provider identity、header、目录响应或任意 Agent setting。

可选 Windows 签名和 Apple 签名/notarization secrets 只能通过仓库受保护的 secret 管理流程配置。不要把凭据值写入文档、会保留在 shell history 中的命令或仓库文件。

使用 `gh` 执行仅验证的 Windows amd64 构建：

```bash
gh workflow run release.yml \
  --repo codeasier/mtls-router \
  --ref main \
  -f version=0.2.0-windows-test.1 \
  -f target=windows-amd64 \
  -f upstream_url=https://router.example.com
```

所选目标会生成 `CodeasierRouter-desktop-windows-amd64`。省略 `upstream_url` 时使用仓库 `UPSTREAM_URL`；省略 `target` 时使用 `all`。Workflow input 在 GitHub Actions 元数据中可见，因此 override 不得包含凭据、token 或敏感 query parameter，并且必须兼容 repository Secrets 中的客户端证书和 upstream CA。

审查所需目标平台启动证据后，通过推送版本 tag 发布桌面 release：

```bash
git tag v0.2.0
git push origin v0.2.0
```
