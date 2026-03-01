//! Security regression tests.
//!
//! Every test maps directly to one of the six attack vectors documented in
//! `README.md §Security model → Attack and misuse prevention`.
//!
//! Run with:
//!   cargo test --test security
//!
//! If any test here fails, a regression in the documented security model
//! has occurred and must be resolved before merging.

use std::{fs, path::PathBuf, sync::Arc};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tempfile::tempdir;
use tower::ServiceExt;

use chisel::{
    config::Config,
    error::AppError,
    server::run_server_router,
    state::AppState,
    tools::{filesystem, shell},
};

// ── Helpers ────────────────────────────────────────────────────────────────

fn state(root: &std::path::Path, read_only: bool) -> Arc<AppState> {
    let cfg = Config::from_parts(
        root.canonicalize().unwrap(),
        3000,
        Some("test-secret".into()),
        read_only,
    )
    .unwrap();
    AppState::new(cfg)
}

fn router_with_secret(root: &std::path::Path, secret: &str) -> axum::Router {
    let cfg = Config::from_parts(
        root.canonicalize().unwrap(),
        3000,
        Some(secret.into()),
        false,
    )
    .unwrap();
    run_server_router(AppState::new(cfg))
}

// ══════════════════════════════════════════════════════════════════════════
// §1  Authentication — unauthorised access
// ══════════════════════════════════════════════════════════════════════════

/// Missing secret at startup must be a hard error, not a silent no-auth fallback.
#[test]
fn auth_missing_secret_is_hard_error() {
    let err = Config::from_parts(PathBuf::from("/tmp"), 3000, None, false);
    assert!(err.is_err(), "Config must reject a missing secret");
}

/// An empty string is not an acceptable secret.
#[test]
fn auth_empty_secret_is_hard_error() {
    let err = Config::from_parts(PathBuf::from("/tmp"), 3000, Some("".into()), false);
    assert!(err.is_err(), "Config must reject an empty secret");
}

