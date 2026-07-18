# Force-Terminate Port Occupant Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the desktop explicitly inspect and force-terminate a fully verified, current-user process occupying `127.0.0.1:19099` after a destructive confirmation.

**Architecture:** Add a pure-Go, build-tagged `occupant` package that maps one loopback TCP endpoint to a complete process/socket identity. A stateful manager service issues 30-second, single-use opaque tokens and revalidates discovery, identity, ownership, and protected-process rules before calling the existing identity-validating process signal path. Two new manager methods are bridged through Tauri to an occupied-state React recovery panel and modal; normal `router.stop` remains unchanged.

**Tech Stack:** Go 1.26, `golang.org/x/sys`, procfs, Darwin `proc_info`, Windows IP Helper/process-token APIs, newline-delimited manager JSON, Rust/Tauri 2, React 19, TypeScript, Vitest, Testing Library.

---

## File Map

**New Go package: `internal/manager/occupant/`**

- `types.go`: platform-independent identity, inspection result, errors, and injectable interfaces.
- `service.go`: token lifecycle, protected-process checks, revalidation, force termination, and release wait.
- `service_test.go`: deterministic token, race, ownership, protection, and signaling tests.
- `inspect_linux.go`, `inspect_linux_test.go`: `/proc/net/tcp*` and `/proc/<pid>/fd` mapping.
- `inspect_darwin.go`, `inspect_darwin_test.go`: `proc_info` FD/socket enumeration.
- `inspect_windows.go`, `inspect_windows_test.go`: `GetExtendedTcpTable` and process-token SID ownership.
- `inspect_unsupported.go`: fail-closed build fallback for unsupported targets.
- `integration_test.go`: current-user listener inspection and termination integration test.

**Existing Go manager files**

- `internal/manager/process/process.go`: expose same-identity comparison and allow a complete identity to be signaled without inventing a managed binary owner.
- `internal/manager/protocol/types.go`: methods, params, results, deadlines, and stable errors.
- `internal/manager/app/app.go`: construct occupant service and register handlers.
- `internal/manager/app/app_test.go`: protocol wiring, sanitization, and error mapping.

**Desktop bridge and UI**

- `desktop/src-tauri/src/types.rs`: occupant response types.
- `desktop/src-tauri/src/commands.rs`: inspect and terminate commands.
- `desktop/src-tauri/src/lib.rs`: command registration.
- `desktop/src-tauri/src/orchestration.rs`: termination refresh behavior and Rust tests.
- `desktop/src/ipc.ts`, `desktop/src/ipc.test.ts`: TypeScript contract and invoke argument tests.
- `desktop/src/test/api.ts`: mock methods.
- `desktop/src/RouterPage.tsx`, `desktop/src/RouterPage.test.tsx`: occupied recovery and confirmation state machine.
- `desktop/src/styles.css`: recovery panel and accessible modal styling.
- `desktop/src/locales/en.ts`, `desktop/src/locales/zh-CN.ts`: all user-visible copy.

**Live specification and documentation**

- `specs/tauri-desktop-app/spec.md`, `specs/tauri-desktop-app/checklist.md`
- `docs/DESKTOP.md`, `docs/zh-CN/DESKTOP.md`
- `docs/TROUBLESHOOTING.md`, `docs/zh-CN/TROUBLESHOOTING.md`

### Task 1: Process Identity Primitives

**Files:**
- Modify: `internal/manager/process/process.go`
- Modify: `internal/manager/process/process_test.go`

- [ ] **Step 1: Write failing tests for identity comparison and unknown-process signaling**

Add tests proving `SameIdentity` compares PID, start identity, and normalized executable, and that `SignalIdentity` still invokes `Validate` immediately before signaling. Include mismatched PID/start/path cases and assert the injected signal function is not called.

```go
func TestSameIdentityRequiresPIDStartAndExecutable(t *testing.T) {
	base := Identity{PID: 42, StartedAt: "start-1", Executable: os.Args[0]}
	if same, err := SameIdentity(base, base); err != nil || !same {
		t.Fatalf("SameIdentity(base, base) = %t, %v", same, err)
	}
	changed := base
	changed.StartedAt = "start-2"
	if same, _ := SameIdentity(base, changed); same {
		t.Fatal("changed start identity matched")
	}
}
```

- [ ] **Step 2: Run the focused tests and verify failure**

