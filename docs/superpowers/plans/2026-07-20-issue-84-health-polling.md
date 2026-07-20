# Issue 84 Reliable Health Polling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent transient management polls from reporting false router failures and reuse dedicated upstream health connections to eliminate avoidable `TIME_WAIT` accumulation.

**Architecture:** Split manager discovery into identity-only and health-aware operations, with separate HTTP budgets and protocol deadlines. Replace the stateless health function with a long-lived dedicated mTLS prober whose transport is reused and closed with the router. Keep desktop polling errors separate from cached observed state so fresh successful values remain authoritative while warnings remain visible.

**Tech Stack:** Go 1.26 (`net/http`, mTLS, table tests), React 19/TypeScript/Vitest, Rust/Tauri/Cargo tests.

---

## File Map

- `internal/health/probe.go`: construct and own a reusable mTLS `Prober`; drain response bodies and close idle connections.
- `internal/health/probe_test.go`: prove connection reuse and preserve probe error behavior.
- `main.go`: create one prober, share it between startup and `/health`, and close it on shutdown.
- `internal/manager/discovery/discovery.go`: add identity-only discovery and a distinct health request timeout.
- `internal/manager/discovery/discovery_test.go`: prove status discovery does not call `/health` and slow healthy responses fit the new budget.
- `internal/manager/app/app.go`: inject and route process-only versus health-aware discovery functions.
- `internal/manager/app/app_test.go`: verify each RPC uses the intended discovery path.
- `internal/manager/protocol/types.go`: increase only `router.health` to 12 seconds.
- `internal/manager/protocol/server_test.go`: lock down protocol deadlines.
- `desktop/src-tauri/src/manager.rs`: increase the Rust health watchdog to 13 seconds.
- `desktop/src-tauri/src/scheduler.rs`: lock down cached-result retention on poll errors.
- `desktop/src-tauri/src/tray.rs`: prefer a cached status over a transient status error.
- `desktop/src/RouterPage.tsx`: separate status-read/start failures and stop forcing health poll errors to degraded.
- `desktop/src/RouterPage.test.tsx`: cover cached and no-cache poll failure presentations.
- `desktop/src/locales/en.ts`: add English status-unavailable copy.
- `desktop/src/locales/zh-CN.ts`: add Chinese status-unavailable copy.

### Task 1: Reusable Upstream Health Prober

**Files:**
- Modify: `internal/health/probe.go`
- Modify: `internal/health/probe_test.go`

- [ ] **Step 1: Write a failing connection-reuse test**

Add a TLS test server with `ConnState` accounting and call one constructed prober twice:

```go
func TestProberReusesUpstreamConnection(t *testing.T) {
	var newConnections atomic.Int32
	server := newMTLSServer(t, http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = io.WriteString(w, "healthy")
	}))
	server.Config.ConnState = func(_ net.Conn, state http.ConnState) {
		if state == http.StateNew {
			newConnections.Add(1)
		}
	}
	server.StartTLS()
	defer server.Close()

	prober, err := NewProber(testProbeOptions(server.URL, serverCerts(server)))
	if err != nil {
		t.Fatal(err)
	}
	defer prober.Close()
	if err := prober.Probe(); err != nil {
		t.Fatal(err)
	}
	if err := prober.Probe(); err != nil {
		t.Fatal(err)
	}
	if got := newConnections.Load(); got != 1 {
		t.Fatalf("new connections = %d, want 1", got)
	}
}
```

Use the package's existing certificate helpers rather than adding a second certificate generator.

- [ ] **Step 2: Run the test and verify it fails**

Run: `go test ./internal/health -run TestProberReusesUpstreamConnection -count=1`

Expected: build failure because `NewProber` and the long-lived `Prober` do not exist.

- [ ] **Step 3: Implement the minimal reusable prober**

Replace per-call TLS construction with a constructor and methods while retaining `ProbeFunc` for handler stubs:

