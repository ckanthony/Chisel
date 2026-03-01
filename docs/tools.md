# MCP Tool Reference

All tools require a valid `Authorization: Bearer <secret>` header. All path arguments are resolved and validated against the configured root directory before any I/O — symlinks are followed to their real path.

---

## patch_apply

Apply a unified diff to an existing file, or create a new file from a `/dev/null` source header.

**Input**

| Field | Type | Description |
|-------|------|-------------|
| `path` | string | Absolute path to the target file |
| `patch` | string | Unified diff. May be wrapped in a markdown code fence (` ```diff … ``` `) |

**Behavior**

- Strips markdown code fences before parsing.
- If the diff's source line is `--- /dev/null`, the file is created with the `+` lines as content (file must not already exist, or will be overwritten).
- Otherwise, reads the existing file and applies all hunks. If any hunk context does not match (file has drifted), returns `PatchFailed` and the file is **not modified** (atomic rename ensures no partial write).
- Returns an error in read-only mode.

**Errors**

| Code | Condition |
|------|-----------|
| `OutsideRoot` | Path resolves outside the configured root |
| `NotFound` | File does not exist (non-`/dev/null` source) |
| `PatchFailed` | Hunk mismatch or malformed diff |
| `ReadOnly` | Server started with `--read-only` |

---

## append

Append content to the end of an existing file. Does **not** create the file if it is absent.

**Input**

| Field | Type | Description |
|-------|------|-------------|
| `path` | string | Absolute path to the target file |
| `content` | string | Content to append |

**Errors**

| Code | Condition |
|------|-----------|
| `NotFound` | File does not exist |
| `OutsideRoot` | Path outside root |
| `ReadOnly` | Read-only mode |

---

## write_file

Write content to a file, creating it if it does not exist or overwriting it entirely if it does. Parent directories are created automatically.

**Input**

| Field | Type | Description |
|-------|------|-------------|
| `path` | string | Absolute path to the file |
| `content` | string | Content to write |

**Errors**

| Code | Condition |
|------|-----------|
| `OutsideRoot` | Path outside root |
| `PermissionDenied` | OS-level permission denied |
| `ReadOnly` | Read-only mode |

---

## create_directory

Create a directory and any missing parent directories.

**Input**

| Field | Type | Description |
|-------|------|-------------|
| `path` | string | Absolute path to the directory to create |

**Errors**

| Code | Condition |
|------|-----------|
| `OutsideRoot` | Path outside root |
| `ReadOnly` | Read-only mode |

---

## move_file

Move or rename a file within the root. Both source and destination are independently validated.

**Input**

| Field | Type | Description |
|-------|------|-------------|
| `source` | string | Absolute path to the source file |
| `destination` | string | Absolute path to the destination |

**Errors**

| Code | Condition |
|------|-----------|
| `OutsideRoot` | Either path outside root. Source is **not moved** if destination fails. |
| `NotFound` | Source does not exist |
| `ReadOnly` | Read-only mode |

---

## shell_exec

Execute a whitelisted shell command. The command is invoked directly — **no shell interpreter** — so shell metacharacters in arguments are inert.

**Allowed commands:** `grep`, `sed`, `awk`, `find`, `cat`, `head`, `tail`, `wc`, `sort`, `uniq`, `cut`, `tr`, `diff`, `file`, `stat`, `ls`, `du`, `rg`

`mkdir` and `mv` are explicitly excluded from this list; use `create_directory` and `move_file` instead.

**Input**

| Field | Type | Description |
|-------|------|-------------|
| `command` | string | The command to run (must be in the whitelist) |
| `args` | string[] | Argument list. Path-like args (starting with `/` or containing `..`) are validated against root before the process is spawned. |

**Output**

```
exit_code: <N>
stdout:
<stdout text>
stderr:
<stderr text>
```

A non-zero exit code is returned as a **success** response (not an MCP error) — it is the caller's responsibility to inspect `exit_code`.

**Errors**

| Code | Condition |
|------|-----------|
| `CommandNotAllowed` | Command not in whitelist. Message lists permitted commands. |
| `OutsideRoot` | A path-like argument resolves outside root. Process is **not spawned**. |