Run: `go test ./internal/manager/process -run 'TestSameIdentity|TestSignalIdentity'`

Expected: FAIL because the exported functions do not exist.

- [ ] **Step 3: Add minimal identity APIs**

Implement:

```go
func SameIdentity(left, right Identity) (bool, error)
func SignalIdentity(expected Identity, signal os.Signal) error
```

`SameIdentity` normalizes both paths and compares PID, `sameStartIdentity`, and `sameExecutable`. `SignalIdentity` calls `Validate(expected, expected.Executable)` and then the private `signalProcess`; it must not expose PID-only signaling. Refactor `Signal` to retain its managed-binary check and avoid changing existing lifecycle semantics.

- [ ] **Step 4: Run process tests**

Run: `go test ./internal/manager/process`

Expected: PASS.

- [ ] **Step 5: Commit the identity primitive**

```bash
git add internal/manager/process/process.go internal/manager/process/process_test.go
git commit -m "feat: add reusable process identity validation"
```

### Task 2: Occupant Service and Confirmation Tokens

**Files:**
- Create: `internal/manager/occupant/types.go`
- Create: `internal/manager/occupant/service.go`
- Create: `internal/manager/occupant/service_test.go`

- [ ] **Step 1: Define tests for the complete service contract**

Use injected `Inspect`, `Discover`, `Signal`, `CurrentUser`, `Random`, `Now`, and `Sleep` functions. Cover: inspect only in `unknown_occupant`; 32 random bytes encoded as base64url; 30-second expiry; one active token; single use; restart loss through a new service; changed PID/start/path/user/socket/address; other-user rejection; desktop/manager/managed-router protection; immediate `os.Kill`; replacement listener during release wait; and no signal on every rejection path.

Core test shape:

```go
inspection, opErr := service.Inspect(context.Background())
if opErr != nil { t.Fatal(opErr) }
result, opErr := service.ForceTerminate(context.Background(), inspection.ConfirmationToken)
if opErr != nil { t.Fatal(opErr) }
if gotSignal != os.Kill || result.State != "absent" { t.Fatalf("unexpected result") }
```

- [ ] **Step 2: Run tests and verify failure**

Run: `go test ./internal/manager/occupant`

Expected: FAIL because the package is not implemented.

- [ ] **Step 3: Add platform-independent types**

Define:

```go
type Identity struct {
	ListenAddr string
	Network string
	SocketID string
	Process process.Identity
	UserID string
}

type Inspection struct {
	PID int `json:"pid"`
	ProcessName string `json:"process_name"`
	Executable string `json:"executable"`
	ListenAddr string `json:"listen_addr"`
	ConfirmationToken string `json:"confirmation_token"`
	ExpiresAt time.Time `json:"expires_at"`
}

type Result struct { State string `json:"state"` }
```

Use package errors that map one-to-one to the stable protocol codes. `Identity` remains internal to the manager and is never serialized.

- [ ] **Step 4: Implement the token service**

`Inspect` must parse and validate loopback TCP address, require discovery `UnknownOccupant`, inspect one unique listener, compare `UserID` with the current-user identity, reject protected PIDs/managed router identity, generate a token, and replace the previous token under one mutex.

`ForceTerminate` must consume the token before validation, rediscover `UnknownOccupant`, reinspect, compare every identity field plus `process.SameIdentity`, repeat protection and ownership checks, call `process.SignalIdentity(identity.Process, os.Kill)`, then poll process absence and TCP refusal for at most two seconds. Return `OCCUPANT_CHANGED` if a replacement listener appears; return `PORT_RELEASE_TIMEOUT` if the original process exits but the address remains occupied.

- [ ] **Step 5: Run service tests and race detector**

Run: `go test -race ./internal/manager/occupant -run 'TestService'`

Expected: PASS with no races.

- [ ] **Step 6: Commit the service**

```bash
git add internal/manager/occupant internal/manager/process
git commit -m "feat: add verified occupant termination service"
```

### Task 3: Linux Listener Discovery

**Files:**
- Create: `internal/manager/occupant/inspect_linux.go`
- Create: `internal/manager/occupant/inspect_linux_test.go`

- [ ] **Step 1: Write parser and correlation tests**

