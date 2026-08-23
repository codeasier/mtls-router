# desktop

Tauri 2 桌面应用（CodeasierRouter）的边界与工作流。上层范围：仓库根 [AGENTS.md](../AGENTS.md)。本目录的完整架构图、逐文件映射与安全约束见 [INDEX.md](INDEX.md)；本文件只写边界定位、构建/测试入口与「做/不做」。

## 边界定位

- 一个目录、多套独立构建目标：npm 包 `mtls-router-desktop`（`package.json`，Node ≥ 22.12、npm 11.6.2）承载 React 19 + TypeScript + Vite 前端；Rust crate（`src-tauri/Cargo.toml`，Tauri 2）承载桌面后端；`go.mod` 声明独立 module `github.com/codeasier/mtls-router/desktop`。Go sidecar 二进制不在本 module 构建：`scripts/build-sidecars.sh` 从仓库根 module 交叉编译 `mtls-router` 与 `mtls-router-manager`，以 target-triple 名输出到 `src-tauri/binaries/`（安装后使用纯名字）。
- 桌面应用是 manager 的宿主，不是 router 的客户端：React UI ──Tauri invoke──▶ Rust `commands.rs` ──stdin/stdout 换行 JSON──▶ `mtls-router-manager serve`（`--desktop-session` + 完整父进程身份 flag 三件套）。桌面端绝不直接与 router 通信；manager 握手时校验 management protocol v4 与 deployment ID。
- CI 按路径分别触发三个 job（`.github/workflows/ci.yml` 的 paths-filter）：frontend（eslint + prettier + tsc + vitest + vite build）、rust（Linux/macOS/Windows 三平台 cargo fmt/test，且先跑根 module 的 `go test ./internal/manager/occupant -count=1` 并先构建 host sidecar）、desktop-package（PR 时抽检 Windows x86_64 NSIS 与 Linux arm64 AppImage 打包）。

## 入口与数据流

- 前端入口 `src/main.tsx` → `src/App.tsx`（根布局、区块导航、启动时一次静默更新检查）；页面为 `RouterPage` / `AgentPage` / `LogsPage` / `SettingsPage` / `ApiKeysPage`。
- Rust 入口 `src-tauri/src/lib.rs`（插件注册、setup：sidecar 校验、manager spawn、调度器、托盘）→ `commands.rs`（全部 `#[tauri::command]` handler）→ `manager.rs`（`ManagerClient` spawn 与通信）。
- 轮询：`scheduler.rs` 周期查询 manager 并向前端 emit `router-poll-snapshot` 事件；`port_recovery.rs` 在 manager 报告端口首次释放后约 10 秒内定期采样，区分 `released` 与 `reoccupied`。
- 前端访问桌面能力的唯一通道是 `src/ipc.ts` 的类型化 `DesktopApi`；敏感文本在客户端脱敏。

## 分层开发

按改动类型选最短反馈循环（完整对照表见 [INDEX.md#分层本地开发](INDEX.md#分层本地开发)）：

- `npm run dev:mock` — 仅 Vite + 浏览器内存 mock `DesktopApi`，不依赖 Go/Cargo；
- `npm run dev:tauri:reuse` — `tauri dev` 复用已有 sidecar，缺失即 fail closed；
- `npm run dev:agent` — 隔离桌面/Agent 数据目录后走 reuse；
- `npm run tauri -- dev` — 先执行 `sidecars:build` 再启动，Go sidecar 变更后必须走这条或先手动重建。

## 测试与验证

- `npm test` — vitest（jsdom），前端单测与组件测试以 `src/*.test.tsx` / `*.test.ts` 与源码同目录存放；
- `npm run rust:test` — `cargo test --manifest-path src-tauri/Cargo.toml --locked`；
- `npm run verify` — 全套：eslint + prettier + tsc + vitest + vite build + cargo fmt + cargo test；仓库根等价入口为 `make desktop-verify`；
- `npm run build` 额外执行 `scripts/verify-production-no-mock.mjs`，确保生产构建不携带 mock 绑定。

## 打包与发布链路

- 本机完整打包：仓库根 `make desktop-package-current` = `npm run sidecars:build` → `npm exec tauri -- build --target <host> --no-sign --ci` → `npm run package:verify`。
- 发布链路：`scripts/prepare-updater-config.sh` 只为精确 stable tag 生成权限受限的 Tauri updater overlay 并校验固定公钥指纹；`scripts/create-macos-updater.sh` / `create-macos-dmg.sh` 产出 macOS 更新包与 DMG；`scripts/verify-package.sh` 从包内 desktop executable 构造一次 Tauri app 验证插件初始化与 manager 握手，并核对六平台 updater 产物与 `.sig` 签名。产物命名规则见根 [AGENTS.md](../AGENTS.md) 的「发布产物命名」。

## 边界规则（做/不做）

- webview 能力上限为 `core:default`，不授予 shell/fs/http/opener 权限（由 `lib.rs` 内测试强制；根 AGENTS.md 同样约束）。
- `dev:mock` 仅在 `import.meta.env.DEV && VITE_MOCK=true` 时可用；生产构建必须绑定真实 Tauri API。
- 不为加速本地调试绕过 sidecar 哈希校验、manager 握手、preview/revision 或事务写入保护。
- cleanup write 属于不可 replay 的 lifecycle 操作：投递结果不确定时不自动重发。
- API key 明文只存在于 Rust 侧 `Zeroizing<String>` 生命周期内；webview 只能保存/删除，不能回读明文。
- 桌面整包更新仅对精确 stable `vX.Y.Z` release 生效，下载必须通过 Tauri updater 签名校验并经用户确认后安装；该链路不改变 CLI router、manager 或 setup 脚本的更新行为。
