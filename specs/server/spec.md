# Server Specification

## Purpose

`chisel` is an HTTP server that speaks the MCP Streamable HTTP transport. It exposes a tightly scoped set of filesystem and shell tools to LLM clients, enforcing bearer token authentication and strict path confinement to a configured root directory.

---

## Requirements

### Requirement: Authentication

The server MUST reject any request that does not carry a valid `Authorization: Bearer <secret>` header. Comparison MUST be constant-time to prevent timing attacks.

#### Scenario: Valid token is accepted
- GIVEN the server is running with secret `"mysecret"`
- WHEN a request arrives with `Authorization: Bearer mysecret`
- THEN the request is processed normally

#### Scenario: Missing or wrong token is rejected
- GIVEN the server is running with secret `"mysecret"`
- WHEN a request arrives with no `Authorization` header, or with a different token
- THEN the server returns HTTP 401 and no tool is invoked

---

### Requirement: Path Confinement

Every path argument passed to any tool MUST be resolved to its canonical absolute path (following symlinks) before any I/O is performed. The resolved path MUST start with the configured root. Any path that escapes the root — via `..` traversal, absolute paths, or symlinks — MUST be rejected with `OutsideRoot`.

#### Scenario: Path inside root is accepted
- GIVEN root is `/data`
- WHEN a tool is called with path `/data/subdir/file.txt`
- THEN the tool proceeds

#### Scenario: Traversal attempt is rejected
- GIVEN root is `/data`
- WHEN a tool is called with path `/data/subdir/../../etc/passwd`
- THEN the tool returns `OutsideRoot` and no I/O is performed

#### Scenario: Symlink pointing outside root is rejected
- GIVEN root is `/data` and `/data/link` is a symlink to `/etc/hosts`
- WHEN a tool is called with path `/data/link`
- THEN the tool returns `OutsideRoot`

#### Scenario: Non-existent path inside root is accepted for write tools
- GIVEN root is `/data` and `/data/new.txt` does not exist
- WHEN `write_file` is called with `/data/new.txt`
- THEN path validation succeeds (parent is canonicalized, filename appended)

---

### Requirement: Read-Only Mode

When started with `--read-only`, the server MUST reject all write tool calls (`patch_apply`, `append`, `write_file`, `create_directory`, `move_file`) with `ReadOnly`. Read operations via `shell_exec` remain available.

#### Scenario: Write tool blocked in read-only mode
- GIVEN the server started with `--read-only`
- WHEN `write_file` is called with any path
- THEN the tool returns `ReadOnly` and no file is written

---

### Requirement: Secret Resolution

The server MUST fail to start if no secret is provided. `MCP_APP_SECRET` environment variable MUST take precedence over the `--secret` CLI flag. Empty string values MUST be treated as absent.

#### Scenario: Env var overrides CLI flag
- GIVEN `MCP_APP_SECRET=env-val` is set and `--secret cli-val` is passed
- WHEN the server starts
- THEN the effective secret is `env-val`

#### Scenario: No secret → fail fast
- GIVEN neither `MCP_APP_SECRET` nor `--secret` is provided
- WHEN the server starts
- THEN it exits with a non-zero code and a clear error message before binding any port

---

### Requirement: Network Binding

The server MUST bind exclusively to `127.0.0.1` and MUST NOT bind to `0.0.0.0`. Remote access is the responsibility of a reverse proxy (e.g. Caddy).

---

### Requirement: Tool — `patch_apply`

Apply a unified diff to an existing file, or create a new file when the diff source is `/dev/null`.