```go
type Prober struct {
	url       string
	timeout   time.Duration
	client    *http.Client
	transport *http.Transport
}

func NewProber(opts ProbeOptions) (*Prober, error) {
	if _, err := url.ParseRequestURI(opts.UpstreamURL); err != nil {
		return nil, fmt.Errorf("invalid probe URL")
	}
	clientCert, rootCAs, err := certs.LoadFromStrings(opts.ClientCert, opts.ClientKey, opts.UpstreamCA)
	if err != nil {
		return nil, fmt.Errorf("load probe mTLS config")
	}
	tlsMin, err := tlspolicy.MinVersion(opts.TLSMin)
	if err != nil {
		return nil, fmt.Errorf("configure probe TLS")
	}
	timeout := opts.Timeout
	if timeout <= 0 {
		timeout = 5 * time.Second
	}
	transport := &http.Transport{TLSClientConfig: &tls.Config{
		Certificates: []tls.Certificate{*clientCert},
		RootCAs: rootCAs,
		MinVersion: tlsMin,
	}}
	return &Prober{url: opts.UpstreamURL, timeout: timeout, client: &http.Client{Transport: transport}, transport: transport}, nil
}

func (p *Prober) Probe(_ ProbeOptions) error {
	ctx, cancel := context.WithTimeout(context.Background(), p.timeout)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, p.url, nil)
	if err != nil {
		return fmt.Errorf("create probe request")
	}
	resp, err := p.client.Do(req)
	if err != nil {
		return fmt.Errorf("probe upstream failed")
	}
	defer resp.Body.Close()
	_, _ = io.Copy(io.Discard, resp.Body)
	if resp.StatusCode >= http.StatusInternalServerError {
		return fmt.Errorf("probe upstream returned status %d", resp.StatusCode)
	}
	return nil
}

func (p *Prober) Close() { p.transport.CloseIdleConnections() }
```

Keep the exported one-shot `Probe(opts)` wrapper if existing package consumers or tests require it; implement it by constructing, deferring `Close`, and invoking the prober once.

- [ ] **Step 4: Run health tests**

Run: `go test ./internal/health -count=1`

Expected: PASS, including reuse, timeout, invalid URL, TLS, and 5xx cases.

- [ ] **Step 5: Commit the prober**

```bash
git add internal/health/probe.go internal/health/probe_test.go
git commit -m "fix: reuse upstream health connections"
```

### Task 2: Share the Prober for Router Lifetime

**Files:**
- Modify: `main.go`
- Modify: `main_test.go`

- [ ] **Step 1: Add a failing construction-boundary test**

Extract a small constructor used by `run` and test that startup and handler callbacks reference the same prober instance. Prefer testing the returned `ProbeFunc` behavior through an injected fake transport if direct pointer identity would expose unnecessary internals.

```go
func TestRuntimeHealthProbeIsShared(t *testing.T) {
	prober, err := health.NewProber(validProbeOptions(t))
	if err != nil {
		t.Fatal(err)
	}
	defer prober.Close()
	if got := reflect.ValueOf(prober.Probe).Pointer(); got == 0 {
		t.Fatal("shared probe callback is nil")
	}
}
```

Adjust this test to the smallest helper boundary selected during implementation; do not expose production internals solely for pointer comparison.

- [ ] **Step 2: Run the focused main test and verify failure**

Run: `go test . -run TestRuntimeHealthProbeIsShared -count=1`

Expected: FAIL because `run` still calls the one-shot `health.Probe` for startup and runtime.

- [ ] **Step 3: Wire one prober into startup and `/health`**

In `run`, construct after config validation, close it with the process, and pass its method to both calls:

```go
prober, err := health.NewProber(probeOptions)
if err != nil {
	return err
}
defer prober.Close()
if err := prober.Probe(probeOptions); err != nil {
	return err
}
// ...
mux.Handle("/health", routermeta.HealthHandler(prober.Probe, probeOptions))
```

The callback parameter remains for compatibility with `routermeta.HealthHandler`; the constructed prober owns the validated immutable options.

- [ ] **Step 4: Run router and handler tests**

Run: `go test . ./internal/routermeta -count=1`

Expected: PASS.

- [ ] **Step 5: Commit runtime wiring**

```bash
git add main.go main_test.go
git commit -m "fix: share router health prober"
```

### Task 3: Split Identity and Health Discovery

**Files:**
- Modify: `internal/manager/discovery/discovery.go`
- Modify: `internal/manager/discovery/discovery_test.go`

- [ ] **Step 1: Write failing process-only and slow-health tests**

Add `HealthRequestTimeout` to test configurations and instrument endpoint calls:

