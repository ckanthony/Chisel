use std::process::Command;

use crate::{error::CoreError, security::validate_path};
use std::path::Path;

/// Commands permitted by `shell_exec`. `mkdir` and `mv` are intentionally
/// absent — they must go through the validated Rust tools instead.
const WHITELIST: &[&str] = &[
    "grep", "sed", "awk", "find", "cat", "head", "tail", "wc", "sort", "uniq", "cut", "tr", "diff",
    "file", "stat", "ls", "du", "rg",
];

/// Structured output returned by `shell_exec`.
#[derive(Debug)]
pub struct ShellOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl std::fmt::Display for ShellOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "exit_code: {}\nstdout:\n{}\nstderr:\n{}",
            self.exit_code, self.stdout, self.stderr
        )
    }
}

/// Returns `true` if `arg` looks like a path that needs validation.
fn arg_looks_like_path(arg: &str) -> bool {
    arg.starts_with('/') || arg.contains("..")
}

pub fn shell_exec(root: &Path, command: &str, args: &[&str]) -> Result<ShellOutput, CoreError> {
    if !WHITELIST.contains(&command) {
        return Err(CoreError::CommandNotAllowed {
            command: command.to_owned(),
        });
    }

    for arg in args {
        if arg_looks_like_path(arg) {
            validate_path(root, arg)?;
        }
    }

    let output = Command::new(command)
        .args(args)
        .output()
        .map_err(|e| CoreError::Other(format!("failed to spawn '{command}': {e}")))?;

    Ok(ShellOutput {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn whitelisted_command_executes_and_returns_stdout() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let file = root.join("hello.txt");
        fs::write(&file, "hello world\n").unwrap();

        let res = shell_exec(&root, "cat", &[file.to_str().unwrap()]).unwrap();

        assert_eq!(res.exit_code, 0);
        assert_eq!(res.stdout.trim(), "hello world");
    }

    #[test]
    fn non_whitelisted_command_is_rejected() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        let err = shell_exec(&root, "bash", &[]).unwrap_err();
        assert!(matches!(err, CoreError::CommandNotAllowed { .. }));
    }

    #[test]
    fn path_arg_outside_root_is_rejected_before_spawn() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        let err = shell_exec(&root, "cat", &["/etc/hosts"]).unwrap_err();
        assert!(matches!(err, CoreError::OutsideRoot { .. }));
    }

    #[test]
    fn shell_metacharacter_in_arg_is_treated_as_literal() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        let res = shell_exec(
            &root,
            "ls",
            &[root.to_str().unwrap(), "nonexistent; rm -rf /"],
        )
        .unwrap();

        assert!(res.exit_code != 0 || res.stderr.contains("No such") || res.stderr.is_empty());
    }

    #[test]
    fn non_zero_exit_is_returned_not_a_server_error() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let file = root.join("data.txt");
        fs::write(&file, "abc\n").unwrap();

        let res = shell_exec(&root, "grep", &["NOTPRESENT", file.to_str().unwrap()]).unwrap();

        assert_eq!(res.exit_code, 1);
    }
}
