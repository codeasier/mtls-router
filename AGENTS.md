# AGENTS.md

本仓库的工作流偏好说明。项目理解、架构、构建元数据及各包 INDEX 导航见 [INDEX.md](INDEX.md) 及各子目录下的 `INDEX.md`。

> 本文件及全仓库所有 `INDEX.md` 均以中文撰写；README 等用户可见文档仍需保持中英对齐（见下方「文档偏好」）。

## 导航

- 项目概览与架构：[INDEX.md](INDEX.md)
- 各子包详情：见 [INDEX.md#包索引](INDEX.md#包索引) 中的链接
- 本文件仅承载开发生命周期各类偏好。

## 本地开发偏好

### Go

```bash
go test ./...                          # 全部 Go 测试
go test ./internal/proxy/...           # 单个包
go test -run TestName ./internal/proxy # 按名称跑单个测试
go vet ./...
test -z "$(gofmt -l .)"                # 格式检查（CI 强制）
./scripts/build.sh                     # 本地构建（生成占位证书 + 两个二进制）
```

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
- `/version` 和 `/health` 必须先于 `/` 注册，以确保优先于代理路由。

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
make test-workflows                    # 桌面 + agent preset + 发布打包工作流测试
```

- Go 测试以 `*_test.go` 形式与包同目录存放；shell 集成测试位于 `tests/setup_*_test.sh`。

### 桌面前端（desktop/ 目录）

```bash
cd desktop && npm ci
npm run static:check                   # eslint + prettier
npm run typecheck                      # tsc --noEmit
npm test                               # vitest run
npm run build                          # tsc + vite build
npm run verify                         # 以上全部 + rust 格式 + rust 测试
```

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
- 新增子包时，为该包创建专属 `INDEX.md`（中文），并在根 [INDEX.md](INDEX.md) 的 [包索引](INDEX.md#包索引) 中追加一行。
