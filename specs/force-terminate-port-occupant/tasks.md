# Force-Terminate Port Occupant Tasks

Tasks are dependency ordered. A task is complete only after its implementation,
tests, and stated verification pass. Product-code execution must not begin until
this specification package is approved.

## Phase 1: Identity and Discovery

- [x] **1.1 Add reusable complete-process identity operations**
  - Add comparison of PID, process start identity, and normalized executable.
  - Add identity-validated signaling for an already complete identity.
  - Do not add a PID-only signal API or weaken managed-router binary checks.
  - Test PID, start identity, executable mismatch, absence, and signal refusal.
  - Verification: `go test ./internal/manager/process`.

- [x] **1.2 Define the occupant identity and native inspection boundary**
  - Add a focused manager package for listener discovery and termination state.
  - Define internal listen address, network, socket identity, process identity,
    and user identity fields.
  - Define fail-closed errors for absent, other-user, incomplete, ambiguous,
    changed, and protected targets.
  - Keep complete identity internal and return only safe presentation metadata.
  - Verification: focused package unit tests.

- [ ] **1.3 Implement Linux listener-owner discovery**
  - Parse `/proc/net/tcp` and `/proc/net/tcp6` listening records.
  - Correlate socket inode to one `/proc/<pid>/fd` owner.
  - Read effective UID and complete process identity from procfs.
  - Reject malformed, inaccessible, wildcard-ambiguous, duplicate, or
    multi-owner results.
  - Add synthetic parser/correlation tests and a native listener test.
  - Verification: native Linux tests and cgo-free Linux amd64/arm64 builds.

- [ ] **1.4 Implement Windows listener-owner discovery**
  - Use `GetExtendedTcpTable` owner-PID listener data for the exact endpoint.
  - Use process-token user SID for ownership and existing process inspection
    for start/executable identity.
  - Reject inaccessible tokens, duplicate owners, unsupported endpoint shapes,
    and incomplete identities.
  - Add injectable native-table and SID tests.
  - Verification: native Windows tests and cgo-free Windows amd64/arm64 builds.

- [x] **1.5 Implement macOS listener-owner discovery**
  - Use Darwin process/socket inspection to enumerate process FDs and exact TCP
    listener metadata.
  - Obtain current-user ownership and reuse complete process inspection.
  - Fail closed on short/unknown native records, permission failures, or
    multiple owners.
  - Add deterministic record-decoding tests and a native listener test.
  - Verification: native macOS tests and cgo-free Darwin amd64/arm64 builds.

## Phase 2: Confirmation and Manager Protocol

- [x] **2.1 Implement occupant inspection tokens**
  - Generate tokens from at least 32 cryptographically secure random bytes.
  - Bind each token to the complete occupant identity and listen address.
  - Keep at most one active token per address in manager memory.
  - Expire tokens after 30 seconds and invalidate them on use, replacement, or
    manager restart.
  - Serialize inspection/termination operations to prevent replay races.
  - Test expiry, replay, supersession, restart loss, and concurrency.
  - Verification: `go test -race ./internal/manager/occupant`.

- [ ] **2.2 Implement protected-target and force-termination behavior**
  - Require discovery to be `unknown_occupant` during inspection and again
    before termination.
  - Reject other-user, desktop, manager, managed-router, and incomplete targets.
  - Consume the token, rediscover the listener, and compare every identity
    field before signaling.
  - Perform final process identity validation immediately before `os.Kill`.
  - Wait at most two seconds for original identity exit and TCP refusal.
  - Never signal a replacement listener and never start the router.
  - Test every no-signal path plus immediate force termination and replacement
    listener behavior.
  - Verification: occupant race tests and native integration tests.

- [x] **2.3 Add manager protocol methods and stable errors**
  - Add `router.inspect_occupant` with a two-second deadline.
  - Add `router.force_terminate_occupant` with a three-second deadline.
  - Accept no inspection params and only `confirmation_token` for termination.
  - Add all eight specified stable occupant error codes.
  - Register handlers and map internal failures to sanitized errors.
  - Ensure diagnostics, state, logs, and unrelated responses contain no token or
    full occupant path.
  - Add method framing, parameter, timeout, sanitization, and error-mapping tests.
  - Verification: `go test -race ./internal/manager/...`.

## Phase 3: Desktop Integration

- [x] **3.1 Bridge occupant operations through Tauri and TypeScript**
  - Add Rust and TypeScript inspection/result types.
  - Add named Tauri inspect and force-terminate commands.
  - Expose frontend APIs that accept no PID or path and submit only the token.
  - Refresh scheduler status after success without invoking router start.
  - Update mock APIs and add Rust/TypeScript request-shape tests.
  - Verification: desktop typecheck, IPC tests, Rust format, and Rust tests.

- [x] **3.2 Add occupied-state recovery UI**
  - Inspect once when entering `unknown_occupant` and support explicit retry.
  - Show process name and PID only for a valid inspection.
  - Keep regular Stop disabled and do not add a tray action.
  - Explain other-user, unverifiable, protected, ambiguous/changed, and
    temporary inspection failures.
  - Discard target details whenever status leaves `unknown_occupant`.
  - Add state and action tests.
  - Verification: focused Router page tests.

- [x] **3.3 Add accessible destructive confirmation**
  - Show process name, PID, complete wrapping executable path, and data-loss
    warning.
  - Focus Cancel by default and support Cancel/Escape before submission.
  - Disable duplicate actions and dismissal during submission.
  - Submit only the opaque token.
  - On success close, refresh, report port release, and leave the router stopped.
  - On changed/expired results clear stale details and require reinspection.
  - Add English and Simplified Chinese copy and responsive danger styling.
  - Verification: frontend static checks, typecheck, tests, and build.

## Phase 4: Documentation and Verification

- [x] **4.1 Update live requirements and paired user documentation**
  - Replace the absolute prohibition on killing unknown occupants with the
    explicit confirmed current-user exception.
  - Preserve the rule that automatic termination, Stop, Quit, and tray actions
    never kill unknown occupants.
  - Document target details, immediate termination risk, no elevation, and
    manual fallback in English and Simplified Chinese.
  - Update the existing desktop acceptance checklist without marking new items
    complete before evidence exists.
  - Verification: documentation claim search plus workflow and shell tests.

- [x] **4.2 Run full repository verification**
  - Format Go, Rust, TypeScript, CSS, and documentation-supported files.
  - Run `go test -race ./...`, `go vet ./...`, and gofmt cleanliness.
  - Run `make test-shell` and `make test-workflows`.
  - Run `npm run verify` from `desktop/`.
  - Cross-build the manager for Darwin, Linux, and Windows amd64/arm64 with
    `CGO_ENABLED=0`.
  - Verify no external process-discovery command dependency was introduced.

- [ ] **4.3 Record native acceptance evidence**
  - On macOS, Linux, and Windows, prove exact listener inspection for a
    disposable current-user process.
  - Prove Cancel sends no termination request and leaves the process alive.
  - Prove confirmation force-terminates the unchanged process and releases the
    endpoint.
  - Prove the router is not automatically started.
  - Record any platform permission limitation as a fail-closed result, not a
    skipped safety requirement.
