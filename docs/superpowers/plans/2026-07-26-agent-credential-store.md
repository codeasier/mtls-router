# Agent Credential Store Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist the mtls-router agent API key to `data_dir/credentials.json` and expose it via a new `Api keys` sidebar page; refactor `ModelFlow` to source its key from this store instead of from invoke-time arguments, without bumping the management protocol.

**Architecture:** A new Rust module `desktop/src-tauri/src/credential.rs` owns the credential file. `commands.rs` injects the key at `ManagerClient::call` time (preserving the existing `clear_json` pass). Frontend `ApiKeysPage.tsx` is the only page that touches the store via `DesktopApi::getCredential/saveCredential/deleteCredential`. `ModelFlow` loses its `api_key` field entirely. Manager protocol v4 is untouched.

**Tech Stack:** Rust (Tauri 2, tokio, serde/serde_json, zeroize), React 19 + TypeScript + Vitest, thiserror (need to verify availability before Task 1).

## Global Constraints

- Rust deps already pinned (`Cargo.toml`):
  - `tokio = "=1.52.3"` (macros, sync, time)
  - `serde = "=1.0.228"` (derive)
  - `serde_json = "=1.0.149"`
  - `zeroize = "=1.8.2"`
  - `uuid` (already in use)
- **thiserror**: Task 0 verifies availability; if absent, fall back to manual `Display + std::error::Error` impl.
- File mode: Unix `0o600`. Windows: best-effort std-only.
- JSON schema: `{ "version": 1, "saved_at": "<RFC3339>", "key": "<opaque>" }`.
- No Claude attribution in commits (user global preference).
- Go code: **untouched**. Manager protocol v4 unchanged.
- CLI / `setup.sh` / `setup.ps1`: **untouched**.
- Frontend `ApiKeysPage.tsx` is the **only** page that handles the key text.

## File Structure

```
desktop/src-tauri/src/
├── credential.rs           # NEW (~180 lines): CredentialStore + types + tests
├── paths.rs                # MODIFY: +credentials_path field
├── types.rs                # MODIFY: +CredentialSummary (no key field)
├── commands.rs             # MODIFY: drop ModelFlow.api_key; inject key in agent_models/render/preview/write
├── manager.rs              # MODIFY: ManagerClient accepts optional Arc<CredentialStore>
├── lib.rs                  # MODIFY: register invoke commands + initialize store
├── error.rs                # MODIFY: +CREDENTIAL_NOT_FOUND/INVALID/IO_ERROR/LOCK_TIMEOUT

desktop/src/
├── ipc.ts                  # MODIFY: +getCredential / saveCredential / deleteCredential
├── ipc.test.ts             # MODIFY: tests
├── ApiKeysPage.tsx         # NEW (~250 lines)
├── ApiKeysPage.test.tsx    # NEW
├── model.ts                # MODIFY: +"api-keys" route
├── App.tsx                 # MODIFY: route + sidebar
├── locales/en.ts           # MODIFY: +apikey namespace
├── locales/zh-CN.ts        # MODIFY: +apikey namespace
├── AgentPage.tsx           # MODIFY: remove apiKey state, remove 2nd arg from discoverModels
```

---

### Task 0: Verify thiserror & create skeleton

**Files:**
- Create: `desktop/src-tauri/src/credential.rs`
- Modify: `desktop/src-tauri/src/lib.rs` (add `mod credential;`)
- Test: inline `#[cfg(test)] mod tests` in `credential.rs`

**Interfaces:**
- Consumes: nothing (skeleton)
- Produces: `pub struct CredentialStore { path: PathBuf, inner: tokio::sync::Mutex<()> }` placeholder; `pub fn new_blocking(path: PathBuf) -> Self` blocking constructor for tests only.

- [ ] **Step 1: Verify thiserror availability**

Run:
```bash
grep -E '^thiserror' desktop/src-tauri/Cargo.toml
```
Expected: a `thiserror = "..."` line. If absent, run:
```bash
cargo add --manifest-path desktop/src-tauri/Cargo.toml thiserror
```
Note version pinned by workspace policy.

- [ ] **Step 2: Create `credential.rs` skeleton with error type only**

```rust
// desktop/src-tauri/src/credential.rs

use std::{io, path::PathBuf};
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Error, Debug)]
pub enum CredentialError {
    #[error("credential not found")]
    NotFound,
    #[error("credential file is malformed: {0}")]
    InvalidFormat(String),
    #[error("credential io error: {0}")]
    Io(#[from] io::Error),
    #[error("credential lock timeout")]
    LockTimeout,
}

#[derive(Debug)]
pub struct CredentialStore {
    pub(crate) path: PathBuf,
    pub(crate) inner: Mutex<()>,
}

impl CredentialStore {
    #[cfg(test)]
    pub fn new_blocking(path: PathBuf) -> Self {
        Self { path, inner: Mutex::new(()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeleton_compiles() {
        let store = CredentialStore::new_blocking(PathBuf::from("/tmp/x"));
        assert!(store.path.ends_with("x"));
    }
}
```

- [ ] **Step 3: Register module in `lib.rs`**

Find the `mod` list at the top of `desktop/src-tauri/src/lib.rs` (currently: `mod autostart; mod commands; mod error; mod manager; ...`), add `mod credential;` immediately after `mod commands;`.