Provide synthetic `/proc/net/tcp` rows for `127.0.0.1:19099` in state `0A`, non-listening rows, IPv6 rows, wildcard rows, malformed rows, duplicate inodes, and two-PID ownership. Provide fake `/proc/<pid>/fd` symlinks and status UIDs. Assert exactly one PID/inode succeeds and every ambiguous/inaccessible case returns identity-unavailable.

- [ ] **Step 2: Run Linux tests and verify failure**

Run on Linux CI: `go test ./internal/manager/occupant -run 'TestParseProc|TestInspectLinux'`

Run elsewhere: `GOOS=linux GOARCH=amd64 CGO_ENABLED=0 go test -c ./internal/manager/occupant -o /tmp/occupant-linux.test`

Expected: FAIL because Linux discovery is absent.

- [ ] **Step 3: Implement procfs discovery**

Read `/proc/net/tcp` and `/proc/net/tcp6`, decode little-endian hex addresses and hex ports, select `TCP_LISTEN`, then scan numeric `/proc` directories and `fd` symlinks matching `socket:[inode]`. Read the effective UID from `/proc/<pid>/status`, call `process.Inspect(pid)`, and return `SocketID` as the inode. Require one unique process and socket identity. Inject proc root/readlink/read-file functions for tests; production defaults to `/proc` and `os` functions.

- [ ] **Step 4: Run Linux package and cross-build tests**

Run on Linux CI: `CGO_ENABLED=0 go test ./internal/manager/occupant`

Run elsewhere: `GOOS=linux GOARCH=amd64 CGO_ENABLED=0 go test -c ./internal/manager/occupant -o /tmp/occupant-linux.test`

Expected: PASS.

- [ ] **Step 5: Commit Linux discovery**

```bash
git add internal/manager/occupant/inspect_linux.go internal/manager/occupant/inspect_linux_test.go
git commit -m "feat: discover Linux TCP listener owners"
```

### Task 4: Windows Listener Discovery

**Files:**
- Create: `internal/manager/occupant/inspect_windows.go`
- Create: `internal/manager/occupant/inspect_windows_test.go`

- [ ] **Step 1: Write table-selection and SID tests**

Test exact IPv4 endpoint selection, TCP listen-state filtering, duplicate owners, IPv6/wildcard fail-closed behavior, current SID match/mismatch, inaccessible token, and process identity inspection failure. Keep native API calls behind injected functions so tests run without terminating processes.

- [ ] **Step 2: Run a Windows cross-compile test and verify failure**

Run: `GOOS=windows GOARCH=amd64 CGO_ENABLED=0 go test -c ./internal/manager/occupant -o /tmp/occupant-windows.test.exe`

Expected: FAIL because Windows discovery is absent.

- [ ] **Step 3: Implement native Windows discovery**

Use `iphlpapi.dll` `GetExtendedTcpTable` with `TCP_TABLE_OWNER_PID_LISTENER` and `AF_INET`, retrying after `ERROR_INSUFFICIENT_BUFFER`. Parse `MIB_TCPROW_OWNER_PID`, match exact local address and network-order port, and require one owner PID. Open the process with query rights, call `OpenProcessToken` and `GetTokenInformation(TokenUser)`, convert SID to its stable string, and reuse `process.Inspect`. Use `SocketID` composed from protocol, local endpoint, and PID; no PowerShell, `netstat`, or `taskkill`.

- [ ] **Step 4: Verify Windows amd64 and arm64 builds**

Run: `GOOS=windows GOARCH=amd64 CGO_ENABLED=0 go test -c ./internal/manager/occupant -o /tmp/occupant-windows-amd64.test.exe`

Run: `GOOS=windows GOARCH=arm64 CGO_ENABLED=0 go test -c ./internal/manager/occupant -o /tmp/occupant-windows-arm64.test.exe`

Expected: both compile successfully; Windows CI later executes tests.

- [ ] **Step 5: Commit Windows discovery**

```bash
git add internal/manager/occupant/inspect_windows.go internal/manager/occupant/inspect_windows_test.go
git commit -m "feat: discover Windows TCP listener owners"
```

### Task 5: macOS Listener Discovery and Platform Integration

**Files:**
- Create: `internal/manager/occupant/inspect_darwin.go`
- Create: `internal/manager/occupant/inspect_darwin_test.go`
- Create: `internal/manager/occupant/inspect_unsupported.go`
- Create: `internal/manager/occupant/integration_test.go`

- [ ] **Step 1: Write Darwin record-decoding and ambiguity tests**