```go
func TestDiscoverStatusDoesNotProbeHealth(t *testing.T) {
	var healthCalls atomic.Int32
	server := routerServer(t, func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/health" {
			healthCalls.Add(1)
			time.Sleep(100 * time.Millisecond)
		}
		serveRouterMetadata(w, r, "ok")
	})
	discoverer := correlatedDiscoverer(t, server.URL, 20*time.Millisecond, 150*time.Millisecond)
	result := discoverer.DiscoverStatus(context.Background())
	if result.Classification != DesktopOwned {
		t.Fatalf("classification = %q", result.Classification)
	}
	if got := healthCalls.Load(); got != 0 {
		t.Fatalf("health calls = %d, want 0", got)
	}
}

func TestDiscoverAcceptsSlowHealthyResponse(t *testing.T) {
	server := delayedHealthRouterServer(t, 40*time.Millisecond, "ok")
	discoverer := correlatedDiscoverer(t, server.URL, 20*time.Millisecond, 100*time.Millisecond)
	result := discoverer.Discover(context.Background())
	if result.Classification != DesktopOwned || result.Health.Status != "ok" {
		t.Fatalf("result = %#v", result)
	}
}
```

- [ ] **Step 2: Run both tests and verify failure**

Run: `go test ./internal/manager/discovery -run 'TestDiscover(StatusDoesNotProbeHealth|AcceptsSlowHealthyResponse)' -count=1`

Expected: build failure for `DiscoverStatus` or failure because the health request uses the short timeout.

- [ ] **Step 3: Add separate discovery modes and budgets**

Extend configuration and delegate both public methods to shared logic:

```go
type Config struct {
	// existing fields
	RequestTimeout       time.Duration
	HealthRequestTimeout time.Duration
}

func (d *Discoverer) DiscoverStatus(ctx context.Context) Result {
	return d.discover(ctx, false)
}

func (d *Discoverer) Discover(ctx context.Context) Result {
	return d.discover(ctx, true)
}
```

Default `HealthRequestTimeout` to `11 * time.Second`. Move the existing body into `discover(ctx, includeHealth bool)`. Always request and classify `/version`; only request `/health` and apply health degradation when `includeHealth` is true. Change `getJSON` to accept the request timeout explicitly:

```go
versionErr := d.getJSON(ctx, "/version", &result.Version, d.config.RequestTimeout)
if includeHealth {
	healthErr = d.getJSON(ctx, "/health", &result.Health, d.config.HealthRequestTimeout)
}
```

Do not set `http.Client.Timeout` to the one-second request timeout; the per-request context is authoritative and supports endpoint-specific budgets.

- [ ] **Step 4: Run all discovery tests**

Run: `go test ./internal/manager/discovery -count=1`

Expected: PASS. Update old tests that intentionally expected unknown occupants to receive `/health`; process-only tests must expect zero health calls, while full `Discover` behavior remains covered.

- [ ] **Step 5: Commit discovery split**

```bash
git add internal/manager/discovery/discovery.go internal/manager/discovery/discovery_test.go
git commit -m "fix: separate status and health discovery"
```

### Task 4: Route Manager RPCs and Align Deadlines

**Files:**
- Modify: `internal/manager/app/app.go`
- Modify: `internal/manager/app/app_test.go`
- Modify: `internal/manager/protocol/types.go`
- Modify: `internal/manager/protocol/server_test.go`

- [ ] **Step 1: Write failing RPC routing tests**

Split app dependencies and count calls:

```go
func TestRouterStatusUsesStatusDiscovery(t *testing.T) {
	statusCalls, healthCalls := 0, 0
	app := newWithDependencies(testConfig(), dependencies{
		discoverStatus: func(context.Context) discovery.Result {
			statusCalls++
			return desktopOwnedResult()
		},
		discoverHealth: func(context.Context) discovery.Result {
			healthCalls++
			return discovery.Result{Classification: discovery.Degraded}
		},
		// existing required fakes
	})
	response := serveRequest(t, app, protocol.MethodRouterStatus)
	assertStatusResponse(t, response, "desktop_owned")
	if statusCalls != 1 || healthCalls != 0 {
		t.Fatalf("status calls=%d health calls=%d", statusCalls, healthCalls)
	}
}
```

Add the complementary `router.health` assertion and verify `router.version` uses status discovery.

- [ ] **Step 2: Run focused app tests and verify failure**

Run: `go test ./internal/manager/app -run 'TestRouter(StatusUsesStatusDiscovery|HealthUsesHealthDiscovery|VersionUsesStatusDiscovery)' -count=1`

Expected: build failure because dependencies still contain one `discover` function.

- [ ] **Step 3: Implement dependency routing**

Change dependencies to:

```go
discoverStatus func(context.Context) discovery.Result
discoverHealth func(context.Context) discovery.Result
```

