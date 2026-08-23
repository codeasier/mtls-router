# AGENTS.md

本仓库的工作流偏好与架构总览。事实类细节（各包文件映射、导出、不变量、依赖）以 [INDEX.md](INDEX.md) 及各子目录 `INDEX.md` 为权威来源，AGENTS.md 只写边界与「做/不做」。

> 全仓库所有 `AGENTS.md` 与 `INDEX.md` 均以中文撰写；README 等用户可见文档仍需保持中英对齐（见下方「文档偏好」）。

## 导航

AGENTS.md 按独立边界分层，每个子文件回链本文件：

| 范围 | 文件 | 边界依据 | 覆盖 |
|---|---|---|---|
| 仓库根 | 本文件 | — | 架构总览、全局工作流偏好、跨边界不变量 |
| 桌面应用 | [desktop/AGENTS.md](desktop/AGENTS.md) | 独立 npm 包 + Cargo crate + Tauri 打包目标；独立前端/Rust 测试目标 | 前端与 Rust 开发分层、测试、sidecar 构建与整包发布链路 |
| Manager 控制面 | [internal/manager/AGENTS.md](internal/manager/AGENTS.md) | 独立 Go 构建目标（`cmd/mtls-router-manager`）；management protocol v4 所有权；大型子系统（14 个子包） | 协议契约、生命周期、Agent 配置/清理边界与测试焦点 |

