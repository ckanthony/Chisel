//! Integration tests — verify the full stack end-to-end.
//! Tool logic is tested through the public tool functions, and auth/transport
//! are tested through the full axum router.

use std::{fs, sync::Arc};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tempfile::tempdir;
use tower::ServiceExt;

use chisel::{
    config::Config,
    server::run_server_router,
    state::AppState,
    tools::{filesystem, shell},
};

fn make_state(root: &std::path::Path, read_only: bool, secret: &str) -> Arc<AppState> {
    let cfg = Config::from_parts(
        root.canonicalize().unwrap(),
        3000,
        Some(secret.to_owned()),
        read_only,
    )
    .unwrap();
    AppState::new(cfg)
}

// ──────────────────────────────────────────────────────────────────────────
// 9.5 — write_file then shell_exec cat
// ──────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn write_file_then_shell_exec_cat_returns_content() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path(), false, "secret");

    let path = tmp.path().canonicalize().unwrap().join("hello.txt");
    let path_str = path.to_str().unwrap().to_string();

    filesystem::write_file(&state, path_str.clone(), "hello integration\n".to_string())
        .await
        .unwrap();

    let out = shell::shell_exec(&state, "cat".to_string(), vec![path_str])
        .await
        .unwrap();
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "hello integration\n");
}

// ──────────────────────────────────────────────────────────────────────────
// 9.6 — patch_apply round-trip
// ──────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn patch_apply_round_trip() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path(), false, "secret");

    let path = tmp.path().canonicalize().unwrap().join("target.txt");
    let path_str = path.to_str().unwrap().to_string();

    // Write initial content
    filesystem::write_file(&state, path_str.clone(), "line1\nline2\n".to_string())
        .await
        .unwrap();

    // Apply a patch that changes line1 → lineA
    let patch = "--- target.txt\n+++ target.txt\n@@ -1,2 +1,2 @@\n-line1\n+lineA\n line2\n";
    filesystem::patch_apply(&state, path_str.clone(), patch.to_string())
        .await
        .unwrap();

    let result = fs::read_to_string(&path).unwrap();
    assert_eq!(result, "lineA\nline2\n");
}

// ──────────────────────────────────────────────────────────────────────────
// 9.7 — read-only flag blocks write_file
// ──────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn read_only_flag_blocks_write_file() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path(), true, "secret");

    let path = tmp.path().canonicalize().unwrap().join("blocked.txt");

    let err = filesystem::write_file(
        &state,
        path.to_str().unwrap().to_string(),
        "data".to_string(),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, chisel::error::AppError::ReadOnly),
        "expected ReadOnly, got {err:?}"
    );
    assert!(!path.exists(), "file must not have been created");
}

// ──────────────────────────────────────────────────────────────────────────
// 9.8 — auth rejection via HTTP
// ──────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn wrong_bearer_token_returns_401() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path(), false, "correct-secret");

    let app = run_server_router(state);

    // No token
    let resp = app
        .clone()
        .oneshot(Request::get("/mcp").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Wrong token
    let resp = app
        .oneshot(
            Request::get("/mcp")
                .header("Authorization", "Bearer wrong-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ──────────────────────────────────────────────────────────────────────────
// 9.9 — shell_exec with /etc/passwd raises OutsideRoot, no process spawned
// ──────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn shell_exec_etc_passwd_raises_outside_root_no_spawn() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path(), false, "secret");

    let err = shell::shell_exec(&state, "cat".to_string(), vec!["/etc/passwd".to_string()])
        .await
        .unwrap_err();

    assert!(
        matches!(err, chisel::error::AppError::OutsideRoot { .. }),
        "expected OutsideRoot, got {err:?}"
    );
}
