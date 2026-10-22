# Enrollment master: persistence and recovery

This is a deployment template for the new personal-device certificate service,
not a claim that production OAuth, CA, domains, CRL delivery or browser enrollment
has been deployed. Native authorization has isolated coverage; real OAuth
application validation and production gateway publication still need acceptance.
See [Chinese](zh-CN/ENROLLMENT-OPERATIONS.md).

## Storage and single-master operation

- Run the foreground `codeasier-router-enroll` under a dedicated Unix account.
  The [unit](../deploy/enroll/codeasier-router-enroll.service) and
  [configuration](../deploy/enroll/enroll.env.example) are examples; replace the
  application/organization IDs with verified values.
- State is private `0700`; database, WAL/SHM, log and key files are `0600`.
  The journal directory is explicitly configured using `--journal-dir` /
  `MTLS_ENROLL_JOURNAL_DIR`. It cannot be inside the state directory, including
  through symlink aliases.
- Provision `/srv/codeasier-enroll-journal` as an independently persistent,
  service-account-owned mount with a separate snapshot/retention policy. The
  example unit requires that mount. A sibling directory on the same rollback
  volume alone is not an independent recovery authority.
- The journal is append-only through the service API. It holds a random store ID,
  sequence/hash chain and deterministic typed SQL changes, including encrypted
  GitCode ciphertext and public certificate results. It is sensitive operational
  state, excluded from diagnostics. Do not include it in ordinary DB rollback.
- A record is fsynced before its SQLite projection/checkpoint commits. A crash
  in between is recovered by replay, preserving the original serial and every
  revocation decision, even one not yet present in a gateway CRL. Replies follow
  both commits. An interrupted response is not evidence of no issuance/revocation.
- File locks serialize service/admin commits. The signing lease also belongs to
  the journal directory, so a second state path cannot start a second signer.
  Before moving hosts, stop and fence the old host; local locks are not distributed
  HA. No standby may sign from a copied authority.

For a **new deployment only**, as the service account:

```sh
codeasier-router-enroll journal-init --journal-dir /srv/codeasier-enroll-journal
```

Existing authority files are never overwritten. Normal startup never recreates a
missing authority. Do not run initialization to fix a lost journal. The hash
chain detects damage/pairing errors, not a privileged operator rolling back both
copies; preserve and verify the latest independent authority operationally.

## Offline root and restricted online issuing CA

Use a dedicated client-auth CA, separate from server TLS, old shared client CA and
updater signing. Never deploy the offline root key to the service, nginx or CI.
Keep root CA index/serial/CRL-number files with its encrypted key under offline
backup. Start in a new private working directory with `umask 077`; create an empty
`index.txt`, initialize both `serial` and `crlnumber` to hexadecimal `1000`, and
create `newcerts/`.
Copy [offline-root.cnf](../deploy/enroll/offline-root.cnf) into that directory.

On the offline root host:

```sh
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -aes-256-cbc -out root-key.pem
openssl req -new -x509 -sha256 -days 3650 -config offline-root.cnf -extensions root_ca_ext -key root-key.pem -out root-cert.pem
```

On the authorized issuing host, generate the online key and a public CSR (only
the CSR goes to the offline root; the key stays private):

```sh
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out issuing-key.pem
openssl req -new -sha256 -key issuing-key.pem -subj /CN=CodeasierRouter-Personal-Issuer -out issuing.csr
```

Back on the offline root host:

```sh
openssl ca -config offline-root.cnf -extensions issuing_ca_ext -days 1095 -in issuing.csr -out issuing-cert.pem -notext
openssl ca -config offline-root.cnf -gencrl -out root-crl.pem
```

Return only the public issuing/root certificates and root CRL. The online service
accepts a matching unencrypted PKCS#8 P-256 issuing key, `pathLen=0`, exactly
`keyCertSign,cRLSign` and `clientAuth` EKU. Leaf identities are constructed by code;
CSR subject/SAN/CA privileges are not copied. Root CRL freshness and subsequent
issuing CRL publication remain separate operational duties.

Provision the independent 32-byte AES token key from a file, not argv/env; for a
new instance `openssl rand -out token-key 32` under `umask 077` is suitable. Put the
dedicated OAuth secret in its own private file using the approved secret manager.
Never print either secret or include it in model traffic/support bundles.

