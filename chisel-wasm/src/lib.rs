/// Re-export the full mcp-fs-core surface for WASM consumers.
///
/// `shell_exec` is excluded: it is cfg-gated out on `wasm32` targets
/// since process spawning is not available under WASI.
pub use chisel_core::error::CoreError;
pub use chisel_core::ops::filesystem::{
    append, create_directory, move_file, patch_apply, strip_diff_fence, write_file,
};
pub use chisel_core::security::validate_path;