| Input | Type | Required |
|-------|------|----------|
| `path` | string (absolute) | yes |
| `patch` | string (unified diff, optionally fenced in ` ```diff `) | yes |

- Markdown code fences MUST be stripped before parsing.
- If the diff source header is `--- /dev/null`, the file is created from the `+` lines.
- The write MUST be atomic: written to a temp file in the same directory, then renamed. A failed patch MUST NOT leave the file in a partial state.
- If any hunk context does not match the current file content, the tool MUST return `PatchFailed` and leave the file unchanged.

#### Scenario: Successful patch
- GIVEN `/data/a.txt` contains `"hello\n"`
- WHEN `patch_apply` is called with a diff replacing `hello` with `world`
- THEN `/data/a.txt` contains `"world\n"` and the tool returns success

#### Scenario: Drifted hunk leaves file unchanged
- GIVEN `/data/a.txt` contains `"original\n"`
- WHEN `patch_apply` is called with a hunk that expects `"wrong context"`
- THEN the tool returns `PatchFailed` and `/data/a.txt` still contains `"original\n"`

#### Scenario: Fenced diff is accepted
- GIVEN a patch string wrapped in ` ```diff … ``` `
- WHEN `patch_apply` is called
- THEN the fence is stripped and the patch is applied normally

---

### Requirement: Tool — `append`

Append content to the end of an existing file. MUST NOT create the file if absent.

#### Scenario: Append to existing file
- GIVEN `/data/log.txt` contains `"line1"`
- WHEN `append` is called with content `"\nline2"`
- THEN `/data/log.txt` contains `"line1\nline2"`

#### Scenario: Append to missing file returns error
- GIVEN `/data/missing.txt` does not exist
- WHEN `append` is called
- THEN the tool returns `NotFound`

---

### Requirement: Tool — `write_file`

Write content to a file, creating it or overwriting it. Parent directories MUST be created automatically.

#### Scenario: Create new file
- GIVEN `/data/new.txt` does not exist
- WHEN `write_file` is called with content `"hello"`
- THEN `/data/new.txt` is created with content `"hello"`

#### Scenario: Overwrite existing file
- GIVEN `/data/exist.txt` contains `"old"`
- WHEN `write_file` is called with content `"new"`
- THEN `/data/exist.txt` contains `"new"`

---

### Requirement: Tool — `create_directory`

Create a directory and any missing parent directories.

#### Scenario: Nested directory creation
- GIVEN `/data/a/b/c` does not exist
- WHEN `create_directory` is called with `/data/a/b/c`
- THEN the full path exists as a directory

---

### Requirement: Tool — `move_file`

Move or rename a file. Both source and destination MUST be independently validated against the root. If destination validation fails, the source MUST NOT be moved.

#### Scenario: Move within root
- GIVEN `/data/old.txt` exists
- WHEN `move_file` is called with source `/data/old.txt`, destination `/data/new.txt`
- THEN `/data/new.txt` exists and `/data/old.txt` does not

#### Scenario: Destination outside root leaves source untouched
- GIVEN `/data/src.txt` exists
- WHEN `move_file` is called with destination `/tmp/evil.txt`
- THEN the tool returns `OutsideRoot` and `/data/src.txt` still exists

---

### Requirement: Tool — `shell_exec`

Execute a command from a fixed whitelist. The command MUST be spawned directly without a shell interpreter; shell metacharacters in arguments MUST be inert. Path-like arguments (starting with `/` or containing `..`) MUST be validated against the root before the process is spawned.

**Permitted commands:** `grep`, `sed`, `awk`, `find`, `cat`, `head`, `tail`, `wc`, `sort`, `uniq`, `cut`, `tr`, `diff`, `file`, `stat`, `ls`, `du`, `rg`

A non-zero exit code MUST be returned as a success response (not an MCP error); the caller inspects `exit_code`.

#### Scenario: Whitelisted command executes
- GIVEN `/data/hello.txt` exists
- WHEN `shell_exec` is called with command `cat`, args `["/data/hello.txt"]`
- THEN `exit_code` is 0 and `stdout` contains the file contents

#### Scenario: Non-whitelisted command is rejected before spawn
- WHEN `shell_exec` is called with command `bash`
- THEN the tool returns `CommandNotAllowed` listing permitted commands; no process is spawned

#### Scenario: Path-like arg outside root is rejected before spawn
- WHEN `shell_exec` is called with command `cat`, args `["/etc/hosts"]`
- THEN the tool returns `OutsideRoot`; no process is spawned

#### Scenario: Shell metacharacters are inert
- WHEN `shell_exec` is called with command `ls`, args `["/data", "nonexistent; rm -rf /"]`
- THEN `rm` is never executed; the second arg is treated as a literal filename