## Online backup and restore

Create a private backup directory owned by the service account, then run:

```sh
codeasier-router-enroll admin backup --state-dir /var/lib/codeasier-router-enroll --journal-dir /srv/codeasier-enroll-journal --output /private-backups/enroll-unique.sqlite3
```

This holds the commit lock across catch-up and `VACUUM INTO`, fsyncs a new `0600`
backup and prints a public store-ID/sequence/hash/checksum manifest. It refuses to
overwrite a backup. Preserve the manifest for file integrity; it does not replace
the latest journal. Keep independently encrypted/backed-up copies of the token
key, issuing key/cert, root public chain and offline root state. Do not generate
replacement keys when restoring the same identity.

1. Stop enrollment and admin writers, fence the old signing host, and preserve the
   currently trusted latest journal independently. Keep the active gateway CRLs;
   do not roll their versions back with the database.
2. Verify backup integrity, ownership, modes, token key, issuing chain and journal
   provenance. Restore the consistent SQLite backup into an empty private state
   directory, without old WAL/SHM sidecars; restore no journal from that backup.
3. Use the current independent journal directory and the original key files.
   Start only one service. It verifies store ID, schema, checkpoint and the hash
   chain, then replays missing committed changes before readiness. A completely
   lost DB can also be rebuilt from the complete authority plus the original keys.
4. Compare public admin certificate/revocation summaries, latest checkpoint and
actual gateway CRL versions/acks before restoring control-plane availability.
    A local decision remains `publication=pending` until publication is confirmed.
5. Missing/damaged/partial/wrong-store logs, a DB ahead of its authority, unsafe
   paths or wrong keys stop recovery. Preserve evidence and obtain the verified
   latest authority. If that is impossible, stop original-CA signing and use an
   explicit CA incident-recovery procedure; never reset a log or lower validation.

The current schema is version 5; journal-anchored versions 2–4 are migrated through
the same commit protocol. Earlier unanchored development databases are
rejected; they cannot prove continuity and are not automatically adopted. Record
an explicit migration/recovery decision rather than blessing an old snapshot.

## Renewal and complete 24-hour result recovery

New renewal requests use exact `POST https://relay.codeasier.top/enroll/renew`.
Only the separate gateway Unix socket accepts this route. The public handler
cannot turn caller-supplied certificate headers into issuance authority. Grant the
nginx worker only access to the socket group and its `0750` runtime directory;
state and key directories remain `0700`. The socket is `0660`, and an occupied or
stale socket is not silently removed.

Renewal rotates the local key, binds operation UUID/new CSR and freshly checks
membership. The 7-day window, single successor and user/token/device versions are
checked again at commit. Background membership checks target 300 seconds;
unavailable GitCode does not mean membership loss. Refresh tokens stay encrypted,
including rotations followed by a temporary membership lookup failure.

`POST https://enroll.codeasier.top/enroll/renew/recover` only reads a committed
public result. It accepts a fresh, purpose/origin/path-separated signature by the
predecessor key, binding issuer, serial, operation and CSR DER SHA-256. The device
keeps both pending keys privately until activation or the recovery deadline.
The full 24 hours start at commit even if the predecessor expires sooner. The
expired predecessor never becomes acceptable to model TLS or new issuance.
Revocation, disabled devices, policy changes or revoked successors still deny
recovery. Unknown operations and expired recovery windows never re-sign.

Device logout uses `/enroll/device/revoke` with its own domain-separated signature.
It affects only the bound device. The response remains `revocation_pending` until
the configured gateways have loaded the complete revocation set; local client
disablement and remote confirmation are separate states.

## Signed CRLs, two gateways and observed acknowledgments

A valid offline-root CRL revoking the issuing CA is **denial material**, not a
malformed update to ignore. Its additive revocations are journal-pinned before
new signing is blocked. The publisher can distribute it with the last still-valid
issuing CRL without using the revoked signing key; gateways load it and deny all
leaves of that issuer. Later CRL numbers cannot remove a remembered root
revocation. Authority replacement remains an explicit incident procedure.

The gateway emits a closed certificate-rejection JSON/marker, hides any
backend-supplied copy of that marker and leaves business statuses unchanged.
Native diagnostics distinguish rejection from confirmed revocation and guard
feedback by generation/serial, so a late failure cannot taint a new account.

