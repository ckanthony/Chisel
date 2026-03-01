# chisel-core

Portable, synchronous Rust library providing path-confined filesystem operations and whitelisted shell execution. Zero async, zero HTTP, zero MCP protocol — drop it into any server and own the transport entirely.

This is the enforcement layer that backs the [`chisel`](../chisel) MCP server. Every security property (kernel-enforced root confinement, atomic writes, shell whitelist) is implemented here and applies identically whether you use the standalone server or embed the library directly.

---

## When to use this instead of the server

| Scenario | Use |
|---|---|
| You are writing a Rust MCP server and want identical safety semantics without running a sidecar | `chisel-core` directly |
| You are writing an MCP server in Node.js, Python, Deno, or any WASI runtime | [`chisel-wasm`](../chisel-wasm) (this library compiled to `wasm32-wasip1`) |
| You just want a ready-to-run MCP server | [`chisel`](../chisel) standalone binary |

---

## API

All functions take a `root: &Path` as their first argument. Every path operation is confined to that root via `cap_std` — kernel-enforced at the `openat` level, not a userspace prefix check.

### Filesystem operations

```rust
use std::path::Path;
use chisel_core::ops::filesystem::{write_file, patch_apply, append, create_directory, move_file};

let root = Path::new("/data");

// Create or overwrite a file (parent dirs created automatically)
write_file(root, "/data/hello.txt", "hello world\n", /*read_only=*/false)?;

// Apply a unified diff atomically — hunk mismatch returns PatchFailed, file untouched
patch_apply(root, "/data/hello.txt",
    "--- a\n+++ b\n@@ -1 +1 @@\n-hello world\n+goodbye world\n",
    /*read_only=*/false)?;

// Append to an existing file (file must already exist)
append(root, "/data/hello.txt", "\nappended line\n", /*read_only=*/false)?;

// mkdir -p semantics
create_directory(root, "/data/sub/dir", /*read_only=*/false)?;

// Move or rename within root
move_file(root, "/data/old.txt", "/data/new.txt", /*read_only=*/false)?;
```

### Shell execution

```rust
use chisel_core::ops::shell::shell_exec;

// Whitelisted commands only: grep rg sed awk find cat head tail wc sort uniq cut tr diff file stat ls du
let out = shell_exec(root, "grep", &["-n", "goodbye", "/data/hello.txt"])?;
println!("exit={} stdout={}", out.exit_code, out.stdout);
```

Commands are spawned via `std::process::Command` directly — no shell interpreter, so metacharacters (`|`, `&&`, `$()`, etc.) in arguments are passed as literals. Path-like arguments are validated against root before the process starts.

> `shell_exec` is **not** available in the WASM build (`chisel-wasm`). Process spawning is not a WASI capability.

---

## Error types

| Variant | Condition |
|---|---|
| `OutsideRoot` | Resolved path escapes the configured root |
| `NotFound` | File or directory does not exist |
| `PatchFailed` | Hunk context does not match current file content |
| `ReadOnly` | Write attempted with `read_only = true` |
| `CommandNotAllowed` | Command not in the compile-time whitelist |
| `PermissionDenied` | OS-level permission denied |

---

## Embedding in Rust

Add to your `Cargo.toml`:

```toml
[dependencies]
chisel-core = { path = "../chisel-core" }   # or publish to crates.io and use a version
```

Inside a synchronous handler, call directly. Inside an async handler, wrap with `spawn_blocking`:

```rust
let result = tokio::task::spawn_blocking(move || {
    chisel_core::ops::filesystem::write_file(&root, &path, &content, read_only)
}).await??;
```

---

## Embedding via WASM (Node.js / Python / Deno)

Build `chisel-wasm` targeting `wasm32-wasip1`:

```bash
rustup target add wasm32-wasip1
cargo build --target wasm32-wasip1 --release -p chisel-wasm
# artifact: target/wasm32-wasip1/release/chisel_wasm.wasm
```

### Node.js (≥ 22)

```js
import { readFile } from "node:fs/promises";
import { WASI } from "node:wasi";
import { argv, env } from "node:process";

const wasi = new WASI({
  version: "preview1",
  args: argv,
  env,
  preopens: { "/data": "/path/to/your/data" },
});

const wasm = await WebAssembly.compile(
  await readFile("target/wasm32-wasip1/release/chisel_wasm.wasm")
);
const instance = await WebAssembly.instantiate(wasm, wasi.getImportObject());
wasi.start(instance);

// Call exported functions via instance.exports
// write_file(root_ptr, path_ptr, content_ptr, read_only) -> result_ptr
```

### Python

```bash
pip install wasmtime
```

```python
from wasmtime import Store, Module, Linker, WasiConfig

store = Store()
config = WasiConfig()
config.preopen_dir("/path/to/your/data", "/data")

linker = Linker(store.engine)
linker.define_wasi()
store.set_wasi(config)

module = Module.from_file(store.engine, "target/wasm32-wasip1/release/chisel_wasm.wasm")
instance = linker.instantiate(store, module)

exports = instance.exports(store)
# write_file, patch_apply, append, create_directory, move_file are available
```

---

## Security properties

All properties are enforced at this layer regardless of how the library is embedded.

| # | Property | Mechanism |
|---|---|---|
| 1 | **Kernel-enforced root confinement** — directory traversal, symlink escape, TOCTOU all blocked | `cap_std::fs::Dir`; every component traversed via `openat(fd, component, O_NOFOLLOW)` |
| 2 | **Atomic writes** — failed patch never corrupts the target file | `Dir::create(".name.PID.tmp")` + `Dir::rename(tmp → target)`; on failure tmp is discarded |
| 3 | **Read-only mode** — blanket write protection | `check_writable(read_only)` runs before any I/O in every write op |
| 4 | **Shell whitelist + direct `execve`** — no injection, no arbitrary commands | Fixed compile-time whitelist; `std::process::Command` spawns directly (no `sh -c`) |

See the [full security model](../README.md#security-model) in the root README for the complete breakdown including attack scenarios and test coverage.
