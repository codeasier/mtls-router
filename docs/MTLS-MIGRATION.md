# Personal mTLS and jump migration runbook

[中文](zh-CN/MTLS-MIGRATION.md) · [Enrollment/CRL operations](ENROLLMENT-OPERATIONS.md) · [Linux CLI](LINUX-CLI.md)

This is a staged operational plan. Repository templates and isolated tests do not
assert that production DNS, CA, OAuth or WireGuard has changed. The linked ops
change must identify its actual hosts and preserve the current
`monitoring/docs/operations/jump2-center-migration.md` order: **business ingress
first, sites/WireGuard second, jump downgrade last**.

## Required operational record

Record verified values for both jump and jump2 before serving new traffic:

| Area | Required evidence |
|---|---|
| Server identity | `DNS:relay.codeasier.top` SAN and the client-pinned server CA; public enrollment SAN/chain separately |
| Client identity | Dedicated offline root/issuing fingerprints, valid root/issuing CRLs, number/digest/expiry |
| Control plane | One fenced signing master, independently persistent journal, dedicated OAuth callback/scopes and stable organization ID |
| Gateways | Exact vhost/template revision, loaded marker/ack, current source to the single master |
| Backend | Each node's verified forward **and return** path, pool members and whether forwarding still crosses jump |
| Rollout | Actual package/tag availability, original updater feed/signature, DNS TTL/cache observation and rollback owner |

Do not reuse an old shared client CA in the new relay vhost. Preserve old IP:9099
and other legacy vhosts independently during migration; do not turn client
verification optional to make onboarding work. Enrollment uses public
`enroll.codeasier.top`; model requests use strict `relay.codeasier.top` with the
original business key and no online auth subrequest.

## Stage A: prepare and force each node independently

1. Prepare both server SANs/trust chains and the new client CA/CRLs before DNS.
   Verify server identity with SNI and hostname checking; a controlled check may
   use public trust only:

   ```sh
   openssl s_client -connect NODE_IP:443 -servername relay.codeasier.top -verify_hostname relay.codeasier.top -verify_return_error -CAfile server-ca.pem
   ```

   This proves server identity only, not authenticated business success.
2. On isolated validation devices, use temporary hosts/dedicated DNS to resolve
   the **same relay hostname** to one node at a time. Do not disable hostname or
   certificate verification. Reuse the same installed personal identity on both
   nodes; entry IP is not part of a device certificate.
3. Confirm valid native authorization/renewal, existing API key/usage/images/SSE,
   request upload streaming, cancellation and unchanged business 401/403/5xx.
   Use the native application's existing key handling, not key values in shell
   arguments/env/logs. Negative identities are controlled test identities, not
   exported end-user keys.
4. Require zero model-path GitCode/enrollment lookups. Verify missing, old shared,
   wrong CA/EKU, expired and revoked identities cannot reach a backend. Keep the
   no-resumption/no-keepalive/no-HTTP2 baseline until equivalent evidence exists.
5. Revoke one of two devices and obtain **both** actual gateway acks with matching
   CRL coverage. New handshakes must reject it while the other device remains
   usable. Partial/expired CRL state is not an acceptable cutover target.

If jump2 reaches the backend through a jump bridge, label this **ingress verified,
still dependent on jump**. It does not prove the site/WG migration is complete.

## Stage B: native package rollout and DNS

Keep `release.codeasier.top`, its server certificate, six-platform feed and
original updater key available. Validate an old desktop can download/verify the
whole new package, then perform one-click enrollment. Preserve API keys, Agent
URLs/files and private identity through subsequent updates. New CLI packages are
verified independently; never overwrite historical Go assets.

Only after both nodes are equivalent and their backend return paths work, move
relay from the verified old node to jump2 with an observed low TTL. Do not add an
untested second A record. Keep the old node serving cached DNS and accepted SSE;
record actual traffic/connection drain beyond TTL instead of shutting down on a
timer. Requests already sent are not retried for the DNS change.

The signing master is independent of relay DNS. A non-signing gateway forwards
only bounded control paths through the restricted pinned-key tunnel; model
traffic stays in the backend pool. Moving the signer is a separate fenced
backup/journal/key recovery procedure, never a second active copy.

## Stage C: sites/WireGuard and retirement

After business ingress is proven off jump, migrate site/WG paths with the ops
runbook's tested forward/return ordering. Validate every site and required client
class; API ingress success cannot stand in for site/WG evidence. Downgrade jump
only after these checks and the retained cached/long-lived traffic have drained.

Retire old shared ingress only after new package availability, adoption,
authorization, renewal and rollback evidence are complete. Remove old shared CA
trust consistently, preserve historical tags and continue the updater domain.

## Rollback without authentication downgrade

Before changing DNS back, prove the old node has the **latest** valid CRLs, the new
personal CA/server SAN and a working backend return path after any WG changes.
Roll back to that strict verified node; do not restore shared credentials,
key-only access, an older revocation set or a stale SQLite snapshot. If signer
continuity cannot be established, stop signing and preserve valid data-plane
operation. Already accepted streams finish normally; an emergency hard disconnect
is a separate explicit operational action.

Attach real member/non-member, two-device, native upgrade, dual-node CRL/DNS,
backend path and WG rollback evidence to the ops change. Missing production or
native-platform evidence remains incomplete even when isolated tests pass.
