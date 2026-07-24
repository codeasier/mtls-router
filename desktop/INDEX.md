# desktop

Tauri 2 desktop application (CodeasierRouter): React frontend + Rust backend that manages mtls-router via the manager sidecar.

## Architecture

```
React UI ──Tauri invoke──▶ Rust commands.rs ──stdin/stdout JSON──▶ mtls-router-manager serve
                                    │
                              scheduler.rs ──poll──▶ manager ──emit──▶ "router-poll-snapshot" event ──▶ React
```

The desktop app never communicates with the router directly. It spawns `mtls-router-manager serve` as a long-lived child process with `--desktop-session`, `--parent-pid/start/executable` flags.

- **Startup failure diagnostics**: post-launch failures terminate and wait for the owned child; lifecycle retains bounded raw output, while the app protocol exposes only sanitized, session-scoped diagnostics.

## Frontend (src/)

| File                                | Role                                                                                                                                       |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `ipc.ts`                            | `DesktopApi` interface + `createDesktopApi()` — typed wrappers for all Tauri commands; `sanitizeSensitiveText()` for client-side redaction |
| `App.tsx`                           | Root layout: sidebar nav (Router/Agents/Logs/Settings) + section rendering                                                                 |
| `RouterPage.tsx`                    | Router status, start/stop, health, occupant inspection/termination                                                                         |
| `AgentPage.tsx`                     | Agent detection, model discovery, config preview/write flow                                                                                |
| `LogsPage.tsx`                      | Bounded, safely filtered router logs with manual refresh                                                                                   |
| `SettingsPage.tsx`                  | Autostart, diagnostics, uninstall prep, language                                                                                           |
| `model.ts`                          | Shared types (`SectionId`, `navigationItems`)                                                                                              |
| `i18n.tsx`                          | I18n context provider with `zh-CN` and `en` locales                                                                                        |
| `locales/zh-CN.ts`, `locales/en.ts` | Translation dictionaries                                                                                                                   |

## Backend (src-tauri/src/)

| File                  | Role                                                                                                                                                                        |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `lib.rs`              | App entry: plugin registration, setup (sidecar validation, manager spawn, scheduler start, tray), invoke handler registration                                               |
| `commands.rs`         | All `#[tauri::command]` handlers; `AppState` (manager client, scheduler, paths, model flows); `ModelFlow` with `Zeroizing<String>` API key                                  |
| `manager.rs`          | `ManagerClient` + `TauriTransportFactory` — spawns manager child, sends JSON requests, reads responses; `validate_handshake()`                                              |
| `scheduler.rs`        | `PollScheduler` — periodic router status/health polling; emits `router-poll-snapshot` events; visibility-aware interval                                                     |
| `sidecar.rs`          | `SidecarPaths::resolve()` — locates `mtls-router[.exe]` and `mtls-router-manager[.exe]` beside the app binary (plain runtime names); validates SHA-256 + native arch/format |
| `tray.rs`             | System tray icon/menu; status-aware labels; window show/hide                                                                                                                |
| `orchestration.rs`    | `first_launch()` — auto-starts router if sidecar is valid and no router is running                                                                                          |
| `model_config.rs`     | Model config import/export JSON validation                                                                                                                                  |
| `paths.rs`            | Desktop data directory resolution (delegates to `MTLS_ROUTER_DESKTOP_DATA_DIR` or OS default)                                                                               |
| `process_identity.rs` | `current()` — captures PID + start time + executable for parent identity flags                                                                                              |
| `autostart.rs`        | Launch-at-login plugin wrapper; default-enabled on first launch                                                                                                             |
| `types.rs`            | Serde types mirroring manager protocol results                                                                                                                              |
| `error.rs`            | `CommandError` — maps manager protocol errors to user-facing strings                                                                                                        |

## Security constraints

- Webview capabilities: only `core:default` — no shell/fs/http/opener permissions (enforced by test in `lib.rs`).
- API keys stored in `Zeroizing<String>` — memory is zeroed on drop.
- CSP: `default-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' asset: http://asset.localhost; style-src 'self' 'unsafe-inline'`.
- Manager handshake validated on startup: version, protocol version, deployment ID.

## Build

```bash
npm run sidecars:build    # builds Go router + manager for host target into src-tauri/binaries/
npm exec tauri -- build   # full Tauri build (requires sidecars present)
```

Sidecar naming: build inputs in `src-tauri/binaries/` use target-triple names (`mtls-router-<target-triple>`, e.g. `mtls-router-aarch64-apple-darwin`); after Tauri packaging the installed binaries use plain names (`mtls-router`, `mtls-router-manager`, with `.exe` on Windows).

## Testing

```bash
npm test                  # vitest (frontend unit tests, jsdom)
npm run rust:test         # cargo test (Rust backend tests)
npm run verify            # full: eslint + prettier + tsc + vitest + vite build + cargo fmt + cargo test
```
