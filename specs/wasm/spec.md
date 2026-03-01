# WASM Build Specification

## Purpose

`chisel-wasm` is a `wasm32-wasip1`-targeted crate that exposes `chisel-core`'s filesystem operations to any WASI-capable runtime (Node.js ≥ 22 native WASI, Deno, Wasmtime, etc.). It allows MCP servers written in Node.js, Python, Go, or any language with a WASI host to reuse the same path-confinement and patch logic without running a separate HTTP server.

---

## Non-Goals

- `shell_exec` is NOT available in the WASM build; process spawning is not a WASI capability.
- This crate does not implement the MCP protocol. It is a pure operations library.
- Browser (`wasm32-unknown-unknown`) is not a target; filesystem access requires WASI.

---

## Requirements

### Requirement: Target Compatibility

The crate MUST compile cleanly for `wasm32-wasip1`. It MUST NOT link against any native-only system library or depend on any crate that does (e.g. no `libc` direct usage, no process spawning).

#### Scenario: Clean WASM compile
- WHEN `cargo build --target wasm32-wasip1 -p chisel-wasm` is run
- THEN it exits with code 0 and produces a `.wasm` artifact with no linker errors

---

### Requirement: Available Operations

The following operations MUST be available in the WASM build with identical behavior to their native counterparts:

| Operation | Available |
|-----------|-----------|
| `validate_path` | YES |
| `patch_apply` | YES |
| `append` | YES |
| `write_file` | YES |
| `create_directory` | YES |
| `move_file` | YES |
| `shell_exec` | NO — excluded at compile time |

#### Scenario: File write inside root succeeds under WASI
- GIVEN the WASM module is loaded with WASI preopened dir `/data`
- WHEN `write_file` is called with root `/data`, path `/data/out.txt`, content `"hello"`
- THEN `/data/out.txt` exists with content `"hello"` on the host filesystem

#### Scenario: Path escape rejected under WASI
- GIVEN the WASM module is loaded with WASI preopened dir `/data`
- WHEN `validate_path` is called with root `/data`, input `/etc/hosts`
- THEN returns `CoreError::OutsideRoot` and no file is accessed

---

### Requirement: Node.js Consumption

A Node.js consumer MUST be able to load and call the module using the Node.js built-in `node:wasi` API (Node.js ≥ 22) without any native addon or build step beyond `cargo build`.

#### Scenario: Node.js calls write_file
- GIVEN a Node.js script that instantiates the `.wasm` module via `node:wasi`
- WHEN `write_file` is invoked with a path inside the preopened directory
- THEN the file is created on disk and the function returns a success string

---

### Requirement: Atomic Write Behavior

`patch_apply` MUST preserve atomic write semantics (temp file + rename) on WASM/WASI targets, identical to native behavior. A failed or interrupted patch MUST NOT leave the target file in a partial state.

#### Scenario: Failed patch leaves file unchanged under WASI
- GIVEN the WASM module is loaded and `/data/a.txt` contains `"original\n"`
- WHEN `patch_apply` is called with a hunk that expects `"wrong context"`
- THEN returns `CoreError::PatchFailed` and `/data/a.txt` still contains `"original\n"`
