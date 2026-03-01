use std::fmt;

#[derive(Debug)]
pub enum CoreError {
    OutsideRoot { path: String, root: String },
    NotFound { path: String },
    PermissionDenied { path: String },
    PatchFailed { reason: String },
    ReadOnly,
    CommandNotAllowed { command: String },
    Other(String),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::OutsideRoot { path, root } => write!(
                f,
                "security error: resolved path '{path}' is outside configured root '{root}'"
            ),
            CoreError::NotFound { path } => write!(f, "not found: '{path}'"),
            CoreError::PermissionDenied { path } => write!(f, "permission denied: '{path}'"),
            CoreError::PatchFailed { reason } => write!(f, "patch failed: {reason}"),
            CoreError::ReadOnly => {
                write!(f, "server is in read-only mode; write operations are disabled")
            }
            CoreError::CommandNotAllowed { command } => write!(
                f,
                "command not allowed: '{command}'. Permitted: grep, sed, awk, find, cat, head, tail, wc, sort, uniq, cut, tr, diff, file, stat, ls, du, rg"
            ),
            CoreError::Other(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

impl std::error::Error for CoreError {}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::NotFound => CoreError::NotFound { path: e.to_string() },
            std::io::ErrorKind::PermissionDenied => {
                CoreError::PermissionDenied { path: e.to_string() }
            }
            _ => CoreError::Other(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outside_root_message_contains_path_and_root() {
        let e = CoreError::OutsideRoot {
            path: "/etc/passwd".into(),
            root: "/data".into(),
        };
        let msg = e.to_string();
        assert!(msg.contains("/etc/passwd"));
        assert!(msg.contains("/data"));
    }

    #[test]
    fn not_found_message_does_not_leak_listing() {
        let e = CoreError::NotFound { path: "/data/secret.txt".into() };
        let msg = e.to_string();
        assert!(msg.contains("/data/secret.txt"));
        assert!(!msg.contains("ls ") && !msg.contains("contents"));
    }

    #[test]
    fn command_not_allowed_lists_permitted_commands() {
        let e = CoreError::CommandNotAllowed { command: "bash".into() };
        let msg = e.to_string();
        assert!(msg.contains("bash"));
        assert!(msg.contains("grep"));
        assert!(msg.contains("cat"));
    }

    #[test]
    fn read_only_message_is_clear() {
        let msg = CoreError::ReadOnly.to_string();
        assert!(msg.contains("read-only"));
    }

    #[test]
    fn implements_std_error() {
        let e: &dyn std::error::Error = &CoreError::ReadOnly;
        assert!(!e.to_string().is_empty());
    }
}
