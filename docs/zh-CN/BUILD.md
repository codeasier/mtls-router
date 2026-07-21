# 构建与发布

[English](../BUILD.md)

本文档面向构建 router、Go manager 或 Tauri 桌面应用的维护者。当前仓库中的 CI 和 release workflow 会构建全部六个原生桌面包目标，并在匹配的 runner 上检查每个包。Tag 触发的 release 会把这些桌面包与 CLI router/manager 二进制及压缩包一起发布。Windows/macOS 签名和 macOS notarization/stapling 取决于完整凭据，而且包检查不会安装或启动应用；每个发布包都必须保留独立签名状态和目标 runner 上成功安装/启动的证据。

## 工具链和 lockfile

- Go：`go.mod` 要求 Go `1.26.2`。
- Node.js：`desktop/package.json` 要求 Node.js `>=22.12.0` 并声明 `npm@11.6.2`；必须结合 `desktop/package-lock.json` 使用 `npm ci`。
- Rust：桌面 crate 声明 `rust-version = 1.77.2`；使用满足要求的 Rust toolchain，并通过 `--locked` 使用 `desktop/src-tauri/Cargo.lock` 构建。
- Tauri：JavaScript 和 Rust Tauri 依赖在 `desktop/package.json` 和 `desktop/src-tauri/Cargo.toml` 中精确锁定，并由各自 lockfile 解析。release 构建不得传入 `--ignore-version-mismatches`。

平台构建还需要 Tauri 2 的操作系统前置条件：受支持 WebView 和原生打包工具、Rust target、Go 交叉编译支持、`openssl` 以及标准压缩/校验工具。桌面包应在目标操作系统上生成并启动验证，不能假设交叉编译出的 bundle 一定可安装。

## Go 检查和构建

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

仓库构建脚本接受仅供 manager 使用的构建环境变量 `SIMPLIFY`。
`scripts/build.sh` 和 `desktop/scripts/build-sidecars.sh` 会在调用任何编译器前
将其规范化：未设置、空值或 `true` 的任意 ASCII 大小写形式变为 `True`；
`false` 的任意 ASCII 大小写形式变为 `False`。包含空白、数字、非 ASCII
相似字符及其他任何值都会在编译前以 `invalid SIMPLIFY value` 失败。规范值只会
link 到 `github.com/codeasier/mtls-router/internal/manager/modelcatalog.Simplify`；
router 二进制绝不会收到它。默认值 `True` 会从 manager 目录排除包含 ASCII `/`
的有效模型 ID；`False` 保留全部有效 ID。这是不可变的 manager 构建策略，
不是运行时设置、配置偏好或 router 选项，也不参与运行时配置优先级。

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

Manager 可从 `AGENT_MODEL_PRESET_BASE64` 接收一份可选构建期 Agent model preset。该值必须是无 key、包含至少一个 Agent section 的规范 version-1 model-config 文档经过 strict standard Base64 编码后的结果。`scripts/build.sh` 和 `desktop/scripts/build-sidecars.sh` 只把它注入 manager 二进制中的 `github.com/codeasier/mtls-router/internal/manager/preset.Encoded`；router 二进制绝不会收到该值。未设置或空值表示无 preset。非空值 malformed 时，manager 会在 protocol serving 或 Agent transaction recovery 前启动失败，且不会打印编码或解码内容。

`mtls-router-manager` 只有一个命令 `serve`。它从 stdin 逐行读取 JSON 请求，串行处理，只把协议响应写到 stdout，并在 stdin EOF 时正常退出。诊断应写 stderr 或日志。绝不能把 API key 加入 manager 参数或环境变量。

Agent 配置使用 management protocol v2。Release 测试通过仓库 parser 与 snapshot 覆盖验证生成的 Claude JSON、opencode JSON 和 Codex TOML/auth 输出。测试使用的精确 current stable Agent/schema 输入，包括 source URL、revision、digest 和 retrieval date，固定在 [`internal/manager/agent/testdata/compatibility.json`](../../internal/manager/agent/testdata/compatibility.json)。更新 pin 时必须审查上游 schema，按需更新 renderer/schema 测试，并保持中英文 Agent 文档一致。

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

`internal/version.ManagementProtocolVersion` 是代码内 protocol ID，当前为 `2`，不是可用 `-X` 注入的 linker 变量。`DeploymentID` 是固定服务环境的非敏感标识。生产构建必须让 router、manager 和 desktop 使用相同的非空、非 `dev`、非 `unknown` deployment ID 和 protocol ID。默认开发身份会有意禁用外部 router 复用。

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

内嵌客户端私钥是共享凭据，任何获得 router 二进制或桌面包的人都可以提取。只能发布给可信内部用户。轮换需要构建包含新凭据材料的替代 release，并在服务端吊销旧凭据；桌面应用没有运行时凭据导入或 sidecar 更新器。

## 桌面检查

