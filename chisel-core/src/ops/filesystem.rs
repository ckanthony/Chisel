use std::{io, path::Path};

use diffy::{Patch, apply};

use crate::error::CoreError;

// ── Shared helpers ─────────────────────────────────────────────────────────

/// Strip an optional markdown code fence (` ```[lang] ` / ` ``` `) that
/// an LLM may wrap around a unified diff.
pub fn strip_diff_fence(input: &str) -> &str {
    let trimmed = input.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return input;
    };
    let rest = rest.trim_start_matches(|c: char| c.is_alphabetic());
    let rest = rest.strip_prefix('\n').unwrap_or(rest);

    if let Some(pos) = rest.rfind("\n```") {
        &rest[..pos + 1]
    } else {
        rest
    }
}

fn check_writable(read_only: bool) -> Result<(), CoreError> {
    if read_only {
        Err(CoreError::ReadOnly)
    } else {
        Ok(())
    }
}

// ── Native implementation — cap-std (kernel-enforced confinement) ──────────
//
// All I/O goes through a `cap_std::fs::Dir` handle rooted at `root`.
// The kernel uses openat + O_NOFOLLOW for every component traversal, so:
//   • symlinks in intermediate components cannot escape root
//   • `..` is tracked per-component and blocked if it would cross root
//   • TOCTOU: a symlink swap between validate_path and the I/O call
//     is caught by the kernel during the openat chain, not in userspace
//
// `validate_path` (security.rs) is intentionally NOT called here — it is
// kept only for shell_exec, where we pass path strings to spawned processes
// that cap-std cannot confine.

#[cfg(not(target_family = "wasm"))]
mod native {
    use super::*;
    use cap_std::{ambient_authority, fs::Dir};
    use std::{io::Write, path::PathBuf};

    pub fn open_root(root: &Path) -> Result<Dir, CoreError> {
        Dir::open_ambient_dir(root, ambient_authority())
            .map_err(|e| CoreError::Other(e.to_string()))
    }

    /// Convert an absolute input path to a root-relative path by stripping
    /// the root prefix.  Relative inputs are returned as-is.
    ///
    /// After this step cap-std does all the actual traversal — `..` and
    /// symlink escapes in the relative path are blocked at the kernel level.
    pub fn to_rel(root: &Path, input: &str) -> Result<PathBuf, CoreError> {
        let p = Path::new(input);
        if p.is_absolute() {
            p.strip_prefix(root)
                .map(|r| r.to_path_buf())
                .map_err(|_| CoreError::OutsideRoot {
                    path: input.to_owned(),
                    root: root.to_string_lossy().into_owned(),
                })
        } else {
            Ok(p.to_path_buf())
        }
    }

    /// Write `content` atomically: write to a `.pid.tmp` file confined inside
    /// `dir`, then rename into place.  Both operations go through openat so
    /// the entire sequence is kernel-confined.
    pub fn atomic_write(dir: &Dir, rel: &Path, content: &str) -> Result<(), CoreError> {
        let filename = rel
            .file_name()
            .ok_or_else(|| CoreError::Other("path has no filename".into()))?;
        let tmp_name = format!(".{}.{}.tmp", filename.to_string_lossy(), std::process::id());
        let parent_rel = rel.parent().filter(|p| !p.as_os_str().is_empty());

        if let Some(parent) = parent_rel {
            let pd = dir
                .open_dir(parent)
                .map_err(|e| CoreError::Other(e.to_string()))?;
            {
                let mut f = pd
                    .create(&tmp_name)
                    .map_err(|e| CoreError::Other(e.to_string()))?;
                f.write_all(content.as_bytes()).map_err(CoreError::from)?;
            }
            pd.rename(&tmp_name, &pd, filename)
                .map_err(|e| CoreError::Other(e.to_string()))?;
        } else {
            {
                let mut f = dir
                    .create(&tmp_name)
                    .map_err(|e| CoreError::Other(e.to_string()))?;
                f.write_all(content.as_bytes()).map_err(CoreError::from)?;
            }
            dir.rename(&tmp_name, dir, filename)
                .map_err(|e| CoreError::Other(e.to_string()))?;
        }
        Ok(())
    }

