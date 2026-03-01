# Core Library Specification

## Purpose

`chisel-core` is a portable Rust library crate that provides path-confined filesystem operations and shell execution. It carries no HTTP, MCP protocol, or async runtime dependencies. Any Rust program — including other MCP servers — can depend on it to get the same security semantics as `chisel` without running a separate process.

---

## Non-Goals

- No HTTP transport or MCP protocol concern belongs in this crate.
- No async runtime (`tokio`, `async-std`) is required or exposed; all operations are synchronous.
- No CLI argument parsing.

---

## Requirements

### Requirement: Path Validation

The library MUST expose `validate_path(root: &Path, input: &str) -> Result<PathBuf, CoreError>`. Behavior MUST be identical to the server's path confinement: symlinks fully resolved, result must start with `root`. For non-existent paths, the parent is canonicalized and the filename appended.

#### Scenario: Valid path inside root
- GIVEN root `/data`, input `/data/file.txt` exists
- WHEN `validate_path` is called
- THEN returns the canonical `PathBuf`

#### Scenario: Traversal rejected
- GIVEN root `/data`
- WHEN `validate_path` is called with `/data/sub/../../etc/passwd`
- THEN returns `CoreError::OutsideRoot`

#### Scenario: Symlink escape rejected
- GIVEN root `/data`, `/data/link` symlinks to `/etc/hosts`
- WHEN `validate_path` is called with `/data/link`
- THEN returns `CoreError::OutsideRoot`

---

### Requirement: Error Type

The library MUST expose a `CoreError` enum with variants covering all observable failure modes. `CoreError` MUST implement `std::error::Error` and `Display`. It MUST NOT depend on any external HTTP or MCP crate.

| Variant | Condition |
|---------|-----------|
| `OutsideRoot { path, root }` | Resolved path escapes root |
| `NotFound { path }` | Path does not exist when required |
| `PermissionDenied { path }` | OS permission denied |
| `PatchFailed { reason }` | Hunk mismatch or malformed diff |
| `ReadOnly` | Operation attempted with `read_only: true` |
| `CommandNotAllowed { command }` | Command not in whitelist (native only) |
| `Other(String)` | Unexpected I/O or internal error |

---

### Requirement: File Operations API

The library MUST expose the following synchronous functions. All take `root: &Path` as the first argument and perform path validation internally before any I/O.

```
patch_apply(root, path, patch_text, read_only) -> Result<String, CoreError>
append(root, path, content, read_only)          -> Result<String, CoreError>
write_file(root, path, content, read_only)       -> Result<String, CoreError>
create_directory(root, path, read_only)          -> Result<String, CoreError>
move_file(root, source, destination, read_only)  -> Result<String, CoreError>
```

Observable behavior MUST be identical to the corresponding server tools (see `specs/server/spec.md`). The `read_only` flag MUST cause all five functions to return `CoreError::ReadOnly` immediately without performing any I/O.

#### Scenario: Core op behaves identically to server tool
- GIVEN root `/data`, `/data/a.txt` contains `"hello\n"`
- WHEN `patch_apply` is called with a diff replacing `hello` with `world`
- THEN `/data/a.txt` contains `"world\n"`

#### Scenario: Read-only blocks all write ops
- GIVEN `read_only = true`
- WHEN any of the five write functions is called
- THEN returns `CoreError::ReadOnly` with no I/O performed

---

### Requirement: Shell Execution API (native only)

On non-WASM targets the library MUST expose:

```
shell_exec(root, command, args) -> Result<ShellOutput, CoreError>
```

`ShellOutput` MUST carry `exit_code: i32`, `stdout: String`, `stderr: String`. Whitelist, argument path validation, and no-shell-interpreter behavior MUST be identical to the server tool.

This function MUST NOT be compiled or available on `wasm32` targets.

#### Scenario: Available on native, absent on WASM
- GIVEN a `wasm32-wasip1` compilation
- WHEN the crate is compiled
- THEN `shell_exec` is not in the public API and the crate compiles without error

---

### Requirement: No Async Runtime Dependency

All public functions in `chisel-core` MUST be synchronous. Callers using async runtimes MAY wrap calls in `spawn_blocking` or equivalent; this is the caller's responsibility.

#### Scenario: Crate compiles without tokio
- GIVEN a project with no async runtime in its dependency tree
- WHEN `chisel-core` is added as a dependency
- THEN the project compiles successfully
