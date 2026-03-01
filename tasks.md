# Task List — Workspace Refactor & Extensibility

Specs: `specs/server/spec.md`, `specs/core/spec.md`, `specs/wasm/spec.md`

---

## Task 1 — Convert to Cargo workspace

**Goal:** Turn the single-crate repo into a Cargo workspace without breaking anything.

- [ ] Rewrite root `Cargo.toml` as a workspace manifest (`[workspace]` with `members = ["chisel"]`)
- [ ] Create `chisel/` directory
- [ ] Move `src/` → `chisel/src/`
- [ ] Create `chisel/Cargo.toml` with the same `[package]` and `[dependencies]` as the old root manifest
- [ ] Move `[dev-dependencies]` into `chisel/Cargo.toml`
- [ ] Verify: `cargo test -p chisel` passes (all existing tests green)

---

## Task 2 — Create `chisel-core` skeleton

**Goal:** An empty lib crate in the workspace that compiles.

- [ ] Create `chisel-core/Cargo.toml` — lib crate, deps: `diffy`, `tempfile`, `anyhow`
- [ ] Create `chisel-core/src/lib.rs` (empty module declarations only)
- [ ] Add `"chisel-core"` to workspace `members`
- [ ] Verify: `cargo build -p chisel-core` exits 0

---

## Task 3 — Port `CoreError` to `chisel-core`

**Spec ref:** `specs/core/spec.md` § Error Type

**Goal:** A clean error type in core with no `rmcp` coupling. `chisel` adapts it.

- [ ] Create `chisel-core/src/error.rs`:
  - `CoreError` enum — same variants as `AppError` except `Other(String)` (not `anyhow::Error`)
  - Implement `std::error::Error`, `Display`
  - `From<std::io::Error>` impl
  - Move existing `AppError` error-message tests here (adjusted for `CoreError`)
- [ ] In `chisel/src/error.rs`:
  - Rename `AppError::Other(anyhow::Error)` stays; add `From<CoreError> for AppError`
  - Keep `IntoContents` impl (rmcp glue stays in `chisel`)
- [ ] Verify: `cargo test` (all crates) passes

---

## Task 4 — Port `security.rs` to `chisel-core`

**Spec ref:** `specs/core/spec.md` § Path Validation

**Goal:** `validate_path` lives in core; `chisel` re-imports it.

- [ ] Copy `chisel/src/security.rs` → `chisel-core/src/security.rs`
  - Change error type from `AppError` → `CoreError`
  - All existing tests move with the file (update `AppError` refs to `CoreError`)
- [ ] In `chisel/src/security.rs` (or delete it): re-export `chisel_core::security::validate_path`
  - Update all `crate::security::validate_path` call sites to use the re-export
- [ ] Verify: `cargo test` (all crates) passes

---

## Task 5 — Port filesystem ops to `chisel-core`

**Spec ref:** `specs/core/spec.md` § File Operations API

**Goal:** Pure sync ops in core; `chisel` tools become thin async wrappers.

- [ ] Create `chisel-core/src/ops/mod.rs` and `chisel-core/src/ops/filesystem.rs`
  - New sync signatures:
    ```
    pub fn patch_apply(root: &Path, path: &str, patch_text: &str, read_only: bool) -> Result<String, CoreError>
    pub fn append(root: &Path, path: &str, content: &str, read_only: bool) -> Result<String, CoreError>
    pub fn write_file(root: &Path, path: &str, content: &str, read_only: bool) -> Result<String, CoreError>
    pub fn create_directory(root: &Path, path: &str, read_only: bool) -> Result<String, CoreError>
    pub fn move_file(root: &Path, source: &str, destination: &str, read_only: bool) -> Result<String, CoreError>
    pub fn strip_diff_fence(input: &str) -> &str   // kept pub for tests
    ```
  - Move all logic from `chisel/src/tools/filesystem.rs` verbatim (change `AppState`→params, `AppError`→`CoreError`, remove `check_writable` call — inline the `read_only` check)
  - Move all `filesystem.rs` tests here (adjust types)
- [ ] Rewrite `chisel/src/tools/filesystem.rs` as thin async wrappers:
  ```rust
  pub async fn patch_apply(state: &AppState, path: String, patch: String) -> Result<String, AppError> {
      chisel_core::ops::patch_apply(&state.config.root, &path, &patch, state.config.read_only)
          .map_err(AppError::from)
  }
  // … same pattern for all five
  ```
- [ ] Verify: `cargo test` (all crates) passes

---

## Task 6 — Port `shell_exec` to `chisel-core`

**Spec ref:** `specs/core/spec.md` § Shell Execution API (native only)

**Goal:** Shell logic in core, cfg-gated off WASM; `chisel` tool is a thin wrapper.

- [ ] Create `chisel-core/src/ops/shell.rs`
  - Gate entire file with `#![cfg(not(target_family = "wasm"))]`
  - New sync signature: `pub fn shell_exec(root: &Path, command: &str, args: &[&str]) -> Result<ShellOutput, CoreError>`
  - `ShellOutput { exit_code: i32, stdout: String, stderr: String }` — implement `Display` only (no `rmcp::IntoContents`)
  - Move whitelist, arg validation, spawn logic verbatim
  - Move all `shell.rs` tests here
- [ ] Expose in `chisel-core/src/ops/mod.rs`:
  ```rust
  #[cfg(not(target_family = "wasm"))]
  pub mod shell;
  ```
- [ ] Rewrite `chisel/src/tools/shell.rs`:
  - Re-export `chisel_core::ops::shell::ShellOutput`
  - Add `IntoContents` impl for `ShellOutput` here (rmcp glue stays in `chisel`)
  - Thin async wrapper: `pub async fn shell_exec(state, cmd, args) -> Result<ShellOutput, AppError>`
- [ ] Verify: `cargo test` (all crates) passes

---

## Task 7 — Create `chisel-wasm` crate

**Spec ref:** `specs/wasm/spec.md`

**Goal:** A crate that compiles to `wasm32-wasip1` and re-exposes core ops.

- [ ] Create `chisel-wasm/Cargo.toml`:
  - `[lib] crate-type = ["cdylib"]`
  - dep: `chisel-core` (path dep, default-features = true)
- [ ] Create `chisel-wasm/.cargo/config.toml`:
  ```toml
  [build]
  target = "wasm32-wasip1"
  ```
- [ ] Create `chisel-wasm/src/lib.rs`:
  - Re-export: `pub use chisel_core::{error::CoreError, security::validate_path, ops::filesystem::*};`
  - No `shell` re-export (excluded by cfg)
- [ ] Add `"chisel-wasm"` to workspace `members`
- [ ] Install target if needed: `rustup target add wasm32-wasip1`
- [ ] Verify: `cargo build --target wasm32-wasip1 -p chisel-wasm` exits 0

---

## Task 8 — Update documentation

**Goal:** README and docs reflect the new workspace layout and extensibility story.

- [ ] Update `README.md`:
  - Add workspace structure diagram (3 crates)
  - Add "Extensibility" section: Rust (depend on `chisel-core`), WASM/Node.js (consume `.wasm` via WASI)
  - Update "Development" section: `cargo test --workspace`, WASM build command
- [ ] Create `docs/extensibility.md`:
  - Server-to-server HTTP delegation (works today, any language)
  - Rust library usage (`chisel-core` as a dependency)
  - Node.js WASM usage (Node.js ≥ 22 `node:wasi` example stub)
- [ ] Verify: no broken links in docs