    // ── patch_apply ──────────────────────────────────────────────────────────

    pub fn patch_apply(
        root: &Path,
        path: &str,
        patch_text: &str,
        read_only: bool,
    ) -> Result<String, CoreError> {
        check_writable(read_only)?;
        let dir = open_root(root)?;
        let rel = to_rel(root, path)?;

        let raw = strip_diff_fence(patch_text);
        let patch = Patch::from_str(raw).map_err(|e| CoreError::PatchFailed {
            reason: e.to_string(),
        })?;

        let existing = if patch.original().map_or(false, |s| s == "/dev/null") {
            String::new()
        } else {
            dir.read_to_string(&rel).map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => CoreError::NotFound {
                    path: path.to_owned(),
                },
                io::ErrorKind::PermissionDenied => CoreError::PermissionDenied {
                    path: path.to_owned(),
                },
                _ => CoreError::Other(e.to_string()),
            })?
        };

        let new_content = apply(&existing, &patch).map_err(|e| CoreError::PatchFailed {
            reason: format!("hunk mismatch (file may have drifted): {e:?}"),
        })?;

        // Ensure parent directory exists (needed for /dev/null → new file case)
        if let Some(parent) = rel.parent().filter(|p| !p.as_os_str().is_empty()) {
            dir.create_dir_all(parent)
                .map_err(|e| CoreError::Other(e.to_string()))?;
        }

        atomic_write(&dir, &rel, &new_content)?;
        Ok(format!("patched: {path}"))
    }

    // ── append ───────────────────────────────────────────────────────────────

    pub fn append(
        root: &Path,
        path: &str,
        content: &str,
        read_only: bool,
    ) -> Result<String, CoreError> {
        check_writable(read_only)?;
        let dir = open_root(root)?;
        let rel = to_rel(root, path)?;

        // Opening with append(true) and no create(true) returns NotFound if the
        // file does not exist — matching the spec's "MUST NOT create" requirement.
        let mut f = dir
            .open_with(&rel, cap_std::fs::OpenOptions::new().append(true))
            .map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => CoreError::NotFound {
                    path: path.to_owned(),
                },
                io::ErrorKind::PermissionDenied => CoreError::PermissionDenied {
                    path: path.to_owned(),
                },
                _ => CoreError::Other(e.to_string()),
            })?;

        use std::io::Write;
        f.write_all(content.as_bytes()).map_err(CoreError::from)?;
        Ok(format!("appended {} bytes to {path}", content.len()))
    }

    // ── write_file ───────────────────────────────────────────────────────────

    pub fn write_file(
        root: &Path,
        path: &str,
        content: &str,
        read_only: bool,
    ) -> Result<String, CoreError> {
        check_writable(read_only)?;
        let dir = open_root(root)?;
        let rel = to_rel(root, path)?;

        if let Some(parent) = rel.parent().filter(|p| !p.as_os_str().is_empty()) {
            dir.create_dir_all(parent)
                .map_err(|e| CoreError::Other(e.to_string()))?;
        }
        dir.write(&rel, content.as_bytes())
            .map_err(CoreError::from)?;
        Ok(format!("wrote {} bytes to {path}", content.len()))
    }

    // ── create_directory ─────────────────────────────────────────────────────

    pub fn create_directory(root: &Path, path: &str, read_only: bool) -> Result<String, CoreError> {
        check_writable(read_only)?;
        let dir = open_root(root)?;
        let rel = to_rel(root, path)?;

        dir.create_dir_all(&rel)
            .map_err(|e| CoreError::Other(e.to_string()))?;
        Ok(format!("created directory: {path}"))
    }

    // ── move_file ────────────────────────────────────────────────────────────

    pub fn move_file(
        root: &Path,
        source: &str,
        destination: &str,
        read_only: bool,
    ) -> Result<String, CoreError> {
        check_writable(read_only)?;
        let dir = open_root(root)?;
        let src_rel = to_rel(root, source)?;
        let dst_rel = to_rel(root, destination)?;

        dir.rename(&src_rel, &dir, &dst_rel)
            .map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => CoreError::NotFound {
                    path: source.to_owned(),
                },
                io::ErrorKind::PermissionDenied => CoreError::PermissionDenied {
                    path: source.to_owned(),
                },
                _ => CoreError::Other(e.to_string()),
            })?;
        Ok(format!("moved {source} → {destination}"))
    }
}

