---
name: chisel
description: Use Chisel MCP filesystem tools efficiently — patch-based edits, minimal token reads, shell exec patterns, and anti-patterns to avoid.
---

# Chisel Agent Skill

You have access to a Chisel MCP server. Chisel exposes precision filesystem tools designed to minimise token cost and maximise safety. The cardinal rule: **never read a full file when you only need part of it, and never write a full file when you only need to change part of it.**

---

## Tool Selection

```
Need to find something in a file or directory?
  └─ shell_exec (grep / rg / find / sed / awk / stat / ls)

Need to edit an existing file?
  ├─ Small targeted change (< whole file)  → patch_apply
  └─ Overwrite entirely / file is tiny     → write_file

Need to add content to the end of a file?
  └─ append

Need to create a directory?
  └─ create_directory

Need to rename or relocate a file?
  └─ move_file
```

---

## Reading — Never Fetch What You Don't Need

Always start with the minimum slice required. Only escalate if the first call does not give enough context.

| Goal | Call |
|------|------|
| Find which lines contain a symbol | `shell_exec grep ["-n", "symbol", "/root/file"]` |
| Fast multi-file symbol search | `shell_exec rg ["-n", "symbol", "/root"]` |
| List all headers in a Markdown file | `shell_exec grep ["-n", "^#", "/root/file.md"]` |
| Extract one section by header | `shell_exec sed ["-n", "/^## Section/,/^## /p", "/root/file.md"]` |
| Read lines 40–60 of a large file | `shell_exec sed ["-n", "40,60p", "/root/file"]` |
| Count occurrences | `shell_exec grep ["-c", "pattern", "/root/file"]` |
| Get file size / line count | `shell_exec wc ["-l", "/root/file"]` |
| List directory contents | `shell_exec ls ["-la", "/root/dir"]` |
| Find files by name | `shell_exec find ["/root", "-name", "*.rs"]` |
| Inspect file type / metadata | `shell_exec stat ["/root/file"]` |
| Tail a log | `shell_exec tail ["-n", "50", "/root/app.log"]` |

**Avoid calling `cat` on a file longer than ~50 lines.** Use `sed -n 'M,Np'` to window in.

---

## Editing — Prefer Diffs Over Full Rewrites

### `patch_apply` — primary edit tool

Send only the changed lines. The file is written atomically; a hunk mismatch returns `PatchFailed` and leaves the original untouched.

```diff
--- a/src/main.rs
+++ b/src/main.rs
@@ -12,7 +12,7 @@
 fn main() {
-    let port = 3000;
+    let port = 8080;
     start_server(port);
 }
```

Rules for a valid patch:
- Context lines (no prefix) must match the file **exactly**, including whitespace.
- `@@ -L,N +L,N @@` must reference the correct line numbers for the **current file state**.
- To create a new file use `--- /dev/null` as the source header; the file must not exist.
- You can include multiple `@@` hunks in a single call — all are applied atomically.

**When `PatchFailed` is returned:** the file has drifted from what you expected. Re-read the relevant section with `sed -n` or `grep -n`, then regenerate the patch against the actual content.

### `write_file` — full overwrite

Use only when:
- Creating a new file with substantial initial content.
- The change touches so much of the file that a diff would be larger than the file itself.
- The file is short (< ~30 lines) and a full rewrite is cleaner.

### `append`

Use for log entries, appending lines to a config, or adding to an incrementally-built file. The file must already exist.

---

## In-place Transformation with `shell_exec sed -i`

For bulk replacements (rename a symbol, fix a repeated pattern), `sed` mutates the file directly — no diff needed.

```
shell_exec sed ["-i", "s/OldName/NewName/g", "/root/src/lib.rs"]
```

> `shell_exec` has no shell interpreter — pipe (`|`), `&&`, `$()` are passed as literals. Compose multi-step operations in your own logic, not in a single command string.

---

## Workflow Patterns

### Locate → Read slice → Patch

The standard low-token edit loop:

1. `grep -n "target_symbol" /root/file` — find the line number.
2. `sed -n "L-5,L+10p" /root/file` — read a tight window around it.
3. `patch_apply` — send a diff against that exact content.

### Explore unfamiliar codebase

1. `ls -la /root` — top-level layout.
2. `find /root -name "*.rs" -not -path "*/target/*"` — enumerate source files.
3. `rg -n "struct MyStruct" /root/src` — locate the definition.
4. `sed -n "L,L+30p" /root/src/file.rs` — read the struct body.

### Create a new file with boilerplate

Use `write_file`. Once it exists, all subsequent edits go through `patch_apply`.

### Rename a symbol across many files

1. `rg -l "OldName" /root/src` — get the file list.
2. For each file: `sed -i "s/OldName/NewName/g" /root/src/file.rs`.

### Scaffold a directory tree

```
create_directory /root/src/handlers
write_file /root/src/handlers/mod.rs "..."
write_file /root/src/handlers/auth.rs "..."
```

---

## Error Handling

| Error | Meaning | Fix |
|-------|---------|-----|
| `PatchFailed` | Context lines don't match current file | Re-read the section with `sed -n`, regenerate patch |
| `OutsideRoot` | Path escapes configured root | Use only paths under the root the server was started with |
| `NotFound` | File doesn't exist | Use `write_file` to create, or check path with `find` |
| `ReadOnly` | Server in read-only mode | Writes are disabled; only reads and `shell_exec` work |
| `CommandNotAllowed` | Command not in whitelist | Use only: `grep rg sed awk find cat head tail wc sort uniq cut tr diff file stat ls du` |

---

## What Not To Do

| Anti-pattern | Why | Do instead |
|---|---|---|
| `cat /root/large_file.rs` | Returns thousands of tokens you mostly won't use | `grep -n` to locate, `sed -n` to slice |
| `write_file` for a 3-line change in a 500-line file | Uploads entire file, risks hallucination corrupting unrelated lines | `patch_apply` with a targeted hunk |
| Constructing shell pipelines (`cmd1 \| cmd2`) | No shell interpreter — metacharacters are literals | Make separate `shell_exec` calls; compose in your logic |
| `find / -name "*.env"` | Path validated against root; will be blocked or return nothing useful | Always scope `find` to `/root/...` |
| Sending a patch with approximate context lines | `PatchFailed` — context must match byte-for-byte | Read the exact content first, then diff against it |