Test PID enumeration, `PROC_PIDLISTFDS`, TCP socket filtering through `PROC_PIDFDSOCKETINFO`, exact IPv4 address/port matching, duplicate owners, inaccessible records, current UID mismatch, and changed process identity. Inject the raw `proc_info` caller so fixture bytes drive deterministic tests.

- [ ] **Step 2: Run Darwin tests and verify failure**

Run on macOS: `CGO_ENABLED=0 go test ./internal/manager/occupant -run 'TestDarwin'`

Run elsewhere: `GOOS=darwin GOARCH=arm64 CGO_ENABLED=0 go test -c ./internal/manager/occupant -o /tmp/occupant-darwin.test`

Expected: FAIL because Darwin discovery is absent.

- [ ] **Step 3: Implement Darwin discovery**

Use the Darwin `proc_info` syscall operations `PROC_ALL_PIDS`, `PROC_PIDLISTFDS`, and `PROC_PIDFDSOCKETINFO`. Decode only fixed-width kernel records required for process FD type, TCP protocol, local address, local port, and listener state. For each exact endpoint match, call existing `process.Inspect`, obtain effective UID through process metadata, and require one unique PID/socket tuple. Keep syscall numbers, flavors, and record layouts architecture-specific where Darwin amd64 and arm64 differ; fail closed on short or unknown records.

- [ ] **Step 4: Add unsupported fallback and integration test**

The fallback returns identity-unavailable for non-Darwin/Linux/Windows builds. The integration test binds an ephemeral `127.0.0.1:0` listener in a helper subprocess, verifies inspection returns the helper's complete current-user identity, obtains a token, force-terminates it, and verifies the address is free. Skip only when the platform reports a documented native permission limitation.

- [ ] **Step 5: Run native tests and all target cross-builds**

Run: `go test -race ./internal/manager/occupant`

Run: `for target in darwin/amd64 darwin/arm64 linux/amd64 linux/arm64 windows/amd64 windows/arm64; do GOOS=${target%/*} GOARCH=${target#*/} CGO_ENABLED=0 go test -c ./internal/manager/occupant -o "/tmp/occupant-${target%/*}-${target#*/}.test"; done`

Expected: native tests PASS and all six targets compile.

- [ ] **Step 6: Commit macOS and integration support**

```bash
git add internal/manager/occupant
git commit -m "feat: discover macOS TCP listener owners"
```

### Task 6: Manager Protocol and Application Wiring

**Files:**
- Modify: `internal/manager/protocol/types.go`
- Modify: `internal/manager/protocol/server_test.go`
- Modify: `internal/manager/app/app.go`
- Modify: `internal/manager/app/app_test.go`

- [ ] **Step 1: Write protocol and handler tests**

Extend the sequential method test with `router.inspect_occupant` and `router.force_terminate_occupant`. Assert inspect rejects non-empty params; terminate requires exactly `confirmation_token`; the protocol output includes only presentation fields; errors map to the eight stable codes; and token/path values never enter diagnostics or unrelated responses.

- [ ] **Step 2: Run focused tests and verify failure**

Run: `go test ./internal/manager/protocol ./internal/manager/app -run 'Occupant|WiresEveryMethod'`

Expected: FAIL because methods and handlers are absent.

- [ ] **Step 3: Add protocol types and deadlines**

Add methods `router.inspect_occupant` and `router.force_terminate_occupant`, two- and three-second deadlines, the eight error codes from the design, `RouterForceTerminateOccupantParams`, `RouterOccupantInspectionResult`, and `RouterOccupantTerminationResult`.

```go
type RouterForceTerminateOccupantParams struct {
	ConfirmationToken string `json:"confirmation_token"`
}
```

- [ ] **Step 4: Wire the service into App**

Add an `occupantService` interface to `dependencies`, construct it in `New`, include current desktop PID, manager identity, router state readers, discoverer, and configured listen address in protection dependencies, register both handlers, and map package errors to sanitized protocol errors. Reject blank tokens as `INVALID_PARAMS`.

- [ ] **Step 5: Run manager tests and race detector**

Run: `go test -race ./internal/manager/...`

Expected: PASS.

- [ ] **Step 6: Commit manager API**

```bash
git add internal/manager/protocol internal/manager/app
git commit -m "feat: expose occupant recovery protocol"
```

### Task 7: Tauri and TypeScript IPC Bridge