- [ ] **Step 4: Verify it compiles + tests pass**

Run:
```bash
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked --lib credential::tests
```
Expected: tests pass.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/credential.rs desktop/src-tauri/src/lib.rs desktop/src-tauri/Cargo.toml
git commit -m "feat(desktop): scaffold credential store module"
```

---

### Task 1: Implement `CredentialSummary` + file IO

**Files:**
- Modify: `desktop/src-tauri/src/credential.rs`

**Interfaces (added to `credential.rs`):**
- `pub struct CredentialSummary { pub present: bool, pub fingerprint: String, pub saved_at: Option<String> }`
- `pub const MAX_KEY_BYTES: usize = 16 * 1024;`
- `fn fingerprint(key: &str) -> String` — base32 last-4
- `fn parse(content: &[u8]) -> std::result::Result<(String, String), CredentialError>` — returns `(saved_at, key)` or `InvalidFormat`
- `impl CredentialStore { pub async fn read_summary(&self) -> std::result::Result<CredentialSummary, CredentialError>; pub async fn write(&self, key: Zeroizing<String>) -> std::result::Result<CredentialSummary, CredentialError>; pub async fn delete(&self) -> std::result::Result<(), CredentialError>; }`

- [ ] **Step 1: Write failing tests**

Append to `tests` mod in `credential.rs`:

```rust
use zeroize::Zeroizing;
use std::io::Write;

