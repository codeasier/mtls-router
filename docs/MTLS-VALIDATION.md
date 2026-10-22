# Personal mTLS validation and capacity evidence

[中文](zh-CN/MTLS-VALIDATION.md)

## Local correctness matrix

From the repository root, with nginx, `ps`, `lsof`, Go, Rust and locked
dependencies available (run as the owner of the isolated nginx processes):

```sh
MTLS_TEST_RUST_NATIVE=1 go test ./internal/routerenroll -run TestRealNginxRenewalAndObservedCRLLoad -count=1 -v
go test -race ./internal/routerenroll ./cmd/codeasier-router-enroll -count=1
cargo test --workspace --locked
make test-workflows
```

The opt-in Rust bridge runs the actual `NativeClient` through generated HTTPS
server trust, the repository nginx template, Go enrollment/fake GitCode,
CSR-bound issuance, mTLS business forwarding and signed logout. Fixture private
CA keys remain in memory; only public trust files/loopback locations are passed
to the test child. Test-only endpoint adaptation is absent from production builds.

The matrix also covers non-members, wrong native proof, same-result response
recovery, one certificate on two gateways, invalid CA/EKU/expiry, independent
business statuses, streaming upload/SSE/cancellation, no model-path control
lookups, no session/connection reuse, reload/header timeout windows, partial and
complete leaf revocation, and root authority revocation. User/account/operation
CAS races and the 24-hour near-expiry recovery boundary have focused tests.
The reload regression pauses an old worker and verifies that a new worker/marker
cannot acknowledge publication while the old TCP listener remains. Resuming the
worker permits listener retirement without waiting for established SSE to finish.

## Isolated load reference

```sh
MTLS_TEST_GATEWAY_SAMPLES=200 go test ./internal/routerenroll -run TestRealNginxRenewalAndObservedCRLLoad -count=1 -v
```

`10..1000` samples are permitted. Each request uses a full mTLS handshake and a
synthetic first SSE event. Reports separate baseline, resident SSE and reload
phases, handshake/first-event p95/p99, driver queue, QPS/failures, nginx master
plus direct worker RSS and a CPU snapshot. The Go driver's heap is reported
separately. These are local synthetic measurements, not LLM token latency or an
entry-server budget; `ps` CPU snapshots are not sustained CPU utilization.

## Entry-machine acceptance record

On an isolated environment equivalent to the actual ingress CPU/memory/WG pool,
record worker count, connection/header budgets, CA/CRL sizes and package revision.
Collect sustained CPU/RSS, new-request QPS, queue depth, handshake and real first
token p95/p99 with representative upload sizes and long-lived SSE. Repeat during
membership/renewal peaks and CRL reload, and confirm model control lookups remain
zero. Set a budget only from that machine and workload, not by scaling a laptop's
sample result. Failure/over-budget cases retain the strict authentication and
no-replay baseline; they do not justify disabling CRL or hostname checks.

Native Linux amd64/arm64 no-GUI login/service replacement, native Windows DACL,
six-platform package upgrade, actual GitHub provenance and real organization/
DNS/WG acceptance require their own runs. Missing inputs/evidence stay open in
the implementation task list; a configured workflow or fixture is not acceptance.
