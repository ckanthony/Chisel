use std::fmt;

use chisel_core::error::CoreError;
use rmcp::model::{Content, IntoContents};

#[derive(Debug)]
#[allow(dead_code)]
pub enum AppError {
    OutsideRoot { path: String, root: String },
    NotFound { path: String },
    PermissionDenied { path: String },
    PatchFailed { reason: String },
    ReadOnly,
    CommandNotAllowed { command: String },
    Other(anyhow::Error),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::OutsideRoot { path, root } => write!(
                f,
                "security error: resolved path '{path}' is outside configured root '{root}'"
            ),
            AppError::NotFound { path } => write!(f, "not found: '{path}'"),
            AppError::PermissionDenied { path } => write!(f, "permission denied: '{path}'"),
            AppError::PatchFailed { reason } => write!(f, "patch failed: {reason}"),
            AppError::ReadOnly => {
                write!(f, "server is in read-only mode; write operations are disabled")
            }
            AppError::CommandNotAllowed { command } => write!(
                f,
                "command not allowed: '{command}'. Permitted: grep, sed, awk, find, cat, head, tail, wc, sort, uniq, cut, tr, diff, file, stat, ls, du, rg"
            ),
            AppError::Other(e) => write!(f, "internal error: {e}"),
        }
    }
}

impl std::error::Error for AppError {}

impl IntoContents for AppError {
    fn into_contents(self) -> Vec<Content> {
        vec![Content::text(self.to_string())]
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        AppError::Other(e)
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::NotFound => AppError::NotFound { path: e.to_string() },
            std::io::ErrorKind::PermissionDenied => {
                AppError::PermissionDenied { path: e.to_string() }
            }
            _ => AppError::Other(anyhow::anyhow!(e)),
        }
    }
}

impl From<CoreError> for AppError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::OutsideRoot { path, root } => AppError::OutsideRoot { path, root },
            CoreError::NotFound { path } => AppError::NotFound { path },
            CoreError::PermissionDenied { path } => AppError::PermissionDenied { path },
            CoreError::PatchFailed { reason } => AppError::PatchFailed { reason },
            CoreError::ReadOnly => AppError::ReadOnly,
            CoreError::CommandNotAllowed { command } => AppError::CommandNotAllowed { command },
            CoreError::Other(msg) => AppError::Other(anyhow::anyhow!(msg)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outside_root_message_contains_path_and_root() {
        let e = AppError::OutsideRoot {
            path: "/etc/passwd".into(),
            root: "/data".into(),
        };
        let msg = e.to_string();
        assert!(msg.contains("/etc/passwd"));
        assert!(msg.contains("/data"));
    }

    #[test]
    fn not_found_message_does_not_leak_listing() {
        let e = AppError::NotFound { path: "/data/secret.txt".into() };
        let msg = e.to_string();
        assert!(msg.contains("/data/secret.txt"));
        assert!(!msg.contains("ls ") && !msg.contains("contents"));
    }

    #[test]
    fn command_not_allowed_lists_permitted_commands() {
        let e = AppError::CommandNotAllowed { command: "bash".into() };
        let msg = e.to_string();
        assert!(msg.contains("bash"));
        assert!(msg.contains("grep"));
        assert!(msg.contains("cat"));
    }

    #[test]
    fn read_only_message_is_clear() {
        let msg = AppError::ReadOnly.to_string();
        assert!(msg.contains("read-only"));
    }

    #[test]
    fn into_contents_produces_text_content() {
        let contents = AppError::ReadOnly.into_contents();
        assert_eq!(contents.len(), 1);
    }

    #[test]
    fn from_core_error_round_trips_variants() {
        assert!(matches!(
            AppError::from(CoreError::ReadOnly),
            AppError::ReadOnly
        ));
        assert!(matches!(
            AppError::from(CoreError::NotFound { path: "x".into() }),
            AppError::NotFound { .. }
        ));
        assert!(matches!(
            AppError::from(CoreError::OutsideRoot { path: "p".into(), root: "r".into() }),
            AppError::OutsideRoot { .. }
        ));
    }
}