1. Install the current offline `root-crl.pem` and a private `0600` two-entry
   [gateway configuration](../deploy/enroll/gateways.json.example) on the signer.
   Set `MTLS_ENROLL_ROOT_CRL_FILE` and `MTLS_ENROLL_GATEWAYS_FILE`. Root CRL
   signature, expiry, issuing-CA revocation and monotonic number/digest are
   checked. Invalid root material blocks new signing; it does not loosen TLS.
2. Provision each gateway with the pinned **public** root/issuing certificates,
   the service binary and root-owned `/var/lib/codeasier-router-gateway` (`0700`).
   No issuing key or OAuth/token key is copied there. Install and adapt the
   [nginx template](../deploy/nginx/codeasier-personal.conf.example), keeping old
    shared-CA vhosts isolated and replacing server certificates/backend routing.
   The loader also requires `ps` and `lsof`, and must run as root or own the nginx
   master/workers so descriptor visibility is not mistaken for retirement. Its
   `--nginx-pid-file` / `MTLS_GATEWAY_NGINX_PID_FILE` defaults to `/run/nginx.pid`
   and must match the PID file in the actual nginx configuration.
3. Pin both SSH host keys in the signer's service-account `known_hosts`. A
   dedicated `codeasier-crl` account must be restricted to the fixed loader;
   configure a root-owned forced command and an exact sudo rule for
   `/usr/local/libexec/codeasier-router-enroll gateway-load --id jump` (or
   `jump2`). Do not grant an arbitrary command, argument wildcard or login shell.
   The loader reads a bounded public JSON bundle from stdin and emits only an ack.
4. Initial provisioning may install valid CRL/marker files but cannot ack until
   nginx is running. Start nginx after its configuration validates, then retry
   the same version. Normal publication refreshes hourly and promptly after new
   revocations. Both issuing and root CRLs are installed in `current-crl.pem`.
5. The loader fsyncs `high-water.json` before installation, validates signatures,
   chain/issuer, dates and number/digest, then atomically installs files and runs
   `nginx -t` / reload. A failed config test preserves the old files but not an
   older high-water mark. After a reload attempt, valid new material is retained
   for retry rather than rolled back.
6. Before acknowledging, the loader snapshots the master's old workers and waits
   until none retains a TCP listening socket. nginx may start a new worker before
   telling old workers to quit, so a new marker alone is insufficient. Established
   SSE sockets may remain; the loader does not wait for those workers to exit.
   Failed or timed-out inspection leaves publication unconfirmed. Ack then requires
   a fresh TCP request to **loopback-only** `127.0.0.1:19102/crl-loaded`
   to observe the exact issuer/number/digest/root-digest marker compiled into the
   same nginx generation as `ssl_crl`. Neither a file copy nor a successful signal
   suffices. Never expose or proxy this endpoint publicly.

The signer persists independent acks, checks coverage of all current revocation
decisions and reports `pending`/`loaded`, each gateway and `target_exceeded` after
the 5-minute revocation target is missed. An hourly refresh of already-loaded
revocations is not falsely classified as a newly missed revocation target.

```sh
codeasier-router-enroll admin publication --state-dir /var/lib/codeasier-router-enroll --journal-dir /srv/codeasier-enroll-journal --issuer-cert-file /etc/codeasier-router-enroll/issuing-cert.pem --root-crl-file /etc/codeasier-router-enroll/root-crl.pem --gateways-file /etc/codeasier-router-enroll/gateways.json
```

Non-signing nodes use the [forwarding unit](../deploy/enroll/codeasier-enroll-forward.service.example)
to reach the same signing master through pinned-key SSH: only loopback 19101 and
the renewal Unix socket are forwarded. On the master restrict the SSH account to
that TCP destination and the gateway socket group, with no shell or private
state/key access. Do not start a second signer. Model traffic stays in nginx's
backend pool and never enters this control-plane tunnel.

Keep gateway high-water state and latest public material through restarts. Do not
restore an older gateway snapshot; recover from the signer's latest publication.
Isolated tests cover partial/fabricated acks, interrupted loads and a real nginx
mTLS renewal/reload/revocation refusal. Actual two-node production/DNS acceptance
remains a separate task.