// ── WASM fallback — validate_path + std::fs ────────────────────────────────
//
// WASM targets have no ambient authority (no openat-based confinement).
// We fall back to the original userspace validate_path approach.
// TOCTOU is a theoretical concern on WASM too, but the execution environment
// is already sandboxed by the WASM runtime.

#[cfg(target_family = "wasm")]
mod wasm_fallback {
    use super::*;
    use crate::security::validate_path;
    use std::{fs, io::Write, path::PathBuf};
    use tempfile::NamedTempFile;

    pub fn patch_apply(
        root: &Path,
        path: &str,
        patch_text: &str,
        read_only: bool,
    ) -> Result<String, CoreError> {
        check_writable(read_only)?;
        let resolved = validate_path(root, path)?;

        let raw = strip_diff_fence(patch_text);
        let patch = Patch::from_str(raw).map_err(|e| CoreError::PatchFailed {
            reason: e.to_string(),
        })?;

        let new_content = if patch.original().map_or(false, |s| s == "/dev/null") {
            apply("", &patch).map_err(|e| CoreError::PatchFailed {
                reason: format!("apply failed: {e:?}"),
            })?
        } else {
            let existing = fs::read_to_string(&resolved).map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => CoreError::NotFound {
                    path: path.to_owned(),
                },
                io::ErrorKind::PermissionDenied => CoreError::PermissionDenied {
                    path: path.to_owned(),
                },
                _ => CoreError::Other(e.to_string()),
            })?;
            apply(&existing, &patch).map_err(|e| CoreError::PatchFailed {
                reason: format!("hunk mismatch (file may have drifted): {e:?}"),
            })?
        };

        let parent = resolved.parent().unwrap_or(std::path::Path::new("."));
        let mut tmp = NamedTempFile::new_in(parent).map_err(|e| CoreError::Other(e.to_string()))?;
        tmp.write_all(new_content.as_bytes())
            .map_err(CoreError::from)?;
        tmp.persist(&resolved)
            .map_err(|e| CoreError::Other(e.error.to_string()))?;

        Ok(format!("patched: {}", resolved.display()))
    }

    pub fn append(
        root: &Path,
        path: &str,
        content: &str,
        read_only: bool,
    ) -> Result<String, CoreError> {
        check_writable(read_only)?;
        let resolved = validate_path(root, path)?;

        if !resolved.exists() {
            return Err(CoreError::NotFound {
                path: path.to_owned(),
            });
        }

        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&resolved)
            .map_err(CoreError::from)?;
        file.write_all(content.as_bytes())
            .map_err(CoreError::from)?;
        Ok(format!(
            "appended {} bytes to {}",
            content.len(),
            resolved.display()
        ))
    }

    pub fn write_file(
        root: &Path,
        path: &str,
        content: &str,
        read_only: bool,
    ) -> Result<String, CoreError> {
        check_writable(read_only)?;
        let resolved = validate_path(root, path)?;

        if let Some(parent) = resolved.parent() {
            fs::create_dir_all(parent).map_err(CoreError::from)?;
        }
        fs::write(&resolved, content.as_bytes()).map_err(CoreError::from)?;
        Ok(format!(
            "wrote {} bytes to {}",
            content.len(),
            resolved.display()
        ))
    }

    pub fn create_directory(root: &Path, path: &str, read_only: bool) -> Result<String, CoreError> {
        check_writable(read_only)?;
        let resolved = validate_path(root, path)?;
        fs::create_dir_all(&resolved).map_err(CoreError::from)?;
        Ok(format!("created directory: {}", resolved.display()))
    }

    pub fn move_file(
        root: &Path,
        source: &str,
        destination: &str,
        read_only: bool,
    ) -> Result<String, CoreError> {
        check_writable(read_only)?;
        let src = validate_path(root, source)?;
        let dst = validate_path(root, destination)?;
        fs::rename(&src, &dst).map_err(CoreError::from)?;
        Ok(format!("moved {} → {}", src.display(), dst.display()))
    }
}