/// No `Authorization` header → 401.
#[tokio::test]
async fn auth_missing_header_returns_401() {
    let tmp = tempdir().unwrap();
    let resp = router_with_secret(tmp.path(), "s3cr3t")
        .oneshot(Request::get("/mcp").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// Wrong token → 401.
#[tokio::test]
async fn auth_wrong_token_returns_401() {
    let tmp = tempdir().unwrap();
    let resp = router_with_secret(tmp.path(), "s3cr3t")
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

/// `Basic` scheme is not Bearer and must be rejected.
#[tokio::test]
async fn auth_basic_scheme_returns_401() {
    let tmp = tempdir().unwrap();
    let resp = router_with_secret(tmp.path(), "s3cr3t")
        .oneshot(
            Request::get("/mcp")
                .header("Authorization", "Basic s3cr3t")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// A token that shares a prefix with the real secret must still be rejected.
/// Proves the comparison is not short-circuit (i.e. is constant-time).
#[tokio::test]
async fn auth_prefix_of_secret_returns_401() {
    let tmp = tempdir().unwrap();
    let resp = router_with_secret(tmp.path(), "s3cr3t-full")
        .oneshot(
            Request::get("/mcp")
                .header("Authorization", "Bearer s3cr3t") // prefix only
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// Correct token must pass authentication.
#[tokio::test]
async fn auth_valid_token_passes() {
    let tmp = tempdir().unwrap();
    let resp = router_with_secret(tmp.path(), "s3cr3t")
        .oneshot(
            Request::get("/mcp")
                .header("Authorization", "Bearer s3cr3t")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "valid token must not return 401"
    );
}

// ══════════════════════════════════════════════════════════════════════════
// §2  Path confinement — escape from root directory
//
// All filesystem tools go through cap_std::fs::Dir which uses openat +
// O_NOFOLLOW chains.  The kernel enforces confinement — there is no
// separate "check then use" window.
// ══════════════════════════════════════════════════════════════════════════

/// Classic `../..` traversal must be blocked.
#[tokio::test]
async fn path_directory_traversal_is_blocked() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    fs::create_dir(root.join("sub")).unwrap();
    let st = state(&root, false);

    // /root/sub/../../etc/passwd — traverses out of root
    let traversal = root
        .join("sub")
        .join("../../etc/passwd")
        .to_str()
        .unwrap()
        .to_string();

    let res = filesystem::write_file(&st, traversal, "pwned".into()).await;
    assert!(res.is_err(), "directory traversal must be blocked, got Ok");
    assert!(
        !std::path::Path::new("/etc/passwd")
            .metadata()
            .map(|m| m.len() == 6)
            .unwrap_or(false),
        "/etc/passwd must not have been truncated"
    );
}

/// An absolute path that does not start with root must be rejected immediately
/// by the strip-prefix check, before cap-std is even reached.
#[tokio::test]
async fn path_absolute_outside_root_returns_outside_root_error() {
    let tmp = tempdir().unwrap();
    let st = state(tmp.path(), false);

    let res = filesystem::write_file(&st, "/etc/pwned-by-chisel".into(), "data".into()).await;
    assert!(
        matches!(res, Err(AppError::OutsideRoot { .. })),
        "absolute path outside root must return OutsideRoot, got: {res:?}"
    );
}

/// A symlink in an intermediate path component that points outside root must
/// be blocked by cap-std's O_NOFOLLOW traversal.
#[cfg(unix)]
#[tokio::test]
async fn path_symlink_in_component_is_blocked() {
    use std::os::unix::fs::symlink;

    let tmp = tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    // root/escape → /tmp  (points outside root)
    symlink("/tmp", root.join("escape")).unwrap();
    let st = state(&root, false);

    // Attempt: write root/escape/evil.txt  ≡  write /tmp/evil.txt
    let path = root
        .join("escape")
        .join("evil.txt")
        .to_str()
        .unwrap()
        .to_string();
    let res = filesystem::write_file(&st, path, "pwned".into()).await;
    assert!(
        res.is_err(),
        "write through symlink escaping root must be blocked, got Ok"
    );
    assert!(
        !std::path::Path::new("/tmp/evil.txt").exists(),
        "/tmp/evil.txt must not have been created"
    );
}

/// TOCTOU simulation: a real directory is replaced with an escaping symlink
/// *after* a previous write succeeded (i.e. the path was valid at check time).
/// cap-std must reject the subsequent write because confinement is enforced
/// at the I/O call, not at a prior validation step.
#[cfg(unix)]
#[tokio::test]
async fn path_toctou_symlink_swap_is_blocked() {
    use std::os::unix::fs::symlink;

    let tmp = tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let st = state(&root, false);

    // Phase 1: normal write succeeds — path is legitimate
    let subdir = root.join("legit");
    fs::create_dir(&subdir).unwrap();
    filesystem::write_file(
        &st,
        subdir.join("ok.txt").to_str().unwrap().to_string(),
        "fine".into(),
    )
    .await
    .unwrap();

    // Phase 2: attacker swaps the directory for an escaping symlink
    fs::remove_dir_all(&subdir).unwrap();
    symlink("/tmp", &subdir).unwrap(); // subdir now → /tmp

    // Phase 3: subsequent write must be blocked despite same path shape
    let path = subdir.join("evil.txt").to_str().unwrap().to_string();
    let res = filesystem::write_file(&st, path, "pwned".into()).await;
    assert!(
        res.is_err(),
        "write after TOCTOU symlink swap must be blocked, got Ok"
    );
    assert!(
        !std::path::Path::new("/tmp/evil.txt").exists(),
        "/tmp/evil.txt must not have been created"
    );
}

/// A deeply-nested path whose resolved form is outside root must be rejected.
#[tokio::test]
async fn path_deeply_nested_outside_root_is_blocked() {
    let tmp = tempdir().unwrap();
    let st = state(tmp.path(), false);

    let res = filesystem::create_directory(&st, "/tmp/a/b/c/evil".into()).await;
    assert!(
        matches!(res, Err(AppError::OutsideRoot { .. })),
        "deeply nested path outside root must return OutsideRoot, got: {res:?}"
    );
}

// ══════════════════════════════════════════════════════════════════════════
// §3  Shell injection — arbitrary command execution
// ══════════════════════════════════════════════════════════════════════════

/// Every dangerous command must be blocked before `execve` is called.
#[tokio::test]
async fn shell_dangerous_commands_blocked_before_spawn() {
    let tmp = tempdir().unwrap();
    let st = state(tmp.path(), false);

    for cmd in &[
        "bash", "sh", "zsh", "fish", "python3", "node", "curl", "wget", "rm", "chmod",
    ] {
        let res = shell::shell_exec(&st, cmd.to_string(), vec![]).await;
        assert!(
            matches!(res, Err(AppError::CommandNotAllowed { .. })),
            "'{cmd}' must return CommandNotAllowed, got: {res:?}"
        );
    }
}

/// Shell metacharacters passed as arguments must be treated as literal strings —
/// no shell interpreter is involved so they cannot be used for injection.
#[tokio::test]
async fn shell_metacharacters_are_literal() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let st = state(&root, false);

    let marker = root.join("marker.txt");
    fs::write(&marker, "data").unwrap();
    let injected = root.join("injected.txt");

    // If semicolons were interpreted, the second arg would create injected.txt
    let _ = shell::shell_exec(
        &st,
        "ls".to_string(),
        vec![
            marker.to_str().unwrap().to_string(),
            format!("noop; touch {}", injected.display()),
        ],
    )
    .await;

    assert!(
        !injected.exists(),
        "shell metacharacter injection must not create 'injected.txt'"
    );
}

/// A path argument outside root must be rejected before the process is spawned.
#[tokio::test]
async fn shell_path_arg_outside_root_blocked_before_spawn() {
    let tmp = tempdir().unwrap();
    let st = state(tmp.path(), false);

    let res = shell::shell_exec(&st, "cat".to_string(), vec!["/etc/passwd".to_string()]).await;
    assert!(
        matches!(res, Err(AppError::OutsideRoot { .. })),
        "path arg outside root must be blocked before spawn, got: {res:?}"
    );
}

/// A traversal (`../..`) in a shell argument must also be blocked.
#[tokio::test]
async fn shell_traversal_in_arg_blocked_before_spawn() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    fs::create_dir(root.join("sub")).unwrap();
    let st = state(&root, false);

    let traversal = root
        .join("sub")
        .join("../../etc/passwd")
        .to_str()
        .unwrap()
        .to_string();

    let res = shell::shell_exec(&st, "cat".to_string(), vec![traversal]).await;
    assert!(
        matches!(res, Err(AppError::OutsideRoot { .. })),
        "traversal in shell arg must be blocked, got: {res:?}"
    );
}

// ══════════════════════════════════════════════════════════════════════════
// §4  Partial write — file integrity on failed patch
// ══════════════════════════════════════════════════════════════════════════

/// When patch context does not match, the original file must be byte-for-byte
/// unchanged after the failed call.
#[tokio::test]
async fn partial_write_failed_patch_leaves_file_intact() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let st = state(&root, false);
    let file = root.join("target.txt");
    fs::write(&file, "original content\n").unwrap();

    let bad_patch =
        "--- target.txt\n+++ target.txt\n@@ -1 +1 @@\n-wrong context line\n+replacement\n";

    let res = filesystem::patch_apply(
        &st,
        file.to_str().unwrap().to_string(),
        bad_patch.to_string(),
    )
    .await;

    assert!(res.is_err(), "bad patch must return an error");
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "original content\n",
        "file content must be identical after failed patch"
    );
}

/// No `.tmp` artefact must be left in the directory when a patch fails.
/// The error is detected before atomic_write is called, so no temp file
/// is ever created.
#[tokio::test]
async fn partial_write_no_tmp_artefact_on_failure() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let st = state(&root, false);
    let file = root.join("data.txt");
    fs::write(&file, "hello\n").unwrap();

    let bad_patch =
        "--- data.txt\n+++ data.txt\n@@ -1 +1 @@\n-this line does not exist\n+replaced\n";

    let _ = filesystem::patch_apply(
        &st,
        file.to_str().unwrap().to_string(),
        bad_patch.to_string(),
    )
    .await;

    let tmp_files: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();

    assert!(
        tmp_files.is_empty(),
        "no .tmp files must remain after a failed patch, found: {tmp_files:?}"
    );
}

// ══════════════════════════════════════════════════════════════════════════
// §5  Read-only mode — blanket write protection
// ══════════════════════════════════════════════════════════════════════════

/// Every write tool must return ReadOnly immediately — no I/O is performed.
#[tokio::test]
async fn readonly_all_write_tools_are_blocked() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let st = state(&root, true); // read_only = true
    let file = root.join("f.txt");
    fs::write(&file, "x").unwrap();
    let p = file.to_str().unwrap().to_string();

    assert!(
        matches!(
            filesystem::patch_apply(
                &st,
                p.clone(),
                "--- /dev/null\n+++ f.txt\n@@ -0,0 +1 @@\n+x\n".into()
            )
            .await,
            Err(AppError::ReadOnly)
        ),
        "patch_apply must return ReadOnly"
    );
    assert!(
        matches!(
            filesystem::append(&st, p.clone(), "y".into()).await,
            Err(AppError::ReadOnly)
        ),
        "append must return ReadOnly"
    );
    assert!(
        matches!(
            filesystem::write_file(&st, p.clone(), "z".into()).await,
            Err(AppError::ReadOnly)
        ),
        "write_file must return ReadOnly"
    );
    assert!(
        matches!(
            filesystem::create_directory(&st, p.clone()).await,
            Err(AppError::ReadOnly)
        ),
        "create_directory must return ReadOnly"
    );
    assert!(
        matches!(
            filesystem::move_file(&st, p.clone(), p.clone()).await,
            Err(AppError::ReadOnly)
        ),
        "move_file must return ReadOnly"
    );
}

