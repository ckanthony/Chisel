use chisel_core::ops::filesystem as core;
use tracing::{info, warn};

use crate::{error::AppError, state::AppState};

pub use core::strip_diff_fence;

pub async fn patch_apply(
    state: &AppState,
    path: String,
    patch_text: String,
) -> Result<String, AppError> {
    let result = core::patch_apply(
        &state.config.root,
        &path,
        &patch_text,
        state.config.read_only,
    )
    .map_err(AppError::from);
    audit("patch_apply", &path, &result);
    result
}

pub async fn append(state: &AppState, path: String, content: String) -> Result<String, AppError> {
    let result = core::append(&state.config.root, &path, &content, state.config.read_only)
        .map_err(AppError::from);
    audit("append", &path, &result);
    result
}

pub async fn write_file(
    state: &AppState,
    path: String,
    content: String,
) -> Result<String, AppError> {
    let result = core::write_file(&state.config.root, &path, &content, state.config.read_only)
        .map_err(AppError::from);
    audit("write_file", &path, &result);
    result
}

pub async fn create_directory(state: &AppState, path: String) -> Result<String, AppError> {
    let result = core::create_directory(&state.config.root, &path, state.config.read_only)
        .map_err(AppError::from);
    audit("create_directory", &path, &result);
    result
}

pub async fn move_file(
    state: &AppState,
    source: String,
    destination: String,
) -> Result<String, AppError> {
    let result = core::move_file(
        &state.config.root,
        &source,
        &destination,
        state.config.read_only,
    )
    .map_err(AppError::from);
    // Log source→destination as the "path" for traceability.
    audit("move_file", &format!("{source} -> {destination}"), &result);
    result
}

fn audit<T>(op: &str, path: &str, result: &Result<T, AppError>) {
    match result {
        Ok(_) => info!(op, path, "ok"),
        Err(e) => warn!(op, path, error = %e, "failed"),
    }
}
