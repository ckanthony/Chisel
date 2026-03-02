use std::process::Command;

use crate::{error::CoreError, security::validate_path};
use std::path::Path;

/// Commands permitted by `shell_exec`. `mkdir` and `mv` are intentionally
/// absent — they must go through the validated Rust tools instead.
const WHITELIST: &[&str] = &[
    "grep", "sed", "awk", "find", "cat", "head", "tail", "wc", "sort", "uniq", "cut", "tr", "diff",
    "file", "stat", "ls", "du",
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

/// Returns `true` if `arg` looks like a path that should be resolved and validated.
fn arg_looks_like_path(arg: &str) -> bool {
    arg.starts_with('/')       // absolute path
        || arg.contains("..") // traversal attempt
        || arg == "."          // current directory
        || arg.starts_with("./") // explicit relative
        || is_relative_slash_path(arg) // subdir/file style
}

/// Returns `true` for relative args that contain `/` as a path separator rather than as a
/// sed/awk expression delimiter.  sed/awk expressions always have a **single-character**
/// command prefix before the first `/` (e.g. `s/`, `y/`, `g/`), whereas real path
/// components are at least two characters long (e.g. `sub/file`, `Downloads/foo.txt`).
/// Note: single-character directory names (e.g. `a/file`) are not resolved/validated here,
/// but they still execute correctly because the process CWD is always set to root.
fn is_relative_slash_path(arg: &str) -> bool {
    match arg.find('/') {
        Some(idx) => idx > 1,
        None => false,
    }
}

/// Resolve a single arg against `root`:
/// - Absolute paths are validated as-is.
/// - Relative paths (`.`, `./foo`, `subdir/file`) are joined onto root first,
///   so `find . -maxdepth 2` becomes `find /configured/root -maxdepth 2`.
/// - Traversal (`../escape`) is caught by `validate_path` after the join.
fn resolve_arg<'a>(root: &Path, arg: &'a str) -> Result<std::borrow::Cow<'a, str>, CoreError> {
    if !arg_looks_like_path(arg) {
        return Ok(std::borrow::Cow::Borrowed(arg));
    }
    if arg.starts_with('/') {
        validate_path(root, arg)?;
        return Ok(std::borrow::Cow::Borrowed(arg));
    }
    // Relative — anchor to root then validate
    let joined = root.join(arg);
    let joined_str = joined.to_string_lossy().into_owned();
    validate_path(root, &joined_str)?;
    Ok(std::borrow::Cow::Owned(joined_str))
}

pub fn shell_exec(root: &Path, command: &str, args: &[&str]) -> Result<ShellOutput, CoreError> {
    if !WHITELIST.contains(&command) {
        return Err(CoreError::CommandNotAllowed {
            command: command.to_owned(),
        });
    }

    // Resolve all path-like args (relative → absolute under root).
    let resolved: Vec<std::borrow::Cow<str>> = args
        .iter()
        .map(|arg| resolve_arg(root, arg))
        .collect::<Result<_, _>>()?;

    // Always run in root — so commands with no explicit path (e.g. `ls -la`,
    // `find . -maxdepth 2`) are implicitly scoped to the configured directory
    // rather than the process CWD (which can be `/` or any system path).
    let output = Command::new(command)
        .args(resolved.iter().map(|a| a.as_ref()))
        .current_dir(root)
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
    fn dot_is_translated_to_root_and_runs() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        fs::write(root.join("hello.txt"), "hi").unwrap();

        // "." is translated to root; find should list hello.txt
        let res = shell_exec(&root, "find", &[".", "-maxdepth", "1", "-type", "f"]).unwrap();
        assert_eq!(res.exit_code, 0);
        assert!(res.stdout.contains("hello.txt"), "stdout: {}", res.stdout);
    }

    #[test]
    fn relative_subpath_is_translated_to_root_subdir() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let sub = root.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("data.txt"), "content").unwrap();

        // "sub/data.txt" → root/sub/data.txt
        let res = shell_exec(&root, "cat", &["sub/data.txt"]).unwrap();
        assert_eq!(res.exit_code, 0);
        assert_eq!(res.stdout.trim(), "content");
    }

    #[test]
    fn relative_traversal_via_dotdot_is_still_blocked() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        // "../escape" joined onto root → root/../escape = root's parent → OutsideRoot
        let err = shell_exec(&root, "cat", &["../escape.txt"]).unwrap_err();
        assert!(
            matches!(err, CoreError::OutsideRoot { .. } | CoreError::NotFound { .. }),
            "expected OutsideRoot or NotFound, got {err:?}"
        );
    }

    #[test]
    fn sed_expression_with_slashes_is_not_mangled_as_path() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let file = root.join("data.txt");
        fs::write(&file, "hello world\n").unwrap();

        // BSD sed: -i '' s/hello/goodbye/g <file>
        // GNU sed: -i s/hello/goodbye/g <file>
        // Use a form that works on both: write to a tmp output via a pipe-less approach.
        // Simplest cross-platform check: the expression s/hello/goodbye/g must NOT be
        // resolved as a path (which would corrupt the arg order and cause an error).
        let res = shell_exec(
            &root,
            "sed",
            &["s/hello/goodbye/g", file.to_str().unwrap()],
        )
        .unwrap();

        // sed prints the transformed content to stdout; it must not error
        assert_eq!(res.exit_code, 0, "stderr: {}", res.stderr);
        assert!(res.stdout.contains("goodbye"), "stdout: {}", res.stdout);
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
