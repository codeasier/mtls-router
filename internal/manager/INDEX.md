# internal/manager

Control plane for mtls-router: router lifecycle management, Agent configuration, and port conflict resolution. Communicates via newline-delimited JSON over stdin/stdout (protocol v3).

## Sub-packages

| Package | Role | Key exports |
|---------|------|-------------|
| `app` | Protocol session wiring; maps all 15 methods to services; enforces API key zeroing | `App`, `New(Config, simplify)`, `Serve(ctx, input, output)` |
| `protocol` | JSON request/response types, method constants, error codes, per-method deadlines | `Request`, `Response`, `Error`, `Method*`, `Code*`, `Deadlines()` |
| `lifecycle` | Router process spawn/stop/reclaim; desktop foreground + CLI detached modes; parent monitoring; unexpected exit detection | `Manager`, `Start(ctx, owner)`, `Stop(ctx)`, `Reclaim()`, `MonitorParent(ctx)` |
| `discovery` | Classifies router state by correlating HTTP `/version` + `/health` with durable state files and OS process identity | `Discoverer`, `Discover(ctx)`, `DiscoverStatus(ctx)`, `Classification` constants |
| `agent` | Agent detection, config rendering (Claude JSON / opencode JSON / Codex TOML + JSON auth), transactional write with backup/rollback, model discovery | `Service`, `Detect()`, `Render()`, `Write()`, `PreviewRequest()`, `DiscoverModels()` |
| `agent/modelconfig` | Canonical key-free model config schema v1: decode, merge, canonical serialization, token signing | `Config`, `Version`, `Decode()`, `DecodeStructural()`, `Canonical()`, `DeepMerge()`, `MaxConfigSize` |
| `trustedrouter` | Authenticated model catalog discovery via router `/v1/models`; binding revalidation before write | `Coordinator`, `Fetch(ctx, owner, apiKey)`, `Revalidate(ctx, owner, apiKey, binding)` |
| `occupant` | Port occupant identity inspection (Linux `/proc`, macOS `SYS_PROC_INFO`, Windows `GetExtendedTcpTable`) and guarded force-termination with single-use confirmation token | `Service`, `Inspect(ctx)`, `ForceTerminate(ctx, token)` |
| `state` | Atomic JSON state file read/write for router process identity; file locking | `RouterState`, `Read(path)`, `Write(path, value)`, `AcquireLock(path)` |
| `process` | PID + start-time + executable triple identity verification; safe signaling | `Identity`, `Inspect(pid)`, `Validate(expected, binaryPath)`, `Signal(expected, binaryPath, sig)` |
| `preset` | Loads immutable build-injected Agent model preset (base64 via `-ldflags -X`) | `Load()` → `*modelconfig.Config` |
| `metadata` | Manager handshake info and production identity validation | `Info()`, `ValidateProduction(artifacts...)` |
| `paths` | Cross-platform per-user path resolution (CLI state dir + desktop data dir) | `Paths`, `Resolve()` |
| `modelcatalog` | Model catalog HTTP client and simplify policy (link-time `Simplify` var filters model IDs containing `/`) | `ParseSimplify()`, `Client` |

## Protocol methods (15)

```
manager.info              diagnostics.collect
router.status             router.start            router.stop
router.health             router.version          router.logs
router.inspect_occupant   router.force_terminate_occupant
agent.detect              agent.models            agent.render
agent.preview             agent.write
```

## Architecture patterns

- **Mostly stateless per-request**: each JSON request is handled independently; long-running state lives in `lifecycle.Manager`, `agent.Service`, and `state` files. A notable exception is `occupant.Service`, which holds an in-memory single-use confirmation token between `Inspect` and `ForceTerminate` (mutex-guarded, expires after 30 s).
- **Identity verification before signaling**: on Unix/macOS the verified-identity path validates PID + start-time + executable before every signal. On Windows the PID-only path re-confirms the listening PID via `InspectPIDOwner` then calls `SignalPID` directly (no start-time/executable check).
- **API key zeroing**: `request.APIKey = ""` on explicit exit paths after successful parameter decode in `app`. Note: if `DecodeParams` itself fails (e.g. unknown fields), already-populated fields may not be zeroed.
- **Transaction recovery**: `agent` writes use a state directory with rollback capability; `NewService()` performs recovery on startup.
- **Discovery classification**: determined by branching on port reachability, state-file validity, process identity, and health results — not a fixed linear priority. When the port is unreachable, stale state is checked before degraded.

## Path conventions

| Path | Content |
|------|---------|
| `~/.mtls-router/setup-state.json` | CLI router state |
| `~/.mtls-router/mtls-router.log` | CLI router log |
| `~/Library/Application Support/com.codeasier.mtls-router/` (macOS) | Desktop data dir |
| `%APPDATA%/com.codeasier.mtls-router/` (Windows) | Desktop data dir |
| `~/.local/share/com.codeasier.mtls-router/` (Linux) | Desktop data dir |

## Entry point

`cmd/mtls-router-manager/main.go` — only command is `serve`; flags: `--router-sidecar`, `--listen`, `--desktop-session`, `--parent-pid`, `--parent-start`, `--parent-executable`.