**Files:**
- Modify: `desktop/src-tauri/src/types.rs`
- Modify: `desktop/src-tauri/src/commands.rs`
- Modify: `desktop/src-tauri/src/orchestration.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src/ipc.ts`
- Modify: `desktop/src/ipc.test.ts`
- Modify: `desktop/src/test/api.ts`

- [ ] **Step 1: Write Rust and TypeScript bridge tests**

Assert inspect calls `router.inspect_occupant` with `{}`; terminate sends only `{confirmation_token}`; success requests scheduler refresh and returns absent status; manager errors retain stable codes; TypeScript never sends PID/path; and mock API supplies deterministic occupant methods.

- [ ] **Step 2: Run bridge tests and verify failure**

Run: `npm test -- src/ipc.test.ts && npm run rust:test`

Workdir: `desktop`

Expected: FAIL because commands and types are absent.

- [ ] **Step 3: Add Rust commands and types**

Add serializable/deserializable `RouterOccupantInspection` and `RouterOccupantTermination`; implement `router_inspect_occupant` and `router_force_terminate_occupant`; route successful termination through orchestration to refresh scheduler status; register both commands in `lib.rs`.

- [ ] **Step 4: Add TypeScript API**

Add commands, interfaces, and methods:

```ts
inspectRouterOccupant(): Promise<RouterOccupantInspection>;
forceTerminateRouterOccupant(confirmationToken: string): Promise<RouterStatus>;
```

The invoke call passes `{ confirmationToken }` to Tauri; Rust serializes it to manager JSON as `confirmation_token`. Do not accept PID or executable arguments.

- [ ] **Step 5: Run bridge verification**

Run: `npm run typecheck && npm test -- src/ipc.test.ts && npm run rust:format && npm run rust:test`

Workdir: `desktop`

Expected: PASS.

- [ ] **Step 6: Commit IPC bridge**

```bash
git add desktop/src-tauri/src desktop/src/ipc.ts desktop/src/ipc.test.ts desktop/src/test/api.ts
git commit -m "feat: bridge occupant recovery to desktop"
```

### Task 8: Router Occupied Recovery UI

**Files:**
- Modify: `desktop/src/RouterPage.tsx`
- Modify: `desktop/src/RouterPage.test.tsx`
- Modify: `desktop/src/styles.css`
- Modify: `desktop/src/locales/en.ts`
- Modify: `desktop/src/locales/zh-CN.ts`

- [ ] **Step 1: Write UI tests for inspectable and blocked occupants**

Cover successful inspection, loading, retry, `OCCUPANT_NOT_OWNED`, identity unavailable, protected, ambiguous/changed, and generic inspect failure. Assert normal Stop remains disabled and the force action appears only with a valid inspection.

- [ ] **Step 2: Write modal and operation tests**

Assert the dialog shows process name, PID, full executable path, and data-loss warning; Cancel is focused; Escape/cancel sends no request; confirm sends only the token once; controls disable during execution; success closes, refreshes status, reports port released, and does not call `startRouter`; expired/changed closes stale details and reinspects.

- [ ] **Step 3: Run Router page tests and verify failure**

Run: `npm test -- src/RouterPage.test.tsx`

Workdir: `desktop`

Expected: FAIL because recovery UI is absent.

- [ ] **Step 4: Implement occupied-state inspection**

When status enters `unknown_occupant`, call `inspectRouterOccupant` once per occupied transition. Abort state updates after unmount/state change. Store only presentation result and token in component state; clear it whenever status leaves occupied. Add retry inspection without coupling it to health polling.

- [ ] **Step 5: Implement the accessible confirmation dialog**

Use native `<dialog>` or the project's accessible modal pattern with `aria-labelledby` and `aria-describedby`. Default focus Cancel, trap interaction while open, close on Cancel/Escape before submission, disable closing during force termination, and render the executable in wrapping `<code>`. Add a distinct destructive button; do not add a tray action.

- [ ] **Step 6: Add localized copy and styles**

Replace “will not be terminated” occupied copy with explicit optional recovery copy. Add English and Chinese strings for inspect/retry, blocked reasons, modal labels, warning, progress, success, changed, expired, and failure. Style the recovery panel/modal consistently with current warm desktop controls, preserve mobile wrapping, and use the existing danger palette.

- [ ] **Step 7: Run frontend verification**