各 Go 包的 INDEX 导航见 [INDEX.md#包索引](INDEX.md#包索引)。

## 架构总览

三层结构（详细事实见 [INDEX.md](INDEX.md)）：

| 组件 | 入口 | 职责与边界 |
|---|---|---|
| `mtls-router`（数据面） | 根目录 `main.go` 的 `run()` | 单二进制本地反向代理：明文 HTTP 监听（默认 `127.0.0.1:19099`），用链接期嵌入的 client cert/key/CA 以 mTLS 转发上游；请求体与 SSE 透传不缓冲；`/version`、`/health` 与代理注册在同一 mux（精确 pattern）。只管路由生命周期，不做 Agent 配置 |
| `mtls-router-manager`（控制面） | `cmd/mtls-router-manager/main.go`（唯一命令 `serve`） | stdin/stdout 换行分隔 JSON 服务（management protocol v4，17 个方法）：spawn/监控 router、端口占用诊断、Agent 检测/配置/清理。边界见 [internal/manager/AGENTS.md](internal/manager/AGENTS.md) |
| 桌面应用 | `desktop/src/main.tsx`（React）+ `desktop/src-tauri/src/lib.rs`（Tauri 2） | 以 sidecar 方式拉起 `mtls-router-manager serve` 并经 JSON 协议通信；绝不直接与 router 通信。边界见 [desktop/AGENTS.md](desktop/AGENTS.md) |

数据流：`本地 Agent 客户端 ──HTTP──▶ mtls-router ──mTLS──▶ 上游`；控制流：`桌面 UI / setup 脚本 ──JSON 协议──▶ manager ──spawn/HTTP──▶ router`。

构建、测试与发布边界：

| 边界 | 入口命令 | 说明 |
|---|---|---|
| CLI 双二进制构建 | `./scripts/build.sh` | 在 `secrets/` 生成占位证书，`-ldflags -X` 注入链接期变量（证书、upstream URL、版本元数据、preset、simplify 策略） |
| Go 测试 | `go test ./...` | `*_test.go` 与包同目录；`internal/manager/occupant` 依赖真实 OS 行为，需 `-count=1` 单独跑（CI 在 Linux/macOS/Windows 三平台各自执行） |
| Shell 集成测试 | `make test-shell` | 临时目录中运行 `tests/setup_*_test.sh`，覆盖 setup 脚本事务性安装与命令分组 |
| 文档/工作流断言 | `make test-workflows` | 桌面、agent preset、发布打包与 INDEX 覆盖/链接校验（`tests/index_docs_test.sh` 会校验根 AGENTS.md 的相对链接） |
| 桌面验证/打包 | `make desktop-verify` / `make desktop-package-current` | 透传到 `desktop/` 内 npm 脚本（见 [desktop/AGENTS.md](desktop/AGENTS.md)） |
| 发布 | `.github/workflows/release.yml` | 6 平台 CLI 归档 + 6 平台桌面包；产物命名见下方「发布产物命名」 |

部署产物：`systemd/mtls-router.service`、`Dockerfile`（`scratch` 静态镜像）与裸机二进制三选一；setup 脚本（`setup.sh` / `setup.ps1`）承担 CLI 侧安装/升级事务。

## 本地开发偏好

### Go

```bash
go test ./...                              # 全部 Go 测试
go test ./internal/proxy/...               # 单个包
go test -run TestName ./internal/proxy     # 按名称跑单个测试
go test ./internal/manager/occupant -count=1  # 必跑：CI/release 单列且禁用缓存
go vet ./...
test -z "$(gofmt -l .)"                    # 格式检查（CI 强制）
./scripts/build.sh                         # 本地构建（生成占位证书 + 两个二进制）
```

- `internal/manager/occupant` 依赖真实 OS 行为，`go test ./...` 的缓存结果不足以代表 CI —— 改动该包或其依赖后，务必单独跑一次带 `-count=1` 的版本。

### 代码规范

- Go 文件必须通过 `gofmt` 检查；CI 强制 `test -z "$(gofmt -l .)"`。
- 日志 API 统一用 `slog`；访问日志只记录 method/path/status/bytes/latency，绝不记录请求体。
- 运行时 flag 都有 `MTLS_` 前缀的 env 孪生项；优先级为 `flag > env > build-time > default`。
- `mtls-router` 二进制只做路由生命周期管理；Agent 配置属于 manager 和 setup 脚本。
- Manager 协议错误码是稳定的、可用于分支判断；错误消息仅作诊断用途。
- `.worktrees/` 是 git worktree 产物；分析产品代码时忽略它。

### 架构边界（不可逾越）

- 不要在 `internal/proxy` 中添加 pass-through 请求管线包装器；hook 应在 mux 调用点组合。
- 不要缓冲请求体；让 `httputil.ReverseProxy` 自行流式处理。
- 不要让 `/health` 在 HTTP 层因 upstream 降级而失败。
- `/version` 与 `/health` 必须以精确 pattern 注册在与反向代理同一个 mux 上；ServeMux 按最具体 pattern 匹配（与注册顺序无关），二者因此不会进入代理链路。新增管理端点一律用精确 pattern，不要引入与 `/` 有歧义的前缀 pattern。

### 密钥与 API key 安全

- 不要提交真实密钥；`.gitignore` 已将 `secrets/` 标记为永不提交。
- 不要部分覆盖 `secrets/`；三个文件要么全部存在，要么全部用占位符生成。
- 不要把 API key 放进环境变量、CLI 参数、model config、日志或临时文件。
- 不要在错误 JSON 中泄露 upstream/证书/key 细节；测试会断言脱敏。
- 不要公开暴露 `/version` 或 `/health`；`/version` 含 commit/构建元数据。

### 桌面 webview 沙箱

- 不要授予桌面 webview 超过 `core:default` 的 shell/fs/http/opener 能力。

### 进程与监管行为

- 不要在 service manager（NSSM/systemd/Docker）下传 `-backend`；后台化由监管器负责。

## 本地测试偏好

### 测试构建

- 需要测试构建时，优先选择使用 `mtls-router-github-test-build` skill 在 GitHub Action 中完成，而非本地构建。

### Shell 集成测试

```bash
make test-shell                        # 在临时目录运行所有 tests/setup_*_test.sh
bash tests/setup_clean_test.sh         # 直接跑单个 shell 测试
make test-workflows                    # 桌面 + agent preset + 发布打包 + INDEX 文档一致性
bash tests/index_docs_test.sh          # 仅跑 INDEX 覆盖与链接校验
```

- Go 测试以 `*_test.go` 形式与包同目录存放；shell 集成测试位于 `tests/setup_*_test.sh`。

### 桌面前端（desktop/ 目录）

```bash
cd desktop && npm ci
npm run dev:mock                       # 仅 Vite + 浏览器 mock DesktopApi（无 Go/Cargo/Tauri）
npm run dev:tauri:reuse                # tauri dev，复用已有 sidecar（缺失则 fail closed）
npm run dev:agent                      # 隔离 Agent/桌面数据目录后走 reuse
npm run static:check                   # eslint + prettier
npm run typecheck                      # tsc --noEmit
npm test                               # vitest run
npm run build                          # tsc + vite build
npm run verify                         # 以上全部 + rust 格式 + rust 测试
make desktop-verify                    # 同上，仓库根目录入口
```

- 分层开发命令与边界见 [desktop/AGENTS.md](desktop/AGENTS.md)、[desktop/INDEX.md](desktop/INDEX.md) 与 [docs/BUILD.md](docs/BUILD.md)。
- 不要为加速本地调试而绕过 sidecar 哈希、manager 握手、preview/revision 或事务写入保护。
- `dev:mock` 仅允许 `import.meta.env.DEV && VITE_MOCK=true`；生产构建必须仍绑定真实 Tauri API。

### 桌面 Rust（desktop/src-tauri/）

```bash
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked
```

## 提交偏好

- 提交保持聚焦；不要把无关重构与功能/修复混在一起。
- 暂存时优先按文件名添加，避免 `git add -A` / `git add .`，以防误入密钥或大文件。
- 绝不提交疑似含密钥的文件（`.env`、`credentials.json`、`secrets/` 下的文件）。
- commit message 应体现「为什么」；diff 本身已说明「做了什么」。

## PR 提交偏好

- 提交 PR 前优先确认所关联的 issue；若无 issue 相关背景，先询问用户是否需要先提交 issue。
- 提交 PR 前尽量 squash 同一职责边界的 commit，使 PR 内的 commit 序列清晰、每个 commit 自洽。

## 构建与发布偏好

### 桌面完整打包构建

```bash
make desktop-package-current           # sidecars:build → tauri build → package:verify
```

### 发布产物命名

CLI 产物（6 目标：`linux/darwin/windows` × `amd64/arm64`）：
- 二进制：`mtls-router-${GOOS}-${GOARCH}[.exe]`、`mtls-router-manager-${GOOS}-${GOARCH}[.exe]`（仅 Windows 带 `.exe`）
- 归档：非 Windows 为 `mtls-router-${os}-${arch}.tar.gz`，Windows 为 `mtls-router-${os}-${arch}.zip`（均含 `SHA256SUMS` + 对应平台 setup 脚本 + router + manager）

桌面应用包：
- macOS：`CodeasierRouter-darwin-${arch}.dmg`
- Windows：`CodeasierRouter-windows-${arch}.exe`（NSIS installer）
- Linux：`CodeasierRouter-linux-${arch}.AppImage`

附属：`SHA256SUMS`（覆盖二进制、归档与桌面包，不含 signing-status 文件）+ `signing-status-${os}-${arch}.txt`（6 个）。

桌面 sidecar 构建输入在 `src-tauri/binaries/` 下用 target-triple 命名（`mtls-router-<target>`）；Tauri 打包后安装的二进制使用纯名字（`mtls-router`、`mtls-router-manager`）。
`setup.ps1` 必须保留 UTF-8 BOM 以兼容 Windows PowerShell 5.1；`main_test.go` 会断言这一点。

## 文档偏好

- `INDEX.md` 与 `AGENTS.md`（全仓库，含各子目录）一律以中文撰写。
- README 及其他用户可见文档在变更用户可见行为时，仍需保持英文版与 `docs/zh-CN/` 版本对齐。

### INDEX 层级与同步规则

- **每一个 `internal/` 下的 Go 包都必须有自己的 `INDEX.md`**（中文），含文件映射、关键导出、不变量与依赖。这条覆盖到最深一层，包括 `internal/manager/*` 与 `internal/manager/agent/modelconfig`。
- 导航分两级，新增包时两处都要更新：
  - `internal/` 的顶层包 → 在根 [INDEX.md](INDEX.md) 的 [包索引](INDEX.md#包索引) 追加一行。
  - `internal/manager/` 的子包 → 在 [internal/manager/INDEX.md](internal/manager/INDEX.md) 的子包表追加一行并链到其 `INDEX.md`；根 INDEX 不逐个列出。
- 覆盖范围**不含** `cmd/`、`scripts/`、`tests/`、`systemd/`：这些目录刻意不设专属 INDEX。
- `make test-workflows`（CI 的 "Scope and workflow assertions" job）会校验上述覆盖与所有 `INDEX.md` / 根 `AGENTS.md` 中相对链接的可解析性，漏建或漏登记会直接失败。
- 事实类内容只写在 `INDEX.md`，`AGENTS.md` 只写「做/不做」并链接过去 —— 同一事实写两遍必然分裂。

### AGENTS.md 层级规则

- `AGENTS.md` 只建于独立边界：当前为根（本文件）、[desktop/AGENTS.md](desktop/AGENTS.md)（独立 npm/Cargo/Tauri 打包与测试目标）、[internal/manager/AGENTS.md](internal/manager/AGENTS.md)（独立 Go 构建目标与协议所有权）。新增前先评估能否并入现有层级，最小的正确层级优先；`internal/` 其余包、`cmd/`、`scripts/`、`tests/`、`systemd/` 不设专属 AGENTS.md。
- 每个子 `AGENTS.md` 必须在开头回链根 [AGENTS.md](AGENTS.md)，并把事实细节链接到对应 `INDEX.md`，不复制。
- 根 `AGENTS.md` 的「导航」表是层级索引：新增或移除子 `AGENTS.md` 时必须同步更新该表。