Wire production dependencies to `discoverer.DiscoverStatus` and `discoverer.Discover`. Use status discovery for status, version, logs, lifecycle/occupant/trusted identity operations, and health discovery for health and diagnostics where upstream health is printed. Update test dependency factories to default both functions so unrelated tests remain concise.

- [ ] **Step 4: Increase manager health deadline**

In `protocol.Deadlines`, retain status at one second and set health to twelve seconds:

```go
MethodRouterStatus: time.Second,
MethodRouterHealth: 12 * time.Second,
```

Update `TestDeadlinesCoverEveryMethod` accordingly.

- [ ] **Step 5: Run manager tests**

Run: `go test ./internal/manager/app ./internal/manager/protocol ./internal/manager/lifecycle ./internal/manager/occupant ./internal/manager/trustedrouter -count=1`

Expected: PASS.

- [ ] **Step 6: Commit manager routing and deadlines**

```bash
git add internal/manager/app/app.go internal/manager/app/app_test.go internal/manager/protocol/types.go internal/manager/protocol/server_test.go
git commit -m "fix: isolate router status from health probes"
```

### Task 5: Preserve Desktop Cached State on Poll Errors

**Files:**
- Modify: `desktop/src/RouterPage.tsx`
- Modify: `desktop/src/RouterPage.test.tsx`
- Modify: `desktop/src/locales/en.ts`
- Modify: `desktop/src/locales/zh-CN.ts`

- [ ] **Step 1: Add failing React regression tests**

Extend the snapshot tests with these assertions:

```tsx
it("keeps cached healthy state after a transient status error", async () => {
  render(<RouterPage api={apiWithSnapshots(healthySnapshot, {
    ...healthySnapshot,
    revision: 2,
    status_error: { code: "OPERATION_TIMEOUT", message: "timed out" },
  })} onNavigateToAgents={() => {}} />);
  expect(await screen.findByText("Router is healthy")).toBeInTheDocument();
  expect(screen.getByText("Unable to read router status.")).toBeInTheDocument();
  expect(screen.queryByText("Router failed to start")).not.toBeInTheDocument();
});

it("shows status unavailable when the first status poll fails", async () => {
  render(<RouterPage api={apiWithSnapshot({ revision: 1, status_error: timeoutError })} onNavigateToAgents={() => {}} />);
  expect(await screen.findByText("Router status unavailable")).toBeInTheDocument();
  expect(screen.queryByText("Router failed to start")).not.toBeInTheDocument();
});

it("keeps fresh healthy state after a transient health error", async () => {
  render(<RouterPage api={apiWithSnapshot({
    ...healthySnapshot,
    health_error: timeoutError,
  })} onNavigateToAgents={() => {}} />);
  expect(await screen.findByText("Router is healthy")).toBeInTheDocument();
  expect(screen.getByText("Healthy")).toBeInTheDocument();
  expect(screen.getByText("Unable to check upstream health.")).toBeInTheDocument();
});
```

Use existing test API builders and exact current localized warning strings.

- [ ] **Step 2: Run focused React tests and verify failure**

Run: `npm test -- RouterPage.test.tsx` from `desktop/`.

Expected: FAIL because status errors force `failed` and health errors force degraded.

- [ ] **Step 3: Separate status-read and action failures**

Add `"unavailable"` to `ViewState`. Replace the overloaded `failed` state with `actionFailed`; pass a separate status-read condition into `viewState`:

```tsx
if (actionFailed || status?.state === "start_failed" || status?.state === "stale") {
  return "failed";
}
if (!status && statusReadFailed) return "unavailable";
```

When a snapshot has `status_error`, set the load warning but do not set `actionFailed`. Successful status snapshots clear only the load warning. Start/stop action failures continue setting `actionFailed`, and a later successful status clears it under the existing reconciliation rules.

- [ ] **Step 4: Preserve cached health semantics**

Stop overriding observed health from `healthFailed`:

```tsx
const observedHealth = checkingHealth
  ? "checking"
  : healthState(health, now);
```

Keep `healthFailed` only if needed to retain and clear the warning; otherwise remove the redundant state and derive warning behavior from snapshots. The existing 30-second timer must remain unchanged.

- [ ] **Step 5: Add localized unavailable copy**

Add keys to both locale files:

```ts
"router.state.unavailable.title": "Router status unavailable",
"router.state.unavailable.signal": "Status unavailable",
"router.state.unavailable.detail": "The desktop could not read the router status. It will retry automatically.",
```