Run: `npm run static:check && npm run typecheck && npm test && npm run build`

Workdir: `desktop`

Expected: PASS.

- [ ] **Step 8: Commit the desktop UI**

```bash
git add desktop/src
git commit -m "feat: confirm force termination of port occupants"
```

### Task 9: Update Live Specification and Documentation

**Files:**
- Modify: `specs/tauri-desktop-app/spec.md`
- Modify: `specs/tauri-desktop-app/checklist.md`
- Modify: `docs/DESKTOP.md`
- Modify: `docs/zh-CN/DESKTOP.md`
- Modify: `docs/TROUBLESHOOTING.md`
- Modify: `docs/zh-CN/TROUBLESHOOTING.md`

- [ ] **Step 1: Update the live desktop requirements**

Replace the absolute “does not kill/never killed” requirement with: no automatic termination; explicit inspect-and-confirm current-user recovery; complete identity/listener revalidation; no signal for other-user, changed, protected, ambiguous, or unverifiable targets; no automatic router start. Add protocol methods, deadlines, UI state, and acceptance checklist entries.

- [ ] **Step 2: Update paired user documentation**

Document the force-terminate button, displayed target data, immediate data-loss risk, current-user restriction, no elevation, and manual fallback. Keep English and Simplified Chinese sections structurally equivalent. Clarify that Quit and normal Stop still never terminate unknown occupants.

- [ ] **Step 3: Audit obsolete claims and links**

Run: `rg -n "never kills|never killed|does not kill|不会终止|绝不.*终止|unknown occupant" README.md docs specs desktop/src/locales`

Expected: only contextually correct claims remain, especially “never automatically terminates” and normal Stop/Quit ownership restrictions.

- [ ] **Step 4: Run docs/workflow consistency tests**

Run: `make test-workflows && make test-shell`

Expected: PASS.

- [ ] **Step 5: Commit documentation**

```bash
git add specs/tauri-desktop-app docs/DESKTOP.md docs/zh-CN/DESKTOP.md docs/TROUBLESHOOTING.md docs/zh-CN/TROUBLESHOOTING.md
git commit -m "docs: describe confirmed port occupant termination"
```

### Task 10: End-to-End Verification

**Files:**
- Modify if required by failures: only files introduced or intentionally changed in Tasks 1-9

- [ ] **Step 1: Format all languages**

Run from repository root: `gofmt -w internal/manager/process internal/manager/occupant internal/manager/protocol internal/manager/app`

Run from `desktop`: `npm run format && cargo fmt --manifest-path src-tauri/Cargo.toml --all`

Expected: formatters complete without errors.

- [ ] **Step 2: Run complete Go verification**

Run: `go test -race ./... && go vet ./... && test -z "$(gofmt -l .)"`

Expected: PASS and no unformatted Go files.

- [ ] **Step 3: Run shell and workflow verification**

Run: `make test-shell && make test-workflows`

Expected: PASS.

- [ ] **Step 4: Run complete desktop verification**

Run: `npm run verify`

Workdir: `desktop`

Expected: ESLint, Prettier, TypeScript, Vitest, Vite build, Rust formatting, and Rust tests all PASS.

- [ ] **Step 5: Verify all release target builds remain cgo-free**

Run: `for target in darwin/amd64 darwin/arm64 linux/amd64 linux/arm64 windows/amd64 windows/arm64; do GOOS=${target%/*} GOARCH=${target#*/} CGO_ENABLED=0 go build -trimpath -o "/tmp/mtls-router-manager-${target%/*}-${target#*/}" ./cmd/mtls-router-manager || exit 1; done`

Expected: all manager binaries build.

- [ ] **Step 6: Perform native manual acceptance**

Start a disposable current-user listener on `127.0.0.1:19099`, open the desktop Router page, verify its identity in the modal, cancel once and prove it remains alive, then confirm force termination and prove the port becomes free without the router starting automatically. Repeat the replacement-listener race using the integration test rather than risking an unrelated process.

- [ ] **Step 7: Inspect final diff and commit verification fixes if any**

Run: `git status --short && git diff --check && git diff --stat`

Expected: only intended feature, test, spec, and documentation files are changed; no whitespace errors or generated secrets exist.

If verification required fixes, stage exactly the files shown as modified by the preceding `git status --short`, after reviewing each diff, then run:

```bash
git commit -m "test: complete occupant recovery verification"
```