在 `desktop/` 中安装 lockfile 固定的 Node 依赖并运行完整检查：

```bash
npm ci
npm run sidecars:build
npm run verify
```

即使运行测试，Rust 构建脚本也需要 native sidecar，因此全新 checkout 中必须在 `verify` 前先构建 sidecar。除非通过 `TAURI_ENV_TARGET_TRIPLE` 或 `TARGET` 选择下文支持的 target triple，否则 sidecar 命令使用当前 Rust host target。

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

## 桌面 sidecar

Tauri `bundle.externalBin` 包含 `binaries/mtls-router-manager` 和 `binaries/mtls-router`。Tauri 运行前，`npm run tauri` 会调用 `desktop/scripts/build-sidecars.sh`。脚本把 Tauri/Rust target triple 映射为 Go 操作系统/架构，构建两个 sidecar，并输出 Tauri 要求的文件名：

```text
src-tauri/binaries/mtls-router-manager-<target-triple>[.exe]
src-tauri/binaries/mtls-router-<target-triple>[.exe]
```

支持以下映射：

| Release 目标 | Rust/Tauri target triple | Go 目标 |
|---|---|---|
| Windows x86_64 | `x86_64-pc-windows-msvc` | `windows/amd64` |
| Windows arm64 | `aarch64-pc-windows-msvc` | `windows/arm64` |
| macOS Intel | `x86_64-apple-darwin` | `darwin/amd64` |
| macOS Apple Silicon | `aarch64-apple-darwin` | `darwin/arm64` |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `linux/amd64` |
| Linux arm64 | `aarch64-unknown-linux-gnu` | `linux/arm64` |

`secrets/` 中三个真实文件全部存在时，sidecar 脚本会使用它们；全部不存在时则生成临时占位文件。因此生产包必须在没有明确提供全部真实凭据输入时 preflight 失败。Rust 构建脚本会拒绝缺失、非 native、格式错误、架构错误或不可执行的 sidecar，并内嵌每个 sidecar 的 SHA-256。运行时启动会再次按哈希校验文件，并执行 manager target/version/deployment/protocol handshake。

`AGENT_MODEL_PRESET_BASE64` 只会转发给打包的 manager sidecar，不会注入 router sidecar，也不是桌面运行时设置。规范化后的 `SIMPLIFY` 同样只会转发给打包的 manager sidecar；release 构建会让 standalone 和 desktop manager 使用同一个值。

本机开发启动：

```bash
DEPLOYMENT_ID=dev VERSION=dev MANAGEMENT_PROTOCOL_VERSION=2 npm run tauri -- dev
```

本机 bundle 构建需要显式设置 release 元数据：

```bash
DEPLOYMENT_ID=production-service \
VERSION=v0.2.0 \
MANAGEMENT_PROTOCOL_VERSION=2 \
npm run tauri -- build --target aarch64-apple-darwin
```

根据表格使用 runner 对应 target triple。`npm run tauri -- build` 会运行前端生产构建和 Tauri bundling；当前 `bundle.targets` 为 `all`。初始预期产物是当前用户 Windows 安装器、macOS 应用/DMG 和 Linux AppImage，但仅仅因为 Tauri 生成了文件，并不表示它可以发布。

## 签名和 notarization

Release workflow 实现了有条件的平台签名和状态验证：

- `WINDOWS_CERTIFICATE` 或 `WINDOWS_CERTIFICATE_PASSWORD` 任一不可用时，Windows 包保持未签名。两者都存在时，workflow 会签名两个 sidecar、桌面可执行文件和 NSIS 安装器，然后使用 Authenticode 验证安装器和包内三个可执行文件。
- `APPLE_CERTIFICATE` 或 `APPLE_CERTIFICATE_PASSWORD` 任一不可用时，macOS 包保持未签名。两者都存在时，workflow 会签名 sidecar、应用可执行文件、应用 bundle 和 DMG，然后验证签名。
- 仅当 `APPLE_ID`、`APPLE_PASSWORD` 和 `APPLE_TEAM_ID` 也全部存在时，才会 notarize 并 staple 已签名的 macOS 应用。执行这些步骤时，workflow 会验证 Gatekeeper assessment 和 stapled ticket。
- Linux 未配置包签名。
- 每个桌面目标都会生成 `signing-status-<os>-<arch>.txt`，明确报告 unsigned、signed 或 signed-and-notarized 状态，以及未达到更强状态的原因。
- 绝不能根据 Tauri 构建成功、文件名、CI job 名称或证书变量存在推断状态。

本地包默认未签名，除非经过单独且验证过的签名流程。CI 有意使用 `--no-sign` 做包验证；release workflow 则根据凭据可用性选择签名或未签名分支并记录结果。组织策略要求签名/notarization，而对应状态文件无法证明时，必须阻止生产分发。

## 包验证

