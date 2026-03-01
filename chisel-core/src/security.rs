use std::{
    io,
    path::{Path, PathBuf},
};

use crate::error::CoreError;

/// Resolve `input` to an absolute canonical path and assert it is within `root`.
///
/// Uses `std::fs::canonicalize` so symlinks are fully resolved before the
/// prefix check, blocking all traversal and symlink-escape attacks.
///
/// For paths that do not yet exist (e.g. new files or deep mkdir targets), the
/// function walks up the ancestor chain to find the deepest existing directory,
/// canonicalizes that, then rejoins the non-existent tail components. This
/// allows `create_directory` to validate deeply nested new paths in one call.
pub fn validate_path(root: &Path, input: &str) -> Result<PathBuf, CoreError> {
    let path = Path::new(input);

    let resolved = match std::fs::canonicalize(path) {
        Ok(p) => p,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // Walk up to the deepest existing ancestor, canonicalize it,
            // then rejoin the non-existent tail in order.
            let mut cursor: &Path = path;
            let mut tail: Vec<&std::ffi::OsStr> = Vec::new();

            loop {
                let parent = cursor.parent().unwrap_or(Path::new("."));
                if parent == cursor {
                    return Err(CoreError::NotFound { path: input.to_owned() });
                }
                tail.push(
                    cursor
                        .file_name()
                        .ok_or_else(|| CoreError::NotFound { path: input.to_owned() })?,
                );
                cursor = parent;
                match std::fs::canonicalize(cursor) {
                    Ok(canon) => {
                        break tail.iter().rev().fold(canon, |acc, c| acc.join(c));
                    }
                    Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                    Err(_) => return Err(CoreError::NotFound { path: input.to_owned() }),
                }
            }
        }
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            return Err(CoreError::PermissionDenied { path: input.to_owned() });
        }
        Err(_) => return Err(CoreError::NotFound { path: input.to_owned() }),
    };

    if !resolved.starts_with(root) {
        return Err(CoreError::OutsideRoot {
            path: resolved.to_string_lossy().into_owned(),
            root: root.to_string_lossy().into_owned(),
        });
    }

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    #[test]
    fn path_inside_root_is_ok() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let file = root.join("file.txt");
        fs::write(&file, "").unwrap();

        let result = validate_path(&root, file.to_str().unwrap());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), file);
    }

    #[test]
    fn absolute_path_outside_root_is_rejected() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        let err = validate_path(&root, "/etc/hosts").unwrap_err();
        assert!(matches!(err, CoreError::OutsideRoot { .. }));
    }

    #[test]
    fn traversal_attempt_is_rejected() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        let inner = root.join("inner");
        fs::create_dir_all(&inner).unwrap();
        fs::write("/tmp/traversal_target.txt", "").unwrap();
        let traversal = inner.join("../../traversal_target.txt").to_string_lossy().into_owned();

        let err = validate_path(&root, &traversal).unwrap_err();
        assert!(matches!(err, CoreError::OutsideRoot { .. }));
    }

    #[test]
    fn symlink_pointing_outside_root_is_rejected() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        let link = root.join("escape.txt");
        symlink("/etc/hosts", &link).unwrap();

        let err = validate_path(&root, link.to_str().unwrap()).unwrap_err();
        assert!(matches!(err, CoreError::OutsideRoot { .. }));
    }

    #[test]
    fn nonexistent_path_inside_root_is_ok() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let new_file = root.join("new_file.txt");

        let result = validate_path(&root, new_file.to_str().unwrap());
        assert!(result.is_ok());
    }

    #[test]
    fn deeply_nested_nonexistent_path_inside_root_is_ok() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        // Neither "a" nor "a/b/c" exist yet
        let deep = root.join("a").join("b").join("c");

        let result = validate_path(&root, deep.to_str().unwrap());
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(result.unwrap(), deep);
    }

    #[test]
    fn deeply_nested_path_outside_root_is_rejected() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        // Deep path that resolves outside root
        let outside = Path::new("/tmp/evil/deep/path");

        let err = validate_path(&root, outside.to_str().unwrap()).unwrap_err();
        assert!(matches!(err, CoreError::OutsideRoot { .. }));
    }
}