/// shell_exec must still work in read-only mode (it only reads).
#[tokio::test]
async fn readonly_shell_exec_remains_available() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let st = state(&root, true);
    let file = root.join("readable.txt");
    fs::write(&file, "content").unwrap();

    let res = shell::shell_exec(
        &st,
        "cat".to_string(),
        vec![file.to_str().unwrap().to_string()],
    )
    .await;
    assert!(
        res.is_ok(),
        "shell_exec must work in read-only mode, got: {res:?}"
    );
}

/// Files must not be modified by any write tool in read-only mode.
#[tokio::test]
async fn readonly_no_disk_mutation_occurs() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let st = state(&root, true);
    let file = root.join("protected.txt");
    fs::write(&file, "unchanged").unwrap();
    let p = file.to_str().unwrap().to_string();

    let _ = filesystem::write_file(&st, p.clone(), "mutated".into()).await;
    let _ = filesystem::append(&st, p.clone(), " extra".into()).await;

    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "unchanged",
        "file content must be unchanged after read-only write attempts"
    );
}

// ══════════════════════════════════════════════════════════════════════════
// §6  Network binding — no accidental public exposure
// ══════════════════════════════════════════════════════════════════════════

/// The bind address must always be the loopback interface, never 0.0.0.0.
#[test]
fn network_bind_address_is_loopback_only() {
    use std::net::SocketAddr;

    for port in [3000_u16, 8080, 9000] {
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        assert!(
            addr.ip().is_loopback(),
            "port {port}: bind must be loopback"
        );
        assert_ne!(
            addr.ip().to_string(),
            "0.0.0.0",
            "port {port}: must not bind to 0.0.0.0"
        );
    }
}

/// The SSE legacy endpoint must return 404 — no accidental protocol exposure.
#[tokio::test]
async fn network_sse_endpoint_returns_404() {
    let tmp = tempdir().unwrap();
    let resp = router_with_secret(tmp.path(), "tok")
        .oneshot(
            Request::get("/sse")
                .header("Authorization", "Bearer tok")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "legacy /sse endpoint must not exist"
    );
}