// ── Public API dispatch ────────────────────────────────────────────────────

#[cfg(not(target_family = "wasm"))]
pub use native::{append, create_directory, move_file, patch_apply, write_file};

#[cfg(target_family = "wasm")]
pub use wasm_fallback::{append, create_directory, move_file, patch_apply, write_file};

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    // ── strip_diff_fence ───────────────────────

    #[test]
    fn fence_is_stripped() {
        let input = "```diff\n--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n```";
        let stripped = strip_diff_fence(input);
        assert!(!stripped.contains("```"));
        assert!(stripped.contains("--- a"));
    }

    #[test]
    fn unfenced_input_passes_through() {
        let input = "--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n";
        assert_eq!(strip_diff_fence(input), input);
    }

    // ── patch_apply ────────────────────────────

    #[test]
    fn patch_applies_correctly() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let file = root.join("a.txt");
        fs::write(&file, "hello\n").unwrap();

        let patch = "--- a.txt\n+++ a.txt\n@@ -1 +1 @@\n-hello\n+world\n";
        let res = patch_apply(&root, file.to_str().unwrap(), patch, false);
        assert!(res.is_ok(), "{res:?}");
        assert_eq!(fs::read_to_string(&file).unwrap(), "world\n");
    }

    #[test]
    fn fenced_patch_is_applied() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let file = root.join("b.txt");
        fs::write(&file, "hello\n").unwrap();

        let patch = "```diff\n--- b.txt\n+++ b.txt\n@@ -1 +1 @@\n-hello\n+world\n```";
        let res = patch_apply(&root, file.to_str().unwrap(), patch, false);
        assert!(res.is_ok(), "{res:?}");
        assert_eq!(fs::read_to_string(&file).unwrap(), "world\n");
    }

    #[test]
    fn dev_null_header_creates_file() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let file = root.join("new.txt");

        let patch = "--- /dev/null\n+++ new.txt\n@@ -0,0 +1,2 @@\n+line1\n+line2\n";
        let res = patch_apply(&root, file.to_str().unwrap(), patch, false);
        assert!(res.is_ok(), "{res:?}");
        assert_eq!(fs::read_to_string(&file).unwrap(), "line1\nline2\n");
    }

    #[test]
    fn drifted_patch_fails_and_file_unchanged() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let file = root.join("c.txt");
        fs::write(&file, "original\n").unwrap();

        let patch = "--- c.txt\n+++ c.txt\n@@ -1 +1 @@\n-wrong context\n+new\n";
        let res = patch_apply(&root, file.to_str().unwrap(), patch, false);
        assert!(res.is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "original\n");
    }

    // ── append ─────────────────────────────────

    #[test]
    fn append_to_existing_file() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let file = root.join("log.txt");
        fs::write(&file, "line1").unwrap();

        let res = append(&root, file.to_str().unwrap(), "\nline2", false);
        assert!(res.is_ok());
        assert_eq!(fs::read_to_string(&file).unwrap(), "line1\nline2");
    }

    #[test]
    fn append_to_missing_file_returns_not_found() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let file = root.join("missing.txt");

        let res = append(&root, file.to_str().unwrap(), "data", false);
        assert!(matches!(res, Err(CoreError::NotFound { .. })));
    }

    // ── write_file ─────────────────────────────

    #[test]
    fn write_creates_new_file() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let file = root.join("new.txt");

        let res = write_file(&root, file.to_str().unwrap(), "content", false);
        assert!(res.is_ok());
        assert_eq!(fs::read_to_string(&file).unwrap(), "content");
    }

    #[test]
    fn write_overwrites_existing_file() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let file = root.join("exist.txt");
        fs::write(&file, "old").unwrap();

        let res = write_file(&root, file.to_str().unwrap(), "new", false);
        assert!(res.is_ok());
        assert_eq!(fs::read_to_string(&file).unwrap(), "new");
    }

    // ── create_directory ───────────────────────

    #[test]
    fn create_directory_within_root() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let dir = root.join("reports");

        let res = create_directory(&root, dir.to_str().unwrap(), false);
        assert!(res.is_ok());
        assert!(dir.is_dir());
    }

    #[test]
    fn create_directory_outside_root_is_rejected() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        let res = create_directory(&root, "/tmp/evil_mcp_test_dir", false);
        assert!(matches!(res, Err(CoreError::OutsideRoot { .. })));
    }

    #[test]
    fn create_directory_deeply_nested_in_one_call() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let deep = root.join("a").join("b").join("c");

        let res = create_directory(&root, deep.to_str().unwrap(), false);
        assert!(res.is_ok(), "{res:?}");
        assert!(deep.is_dir());
    }

    // ── move_file ──────────────────────────────

    #[test]
    fn move_renames_within_root() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let src = root.join("old.txt");
        let dst = root.join("new.txt");
        fs::write(&src, "data").unwrap();

        let res = move_file(&root, src.to_str().unwrap(), dst.to_str().unwrap(), false);
        assert!(res.is_ok());
        assert!(!src.exists());
        assert!(dst.exists());
    }

    #[test]
    fn move_with_destination_outside_root_is_rejected() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let src = root.join("src.txt");
        fs::write(&src, "data").unwrap();

        let res = move_file(&root, src.to_str().unwrap(), "/tmp/evil_dst.txt", false);
        assert!(matches!(res, Err(CoreError::OutsideRoot { .. })));
        assert!(src.exists());
    }

    // ── read-only mode ─────────────────────────

    #[test]
    fn read_only_blocks_all_write_ops() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let path = root.join("f.txt");
        fs::write(&path, "x").unwrap();
        let p = path.to_str().unwrap();

        assert!(matches!(
            patch_apply(
                &root,
                p,
                "--- /dev/null\n+++ f.txt\n@@ -0,0 +1 @@\n+x\n",
                true
            ),
            Err(CoreError::ReadOnly)
        ));
        assert!(matches!(
            append(&root, p, "y", true),
            Err(CoreError::ReadOnly)
        ));
        assert!(matches!(
            write_file(&root, p, "z", true),
            Err(CoreError::ReadOnly)
        ));
        assert!(matches!(
            create_directory(&root, p, true),
            Err(CoreError::ReadOnly)
        ));
        assert!(matches!(
            move_file(&root, p, p, true),
            Err(CoreError::ReadOnly)
        ));
    }

    // ── symlink escape via cap-std (native only) ───────────────────────────

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn symlink_in_path_component_cannot_escape_root() {
        use std::os::unix::fs::symlink;

        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        // Create a symlink inside root pointing outside: root/escape -> /tmp
        let link = root.join("escape");
        symlink("/tmp", &link).unwrap();

        // Attempt to write through the symlink to /tmp/evil.txt
        let attempt = root.join("escape").join("evil.txt");
        let res = write_file(&root, attempt.to_str().unwrap(), "pwned", false);

        // cap-std must reject this — the symlink "escape" points outside root
        assert!(
            res.is_err(),
            "cap-std should have blocked symlink escape, but got: {res:?}"
        );
    }
}