两个 workflow 都会在原生匹配 runner 上对六个包逐一调用 `desktop/scripts/verify-package.sh`。该脚本会拒绝 host/target 不匹配；解包 NSIS、DMG 或 AppImage；检查包/版本身份；检查 desktop、manager 和 router 的格式及架构；比较打包 sidecar 与本 job 构建 sidecar 的哈希；检查 macOS/Linux 可执行权限；并验证 manager 版本、目标、deployment ID 和 protocol。Release workflow 还会在发布前验证每个生成的 `.sha256`。

这些自动包检查不会安装或启动打包应用。发布前，必须保留 workflow 检查输出，并从每个匹配目标 runner 保留完整 release checklist 的独立证据：

1. 确认包和可执行文件架构与目标一致。
2. 检查包内容，确保只有一组架构兼容的 manager/router sidecar，且不存在原始 PEM/key 文件。
3. 确认 macOS/Linux 执行权限，并验证无需提权的当前用户安装/启动。
4. 重新计算打包 sidecar SHA-256，并与桌面构建内嵌值比较。
5. 运行 `manager.info` 和 router `/version`；要求 desktop、manager、router、setup metadata 和 release artifact metadata 的版本、非默认 deployment ID 及 management protocol `2` 一致。在任何 key-bearing Agent 请求前拒绝全部混合 v1/v2 组合。
6. 使用平台原生工具验证 Windows 签名，或 macOS code signature、notarization 和 stapling；状态缺失时必须明确记录。
7. 安装并启动，验证首次启动、第二实例激活、sidecar 失败、托盘/关闭/退出、默认 autostart、外部复用、未知端口冲突、Agent 预览/写入/回滚、日志以及卸载准备/清理。
8. 确认 Windows 卸载移除当前用户 autostart。确认 macOS/Linux **准备卸载**在删除前移除 autostart 并退出。
9. 确认卸载不删除或重写 Agent 文件、敏感备份、日志或状态。
10. 扫描源码、日志、诊断、router 之外的包内容和发布校验文件，排除意外 API key 或凭据文件。

任何目标缺少包检查、签名状态和成功安装/启动证据时都不能发布。Workflow 配置、已上传 artifact、本地 Tauri 构建和包检查本身都不属于启动证据。

## Release workflow

当前 `.github/workflows/release.yml` 为六个 Go 目标构建 router 和 manager，并创建六个平台压缩包；每个包包含精确 router/manager 二进制对和安装脚本。同时，六个原生 runner 会构建并检查 Windows x86_64/arm64 NSIS 安装器、macOS Intel/Apple Silicon DMG，以及 Linux x86_64/arm64 AppImage。手工 dispatch 只用于验证，可以选择一组配套 CLI/desktop 目标及可选 HTTPS upstream override。版本 tag 始终忽略验证 override，等待全部 12 个构建 job，验证六个桌面包 checksum，汇总一个 `SHA256SUMS`，然后发布并镜像 CLI、桌面 asset 和六个签名状态文件。

生产 CLI 和桌面 sidecar 需要 repository secrets `CLIENT_CERT_PEM`、`CLIENT_KEY_PEM`、`UPSTREAM_CA_PEM`，以及 variables `UPSTREAM_URL` 和非默认 `DEPLOYMENT_ID`。可选 repository variable `AGENT_MODEL_PRESET_BASE64` 会向每个 standalone manager 和 desktop manager sidecar 提供相同 preset；空值有效并表示无 preset。Release preflight 会在 matrix build 前通过 manager loader 校验已配置的值，且不打印其内容。可选 repository variable `SIMPLIFY` 遵循上述规范化规则，未设置或为空时默认为 `True`。它会在 matrix fan-out 前规范化，并以同一个规范值传给所有 standalone 和 desktop manager；desktop 构建脚本可以再次执行幂等校验和规范化。Router build 绝不会收到这两个仅供 manager 使用的值。可选平台凭据会选择上文所述的签名/notarization release 分支。

每个 CLI 和 desktop matrix producer 都会生成 code-owned protocol metadata。`scripts/package-release.sh` 在组装 archive 前要求每个 producer 恰好一个 metadata 文件，并要求全部文件声明 schema `1` 与 management protocol `2`。正常发布和恢复发布共用此 preflight，因此有效但混合 v1/v2 的 artifact set 无法发布。

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

所选目标会同时生成 `mtls-router-cli-windows-amd64` 和 `mtls-router-desktop-windows-amd64`。省略 `upstream_url` 时使用仓库 `UPSTREAM_URL`；省略 `target` 时使用 `all`。Workflow input 在 GitHub Actions 元数据中可见，因此 override 不得包含凭据、token 或敏感 query parameter，并且必须兼容 repository Secrets 中的客户端证书和 upstream CA。

审查所需目标平台启动证据后，通过推送版本 tag 发布 CLI 和桌面 release：

```bash
git tag v0.2.0
git push origin v0.2.0
```