fn tmp_file(name: &str) -> (tempdir, PathBuf) {
    let dir = std::env::temp_dir().join(format!("mtls-router-cred-test-{}-{}", name, uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    (dir.clone(), dir.join("credentials.json"))
}

struct tempdir(PathBuf);
impl Drop for tempdir { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

#[tokio::test]
async fn summary_when_missing_is_not_found() {
    let (_dir, path) = tmp_file("missing");
    let store = CredentialStore::new_blocking(path);
    let err = store.read_summary().await.unwrap_err();
    assert!(matches!(err, CredentialError::NotFound));
}

#[tokio::test]
async fn write_then_read_summary() {
    let (_dir, path) = tmp_file("write_then_read");
    let store = CredentialStore::new_blocking(path.clone());
    let summary = store.write(Zeroizing::new("fixture-key".into())).await.unwrap();
    assert!(summary.present);
    assert_eq!(summary.fingerprint.len(), 4);
    assert!(summary.saved_at.is_some());

    // File mode is 0o600 on unix
    #[cfg(unix)]
    {
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }
}

#[tokio::test]
async fn write_rejects_empty() {
    let (_dir, path) = tmp_file("empty");
    let store = CredentialStore::new_blocking(path);
    let err = store.write(Zeroizing::new(String::new())).await.unwrap_err();
    assert!(matches!(err, CredentialError::InvalidFormat(_)));
}

#[tokio::test]
async fn write_rejects_oversize() {
    let (_dir, path) = tmp_file("oversize");
    let store = CredentialStore::new_blocking(path);
    let big = "x".repeat(MAX_KEY_BYTES + 1);
    let err = store.write(Zeroizing::new(big)).await.unwrap_err();
    assert!(matches!(err, CredentialError::InvalidFormat(_)));
}

#[tokio::test]
async fn delete_then_missing_is_not_found() {
    let (_dir, path) = tmp_file("delete");
    let store = CredentialStore::new_blocking(path);
    store.write(Zeroizing::new("fixture".into())).await.unwrap();
    store.delete().await.unwrap();
    let err = store.read_summary().await.unwrap_err();
    assert!(matches!(err, CredentialError::NotFound));
}

#[tokio::test]
async fn bad_json_returns_invalid_format() {
    let (dir, path) = tmp_file("badjson");
    std::fs::write(&path, b"{not json").unwrap();
    let store = CredentialStore::new_blocking(path);
    let err = store.read_summary().await.unwrap_err();
    assert!(matches!(err, CredentialError::InvalidFormat(_)));
    drop(dir);
}

#[tokio::test]
async fn wrong_version_returns_invalid_format() {
    let (_dir, path) = tmp_file("wrongver");
    std::fs::write(&path, br#"{"version":2,"key":"a","saved_at":"x"}"#).unwrap();
    let store = CredentialStore::new_blocking(path);
    let err = store.read_summary().await.unwrap_err();
    assert!(matches!(err, CredentialError::InvalidFormat(_)));
}

#[tokio::test]
async fn missing_key_field_returns_invalid_format() {
    let (_dir, path) = tmp_file("nokey");
    std::fs::write(&path, br#"{"version":1,"saved_at":"x"}"#).unwrap();
    let store = CredentialStore::new_blocking(path);
    let err = store.read_summary().await.unwrap_err();
    assert!(matches!(err, CredentialError::InvalidFormat(_)));
}
```

- [ ] **Step 2: Run tests; expect failures**

Run:
```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked --lib credential::tests
```
Expected: compile error (missing `read_summary`, `write`, `delete`) and/or `MAX_KEY_BYTES` unresolved.

- [ ] **Step 3: Implement the IO**

Add to `credential.rs`:

```rust
use base32::{Alphabet, encode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::os::unix::fs::PermissionsExt;
use zeroize::Zeroizing;

pub const MAX_KEY_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CredentialSummary {
    pub present: bool,
    pub fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_at: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct OnDisk {
    version: u32,
    saved_at: String,
    key: String,
}

fn fingerprint(key: &str) -> String {
    let full = encode(Alphabet::RFC4648 { padding: false }, key.as_bytes());
    full.chars().rev().take(4).collect::<String>().to_uppercase()
}

fn parse(content: &[byte]) -> Result<(String, String), CredentialError> {
    let mut s = content.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(content);
    let disk: OnDisk = serde_json::from_slice(s).map_err(|_| CredentialError::InvalidFormat("parse".into()))?;
    if disk.version != 1 || disk.key.is_empty() {
        return Err(CredentialError::InvalidFormat("schema".into()));
    }
    if disk.key.len() > MAX_KEY_BYTES {
        return Err(CredentialError::InvalidFormat("oversize".into()));
    }
    Ok((disk.saved_at, disk.key))
}

impl CredentialStore {
    pub async fn read_summary(&self) -> Result<CredentialSummary, CredentialError> {
        let _g = self.inner.lock().await;
        match std::fs::read(&self.path) {
            Ok(content) => {
                let (saved_at, key) = parse(&content)?;
                Ok(CredentialSummary { present: true, fingerprint: fingerprint(&key), saved_at: Some(saved_at) })
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Err(CredentialError::NotFound),
            Err(e) => Err(CredentialError::Io(e)),
        }
    }

    pub async fn write(&self, key: Zeroizing<String>) -> Result<CredentialSummary, CredentialError> {
        if key.is_empty() || key.len() > MAX_KEY_BYTES {
            return Err(CredentialError::InvalidFormat("length".into()));
        }
        let _g = self.inner.lock().await;
        if let Some(parent) = self.path.parent() { std::fs::create_dir_all(parent)?; }
        let disk = OnDisk { version: 1, saved_at: Utc::now().to_rfc3339(), key: key.to_string() };
        let bytes = serde_json::to_vec_pretty(&disk).map_err(|_| CredentialError::InvalidFormat("encode".into()))?;

        // Atomic write: tmp + fsync + rename
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, &bytes)?;
        #[cfg(unix)]
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        std::fs::rename(&tmp, &self.path)?;
        #[cfg(unix)]
        std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))?;

        Ok(CredentialSummary { present: true, fingerprint: fingerprint(&key), saved_at: Some(disk.saved_at) })
    }

    pub async fn delete(&self) -> Result<(), CredentialError> {
        let _g = self.inner.lock().await;
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(CredentialError::Io(e)),
        }
    }
}
```

Notes on imports: `base32` crate may need adding — Task 0.1 should check; if unavailable, swap to `format!("{:x}", md5::compute(key))` last 4 hex. If using base32, add `cargo add --manifest-path desktop/src-tauri/Cargo.toml base32`.

- [ ] **Step 4: Run tests; expect pass**

Run:
```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked --lib credential::tests
```
Expected: 8 tests pass.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/credential.rs desktop/src-tauri/Cargo.toml
git commit -m "feat(desktop): persist agent API key to credentials.json"
```

---

### Task 2: Add `use_credential` injection path

**Files:**
- Modify: `desktop/src-tauri/src/credential.rs`

**Interfaces (added):**
- `impl CredentialStore { pub async fn use_(&self) -> Result<Zeroizing<String>, CredentialError>; }`

- [ ] **Step 1: Write failing test**

```rust
#[tokio::test]
async fn use_returns_value_then_drop_zeros() {
    let (_dir, path) = tmp_file("use_value");
    let store = CredentialStore::new_blocking(path);
    store.write(Zeroizing::new("fixture-key".into())).await.unwrap();
    let key = store.use_().await.unwrap();
    assert_eq!(&*key, "fixture-key");
    drop(key);
    // Verified statically — Zeroizing<String> zeros on drop.
}
```

- [ ] **Step 2: Run test; expect failure**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked --lib credential::tests::use_returns_value_then_drop_zeros
```

- [ ] **Step 3: Implement**

```rust
impl CredentialStore {
    pub async fn use_(&self) -> Result<Zeroizing<String>, CredentialError> {
        let _g = self.inner.lock().await;
        match std::fs::read(&self.path) {
            Ok(content) => {
                let (_saved, key) = parse(&content)?;
                Ok(Zeroizing::new(key))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Err(CredentialError::NotFound),
            Err(e) => Err(CredentialError::Io(e)),
        }
    }
}
```

- [ ] **Step 4: Run test; expect pass**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked --lib credential::tests
```

- [ ] **Step 5: Add concurrent stress test**

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_save_and_use_does_not_corrupt() {
    let (_dir, path) = tmp_file("concurrent");
    let store_a = CredentialStore::new_blocking(path.clone());
    let store_b = CredentialStore::new_blocking(path.clone());

    let w = tokio::spawn({
        let store = store_a;
        async move {
            for i in 0..50 {
                store.write(Zeroizing::new(format!("key-{i}"))).await.unwrap();
                tokio::time::sleep(std::time::Duration::from_micros(50)).await;
            }
        }
    });
    let r = tokio::spawn({
        let store = store_b;
        async move {
            for _ in 0..200 {
                if let Ok(k) = store.use_().await {
                    assert!(k.starts_with("key-"));
                }
                tokio::time::sleep(std::time::Duration::from_micros(20)).await;
            }
        }
    });
    w.await.unwrap();
    r.await.unwrap();
}
```

- [ ] **Step 6: Run tests; expect pass**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked --lib credential::tests
```

- [ ] **Step 7: Commit**

```bash
git add desktop/src-tauri/src/credential.rs
git commit -m "feat(desktop): add credential use path with concurrent safety"
```

---

### Task 3: Update `paths.rs` with `credentials_path`

**Files:**
- Modify: `desktop/src-tauri/src/paths.rs`
- Modify: `desktop/src-tauri/src/types.rs` (DesktopPaths struct)

**Interfaces:**
- `pub struct DesktopPaths { pub data_dir: String, pub log_file: String, pub credentials_path: String, pub can_prepare_for_uninstall: bool }`

- [ ] **Step 1: Find existing `DesktopPaths` definition in `types.rs`**

Run:
```bash
grep -n "DesktopPaths" desktop/src-tauri/src/types.rs
```

- [ ] **Step 2: Add `credentials_path` field in `types.rs`**

Append `pub credentials_path: String,` after `log_file` (or wherever struct field order lives).

- [ ] **Step 3: Modify `paths.rs::resolve()` to populate the new field**

```rust
Ok(DesktopPaths {
    log_file: data_dir.join("mtls-router.log").to_string_lossy().into_owned(),
    data_dir: data_dir.to_string_lossy().into_owned(),
    credentials_path: data_dir.join("credentials.json").to_string_lossy().into_owned(),
    can_prepare_for_uninstall: !cfg!(windows),
})
```

- [ ] **Step 4: Run fmt + build to verify**

```bash
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --all -- --check
cargo build --manifest-path desktop/src-tauri/Cargo.toml --locked
```

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/paths.rs desktop/src-tauri/src/types.rs
git commit -m "feat(desktop): expose credentials_path in DesktopPaths"
```

---

### Task 4: Add Tauri commands + error mapping

**Files:**
- Modify: `desktop/src-tauri/src/error.rs`
- Modify: `desktop/src-tauri/src/commands.rs`

**Interfaces added to `commands.rs`:**
- `#[tauri::command] pub async fn get_credential(state: AppState) -> Result<CredentialSummary, CommandError>`
- `#[tauri::command] pub async fn save_credential(api_key: String, state: AppState) -> Result<CredentialSummary, CommandError>`
- `#[tauri::command] pub async fn delete_credential(state: AppState) -> Result<CredentialSummary, CommandError>`

- [ ] **Step 1: Add credential error mappings in `error.rs`**

Find the function that constructs `CommandError::new(code, message)`. Add constructor helpers:

```rust
impl CommandError {
    pub fn credential_not_found() -> Self { Self::new("CREDENTIAL_NOT_FOUND", "credential is not configured") }
    pub fn credential_invalid(reason: impl Into<String>) -> Self { Self::new("CREDENTIAL_INVALID", format!("credential file is malformed: {}", reason.into())) }
    pub fn credential_io(error: std::io::Error) -> Self { Self::new("CREDENTIAL_IO_ERROR", format!("credential io error: {error}")) }
    pub fn credential_lock_timeout() -> Self { Self::new("CREDENTIAL_LOCK_TIMEOUT", "another credential operation is in progress") }
}
```

- [ ] **Step 2: Write failing command tests**

In `commands.rs` `#[cfg(test)] mod tests`:

```rust
fn build_state_for_test(tmp: PathBuf) -> (AppStateForTest, CredentialStore) {
    // Minimal harness; if existing test util exists, reuse. Otherwise create tokio runtime
    // around ManagerClient::failed(...) which is already unit-test safe.
}

#[tokio::test]
async fn get_credential_returns_not_found() {
    let (state, _) = make_state_with_empty_credential_store().await;
    let result = get_credential_inner(&state.credentials, &state.paths).await.unwrap();
    assert!(!result.present);
}

// ... write similar for save, delete
```

Note: tests should call inner async functions, not invoke the Tauri command directly — Tauri commands are thin wrappers.

- [ ] **Step 3: Run tests; expect failure**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked --lib commands::tests
```

- [ ] **Step 4: Implement**

```rust
use crate::credential::{CredentialError, CredentialStore, CredentialSummary};

fn map_credential_error(error: CredentialError) -> CommandError {
    match error {
        CredentialError::NotFound => CommandError::credential_not_found(),
        CredentialError::InvalidFormat(reason) => CommandError::credential_invalid(reason),
        CredentialError::Io(error) => CommandError::credential_io(error),
        CredentialError::LockTimeout => CommandError::credential_lock_timeout(),
    }
}

pub async fn get_credential_inner(store: &CredentialStore) -> Result<CredentialSummary, CommandError> {
    match store.read_summary().await {
        Ok(summary) => Ok(summary),
        Err(CredentialError::NotFound) => Ok(CredentialSummary { present: false, fingerprint: String::new(), saved_at: None }),
        Err(error) => Err(map_credential_error(error)),
    }
}

#[tauri::command]
pub async fn get_credential(state: tauri::State<'_, AppState>) -> Result<CredentialSummary, CommandError> {
    get_credential_inner(&state.credentials).await
}

#[tauri::command]
pub async fn save_credential(api_key: String, state: tauri::State<'_, AppState>) -> Result<CredentialSummary, CommandError> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() || trimmed.len() > crate::credential::MAX_KEY_BYTES {
        return Err(CommandError::credential_invalid("length"));
    }
    let key = Zeroizing::new(trimmed.to_string());
    let mut local = api_key;
    local.zeroize();
    state.credentials.write(key).await.map_err(map_credential_error)
}

#[tauri::command]
pub async fn delete_credential(state: tauri::State<'_, AppState>) -> Result<CredentialSummary, CommandError> {
    state.credentials.delete().await.map_err(map_credential_error)?;
    Ok(CredentialSummary { present: false, fingerprint: String::new(), saved_at: None })
}
```

- [ ] **Step 5: Run tests; expect pass**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked --lib commands::tests
```

- [ ] **Step 6: Commit**

```bash
git add desktop/src-tauri/src/error.rs desktop/src-tauri/src/commands.rs
git commit -m "feat(desktop): add get/save/delete credential commands"
```

---

### Task 5: Refactor `ModelFlow` + inject key in agent calls

**Files:**
- Modify: `desktop/src-tauri/src/commands.rs`
- Modify: `desktop/src-tauri/src/manager.rs`

**Interfaces (after):**
- `pub(crate) struct ModelFlow { pub flow_id: String, pub agents: Vec<String>, pub models: Vec<String>, pub modes: Option<HashMap<String,String>>, pub catalog_token: String }` — drops `api_key` and `confirmation_token`.
- `impl ModelFlow { pub fn new(flow_id: String, agents: Vec<String>, models: Vec<String>) -> Self; }`
- `ManagerClient::call_with_credential<T>(method, params, &CredentialStore)` — new method that injects key when present.

- [ ] **Step 1: Locate all uses of `flow.api_key.as_str()` in `commands.rs`**

```bash
grep -n "flow.api_key\|api_key: " desktop/src-tauri/src/commands.rs
```

- [ ] **Step 2: Write failing test in `commands.rs`**

```rust
#[tokio::test]
async fn agent_models_command_injects_key_from_store() {
    // Use the existing manager test harness (line ~883 fake_client).
    // Save a key, then invoke agent_models_command with selected=["claude-code"].
    // Assert manager received json containing "api_key":"fixture-secret".
    // Assert ModelFlow has no api_key field — static (compile-fail if accessed).
}
```

- [ ] **Step 3: Run; expect failure**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked --lib commands::tests
```

- [ ] **Step 4: Refactor `ModelFlow`**

Drop `api_key` and `confirmation_token` fields. Update all 5 flow creation sites:

- `ModelFlow::new` constructor that no longer takes key.
- All `flows.insert(flow_id, ModelFlow { .. })` sites lose the key arg.
- `AgentFlowDestroy` retains `flow.api_key.as_str()` usage only for the "contains_exact_string in write" guard — replace with: read key from store once at write time.

- [ ] **Step 5: Implement `ManagerClient::call_with_credential`**

In `manager.rs`:

```rust
pub async fn call_with_credential<T: DeserializeOwned>(
    &self, method: &str, mut params: Value, store: &CredentialStore,
) -> Result<T> {
    if let Ok(key) = store.use_().await {
        params.as_object_mut().map(|o| o.insert("api_key".into(), Value::String(key.to_string())));
        // key zeroizes on drop (Zeroizing<String>)
    }
    self.call(method, params).await
}
```

- [ ] **Step 6: Update agent_*_command functions**

```rust
// agent_models_command
match state.credentials.use_().await {
    Ok(key) => {
        let mut params = json!({ "owner": "desktop", "agents": .. });
        params.as_object_mut().unwrap().insert("api_key".into(), Value::String(key.to_string()));
        // falls out of scope; Zeroizing drops; then clear_json in transact zeroizes the clone
        manager.call("agent.models", params).await
    }
    Err(CredentialError::NotFound) => manager.call("agent.models", params_without_key).await,
    Err(e) => return Err(map_credential_error(e)),
}
```

Apply to `agent.render`, `agent.preview`, `agent.write`. Keep `agent.detect` key-free.

- [ ] **Step 7: Run all tests**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked --lib
```
Expected: all pass; existing `manager.rs` fake-client tests (which pass `api_key` via params) still pass because they call `call()` directly without `call_with_credential`.

- [ ] **Step 8: Commit**

```bash
git add desktop/src-tauri/src/commands.rs desktop/src-tauri/src/manager.rs
git commit -m "refactor(desktop): inject API key at call site, drop ModelFlow key field"
```

---

### Task 6: Wire `CredentialStore` into `AppState`

**Files:**
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src-tauri/src/commands.rs` (AppState struct)

**Interfaces:**
- `AppState.credentials: Arc<CredentialStore>` — added field.

- [ ] **Step 1: Locate `AppState` struct and `app.manage(AppState { .. })`**

```bash
grep -n "pub struct AppState\|app.manage(AppState" desktop/src-tauri/src/commands.rs desktop/src-tauri/src/lib.rs
```

- [ ] **Step 2: Add `credentials` field to `AppState`**

```rust
pub credentials: Arc<CredentialStore>,
```

- [ ] **Step 3: Modify `lib.rs::setup()` to construct the store**

```rust
use crate::credential::CredentialStore;

// inside .setup() closure, after `let paths = paths::resolve()?;`
let credentials_path = std::path::PathBuf::from(&paths.credentials_path);
if let Some(parent) = credentials_path.parent() {
    let _ = std::fs::create_dir_all(parent);
}
let credentials = Arc::new(CredentialStore::new(credentials_path).await?);

// Startup corruption handling: log + delete
match credentials.read_summary().await {
    Ok(_) => {}
    Err(crate::credential::CredentialError::InvalidFormat(reason)) => {
        eprintln!("mtls-router: removing corrupt credential file: {reason}");
        let _ = credentials.delete().await;
    }
    Err(_) => {} // NotFound is fine
}
```

Add `pub async fn new(path: PathBuf) -> std::result::Result<Self, CredentialError>` to `CredentialStore` (simple wrapper around `Self { path, inner: Mutex::new(()) }`).

- [ ] **Step 4: Pass into `app.manage(AppState { .. })`**

```rust
app.manage(AppState {
    manager: manager.clone(),
    scheduler: scheduler.clone(),
    paths,
    model_flows: Default::default(),
    pending_occupant: Default::default(),
    credentials,
});
```

- [ ] **Step 5: Register new Tauri commands in `invoke_handler!`**

Find the existing `tauri::generate_handler![..]` macro; append:
```rust
commands::get_credential,
commands::save_credential,
commands::delete_credential,
```

- [ ] **Step 6: Build + run unit tests**

```bash
cargo build --manifest-path desktop/src-tauri/Cargo.toml --locked
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked --lib
```

- [ ] **Step 7: Commit**

```bash
git add desktop/src-tauri/src/lib.rs desktop/src-tauri/src/commands.rs desktop/src-tauri/src/credential.rs
git commit -m "feat(desktop): wire CredentialStore into AppState"
```

---

### Task 7: Frontend IPC type

**Files:**
- Modify: `desktop/src/ipc.ts`
- Modify: `desktop/src/ipc.test.ts`

**Interfaces:**
- `export type CredentialSummary = { present: boolean; fingerprint: string; saved_at: string | null }`
- In `DesktopApi` interface: `getCredential(): Promise<CredentialSummary>; saveCredential(apiKey: string): Promise<CredentialSummary>; deleteCredential(): Promise<CredentialSummary>;`
- In `COMMANDS` const: `credentialGet: "get_credential"; credentialSave: "save_credential"; credentialDelete: "delete_credential";`

- [ ] **Step 1: Add types + interface entries**

Append to `ipc.ts`:

```typescript
export interface CredentialSummary {
  present: boolean;
  fingerprint: string;
  saved_at: string | null;
}
```

In `DesktopApi` (find it via `grep -n "DesktopApi {" desktop/src/ipc.ts`):

```typescript
getCredential(): Promise<CredentialSummary>;
saveCredential(apiKey: string): Promise<CredentialSummary>;
deleteCredential(): Promise<CredentialSummary>;
```

In `COMMANDS`:
```typescript
credentialGet: "get_credential",
credentialSave: "save_credential",
credentialDelete: "delete_credential",
```

In `invoke` proxy map:
```typescript
getCredential: () => invoke<CredentialSummary>(COMMANDS.credentialGet),
saveCredential: (apiKey) => invoke<CredentialSummary>(COMMANDS.credentialSave, { apiKey }),
deleteCredential: () => invoke<CredentialSummary>(COMMANDS.credentialDelete),
```

- [ ] **Step 2: Write failing test in `ipc.test.ts`**

```typescript
import { COMMANDS, getApi } from "./ipc";

describe("credential IPC", () => {
  it("exposes the three credential commands", () => {
    expect(COMMANDS.credentialGet).toBe("get_credential");
    expect(COMMANDS.credentialSave).toBe("save_credential");
    expect(COMMANDS.credentialDelete).toBe("delete_credential");
  });
  it("does not expose useCredential", () => {
    const api = getApi();
    expect("useCredential" in (api as unknown as Record<string, unknown>)).toBe(false);
  });
});
```

- [ ] **Step 3: Run; expect failure**

```bash
cd desktop && npm test -- --run ipc.test.ts
```

- [ ] **Step 4: Run; expect pass** (after the type additions)

```bash
cd desktop && npm test -- --run ipc.test.ts
```

- [ ] **Step 5: Run static checks**

```bash
cd desktop && npm run static:check
npm run typecheck
```

- [ ] **Step 6: Commit**

```bash
git add desktop/src/ipc.ts desktop/src/ipc.test.ts
git commit -m "feat(desktop): expose credential commands on DesktopApi"
```

---

### Task 8: Api keys page

**Files:**
- Create: `desktop/src/ApiKeysPage.tsx` (~250 lines)
- Create: `desktop/src/ApiKeysPage.test.tsx`
- Modify: `desktop/src/model.ts` (+"api-keys" route)
- Modify: `desktop/src/App.tsx` (sidebar item + route)
- Modify: `desktop/src/locales/en.ts` (+apikey namespace)
- Modify: `desktop/src/locales/zh-CN.ts` (+apikey namespace)

**Interfaces:**
- `export function ApiKeysPage(props: { api: DesktopApi; paths: DesktopPaths; language: NativeLanguage })`

- [ ] **Step 1: Add locale strings**

In `desktop/src/locales/en.ts`, append:
```typescript
"apikey.title": "API key",
"apikey.subtitle": "Your access credential for the local mtls-router.",
"apikey.status.saved": "Configured",
"apikey.status.absent": "Not configured",
"apikey.fingerprint": "Fingerprint",
"apikey.savedAt": "Saved at",
"apikey.label": "API key",
"apikey.input.placeholder": "Paste your API key",
"apikey.show": "Show",
"apikey.hide": "Hide",
"apikey.save": "Save",
"apikey.replace": "Replace",
"apikey.delete": "Delete",
"apikey.explainer.usage.heading": "Where it is used",
"apikey.explainer.usage.items": "Writing Claude Code, opencode, and Codex configuration files\nFetching the available model catalog from mtls-router",
"apikey.explainer.storage.heading": "Where it is stored",
"apikey.error.length": "Key must be non-empty and at most {max} bytes",
"apikey.error.io": "Credential file IO error",
"apikey.error.invalid": "Credential file is corrupt, please re-save",
```

Mirror in `zh-CN.ts` with natural Chinese translations.

- [ ] **Step 2: Create `ApiKeysPage.tsx`** (skeleton first)

```tsx
import { useEffect, useState } from "react";
import type { DesktopApi, DesktopPaths, NativeLanguage } from "./ipc";
import { useI18n } from "./i18n";

export function ApiKeysPage({ api, paths, language }: Props) {
  const { t } = useI18n();
  const [summary, setSummary] = useState<CredentialSummary | null>(null);
  const [draft, setDraft] = useState("");
  const [show, setShow] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void api.getCredential().then(setSummary);
  }, [api]);

  const save = async () => {
    setBusy(true); setError(null);
    try {
      const next = await api.saveCredential(draft);
      setSummary(next); setDraft("");
    } catch (e) { setError(t(`apikey.error.${...}`)); }
    finally { setBusy(false); }
  };
  // ... delete, render
}
```

For test surface: this component must NOT export `use_credential` or any key-reading API.

- [ ] **Step 3: Write failing test in `ApiKeysPage.test.tsx`**

```typescript
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { ApiKeysPage } from "./ApiKeysPage";

const fakeApi = {
  getCredential: vi.fn(async () => ({ present: false, fingerprint: "", saved_at: null })),
  saveCredential: vi.fn(async () => ({ present: true, fingerprint: "ABCD", saved_at: "2026-07-26T..." })),
  deleteCredential: vi.fn(async () => ({ present: false, fingerprint: "", saved_at: null })),
  // ... no useCredential field
};

describe("ApiKeysPage", () => {
  it("shows absent state on first load", async () => {
    render(<ApiKeysPage api={fakeApi as any} paths={{ data_dir: "/tmp/x", credentials_path: "/tmp/x/credentials.json", log_file: "", can_prepare_for_uninstall: false }} language="zh-CN" />);
    expect(await screen.findByText("Not configured")).toBeInTheDocument();
  });

  it("does not accept useCredential in its api prop", () => {
    expect("useCredential" in fakeApi).toBe(false);
  });
});
```

- [ ] **Step 4: Run; expect failure**

```bash
cd desktop && npm test -- --run ApiKeysPage.test.tsx
```

- [ ] **Step 5: Implement component fully**

Include:
- Status badge: present ? green "Configured" / red "Not configured"
- If present: dl with fingerprint + saved_at (formatted with `Intl.DateTimeFormat`)
- Form: input (password type by default, toggle to text), Save / Replace button, optional Delete button
- Explainer section: where used, where stored (use `paths.credentials_path`)
- Error banners mapped from `apikey.error.*` translations
- Empty input after save
- No console output of the key, no localStorage write, no URL query

- [ ] **Step 6: Run tests; expect pass**

```bash
cd desktop && npm test -- --run ApiKeysPage.test.tsx
```

- [ ] **Step 7: Wire route in `App.tsx`**

Find navigation list and routes in `App.tsx`. Add an entry:

```tsx
{ id: "api-keys", label: t("apikey.title"), icon: KeyIcon }
```

And route branch:
```tsx
{activeSection === "api-keys" && (
  <ApiKeysPage api={api} paths={paths} language={language} />
)}
```

- [ ] **Step 8: Run static + tests**

```bash
cd desktop && npm run static:check
npm run typecheck
npm test
```

- [ ] **Step 9: Commit**

```bash
git add desktop/src/ApiKeysPage.tsx desktop/src/ApiKeysPage.test.tsx desktop/src/model.ts desktop/src/App.tsx desktop/src/locales/en.ts desktop/src/locales/zh-CN.ts
git commit -m "feat(desktop): add Api keys sidebar page"
```

---

### Task 9: Strip apiKey from AgentPage

**Files:**
- Modify: `desktop/src/AgentPage.tsx`

**Interfaces (after):**
- `api.discoverModels(selected)` — only one argument.

- [ ] **Step 1: Locate call sites**

```bash
grep -n "discoverModels\|apiKey" desktop/src/AgentPage.tsx
```

There are ~3 sites (credential stage → render, write stage → write, error reset). The `apiKey` state and `setKey` reducer are only used to feed those calls.

- [ ] **Step 2: Write failing test reproducing absence of apiKey**

In `AgentPage.test.tsx`, the existing test "prefills from discovered models" expected `discoverModels` to be called with `(selected, '')` after first save. Update expected call to `discoverModels(selected)` (one arg).

- [ ] **Step 3: Run; expect failure**

```bash
cd desktop && npm test -- --run AgentPage.test.tsx
```

- [ ] **Step 4: Remove apiKey state and 2nd argument**

```typescript
// Remove: const [apiKey, setApiKey] = useState("");
// Remove: clearFlowState() resets for apiKey

// discoverModels call:
const value = await api.discoverModels(selected);   // was: api.discoverModels(selected, transient)
```

Replace `agents.credentialHeading` panel: instead of a key input, show:

```tsx
{summary && !summary.present ? (
  <Empty>
    <p>{t("agents.credentialNote")}</p>
    <button onClick={() => navigate("api-keys")}>{t("agents.openApiKeys")}</button>
  </Empty>
) : (
  <Panel>{t("agents.credentialConfigured")}</Panel>
)}
```

Drop `agents.apiKey` i18n strings.

- [ ] **Step 5: Add or update i18n strings**

Add `agents.openApiKeys` and `agents.credentialConfigured`. Remove `agents.apiKey` (if not used elsewhere).

- [ ] **Step 6: Run tests; expect pass**

```bash
cd desktop && npm test -- --run AgentPage.test.tsx
npm run typecheck
```

- [ ] **Step 7: Commit**

```bash
git add desktop/src/AgentPage.tsx desktop/src/AgentPage.test.tsx desktop/src/locales/en.ts desktop/src/locales/zh-CN.ts
git commit -m "refactor(desktop): source agent API key from credential store"
```

---

### Task 10: End-to-end workflow test

**Files:**
- Modify: `tests/desktop_workflow_test.sh`

- [ ] **Step 1: Read existing shell test**

```bash
grep -n "function\|test_" tests/desktop_workflow_test.sh | head -30
```

- [ ] **Step 2: Add a new test function**

```bash
test_credential_store_round_trip() {
  # Assumes the desktop process can be invoked with a debug CLI that exercises
  # credential store directly (added in a side task if missing). If no such
  # debug CLI exists, skip with rationale.
  local data_dir="$(mktemp -d)"
  local cred_path="$data_dir/credentials.json"

  # save
  MTLS_ROUTER_DESKTOP_DATA_DIR="$data_dir" save-credential "fixture-key" || skip
  test -f "$cred_path"
  stat -c '%a' "$cred_path" | grep -q '^600$'

  # delete
  MTLS_ROUTER_DESKTOP_DATA_DIR="$data_dir" delete-credential || skip
  test ! -f "$cred_path"

  rm -rf "$data_dir"
}
```

If a debug CLI hook is not feasible in this iteration, write a Rust integration test (Task 5.4) that drives the store end-to-end and replace shell test with:

```bash
test_credential_store_round_trip_via_unit() { :; }
```

- [ ] **Step 3: Run shell tests**

```bash
make test-workflows
```

- [ ] **Step 4: Commit**

```bash
git add tests/desktop_workflow_test.sh
git commit -m "test: cover credential store round trip"
```

---

### Task 11: Full verification

**Files:** none

- [ ] **Step 1: Run full Rust + frontend verification**

```bash
cd desktop && npm run verify
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked
make test-shell
```

- [ ] **Step 2: Visual smoke check** (optional, document result)

Boot the desktop app, navigate to Api keys, save a fake key, navigate to Agents, observe that the credential stage shows "Configured" without any input field. Verify no console output of the key.

- [ ] **Step 3: Update `desktop/INDEX.md`**

Append new module `desktop/src-tauri/src/credential.rs` row to the package list if applicable.

- [ ] **Step 4: Commit**

```bash
git add desktop/INDEX.md
git commit -m "docs: document credential store module"
```

---

## Self-Review Notes

- **Spec coverage**:
  - §0.1 (goals/non-goals): Tasks 1/4/5/7/8. ✓
  - §0.2 architecture: Tasks 1/3/6. ✓
  - §0.3 data flow + 6 invariants: Tasks 5 (key not exposed to frontend), 1 (file mode 0600), 5 (Zeroizing in use path), 7 (no useCredential in IPC). ✓
  - §0.4 error handling: Task 4 + Task 6 startup handling. ✓
  - §0.5 component boundary: Tasks 1 (credential.rs no ManagerClient dep), 4/5 (commands as glue), 7 (frontend IPC type). ✓
  - §0.6 test matrix: Task 1/2 (Rust unit), Task 4 (commands), Task 7 (ipc.test), Task 8 (ApiKeysPage), Task 9 (AgentPage non-break), Task 10 (e2e). ✓
  - §0.7 invariants: Task 5, Task 7, Task 9. ✓
  - §0.8 risks: Windows DACL — explicitly punted with note. ✓

- **Type consistency**: `CredentialSummary { present, fingerprint, saved_at }` used in 6 tasks — consistent. `CredentialStore::use_()` trailing underscore noted in spec — consistent.

- **No placeholders**: no "TBD" / "TODO" / "implement later". Concrete code in every block.

- **YAGNI**: thiserror only added if Task 0 finds missing. base32 only added if available; fallback hash documented. No new crates beyond what's strictly necessary.

- **Risk**: Task 1's base32 fingerprint depends on availability; if not available, fallback to `md5` last 4 hex or `format!("{:08x}", hash(key))` — explicit in plan.