```ts
"router.state.unavailable.title": "路由状态暂时不可用",
"router.state.unavailable.signal": "状态不可用",
"router.state.unavailable.detail": "桌面端暂时无法读取路由状态，将自动重试。",
```

- [ ] **Step 6: Run React checks**

Run from `desktop/`:

```bash
npm test -- RouterPage.test.tsx
npm run typecheck
npm run static:check
```

Expected: PASS.

- [ ] **Step 7: Commit desktop presentation**

```bash
git add desktop/src/RouterPage.tsx desktop/src/RouterPage.test.tsx desktop/src/locales/en.ts desktop/src/locales/zh-CN.ts
git commit -m "fix: preserve desktop state across poll errors"
```

### Task 6: Align Rust Watchdog and Tray Semantics

**Files:**
- Modify: `desktop/src-tauri/src/manager.rs`
- Modify: `desktop/src-tauri/src/scheduler.rs`
- Modify: `desktop/src-tauri/src/tray.rs`

- [ ] **Step 1: Update and extend failing Rust tests**

Update watchdog expectations and add cached error cases:

```rust
assert_eq!(watchdog("router.status").unwrap(), Duration::from_secs(2));
assert_eq!(watchdog("router.health").unwrap(), Duration::from_secs(13));
```

Add scheduler assertions that `set_status_error` and `set_health_error` leave the previous `status` and `health` values intact. Add a pure tray presentation helper test showing cached `desktop_owned` plus `status_error` remains running/healthy according to cached health.

- [ ] **Step 2: Run focused Rust tests and verify failure**

Run from `desktop/`:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked manager::tests::watchdogs_exceed_manager_deadlines
cargo test --manifest-path src-tauri/Cargo.toml --locked scheduler::tests
cargo test --manifest-path src-tauri/Cargo.toml --locked tray::tests
```

Expected: watchdog and tray tests fail under current behavior.

- [ ] **Step 3: Set the health watchdog to 13 seconds**

Give `router.health` its own 12-second manager budget before the existing one-second watchdog margin:

```rust
"router.health" => 12,
```

Remove it from the five-second method group.

- [ ] **Step 4: Prefer cached tray status over transient errors**

Change `update_poll_snapshot` ordering so an available `snapshot.status` is converted and presented even when `status_error` is present. Call `apply_error` only when `status_error` exists and no cached status is available:

```rust
let Some(status) = snapshot.status.clone() else {
    if snapshot.status_error.is_some() {
        apply_error(app);
    }
    return;
};
```

Keep existing explicit health degradation and stale handling.

- [ ] **Step 5: Run Rust formatting and tests**

Run from `desktop/`:

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

Expected: PASS.

- [ ] **Step 6: Commit Rust changes**

```bash
git add desktop/src-tauri/src/manager.rs desktop/src-tauri/src/scheduler.rs desktop/src-tauri/src/tray.rs
git commit -m "fix: retain cached desktop poll state"
```

### Task 7: Full Verification and Final Review

**Files:**
- Modify only files required to fix failures caused by the preceding tasks.

- [ ] **Step 1: Format Go files**

Run: `gofmt -w main.go main_test.go internal/health/probe.go internal/health/probe_test.go internal/manager/discovery/discovery.go internal/manager/discovery/discovery_test.go internal/manager/app/app.go internal/manager/app/app_test.go internal/manager/protocol/types.go internal/manager/protocol/server_test.go`

Expected: command succeeds.

- [ ] **Step 2: Run full Go verification**

```bash
go test ./...
go vet ./...
make test-shell
test -z "$(gofmt -l .)"
```

Expected: all commands pass.

- [ ] **Step 3: Run full desktop verification**

Run from `desktop/`:

```bash
npm run verify
```

Expected: ESLint, Prettier, TypeScript, Vitest, Vite build, Cargo formatting, and Cargo tests all pass.

- [ ] **Step 4: Inspect the final diff and worktree**

```bash
git status --short --branch
git diff --check
git diff main...HEAD --stat
git log --oneline --decorate -10
```

Expected: only intended Issue 84 files are changed; no secrets, generated packages, or unrelated files are present.

- [ ] **Step 5: Commit any verification-only fixes**

If verification required tracked corrections, stage only those files and commit:

```bash
git commit -m "test: complete health polling regression coverage"
```

If the tree is already clean, do not create an empty commit.
